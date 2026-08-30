use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(format!(concat!($prefix, ":{}"), Uuid::new_v4()))
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, ModelError> {
                let value = value.into();
                let Some(raw) = value.strip_prefix(concat!($prefix, ":")) else {
                    return Err(ModelError::InvalidIdentity(stringify!($name)));
                };
                Uuid::parse_str(raw).map_err(|_| ModelError::InvalidIdentity(stringify!($name)))?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

uuid_id!(ConnectionId, "connection");
uuid_id!(CredentialRef, "credential");
uuid_id!(BindingId, "binding");
uuid_id!(VoxAccountId, "vox-account");
uuid_id!(UserId, "user");
uuid_id!(RoleId, "role");

pub type BrokerConnectionId = ConnectionId;
pub type AuditActor = UserId;

impl fmt::Display for CredentialRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[credential-ref]")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub const T_INVEST: &'static str = "T_INVEST";

    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
        {
            return Err(ModelError::InvalidProviderId);
        }
        Ok(Self(value))
    }

    pub fn tinvest() -> Self {
        Self(Self::T_INVEST.to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerEnvironment {
    Production,
    Sandbox,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialStatus {
    PendingValidation,
    Valid,
    Invalid,
    ExpiredOrInactive,
    PendingDisable,
    Disabled,
    PendingDelete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialClass {
    Unknown,
    ReadOnly,
    FullAccess,
    TransferAccess,
    Sandbox,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialScope {
    NotConfirmed,
    SingleAccountRestricted,
    AllAccessibleAccounts,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionCapability {
    AccountDiscovery,
    PortfolioRead,
    PositionsRead,
    OperationsRead,
    StreamHealth,
    SandboxOrders,
    ProductionOrdersProviderAllowed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionHealthState {
    Unknown,
    Validating,
    Healthy,
    InvalidCredential,
    InsufficientPermission,
    ProviderUnavailable,
    AccountAccessChanged,
    Disabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionHealthReason {
    None,
    InvalidCredential,
    ExpiredOrInactive,
    PermissionDenied,
    WrongEnvironment,
    ProviderUnavailable,
    AccountAccessChanged,
    DisabledByOperator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionHealth {
    pub state: ConnectionHealthState,
    pub checked_at_unix_ms: Option<i64>,
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub reason_code: ConnectionHealthReason,
    pub safe_detail: Option<String>,
    pub retryable: bool,
}

impl ConnectionHealth {
    #[must_use]
    pub fn unknown(provider: ProviderId, environment: BrokerEnvironment) -> Self {
        Self {
            state: ConnectionHealthState::Unknown,
            checked_at_unix_ms: None,
            provider,
            environment,
            reason_code: ConnectionHealthReason::None,
            safe_detail: None,
            retryable: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerConnection {
    pub id: ConnectionId,
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub credential_ref: CredentialRef,
    pub label: String,
    pub credential_fingerprint: String,
    pub credential_status: CredentialStatus,
    pub credential_class: CredentialClass,
    pub credential_scope: CredentialScope,
    pub enabled: bool,
    pub health: ConnectionHealth,
    pub capabilities: BTreeSet<ConnectionCapability>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerAccount {
    pub connection_id: ConnectionId,
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub provider_account_id: String,
    pub display_name: Option<String>,
    pub account_type: BrokerAccountType,
    pub status: BrokerAccountStatus,
    pub access_level: BrokerAccessLevel,
    pub opened_at_unix_ms: Option<i64>,
    pub closed_at_unix_ms: Option<i64>,
    pub accessible: bool,
    pub capabilities: BTreeSet<ConnectionCapability>,
    pub discovered_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerAccountType {
    Unspecified,
    Brokerage,
    IndividualInvestment,
    InvestBox,
    MoneyMarketFund,
    Debit,
    Savings,
    DigitalFinancialAssets,
    UnknownProviderValue(i32),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerAccountStatus {
    Unspecified,
    New,
    Open,
    Closed,
    UnknownProviderValue(i32),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerAccessLevel {
    Unspecified,
    Full,
    ReadOnly,
    NoAccess,
    UnknownProviderValue(i32),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerAccountBinding {
    pub id: BindingId,
    pub connection_id: ConnectionId,
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub provider_account_id: String,
    pub vox_account_id: VoxAccountId,
    pub enabled: bool,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountTarget {
    pub connection_id: ConnectionId,
    pub provider: ProviderId,
    pub environment: BrokerEnvironment,
    pub provider_account_id: String,
    pub vox_account_id: VoxAccountId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionAuthorization {
    pub connection_id: ConnectionId,
    pub provider_account_id: String,
    pub mode: ExecutionAuthorizationMode,
    pub authorization_revision: u64,
    pub changed_by: UserId,
    pub changed_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionAuthorizationMode {
    Disabled,
    ManualAllowed,
    AutomatedAllowed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Permission {
    ViewConnectionMetadata,
    ManageCredentials,
    DisableDeleteConnection,
    DiscoverAccounts,
    BindAccounts,
    ViewPortfolio,
    SubmitSandboxOrders,
    SubmitProductionManualOrders,
    EnableAutomatedProductionExecution,
    ChangeRiskPolicy,
    EmergencyHalt,
    SecurityAdmin,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct User {
    pub id: UserId,
    pub display_name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Role {
    pub id: RoleId,
    pub name: String,
    pub permissions: BTreeSet<Permission>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditRecord {
    pub actor: UserId,
    pub action: String,
    pub target_ref: String,
    pub previous_state: Option<String>,
    pub new_state: Option<String>,
    pub correlation_id: String,
    pub occurred_at_unix_ms: i64,
}

pub type AuditEvent = AuditRecord;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelError {
    #[error("invalid {0}")]
    InvalidIdentity(&'static str),
    #[error("provider id must be bounded SCREAMING_SNAKE_CASE")]
    InvalidProviderId,
    #[error("required field {0} is empty")]
    Empty(&'static str),
    #[error("unsafe metadata in {0}")]
    UnsafeMetadata(&'static str),
}

pub(crate) fn safe_text(
    value: impl Into<String>,
    field: &'static str,
    max_len: usize,
) -> Result<String, ModelError> {
    let value = value.into();
    let lower = value.to_ascii_lowercase();
    let trimmed_lower = lower.trim_start();
    if value.trim().is_empty() {
        return Err(ModelError::Empty(field));
    }
    if value.len() > max_len
        || lower.contains("bearer ")
        || lower.contains("token=")
        || lower.contains("secret=")
        || trimmed_lower.starts_with("t.")
        || trimmed_lower.starts_with("eyj")
        || value.chars().any(char::is_control)
    {
        return Err(ModelError::UnsafeMetadata(field));
    }
    Ok(value)
}
