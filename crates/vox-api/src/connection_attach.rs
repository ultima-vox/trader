//! #17 application adapter. Transport DTOs stop here; provider and secret types stay behind it.

use std::sync::Arc;

use async_trait::async_trait;
use vox_connections::{
    BrokerAccessLevel, BrokerAccount, BrokerAccountBinding, BrokerAccountStatus, BrokerAccountType,
    BrokerConnection, BrokerEnvironment as DomainEnvironment, BrokerProviderPort,
    ConnectionCapability, ConnectionHealthReason, ConnectionHealthState, ConnectionId,
    ConnectionRepository, ConnectionService, CreateConnectionRequest, CredentialClass,
    CredentialScope, CredentialStatus, ExecutionAuthorization, ExecutionAuthorizationMode,
    ProviderErrorKind, ProviderId, RepositoryError, SecretBytes, SecretStore, SecretStoreError,
    SecurityContext, ServiceError, UserId, VoxAccountId,
};

use crate::application::{ConnectionAdministration, ConnectionRequestContext};
use crate::contract::connections::{
    BindBrokerAccountRequest, BrokerAccountBindingDto, BrokerConnectionMetadataDto,
    ChangeExecutionAuthorizationRequest, ConnectionCapabilityDto, ConnectionDetailsDto,
    ConnectionHealthDto, ConnectionHealthReasonDto, ConnectionHealthStateDto,
    CreateBrokerConnectionRequest, CredentialClassDto, CredentialRotationResultDto,
    CredentialScopeDto, CredentialStatusDto, DiscoveredBrokerAccountDto, ExecutionAuthorizationDto,
    ExecutionAuthorizationModeDto, RotateCredentialRequest,
};
use crate::contract::scope::{BrokerEnvironment, ProviderDto};
use crate::error::{ApiError, ErrorCategory, FieldError};

pub struct ConnectionAdministrationAdapter<R, S, P> {
    service: Arc<ConnectionService<R, S, P>>,
}

impl<R, S, P> ConnectionAdministrationAdapter<R, S, P> {
    #[must_use]
    pub const fn new(service: Arc<ConnectionService<R, S, P>>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl<R, S, P> ConnectionAdministration for ConnectionAdministrationAdapter<R, S, P>
where
    R: ConnectionRepository + 'static,
    S: SecretStore + 'static,
    P: BrokerProviderPort + 'static,
{
    async fn list_connections(
        &self,
        context: &ConnectionRequestContext,
    ) -> Result<Vec<BrokerConnectionMetadataDto>, ApiError> {
        let security = security_context(context)?;
        self.service
            .list_connections(&security)
            .map(|items| items.into_iter().map(connection_dto).collect())
            .map_err(api_error)
    }

    async fn create_connection(
        &self,
        context: &ConnectionRequestContext,
        request: CreateBrokerConnectionRequest,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        let security = security_context(context)?;
        let credential = SecretBytes::new(request.credential.into_bytes()).map_err(api_error)?;
        self.service
            .create_connection(
                &security,
                CreateConnectionRequest {
                    provider: provider(request.provider),
                    environment: environment(request.environment),
                    label: request.display_label,
                },
                credential,
            )
            .await
            .map(connection_dto)
            .map_err(api_error)
    }

    async fn connection_details(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<ConnectionDetailsDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        let connection = self
            .service
            .list_connections(&security)
            .map_err(api_error)?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(not_found)?;
        let domain_accounts = self
            .service
            .discovered_accounts(&security, &id)
            .map_err(api_error)?;
        let mut execution_authorizations = Vec::with_capacity(domain_accounts.len());
        for account in &domain_accounts {
            execution_authorizations.push(authorization_dto(
                self.service
                    .execution_authorization(&security, &id, &account.provider_account_id)
                    .map_err(api_error)?,
            ));
        }
        let accounts = domain_accounts.into_iter().map(account_dto).collect();
        let bindings = self
            .service
            .account_bindings(&security, &id)
            .map_err(api_error)?
            .into_iter()
            .map(binding_dto)
            .collect();
        Ok(ConnectionDetailsDto {
            connection: connection_dto(connection),
            accounts,
            bindings,
            execution_authorizations,
        })
    }

    async fn revalidate_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .revalidate(&security, &id)
            .await
            .map(connection_dto)
            .map_err(api_error)
    }

    async fn rotate_credential(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: RotateCredentialRequest,
    ) -> Result<CredentialRotationResultDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        let credential = SecretBytes::new(request.credential.into_bytes()).map_err(api_error)?;
        self.service
            .rotate_credential(&security, &id, credential)
            .await
            .map(|result| CredentialRotationResultDto {
                connection: connection_dto(result.connection),
                reconnect_required: result.reconnect_required,
            })
            .map_err(api_error)
    }

