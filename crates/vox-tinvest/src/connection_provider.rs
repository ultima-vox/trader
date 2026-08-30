use std::collections::BTreeSet;

use async_trait::async_trait;
use tonic::Code;
use vox_connections::{
    BrokerAccessLevel, BrokerAccountStatus, BrokerAccountType, BrokerEnvironment,
    BrokerProviderPort, ConnectionCapability, CredentialClass, CredentialScope,
    ProviderAccountFact, ProviderDiscovery, ProviderError, ProviderErrorKind, ProviderId,
    SecretBytes,
};

use crate::account::{AccountReadClient, AccountReadError, CanonicalAccount};
use crate::generated::v1::{AccessLevel, AccountStatus};
use crate::{GrpcConfigError, GrpcCredential, GrpcErrorKind, SecretToken, TInvestGrpcClient};

#[derive(Clone, Copy, Debug, Default)]
pub struct TInvestConnectionProvider;

#[async_trait]
impl BrokerProviderPort for TInvestConnectionProvider {
    async fn validate_and_discover(
        &self,
        provider: &ProviderId,
        environment: BrokerEnvironment,
        credential: &SecretBytes,
    ) -> Result<ProviderDiscovery, ProviderError> {
        if provider.as_str() != ProviderId::T_INVEST {
            return Err(provider_error(
                ProviderErrorKind::UnsupportedProvider,
                "provider is not supported by T-Invest adapter",
            ));
        }
        let token_text = std::str::from_utf8(credential.expose_secret()).map_err(|_| {
            provider_error(
                ProviderErrorKind::InvalidCredential,
                "credential is not valid UTF-8",
            )
        })?;
        let token = SecretToken::new(token_text.to_owned()).map_err(|_| {
            provider_error(
                ProviderErrorKind::InvalidCredential,
                "credential is not a valid bearer token",
            )
        })?;
        let (grpc, sandbox) = match environment {
            BrokerEnvironment::Production => (
                TInvestGrpcClient::production(GrpcCredential::Production(token)),
                false,
            ),
            BrokerEnvironment::Sandbox => (
                TInvestGrpcClient::sandbox(GrpcCredential::Sandbox(token)),
                true,
            ),
        };
        let client = AccountReadClient::new(grpc.map_err(map_config_error)?);
        let catalogue = if sandbox {
            client.sandbox_accounts().await
        } else {
            client.accounts(None).await
        }
        .map_err(map_account_error)?;

        let accounts = catalogue
            .accounts
            .into_iter()
            .map(|account| provider_account(account, sandbox))
            .collect::<Vec<_>>();
        let mut connection_capabilities = BTreeSet::from([
            ConnectionCapability::AccountDiscovery,
            ConnectionCapability::PortfolioRead,
            ConnectionCapability::PositionsRead,
            ConnectionCapability::OperationsRead,
            ConnectionCapability::StreamHealth,
        ]);
        if sandbox && accounts.iter().any(|account| account.accessible) {
            connection_capabilities.insert(ConnectionCapability::SandboxOrders);
        }
        if !sandbox
            && accounts.iter().any(|account| {
                account
                    .capabilities
                    .contains(&ConnectionCapability::ProductionOrdersProviderAllowed)
            })
        {
            connection_capabilities.insert(ConnectionCapability::ProductionOrdersProviderAllowed);
        }
        // GetAccounts access_level is an account fact, not authoritative token-class
        // introspection. Only sandbox class is proven by explicit contour onboarding.
        let credential_class = credential_class(sandbox);
        Ok(ProviderDiscovery {
            credential_class,
            // GetAccounts does not prove whether one returned account means a restricted token.
            credential_scope: CredentialScope::NotConfirmed,
            connection_capabilities,
            accounts,
        })
    }
}

fn credential_class(sandbox: bool) -> CredentialClass {
    if sandbox {
        CredentialClass::Sandbox
    } else {
        CredentialClass::Unknown
    }
}

fn provider_account(account: CanonicalAccount, sandbox: bool) -> ProviderAccountFact {
    let accessible = account.status == AccountStatus::Open as i32
        && matches!(
            account.access_level,
            value if value == AccessLevel::AccountAccessLevelFullAccess as i32
                || value == AccessLevel::AccountAccessLevelReadOnly as i32
        );
    let mut capabilities = if accessible {
        BTreeSet::from([
            ConnectionCapability::PortfolioRead,
            ConnectionCapability::PositionsRead,
            ConnectionCapability::OperationsRead,
            ConnectionCapability::StreamHealth,
        ])
    } else {
        BTreeSet::new()
    };
    if accessible && sandbox {
        capabilities.insert(ConnectionCapability::SandboxOrders);
    }
    if accessible
        && !sandbox
        && account.access_level == AccessLevel::AccountAccessLevelFullAccess as i32
    {
        capabilities.insert(ConnectionCapability::ProductionOrdersProviderAllowed);
    }
    ProviderAccountFact {
        provider_account_id: account.account_id,
        display_name: account.name,
        account_type: account_type(account.account_type),
        status: account_status(account.status),
        access_level: access_level(account.access_level),
        opened_at_unix_ms: account.opened_at.and_then(timestamp_millis),
        closed_at_unix_ms: account.closed_at.and_then(timestamp_millis),
        accessible,
        capabilities,
    }
}

