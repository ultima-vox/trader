//! Deterministic acceptance policy for issue #9 account read-side qualification.

use std::collections::BTreeMap;

use thiserror::Error;
use tonic::Code;

use crate::account::CanonicalAccount;
use crate::{GrpcError, GrpcErrorKind, GrpcProviderError};

pub const ACCOUNT_INVENTORY_METHODS: [&str; 38] = [
    "GetAccounts",
    "GetMarginAttributes",
    "GetUserTariff",
    "GetInfo",
    "GetBankAccounts",
    "CurrencyTransfer",
    "PayIn",
    "GetAccountValues",
    "GetOperations",
    "GetPortfolio",
    "GetPositions",
    "GetWithdrawLimits",
    "GetBrokerReport",
    "GetDividendsForeignIssuer",
    "GetOperationsByCursor",
    "PortfolioStream",
    "PositionsStream",
    "OperationsStream",
    "OpenSandboxAccount",
    "GetSandboxAccounts",
    "CloseSandboxAccount",
    "PostSandboxOrder",
    "PostSandboxOrderAsync",
    "ReplaceSandboxOrder",
    "GetSandboxOrders",
    "CancelSandboxOrder",
    "GetSandboxOrderState",
    "GetSandboxOrderPrice",
    "GetSandboxPositions",
    "GetSandboxOperations",
    "GetSandboxOperationsByCursor",
    "GetSandboxPortfolio",
    "SandboxPayIn",
    "GetSandboxWithdrawLimits",
    "GetSandboxMaxLots",
    "PostSandboxStopOrder",
    "GetSandboxStopOrders",
    "CancelSandboxStopOrder",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QualificationMode {
    SandboxOnly,
    ProductionReadOnly,
}

impl QualificationMode {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::SandboxOnly => "SANDBOX_ONLY",
            Self::ProductionReadOnly => "PRODUCTION_READ_ONLY",
        }
    }
}