    async fn disable_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .disable_connection(&security, &id)
            .map(connection_dto)
            .map_err(api_error)
    }

    async fn delete_connection(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<(), ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .delete_connection(&security, &id)
            .map_err(api_error)
    }

    async fn accounts(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<DiscoveredBrokerAccountDto>, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .discovered_accounts(&security, &id)
            .map(|items| items.into_iter().map(account_dto).collect())
            .map_err(api_error)
    }

    async fn bindings(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<BrokerAccountBindingDto>, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .account_bindings(&security, &id)
            .map(|items| items.into_iter().map(binding_dto).collect())
            .map_err(api_error)
    }

    async fn bind_account(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: BindBrokerAccountRequest,
    ) -> Result<BrokerAccountBindingDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        let account_id = VoxAccountId::parse(request.account_id).map_err(validation_error)?;
        self.service
            .bind_account(&security, &id, request.provider_account_id, account_id)
            .map(binding_dto)
            .map_err(api_error)
    }

    async fn unbind_account(
        &self,
        context: &ConnectionRequestContext,
        binding_id: &str,
    ) -> Result<(), ApiError> {
        let security = security_context(context)?;
        let binding_id = vox_connections::BindingId::parse(binding_id).map_err(validation_error)?;
        self.service
            .unbind_account(&security, &binding_id)
            .map_err(api_error)
    }

    async fn change_execution_authorization(
        &self,
        context: &ConnectionRequestContext,
        connection_id: &str,
        request: ChangeExecutionAuthorizationRequest,
    ) -> Result<ExecutionAuthorizationDto, ApiError> {
        let security = security_context(context)?;
        let id = parse_connection_id(connection_id)?;
        self.service
            .set_execution_authorization(
                &security,
                &id,
                &request.provider_account_id,
                authorization_mode(request.mode),
            )
            .map(authorization_dto)
            .map_err(api_error)
    }
}

fn security_context(context: &ConnectionRequestContext) -> Result<SecurityContext, ApiError> {
    let actor = UserId::parse(context.actor.user_id.clone()).map_err(|_| {
        ApiError::new(
            ErrorCategory::Authentication,
            "INVALID_AUTHENTICATED_ACTOR",
            "authenticated actor identity is invalid",
        )
    })?;
    SecurityContext::new(actor, context.correlation_id.clone(), context.now_unix_ms)
        .map_err(validation_error)
}

fn parse_connection_id(value: &str) -> Result<ConnectionId, ApiError> {
    ConnectionId::parse(value.to_owned()).map_err(validation_error)
}

fn provider(value: ProviderDto) -> ProviderId {
    match value {
        ProviderDto::TInvest => ProviderId::tinvest(),
    }
}

fn environment(value: BrokerEnvironment) -> DomainEnvironment {
    match value {
        BrokerEnvironment::Sandbox => DomainEnvironment::Sandbox,
        BrokerEnvironment::Production => DomainEnvironment::Production,
    }
}

fn provider_dto(value: &ProviderId) -> ProviderDto {
    debug_assert_eq!(value.as_str(), ProviderId::T_INVEST);
    ProviderDto::TInvest
}

fn environment_dto(value: DomainEnvironment) -> BrokerEnvironment {
    match value {
        DomainEnvironment::Sandbox => BrokerEnvironment::Sandbox,
        DomainEnvironment::Production => BrokerEnvironment::Production,
    }
}

fn connection_dto(value: BrokerConnection) -> BrokerConnectionMetadataDto {
    BrokerConnectionMetadataDto {
        connection_id: value.id.as_str().to_owned(),
        provider: provider_dto(&value.provider),
        environment: environment_dto(value.environment),
        display_label: value.label,
        enabled: value.enabled,
        credential_status: credential_status(value.credential_status),
        credential_class: credential_class(value.credential_class),
        credential_scope: credential_scope(value.credential_scope),
        capabilities: value.capabilities.into_iter().map(capability).collect(),
        health: ConnectionHealthDto {
            state: health_state(value.health.state),
            checked_at_unix_ms: value.health.checked_at_unix_ms,
            reason_code: health_reason(value.health.reason_code),
            safe_detail: value.health.safe_detail,
            retryable: value.health.retryable,
        },
        created_at_unix_ms: value.created_at_unix_ms,
        updated_at_unix_ms: value.updated_at_unix_ms,
    }
}

