//! Broker connection administration contracts owned by #17.
//!
//! Credential material exists only in write-only request bodies. Response types expose
//! metadata and safe broker facts, never `CredentialRef`, ciphertext, fingerprints, or tokens.

use core::fmt;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::scope::{BrokerEnvironment, ProviderDto};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialStatusDto {
    PendingValidation,
    Valid,
    Invalid,
    ExpiredOrInactive,
    PendingDisable,
    Disabled,
    PendingDelete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialClassDto {
    Unknown,
    ReadOnly,
    FullAccess,
    TransferAccess,
    Sandbox,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CredentialScopeDto {
    NotConfirmed,
    SingleAccountRestricted,
    AllAccessibleAccounts,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionCapabilityDto {
    AccountDiscovery,
    PortfolioRead,
    PositionsRead,
    OperationsRead,
    StreamHealth,
    SandboxOrders,
    ProductionOrdersProviderAllowed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionHealthStateDto {
    Unknown,
    Validating,
    Healthy,
    InvalidCredential,
    InsufficientPermission,
    ProviderUnavailable,
    AccountAccessChanged,
    Disabled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionHealthReasonDto {
    None,
    InvalidCredential,
    ExpiredOrInactive,
    PermissionDenied,
    WrongEnvironment,
    ProviderUnavailable,
    AccountAccessChanged,
    DisabledByOperator,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct ConnectionHealthDto {
    pub state: ConnectionHealthStateDto,
    pub checked_at_unix_ms: Option<i64>,
    pub reason_code: ConnectionHealthReasonDto,
    pub safe_detail: Option<String>,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct BrokerConnectionMetadataDto {
    pub connection_id: String,
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub display_label: String,
    pub enabled: bool,
    pub credential_status: CredentialStatusDto,
    pub credential_class: CredentialClassDto,
    pub credential_scope: CredentialScopeDto,
    pub capabilities: Vec<ConnectionCapabilityDto>,
    pub health: ConnectionHealthDto,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateBrokerConnectionRequest {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub display_label: String,
    #[schema(write_only)]
    pub credential: String,
}

impl fmt::Debug for CreateBrokerConnectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateBrokerConnectionRequest")
            .field("provider", &self.provider)
            .field("environment", &self.environment)
            .field("display_label", &self.display_label)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

#[derive(Deserialize, ToSchema)]
pub struct RotateCredentialRequest {
    #[schema(write_only)]
    pub credential: String,
}

impl fmt::Debug for RotateCredentialRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RotateCredentialRequest")
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CredentialRotationResultDto {
    pub connection: BrokerConnectionMetadataDto,
    pub reconnect_required: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct DiscoveredBrokerAccountDto {
    pub connection_id: String,
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub provider_account_id: String,
    pub display_name: Option<String>,
    pub account_type: String,
    pub account_status: String,
    pub access_level: String,
    pub opened_at_unix_ms: Option<i64>,
    pub closed_at_unix_ms: Option<i64>,
    pub accessible: bool,
    pub capabilities: Vec<ConnectionCapabilityDto>,
    pub discovered_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct BrokerAccountBindingDto {
    pub binding_id: String,
    pub connection_id: String,
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub provider_account_id: String,
    pub account_id: String,
    pub enabled: bool,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, ToSchema)]
pub struct BindBrokerAccountRequest {
    pub provider_account_id: String,
    pub account_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionAuthorizationModeDto {
    Disabled,
    ManualAllowed,
    AutomatedAllowed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, ToSchema)]
pub struct ChangeExecutionAuthorizationRequest {
    pub provider_account_id: String,
    pub mode: ExecutionAuthorizationModeDto,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct ExecutionAuthorizationDto {
    pub connection_id: String,
    pub provider_account_id: String,
    pub mode: ExecutionAuthorizationModeDto,
    pub authorization_revision: u64,
    pub changed_by: String,
    pub changed_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct ConnectionDetailsDto {
    pub connection: BrokerConnectionMetadataDto,
    pub accounts: Vec<DiscoveredBrokerAccountDto>,
    pub bindings: Vec<BrokerAccountBindingDto>,
    pub execution_authorizations: Vec<ExecutionAuthorizationDto>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_requests_redact_debug() {
        let create = CreateBrokerConnectionRequest {
            provider: ProviderDto::TInvest,
            environment: BrokerEnvironment::Sandbox,
            display_label: "sandbox".to_owned(),
            credential: "credential-material".to_owned(),
        };
        let rotate = RotateCredentialRequest {
            credential: "replacement-material".to_owned(),
        };
        assert!(!format!("{create:?}").contains("credential-material"));
        assert!(!format!("{rotate:?}").contains("replacement-material"));
    }
}