pub fn select_qualification_mode(
    explicit: Option<&str>,
    production_token: bool,
    sandbox_token: bool,
) -> Result<QualificationMode, QualificationModeError> {
    match explicit {
        Some("SANDBOX_ONLY") if sandbox_token => Ok(QualificationMode::SandboxOnly),
        Some("SANDBOX_ONLY") => Err(QualificationModeError::MissingSandboxToken),
        Some("PRODUCTION_READ_ONLY") if production_token => {
            Ok(QualificationMode::ProductionReadOnly)
        }
        Some("PRODUCTION_READ_ONLY") => Err(QualificationModeError::MissingProductionToken),
        Some(value) => Err(QualificationModeError::UnknownMode(value.to_owned())),
        None if production_token => Ok(QualificationMode::ProductionReadOnly),
        None if sandbox_token => Ok(QualificationMode::SandboxOnly),
        None => Err(QualificationModeError::NoCredential),
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum QualificationModeError {
    #[error("SANDBOX_ONLY requires TINVEST_SANDBOX_TOKEN")]
    MissingSandboxToken,
    #[error("PRODUCTION_READ_ONLY requires TINVEST_TOKEN")]
    MissingProductionToken,
    #[error("unknown TINVEST_QUALIFICATION_ENV value: {0}")]
    UnknownMode(String),
    #[error("configure TINVEST_SANDBOX_TOKEN or TINVEST_TOKEN")]
    NoCredential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MethodEnvironment {
    ProductionAndSandbox,
    ProductionOnly,
    SandboxOnly,
}

impl MethodEnvironment {
    #[must_use]
    pub const fn matrix_name(self) -> &'static str {
        match self {
            Self::ProductionAndSandbox => "PROD_AND_SANDBOX",
            Self::ProductionOnly => "PROD",
            Self::SandboxOnly => "SANDBOX",
        }
    }
}

#[must_use]
pub fn method_environment(method: &str) -> Option<MethodEnvironment> {
    if !ACCOUNT_INVENTORY_METHODS.contains(&method) {
        return None;
    }
    if matches!(method, "GetBrokerReport" | "GetDividendsForeignIssuer") {
        Some(MethodEnvironment::ProductionOnly)
    } else if method.contains("Sandbox") {
        Some(MethodEnvironment::SandboxOnly)
    } else {
        Some(MethodEnvironment::ProductionAndSandbox)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountPurpose {
    GeneralRead,
    Margin,
    Report,
    OperationsStream,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSelection<'a> {
    pub selected: Vec<&'a CanonicalAccount>,
    pub rejected: Vec<(&'a str, &'static str)>,
}

/// Selects only active, readable investment accounts. Margin and report methods are narrower:
/// ordinary brokerage accounts and IIS only. Bank/savings, closed/new, unknown and no-access
/// accounts never enter downstream requests.
#[must_use]
pub fn select_accounts(
    accounts: &[CanonicalAccount],
    _purpose: AccountPurpose,
) -> AccountSelection<'_> {
    let mut selected = Vec::new();
    let mut rejected = Vec::new();
    for account in accounts {
        let reason = if account.status != 2 {
            Some("status is not ACCOUNT_STATUS_OPEN")
        } else if !matches!(account.access_level, 1 | 2) {
            Some("access is neither FULL_ACCESS nor READ_ONLY")
        } else if !matches!(account.account_type, 1 | 2) {
            Some("qualification requires brokerage or IIS; special account excluded")
        } else {
            None
        };
        if let Some(reason) = reason {
            rejected.push((account.account_id.as_str(), reason));
        } else {
            selected.push(account);
        }
    }
    selected.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    AccountSelection { selected, rejected }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightFailure {
    CredentialInvalidOrInactive,
    InsufficientPermission,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxAvailability {
    TokenNotConfigured,
    AuthenticatedNoEligibleAccount,
    Ready,
}

#[must_use]
pub const fn sandbox_availability(
    token_configured: bool,
    eligible_accounts: usize,
) -> SandboxAvailability {
    if !token_configured {
        SandboxAvailability::TokenNotConfigured
    } else if eligible_accounts == 0 {
        SandboxAvailability::AuthenticatedNoEligibleAccount
    } else {
        SandboxAvailability::Ready
    }
}

#[must_use]
pub fn classify_preflight(error: &GrpcError) -> PreflightFailure {
    let GrpcErrorKind::Provider(provider) = &error.kind else {
        return PreflightFailure::Other;
    };
    if provider.code == Code::Unauthenticated && provider.has_provider_code("40003") {
        PreflightFailure::CredentialInvalidOrInactive
    } else if provider.code == Code::PermissionDenied && provider.has_provider_code("40002") {
        PreflightFailure::InsufficientPermission
    } else {
        PreflightFailure::Other
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityGate {
    InsufficientPermission,
    DeprecatedUnavailable,
}

/// Method-specific provider gate. Unknown methods/codes and arbitrary INVALID_ARGUMENT never gate.
#[must_use]
pub fn classify_method_gate(method: &str, provider: &GrpcProviderError) -> Option<CapabilityGate> {
    if account_read_method(method)
        && provider.code == Code::PermissionDenied
        && provider.has_provider_code("40002")
    {
        return Some(CapabilityGate::InsufficientPermission);
    }
    if matches!(method, "GetOperations" | "GetSandboxOperations")
        && provider.code == Code::Unavailable
        && provider.has_provider_code("12002")
    {
        return Some(CapabilityGate::DeprecatedUnavailable);
    }
    None
}

fn account_read_method(method: &str) -> bool {
    matches!(
        method,
        "GetMarginAttributes"
            | "GetUserTariff"
            | "GetInfo"
            | "GetBankAccounts"
            | "GetAccountValues"
            | "GetOperations"
            | "GetPortfolio"
            | "GetPositions"
            | "GetWithdrawLimits"
            | "GetBrokerReport"
            | "GetDividendsForeignIssuer"
            | "GetOperationsByCursor"
            | "PortfolioStream"
            | "PositionsStream"
            | "OperationsStream"
            | "GetSandboxPortfolio"
            | "GetSandboxPositions"
            | "GetSandboxWithdrawLimits"
            | "GetSandboxOperations"
            | "GetSandboxOperationsByCursor"
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Evidence {
    Qualified(String),
    GatedUnavailable(String),
}

#[derive(Debug, Default)]
pub struct QualificationLedger {
    rows: BTreeMap<&'static str, Evidence>,
}

impl QualificationLedger {
    pub fn record(&mut self, method: &'static str, evidence: Evidence) -> Result<(), LedgerError> {
        if !ACCOUNT_INVENTORY_METHODS.contains(&method) {
            return Err(LedgerError::UnknownMethod(method.to_owned()));
        }
        if self.rows.insert(method, evidence).is_some() {
            return Err(LedgerError::DuplicateMethod(method));
        }
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<(&'static str, Evidence)>, LedgerError> {
        let missing = ACCOUNT_INVENTORY_METHODS
            .iter()
            .copied()
            .filter(|method| !self.rows.contains_key(method))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(LedgerError::MissingMethods(missing));
        }
        Ok(ACCOUNT_INVENTORY_METHODS
            .into_iter()
            .map(|method| {
                let evidence = self.rows.get(method).cloned().unwrap_or_else(|| {
                    unreachable!("missing methods checked before ordered ledger output")
                });
                (method, evidence)
            })
            .collect())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum LedgerError {
    #[error("unknown qualification inventory method: {0}")]
    UnknownMethod(String),
    #[error("duplicate qualification result for {0}")]
    DuplicateMethod(&'static str),
    #[error("qualification results missing for: {0:?}")]
    MissingMethods(Vec<&'static str>),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GrpcRequestMetadata;
    use crate::account::CanonicalAccount;
    use uuid::Uuid;

    fn account(id: &str, account_type: i32, status: i32, access_level: i32) -> CanonicalAccount {
        CanonicalAccount {
            account_id: id.to_owned(),
            account_type,
            name: None,
            status,
            opened_at: None,
            closed_at: None,
            access_level,
        }
    }

    fn provider(code: Code, provider_code: &str) -> GrpcProviderError {
        GrpcProviderError {
            code,
            message: format!("provider code {provider_code}"),
            details: Vec::new(),
            tracking_id: None,
        }
    }

    fn grpc_error(code: Code, provider_code: &str) -> GrpcError {
        GrpcError {
            metadata: GrpcRequestMetadata {
                request_id: Uuid::nil(),
                method: "GetAccounts",
                attempt: 1,
                mutation: false,
            },
            kind: GrpcErrorKind::Provider(provider(code, provider_code)),
        }
    }

    #[test]
    fn mixed_catalogue_selects_only_method_eligible_accounts() {
        let accounts = vec![
            account("open-iis", 2, 2, 2),
            account("closed", 1, 3, 1),
            account("no-access", 1, 2, 3),
            account("debit", 5, 2, 1),
            account("invest-box", 3, 2, 1),
        ];
        let general = select_accounts(&accounts, AccountPurpose::GeneralRead);
        assert_eq!(
            general
                .selected
                .iter()
                .map(|account| account.account_id.as_str())
                .collect::<Vec<_>>(),
            ["open-iis"]
        );
        let reports = select_accounts(&accounts, AccountPurpose::Report);
        assert_eq!(reports.selected[0].account_id, "open-iis");
        let stream = select_accounts(&accounts, AccountPurpose::OperationsStream);
        assert_eq!(
            stream
                .selected
                .iter()
                .map(|account| account.account_id.as_str())
                .collect::<Vec<_>>(),
            ["open-iis"]
        );
    }

    #[test]
    fn invalid_credential_and_insufficient_scope_stay_distinct() {
        assert_eq!(
            classify_preflight(&grpc_error(Code::Unauthenticated, "40003")),
            PreflightFailure::CredentialInvalidOrInactive
        );
        assert_eq!(
            classify_preflight(&grpc_error(Code::PermissionDenied, "40002")),
            PreflightFailure::InsufficientPermission
        );
    }

    #[test]
    fn gates_are_method_and_provider_code_specific() {
        assert_eq!(
            classify_method_gate("GetPortfolio", &provider(Code::PermissionDenied, "40002")),
            Some(CapabilityGate::InsufficientPermission)
        );
        assert_eq!(
            classify_method_gate("GetPortfolio", &provider(Code::InvalidArgument, "30058")),
            None
        );
        assert_eq!(
            classify_method_gate("GetAccounts", &provider(Code::PermissionDenied, "40002")),
            None
        );
    }

    #[test]
    fn absent_sandbox_token_differs_from_authenticated_empty_sandbox() {
        assert_eq!(
            sandbox_availability(false, 0),
            SandboxAvailability::TokenNotConfigured
        );
        assert_eq!(
            sandbox_availability(true, 0),
            SandboxAvailability::AuthenticatedNoEligibleAccount
        );
        assert_eq!(sandbox_availability(true, 1), SandboxAvailability::Ready);
    }

    #[test]
    fn sandbox_only_auto_selects_without_production_token() {
        assert_eq!(
            select_qualification_mode(None, false, true),
            Ok(QualificationMode::SandboxOnly)
        );
        assert_eq!(
            select_qualification_mode(Some("SANDBOX_ONLY"), false, true),
            Ok(QualificationMode::SandboxOnly)
        );
        assert_eq!(
            select_qualification_mode(Some("PRODUCTION_READ_ONLY"), false, true),
            Err(QualificationModeError::MissingProductionToken)
        );
    }

    #[test]
    fn every_inventory_row_has_explicit_environment_support() {
        for method in ACCOUNT_INVENTORY_METHODS {
            assert!(method_environment(method).is_some(), "missing {method}");
        }
        assert_eq!(
            method_environment("GetAccounts"),
            Some(MethodEnvironment::ProductionAndSandbox)
        );
        assert_eq!(
            method_environment("GetBrokerReport"),
            Some(MethodEnvironment::ProductionOnly)
        );
        assert_eq!(
            method_environment("GetSandboxAccounts"),
            Some(MethodEnvironment::SandboxOnly)
        );
    }

    #[test]
    fn ledger_rejects_silent_skip_and_emits_exact_inventory_order() {
        let mut incomplete = QualificationLedger::default();
        incomplete
            .record("GetAccounts", Evidence::Qualified("preflight".into()))
            .expect("known row");
        assert!(matches!(
            incomplete.finish(),
            Err(LedgerError::MissingMethods(_))
        ));

        let mut complete = QualificationLedger::default();
        for method in ACCOUNT_INVENTORY_METHODS {
            complete
                .record(method, Evidence::Qualified("offline".into()))
                .expect("unique row");
        }
        let rows = complete.finish().expect("complete ledger");
        assert_eq!(rows.len(), 38);
        assert_eq!(rows[0].0, "GetAccounts");
        assert_eq!(rows[37].0, "CancelSandboxStopOrder");
    }
}