fn account_dto(value: BrokerAccount) -> DiscoveredBrokerAccountDto {
    DiscoveredBrokerAccountDto {
        connection_id: value.connection_id.as_str().to_owned(),
        provider: provider_dto(&value.provider),
        environment: environment_dto(value.environment),
        provider_account_id: value.provider_account_id,
        display_name: value.display_name,
        account_type: account_type(value.account_type),
        account_status: account_status(value.status),
        access_level: access_level(value.access_level),
        opened_at_unix_ms: value.opened_at_unix_ms,
        closed_at_unix_ms: value.closed_at_unix_ms,
        accessible: value.accessible,
        capabilities: value.capabilities.into_iter().map(capability).collect(),
        discovered_at_unix_ms: value.discovered_at_unix_ms,
    }
}

fn binding_dto(value: BrokerAccountBinding) -> BrokerAccountBindingDto {
    BrokerAccountBindingDto {
        binding_id: value.id.as_str().to_owned(),
        connection_id: value.connection_id.as_str().to_owned(),
        provider: provider_dto(&value.provider),
        environment: environment_dto(value.environment),
        provider_account_id: value.provider_account_id,
        account_id: value.vox_account_id.as_str().to_owned(),
        enabled: value.enabled,
        created_at_unix_ms: value.created_at_unix_ms,
        updated_at_unix_ms: value.updated_at_unix_ms,
    }
}

fn authorization_dto(value: ExecutionAuthorization) -> ExecutionAuthorizationDto {
    ExecutionAuthorizationDto {
        connection_id: value.connection_id.as_str().to_owned(),
        provider_account_id: value.provider_account_id,
        mode: authorization_mode_dto(value.mode),
        authorization_revision: value.authorization_revision,
        changed_by: value.changed_by.as_str().to_owned(),
        changed_at_unix_ms: value.changed_at_unix_ms,
    }
}

fn credential_status(value: CredentialStatus) -> CredentialStatusDto {
    match value {
        CredentialStatus::PendingValidation => CredentialStatusDto::PendingValidation,
        CredentialStatus::Valid => CredentialStatusDto::Valid,
        CredentialStatus::Invalid => CredentialStatusDto::Invalid,
        CredentialStatus::ExpiredOrInactive => CredentialStatusDto::ExpiredOrInactive,
        CredentialStatus::PendingDisable => CredentialStatusDto::PendingDisable,
        CredentialStatus::Disabled => CredentialStatusDto::Disabled,
        CredentialStatus::PendingDelete => CredentialStatusDto::PendingDelete,
    }
}

fn credential_class(value: CredentialClass) -> CredentialClassDto {
    match value {
        CredentialClass::Unknown => CredentialClassDto::Unknown,
        CredentialClass::ReadOnly => CredentialClassDto::ReadOnly,
        CredentialClass::FullAccess => CredentialClassDto::FullAccess,
        CredentialClass::TransferAccess => CredentialClassDto::TransferAccess,
        CredentialClass::Sandbox => CredentialClassDto::Sandbox,
    }
}

fn credential_scope(value: CredentialScope) -> CredentialScopeDto {
    match value {
        CredentialScope::NotConfirmed => CredentialScopeDto::NotConfirmed,
        CredentialScope::SingleAccountRestricted => CredentialScopeDto::SingleAccountRestricted,
        CredentialScope::AllAccessibleAccounts => CredentialScopeDto::AllAccessibleAccounts,
    }
}

fn capability(value: ConnectionCapability) -> ConnectionCapabilityDto {
    match value {
        ConnectionCapability::AccountDiscovery => ConnectionCapabilityDto::AccountDiscovery,
        ConnectionCapability::PortfolioRead => ConnectionCapabilityDto::PortfolioRead,
        ConnectionCapability::PositionsRead => ConnectionCapabilityDto::PositionsRead,
        ConnectionCapability::OperationsRead => ConnectionCapabilityDto::OperationsRead,
        ConnectionCapability::StreamHealth => ConnectionCapabilityDto::StreamHealth,
        ConnectionCapability::SandboxOrders => ConnectionCapabilityDto::SandboxOrders,
        ConnectionCapability::ProductionOrdersProviderAllowed => {
            ConnectionCapabilityDto::ProductionOrdersProviderAllowed
        }
    }
}

