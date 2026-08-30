use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskOutcome {
    Approve,
    Resize,
    Reject,
    ReduceOnly,
    Halt,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskState {
    Normal,
    Warning,
    ReduceOnly,
    Halted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskSource {
    Manual,
    Strategy,
    Ml,
    Ai,
    EmergencyOperator,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskReasonCode {
    Approved,
    ResizedToProviderLimit,
    ResizedToPolicyLimit,
    InvalidQuantity,
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
    ProviderLimitUnavailable,
    ProviderLimitExceeded,
    MaxSingleOrderExceeded,
    MaxPositionExceeded,
    MaxGrossExposureExceeded,
    MaxNetExposureExceeded,
    MaxInstrumentExposureExceeded,
    DailyLossExceeded,
    ProtectionRequired,
    PersistenceFailure,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskReason {
    pub code: RiskReasonCode,
    pub message: String,
}

impl RiskReason {
    #[must_use]
    pub fn new(code: RiskReasonCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BuyLotLimit {
    pub max_lots: i64,
    pub max_market_lots: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SellLotLimit {
    pub max_lots: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerLotLimits {
    pub buy_own: Option<BuyLotLimit>,
    pub buy_margin: Option<BuyLotLimit>,
    pub sell_own: Option<SellLotLimit>,
    pub sell_margin: Option<SellLotLimit>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerMarginFacts {
    pub liquid_portfolio_nanos: i128,
    pub starting_margin_nanos: i128,
    pub minimal_margin_nanos: i128,
    pub corrected_margin_nanos: i128,
    pub funds_sufficiency_ppm: Option<i64>,
    pub amount_of_missing_funds_nanos: i128,
    pub guarantee_for_futures_nanos: i128,
    pub broker_as_of_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskValidityContext {
    pub runtime_epoch: u64,
    pub reconciliation_revision: u64,
    pub position_revision: u64,
    pub order_revision: u64,
    pub market_data_as_of_unix_ms: Option<i64>,
    pub instrument_constraints_revision: u64,
    pub policy_revision: u64,
    pub execution_authorization_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskSnapshot {
    pub runtime_ready: bool,
    pub execution_authorized: bool,
    pub unresolved_unknown_conflict: bool,
    pub current_position_lots: i64,
    pub open_order_delta_lots: i64,
    pub unresolved_unknown_delta_lots: i64,
    pub active_reservation_delta_lots: i64,
    pub gross_exposure_nanos: i128,
    pub net_exposure_nanos: i128,
    pub instrument_exposure_nanos: i128,
    pub broker_daily_pnl_nanos: Option<i128>,
    pub broker_lot_limits: Option<BrokerLotLimits>,
    pub margin: Option<BrokerMarginFacts>,
    pub validity: RiskValidityContext,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskRequest {
    pub request_id: String,
    pub account_id: String,
    pub broker_connection_id: String,
    pub instrument_id: String,
    pub strategy_id: Option<String>,
    pub source: RiskSource,
    /// Signed lots: buy is positive, sell is negative.
    pub requested_delta_lots: i64,
    pub requested_notional_nanos: i128,
    pub is_market_order: bool,
    pub confirm_margin_trade: bool,
    pub protection_established_or_planned: bool,
    pub emergency_reduction: bool,
    pub now_unix_ms: i64,
    pub snapshot: RiskSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskPolicySet {
    pub revision: u64,
    pub state: RiskState,
    pub allow_margin: bool,
    pub require_provider_lot_limit: bool,
    pub max_market_data_age_ms: Option<i64>,
    pub max_single_order_lots: Option<i64>,
    pub max_position_abs_lots: Option<i64>,
    pub max_gross_exposure_nanos: Option<i128>,
    pub max_net_exposure_abs_nanos: Option<i128>,
    pub max_instrument_exposure_nanos: Option<i128>,
    pub max_daily_loss_nanos: Option<i128>,
    pub protection_required_for_new_exposure: bool,
}

impl RiskPolicySet {
    #[must_use]
    pub const fn fail_closed(revision: u64) -> Self {
        Self {
            revision,
            state: RiskState::ReduceOnly,
            allow_margin: false,
            require_provider_lot_limit: true,
            max_market_data_age_ms: Some(0),
            max_single_order_lots: None,
            max_position_abs_lots: None,
            max_gross_exposure_nanos: None,
            max_net_exposure_abs_nanos: None,
            max_instrument_exposure_nanos: None,
            max_daily_loss_nanos: None,
            protection_required_for_new_exposure: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskDecision {
    pub decision_id: String,
    pub request_id: String,
    pub policy_revision: u64,
    pub account_id: String,
    pub requested_delta_lots: i64,
    pub approved_delta_lots: i64,
    pub outcome: RiskOutcome,
    pub reasons: Vec<RiskReason>,
    pub reservation_id: Option<String>,
    pub expires_at_unix_ms: Option<i64>,
    pub validity: RiskValidityContext,
}

impl RiskDecision {
    #[must_use]
    pub fn new_id() -> String {
        format!("risk-decision:{}", Uuid::new_v4())
    }

    #[must_use]
    pub const fn permits_dispatch(&self) -> bool {
        matches!(self.outcome, RiskOutcome::Approve | RiskOutcome::Resize)
            && self.approved_delta_lots != 0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReservationState {
    Active,
    PartiallyConsumed,
    Consumed,
    Released,
    UnknownHeld,
    Orphaned,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RiskReservation {
    pub reservation_id: String,
    pub account_id: String,
    pub instrument_id: String,
    pub strategy_id: Option<String>,
    pub source: RiskSource,
    pub logical_request_id: String,
    pub reserved_delta_lots: i64,
    pub remaining_delta_lots: i64,
    pub reserved_notional_nanos: i128,
    pub state: ReservationState,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub expires_at_unix_ms: Option<i64>,
}

impl RiskReservation {
    #[must_use]
    pub fn new_id() -> String {
        format!("risk-reservation:{}", Uuid::new_v4())
    }
}
