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
    AccountValues,
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

/// Reproducible sandbox provider defect: official contract advertises GetBankAccounts, but the
/// sandbox endpoint persistently returns documented generic internal error 70001 for the exact
/// empty generated request while sibling UsersService methods succeed. This is not "unsupported".
#[must_use]
pub fn persistent_sandbox_provider_limitation(method: &str, error: &GrpcError) -> bool {
    method == "GetBankAccounts"
        && error.metadata.attempt >= 3
        && matches!(
            &error.kind,
            GrpcErrorKind::Provider(provider)
                if provider.code == Code::Internal && provider.has_provider_code("70001")
        )
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
    BlockedByPrerequisite(String),
    Failed(FailureEvidence),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureClass {
    ProviderInternal,
    Provider,
    Adapter,
}

impl FailureClass {
    #[must_use]
    pub const fn wire_name(&self) -> &'static str {
        match self {
            Self::ProviderInternal => "FAILED_PROVIDER_INTERNAL",
            Self::Provider => "FAILED_PROVIDER",
            Self::Adapter => "FAILED_ADAPTER",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureEvidence {
    pub class: FailureClass,
    pub grpc_status: Option<Code>,
    pub provider_code: Option<String>,
    pub method: &'static str,
    pub attempt: Option<u32>,
    pub tracking_id: Option<String>,
    pub provider_message: Option<String>,
    pub environment: Option<String>,
    pub request_shape: Option<String>,
    pub detail: String,
}

impl FailureEvidence {
    #[must_use]
    pub fn with_live_context(
        mut self,
        environment: impl Into<String>,
        request_shape: impl Into<String>,
        sensitive_values: &[String],
    ) -> Self {
        self.environment = Some(environment.into());
        self.request_shape = Some(request_shape.into());
        for sensitive in sensitive_values {
            if !sensitive.is_empty() {
                if let Some(message) = &mut self.provider_message {
                    *message = message.replace(sensitive, "<redacted-account-id>");
                }
                self.detail = self.detail.replace(sensitive, "<redacted-account-id>");
            }
        }
        self
    }
}

#[must_use]
pub fn grpc_failure(method: &'static str, error: &GrpcError) -> FailureEvidence {
    match &error.kind {
        GrpcErrorKind::Provider(provider) => {
            let provider_code = provider.provider_code();
            let class = if provider.code == Code::Internal
                && provider_code
                    .as_deref()
                    .is_some_and(|code| matches!(code, "70001" | "70002" | "70003"))
            {
                FailureClass::ProviderInternal
            } else {
                FailureClass::Provider
            };
            FailureEvidence {
                class,
                grpc_status: Some(provider.code),
                provider_code,
                method,
                attempt: Some(error.metadata.attempt),
                tracking_id: provider.tracking_id.clone(),
                provider_message: Some(provider.message.clone()),
                environment: None,
                request_shape: None,
                detail: "provider request failed after bounded safe-read policy".into(),
            }
        }
        kind => FailureEvidence {
            class: FailureClass::Adapter,
            grpc_status: None,
            provider_code: None,
            method,
            attempt: Some(error.metadata.attempt),
            tracking_id: None,
            provider_message: None,
            environment: None,
            request_shape: None,
            detail: kind.to_string(),
        },
    }
}

#[must_use]
pub fn adapter_failure(method: &'static str, detail: impl Into<String>) -> FailureEvidence {
    FailureEvidence {
        class: FailureClass::Adapter,
        grpc_status: None,
        provider_code: None,
        method,
        attempt: None,
        tracking_id: None,
        provider_message: None,
        environment: None,
        request_shape: None,
        detail: detail.into(),
    }
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct QualificationSummary {
    pub qualified: Vec<&'static str>,
    pub gated: Vec<&'static str>,
    pub blocked: Vec<&'static str>,
    pub failed: Vec<&'static str>,
}

impl QualificationSummary {
    #[must_use]
    pub fn from_rows(rows: &[(&'static str, Evidence)]) -> Self {
        let mut summary = Self::default();
        for (method, evidence) in rows {
            match evidence {
                Evidence::Qualified(_) => summary.qualified.push(*method),
                Evidence::GatedUnavailable(_) => summary.gated.push(*method),
                Evidence::BlockedByPrerequisite(_) => summary.blocked.push(*method),
                Evidence::Failed(_) => summary.failed.push(*method),
            }
        }
        summary
    }

    #[must_use]
    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }

    pub fn ensure_success(&self) -> Result<(), QualificationFailures> {
        if self.failed.is_empty() {
            Ok(())
        } else {
            Err(QualificationFailures(self.failed.clone()))
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("qualification completed with FAILED rows: {0:?}")]
pub struct QualificationFailures(pub Vec<&'static str>);

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
        let values = select_accounts(&accounts, AccountPurpose::AccountValues);
        assert_eq!(
            values
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
        assert_eq!(
            select_qualification_mode(None, true, true),
            Ok(QualificationMode::ProductionReadOnly)
        );
        assert_eq!(
            select_qualification_mode(Some("TYPO"), false, true),
            Err(QualificationModeError::UnknownMode("TYPO".to_owned()))
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

    #[test]
    fn ledger_continues_after_failure_and_summary_fails_closed() {
        let mut ledger = QualificationLedger::default();
        for method in ACCOUNT_INVENTORY_METHODS {
            let evidence = match method {
                "GetBankAccounts" => Evidence::Failed(adapter_failure(method, "persistent 70001")),
                "GetPortfolio" => Evidence::BlockedByPrerequisite("GetAccounts failed".into()),
                _ => Evidence::Qualified("continued".into()),
            };
            ledger.record(method, evidence).expect("unique row");
        }
        let rows = ledger.finish().expect("complete aggregate ledger");
        let summary = QualificationSummary::from_rows(&rows);
        assert!(summary.has_failures());
        assert!(summary.ensure_success().is_err());
        assert_eq!(summary.failed, ["GetBankAccounts"]);
        assert_eq!(summary.blocked, ["GetPortfolio"]);
        assert_eq!(summary.qualified.len(), 36);
    }

    #[test]
    fn persistent_provider_internal_keeps_code_attempt_and_tracking() {
        let mut error = grpc_error(Code::Internal, "70001");
        error.metadata.attempt = 3;
        let GrpcErrorKind::Provider(provider) = &mut error.kind else {
            panic!("provider fixture");
        };
        provider.tracking_id = Some("tracking-70001".into());
        let failed = grpc_failure("GetBankAccounts", &error);
        assert_eq!(failed.class, FailureClass::ProviderInternal);
        assert_eq!(failed.grpc_status, Some(Code::Internal));
        assert_eq!(failed.provider_code.as_deref(), Some("70001"));
        assert_eq!(failed.attempt, Some(3));
        assert_eq!(failed.tracking_id.as_deref(), Some("tracking-70001"));
        let contextual = failed.with_live_context(
            "SANDBOX_ONLY",
            "GetBankAccountsRequest {}",
            &["secret-account".into()],
        );
        assert_eq!(contextual.environment.as_deref(), Some("SANDBOX_ONLY"));
        assert_eq!(
            contextual.request_shape.as_deref(),
            Some("GetBankAccountsRequest {}")
        );
        assert_eq!(
            contextual.provider_message.as_deref(),
            Some("provider code 70001")
        );
        assert!(persistent_sandbox_provider_limitation(
            "GetBankAccounts",
            &error
        ));
        assert!(!persistent_sandbox_provider_limitation(
            "GetAccountValues",
            &error
        ));
        error.metadata.attempt = 2;
        assert!(!persistent_sandbox_provider_limitation(
            "GetBankAccounts",
            &error
        ));
    }
}