fn health_state(value: ConnectionHealthState) -> ConnectionHealthStateDto {
    match value {
        ConnectionHealthState::Unknown => ConnectionHealthStateDto::Unknown,
        ConnectionHealthState::Validating => ConnectionHealthStateDto::Validating,
        ConnectionHealthState::Healthy => ConnectionHealthStateDto::Healthy,
        ConnectionHealthState::InvalidCredential => ConnectionHealthStateDto::InvalidCredential,
        ConnectionHealthState::InsufficientPermission => {
            ConnectionHealthStateDto::InsufficientPermission
        }
        ConnectionHealthState::ProviderUnavailable => ConnectionHealthStateDto::ProviderUnavailable,
        ConnectionHealthState::AccountAccessChanged => {
            ConnectionHealthStateDto::AccountAccessChanged
        }
        ConnectionHealthState::Disabled => ConnectionHealthStateDto::Disabled,
    }
}

fn health_reason(value: ConnectionHealthReason) -> ConnectionHealthReasonDto {
    match value {
        ConnectionHealthReason::None => ConnectionHealthReasonDto::None,
        ConnectionHealthReason::InvalidCredential => ConnectionHealthReasonDto::InvalidCredential,
        ConnectionHealthReason::ExpiredOrInactive => ConnectionHealthReasonDto::ExpiredOrInactive,
        ConnectionHealthReason::PermissionDenied => ConnectionHealthReasonDto::PermissionDenied,
        ConnectionHealthReason::WrongEnvironment => ConnectionHealthReasonDto::WrongEnvironment,
        ConnectionHealthReason::ProviderUnavailable => {
            ConnectionHealthReasonDto::ProviderUnavailable
        }
        ConnectionHealthReason::AccountAccessChanged => {
            ConnectionHealthReasonDto::AccountAccessChanged
        }
        ConnectionHealthReason::DisabledByOperator => ConnectionHealthReasonDto::DisabledByOperator,
    }
}

fn authorization_mode(value: ExecutionAuthorizationModeDto) -> ExecutionAuthorizationMode {
    match value {
        ExecutionAuthorizationModeDto::Disabled => ExecutionAuthorizationMode::Disabled,
        ExecutionAuthorizationModeDto::ManualAllowed => ExecutionAuthorizationMode::ManualAllowed,
        ExecutionAuthorizationModeDto::AutomatedAllowed => {
            ExecutionAuthorizationMode::AutomatedAllowed
        }
    }
}

fn authorization_mode_dto(value: ExecutionAuthorizationMode) -> ExecutionAuthorizationModeDto {
    match value {
        ExecutionAuthorizationMode::Disabled => ExecutionAuthorizationModeDto::Disabled,
        ExecutionAuthorizationMode::ManualAllowed => ExecutionAuthorizationModeDto::ManualAllowed,
        ExecutionAuthorizationMode::AutomatedAllowed => {
            ExecutionAuthorizationModeDto::AutomatedAllowed
        }
    }
}

fn account_type(value: BrokerAccountType) -> String {
    match value {
        BrokerAccountType::Unspecified => "UNSPECIFIED".to_owned(),
        BrokerAccountType::Brokerage => "BROKERAGE".to_owned(),
        BrokerAccountType::IndividualInvestment => "INDIVIDUAL_INVESTMENT".to_owned(),
        BrokerAccountType::InvestBox => "INVEST_BOX".to_owned(),
        BrokerAccountType::MoneyMarketFund => "MONEY_MARKET_FUND".to_owned(),
        BrokerAccountType::Debit => "DEBIT".to_owned(),
        BrokerAccountType::Savings => "SAVINGS".to_owned(),
        BrokerAccountType::DigitalFinancialAssets => "DIGITAL_FINANCIAL_ASSETS".to_owned(),
        BrokerAccountType::UnknownProviderValue(raw) => format!("UNKNOWN_PROVIDER_VALUE_{raw}"),
    }
}

fn account_status(value: BrokerAccountStatus) -> String {
    match value {
        BrokerAccountStatus::Unspecified => "UNSPECIFIED".to_owned(),
        BrokerAccountStatus::New => "NEW".to_owned(),
        BrokerAccountStatus::Open => "OPEN".to_owned(),
        BrokerAccountStatus::Closed => "CLOSED".to_owned(),
        BrokerAccountStatus::UnknownProviderValue(raw) => format!("UNKNOWN_PROVIDER_VALUE_{raw}"),
    }
}