fn timestamp_millis(value: crate::account::ProviderTimestamp) -> Option<i64> {
    value
        .seconds
        .checked_mul(1_000)
        .and_then(|seconds| seconds.checked_add(i64::from(value.nanos / 1_000_000)))
}

fn account_type(value: i32) -> BrokerAccountType {
    match value {
        0 => BrokerAccountType::Unspecified,
        1 => BrokerAccountType::Brokerage,
        2 => BrokerAccountType::IndividualInvestment,
        3 => BrokerAccountType::InvestBox,
        4 => BrokerAccountType::MoneyMarketFund,
        5 => BrokerAccountType::Debit,
        6 => BrokerAccountType::Savings,
        7 => BrokerAccountType::DigitalFinancialAssets,
        raw => BrokerAccountType::UnknownProviderValue(raw),
    }
}

fn account_status(value: i32) -> BrokerAccountStatus {
    match value {
        0 => BrokerAccountStatus::Unspecified,
        1 => BrokerAccountStatus::New,
        2 => BrokerAccountStatus::Open,
        3 => BrokerAccountStatus::Closed,
        raw => BrokerAccountStatus::UnknownProviderValue(raw),
    }
}

fn access_level(value: i32) -> BrokerAccessLevel {
    match value {
        0 => BrokerAccessLevel::Unspecified,
        1 => BrokerAccessLevel::Full,
        2 => BrokerAccessLevel::ReadOnly,
        3 => BrokerAccessLevel::NoAccess,
        raw => BrokerAccessLevel::UnknownProviderValue(raw),
    }
}

fn map_account_error(error: AccountReadError) -> ProviderError {
    match error {
        AccountReadError::Provider(grpc) => match grpc.kind {
            GrpcErrorKind::Provider(provider) => match provider.code {
                Code::Unauthenticated => provider_error(
                    ProviderErrorKind::InvalidCredential,
                    "T-Invest rejected credential",
                ),
                Code::PermissionDenied => provider_error(
                    ProviderErrorKind::InsufficientPermission,
                    "T-Invest credential lacks account-discovery permission",
                ),
                Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted => {
                    provider_error(
                        ProviderErrorKind::ProviderUnavailable,
                        "T-Invest account discovery unavailable",
                    )
                }
                _ => provider_error(
                    ProviderErrorKind::ProviderUnavailable,
                    "T-Invest account discovery failed",
                ),
            },
            _ => provider_error(
                ProviderErrorKind::ProviderUnavailable,
                "T-Invest account discovery transport failed",
            ),
        },
        AccountReadError::Canonical(_)
        | AccountReadError::MissingResponseAccount { .. }
        | AccountReadError::ResponseAccountMismatch { .. } => provider_error(
            ProviderErrorKind::ProviderUnavailable,
            "T-Invest returned invalid account discovery data",
        ),
    }
}

fn map_config_error(_error: GrpcConfigError) -> ProviderError {
    provider_error(
        ProviderErrorKind::WrongEnvironment,
        "T-Invest credential environment routing failed",
    )
}

fn provider_error(kind: ProviderErrorKind, safe_message: &str) -> ProviderError {
    ProviderError::new(kind, safe_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_access_does_not_create_vox_execution_authorization() {
        let fact = provider_account(
            CanonicalAccount {
                account_id: "account".to_owned(),
                account_type: 1,
                name: Some("Main".to_owned()),
                status: AccountStatus::Open as i32,
                opened_at: None,
                closed_at: None,
                access_level: AccessLevel::AccountAccessLevelFullAccess as i32,
            },
            false,
        );
        assert!(
            fact.capabilities
                .contains(&ConnectionCapability::ProductionOrdersProviderAllowed)
        );
        assert_eq!(credential_class(false), CredentialClass::Unknown);
        // Provider scope is a fact only. Vox ExecutionAuthorization lives in connection service
        // and is always inserted disabled during onboarding.
    }

    #[test]
    fn unknown_provider_account_values_are_preserved() {
        let fact = provider_account(
            CanonicalAccount {
                account_id: "account".to_owned(),
                account_type: 91,
                name: None,
                status: 92,
                opened_at: None,
                closed_at: None,
                access_level: 93,
            },
            false,
        );
        assert_eq!(
            fact.account_type,
            BrokerAccountType::UnknownProviderValue(91)
        );
        assert_eq!(fact.status, BrokerAccountStatus::UnknownProviderValue(92));
        assert_eq!(
            fact.access_level,
            BrokerAccessLevel::UnknownProviderValue(93)
        );
        assert!(!fact.accessible);
        assert!(fact.capabilities.is_empty());
    }
}
