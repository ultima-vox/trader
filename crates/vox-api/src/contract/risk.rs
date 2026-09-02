//! Public application-facing risk contracts owned by #21.
//!
//! The transport renders these types but never derives risk locally. Provider facts and
//! internal persistence remain behind the application port.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::scope::ExecutionScope;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskOutcomeDto {
    Approve,
    Resize,
    Reject,
    ReduceOnly,
    Halt,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskStateDto {
    Normal,
    Warning,
    ReduceOnly,
    Halted,
    KillSwitch,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskActionKindDto {
    DirectionalOrder,
    ReplaceDirectionalOrder,
    CancelOrder,
    ProtectionMaintenance,
    CancelProtection,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskReasonCodeDto {
    Approved,
    ResizedToProviderLimit,
    ResizedToPolicyLimit,
    InvalidQuantity,
    InstrumentUnavailable,
    InstrumentNotTradable,
    PriceUnavailable,
    PositionLotMismatch,
    CriticalInputMissing,
    RuntimeNotReady,
    ExecutionUnauthorized,
    AuthorizationRevisionChanged,
    PolicyRevisionChanged,
    ReconciliationRevisionChanged,
    PositionRevisionChanged,
    OrderRevisionChanged,
    InstrumentConstraintRevisionChanged,
    MarketDataMissing,
    MarketDataStale,
    UnknownMutationConflict,
    ReduceOnly,
    Halted,
    MarginNotAllowed,
    MarginConfirmationRequired,
    MarginUtilizationExceeded,
    ProviderLimitUnavailable,
    ProviderLimitExceeded,
    MaxSingleOrderExceeded,
    MaxPositionExceeded,
    MaxGrossExposureExceeded,
    MaxNetExposureExceeded,
    MaxInstrumentExposureExceeded,
    DailyLossExceeded,
    ProtectionRequired,
    KillSwitchActive,
    PersistenceFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskReasonDto {
    pub code: RiskReasonCodeDto,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskValidityDto {
    pub runtime_epoch: u64,
    pub reconciliation_revision: u64,
    pub position_revision: u64,
    pub order_revision: u64,
    pub market_data_as_of_unix_ms: Option<i64>,
    pub instrument_constraints_revision: u64,
    pub policy_revision: u64,
    pub execution_authorization_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskDecisionDto {
    pub decision_id: String,
    pub policy_revision: u64,
    pub action: RiskActionKindDto,
    pub requested_delta_lots: i64,
    pub approved_delta_lots: i64,
    pub outcome: RiskOutcomeDto,
    pub reasons: Vec<RiskReasonDto>,
    pub reservation_id: Option<String>,
    pub validity: RiskValidityDto,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReservationStateDto {
    Active,
    PartiallyConsumed,
    Consumed,
    Released,
    UnknownHeld,
    Orphaned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskReservationDto {
    pub reservation_id: String,
    pub scope: ExecutionScope,
    pub instrument_id: String,
    pub logical_request_id: String,
    pub remaining_delta_lots: i64,
    pub state: ReservationStateDto,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskLimitUsageDto {
    pub name: String,
    /// Exact decimal string in the unit named by `unit`; never a binary float.
    pub used: String,
    pub limit: Option<String>,
    pub unit: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct RiskStatusDto {
    pub scope: ExecutionScope,
    pub state: RiskStateDto,
    pub policy_revision: u64,
    pub limits: Vec<RiskLimitUsageDto>,
    pub reasons: Vec<RiskReasonDto>,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct ChangeRiskStateRequest {
    pub scope: ExecutionScope,
    pub state: RiskStateDto,
    /// Optimistic concurrency: stale operator state cannot overwrite a newer control action.
    pub expected_policy_revision: u64,
    pub reason: String,
}