fn access_level(value: BrokerAccessLevel) -> String {
    match value {
        BrokerAccessLevel::Unspecified => "UNSPECIFIED".to_owned(),
        BrokerAccessLevel::Full => "FULL".to_owned(),
        BrokerAccessLevel::ReadOnly => "READ_ONLY".to_owned(),
        BrokerAccessLevel::NoAccess => "NO_ACCESS".to_owned(),
        BrokerAccessLevel::UnknownProviderValue(raw) => format!("UNKNOWN_PROVIDER_VALUE_{raw}"),
    }
}

fn api_error(error: impl Into<ServiceError>) -> ApiError {
    let error = error.into();
    let (category, code, message) = match &error {
        ServiceError::PermissionDenied(_) => (
            ErrorCategory::Permission,
            "CONNECTION_PERMISSION_DENIED",
            "actor is not permitted to perform this connection operation",
        ),
        ServiceError::ConnectionNotFound
        | ServiceError::AccountNotFound
        | ServiceError::Repository(RepositoryError::NotFound) => (
            ErrorCategory::NotFound,
            "CONNECTION_RESOURCE_NOT_FOUND",
            "connection resource was not found",
        ),
        ServiceError::Provider(provider) => match provider.kind() {
            ProviderErrorKind::InvalidCredential | ProviderErrorKind::ExpiredOrInactive => (
                ErrorCategory::Authentication,
                "BROKER_CREDENTIAL_REJECTED",
                "broker credential was rejected",
            ),
            ProviderErrorKind::InsufficientPermission => (
                ErrorCategory::Permission,
                "BROKER_PERMISSION_DENIED",
                "broker denied requested credential capability",
            ),
            ProviderErrorKind::WrongEnvironment => (
                ErrorCategory::Validation,
                "BROKER_ENVIRONMENT_MISMATCH",
                "credential does not match selected broker environment",
            ),
            ProviderErrorKind::ProviderUnavailable => (
                ErrorCategory::Transient,
                "BROKER_UNAVAILABLE",
                "broker validation service is unavailable",
            ),
            ProviderErrorKind::AccountAccessChanged => (
                ErrorCategory::Conflict,
                "BROKER_ACCOUNT_ACCESS_CHANGED",
                "broker account access changed",
            ),
            ProviderErrorKind::UnsupportedProvider => (
                ErrorCategory::Validation,
                "BROKER_PROVIDER_UNSUPPORTED",
                "broker provider is unsupported",
            ),
        },
        ServiceError::ExecutionUnauthorized
        | ServiceError::ProviderDoesNotAllowProductionOrders => (
            ErrorCategory::Permission,
            "EXECUTION_NOT_AUTHORIZED",
            "execution is not authorized for this exact connection and account",
        ),
        ServiceError::AccountUnavailable
        | ServiceError::ConnectionDisabled
        | ServiceError::ConnectionUnavailable
        | ServiceError::DisableBeforeDelete
        | ServiceError::AccountBindingMismatch
        | ServiceError::StaleExecutionAuthorization
        | ServiceError::Repository(RepositoryError::StaleAuthorizationRevision) => (
            ErrorCategory::Conflict,
            "CONNECTION_STATE_CONFLICT",
            "connection state does not permit this operation",
        ),
        ServiceError::Model(_) | ServiceError::DuplicateProviderAccount => (
            ErrorCategory::Validation,
            "CONNECTION_VALIDATION_FAILED",
            "connection request is invalid",
        ),
        ServiceError::SecretStore(
            SecretStoreError::EmptySecret | SecretStoreError::SecretTooLarge,
        ) => (
            ErrorCategory::Validation,
            "INVALID_CREDENTIAL_INPUT",
            "credential input is empty or exceeds the accepted size",
        ),
        ServiceError::CredentialRotationCompensationFailed
        | ServiceError::AuthorizationRevisionOverflow
        | ServiceError::Repository(_)
        | ServiceError::SecretStore(_) => (
            ErrorCategory::Internal,
            "CONNECTION_OPERATION_FAILED",
            "connection operation failed closed",
        ),
    };
    ApiError::new(category, code, message)
}

fn validation_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::validation(
        "connection request is invalid",
        vec![FieldError {
            field: "identity".to_owned(),
            message: error.to_string(),
        }],
    )
}

fn not_found() -> ApiError {
    ApiError::new(
        ErrorCategory::NotFound,
        "CONNECTION_NOT_FOUND",
        "connection was not found",
    )
}
