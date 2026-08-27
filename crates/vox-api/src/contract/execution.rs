//! Execution commands and their receipts.
//!
//! Every capital-affecting command carries its immutable target scope and a client-supplied
//! logical request id, and returns a receipt whose journal state is the runtime's own. The
//! browser never learns whether it may retry from its own reasoning: `MutationDecision`
//! comes from the backend.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_domain::{
    ExecutionPriceConvention, OrderSide, ProtectionEstablishmentState, RegularOrderType,
    TimeInForce, TrailingDistanceMode,
};

use super::money::Decimal;
use super::scope::ExecutionScope;

/// Which side of the market a command takes. The side is the action, never a mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSideDto {
    Buy,
    Sell,
}

impl From<OrderSideDto> for OrderSide {
    fn from(value: OrderSideDto) -> Self {
        match value {
            OrderSideDto::Buy => Self::Buy,
            OrderSideDto::Sell => Self::Sell,
        }
    }
}

impl From<OrderSide> for OrderSideDto {
    fn from(value: OrderSide) -> Self {
        match value {
            OrderSide::Buy => Self::Buy,
            OrderSide::Sell => Self::Sell,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderTypeDto {
    Limit,
    Market,
    BestPrice,
}

impl From<OrderTypeDto> for RegularOrderType {
    fn from(value: OrderTypeDto) -> Self {
        match value {
            OrderTypeDto::Limit => Self::Limit,
            OrderTypeDto::Market => Self::Market,
            OrderTypeDto::BestPrice => Self::BestPrice,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimeInForceDto {
    Day,
    FillAndKill,
    FillOrKill,
}

impl From<TimeInForceDto> for TimeInForce {
    fn from(value: TimeInForceDto) -> Self {
        match value {
            TimeInForceDto::Day => Self::Day,
            TimeInForceDto::FillAndKill => Self::FillAndKill,
            TimeInForceDto::FillOrKill => Self::FillOrKill,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PriceConventionDto {
    SettlementCurrency,
    Points,
}

impl From<PriceConventionDto> for ExecutionPriceConvention {
    fn from(value: PriceConventionDto) -> Self {
        match value {
            PriceConventionDto::SettlementCurrency => Self::SettlementCurrency,
            PriceConventionDto::Points => Self::Points,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrailingModeDto {
    AbsolutePrice,
    RelativePercent,
}

impl From<TrailingModeDto> for TrailingDistanceMode {
    fn from(value: TrailingModeDto) -> Self {
        match value {
            TrailingModeDto::AbsolutePrice => Self::AbsolutePrice,
            TrailingModeDto::RelativePercent => Self::RelativePercent,
        }
    }
}

/// The canonical protection lifecycle, all ten states.
///
/// Two of them carry data and are the two an operator most needs to see: a position that
/// filled only partly and is therefore only partly protected, and protection that failed
/// after the position was already open. `STALE` is deliberately absent: staleness is the age
/// of the last broker answer, carried by stream health, not a lifecycle state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectionStateDto {
    AwaitingEntry,
    /// The entry filled in part, so only part of the position carries protection.
    EntryPartiallyFilled {
        filled_lots: i64,
        protected_lots: i64,
    },
    Establishing,
    Active,
    /// The position is open and its protection did not establish. It is unprotected.
    FailedAfterEntry {
        reason: String,
    },
    UnknownAfterDispatch,
    ReconciliationRequired,
    ClosingPosition,
    Orphaned,
    Terminal,
}

impl From<ProtectionEstablishmentState> for ProtectionStateDto {
    fn from(value: ProtectionEstablishmentState) -> Self {
        match value {
            ProtectionEstablishmentState::AwaitingEntry => Self::AwaitingEntry,
            ProtectionEstablishmentState::EntryPartiallyFilled {
                filled_lots,
                protected_lots,
            } => Self::EntryPartiallyFilled {
                filled_lots,
                protected_lots,
            },
            ProtectionEstablishmentState::Establishing => Self::Establishing,
            ProtectionEstablishmentState::Active => Self::Active,
            ProtectionEstablishmentState::FailedAfterEntry { reason } => {
                Self::FailedAfterEntry { reason }
            }
            ProtectionEstablishmentState::UnknownAfterDispatch => Self::UnknownAfterDispatch,
            ProtectionEstablishmentState::ReconciliationRequired => Self::ReconciliationRequired,
            ProtectionEstablishmentState::ClosingPosition => Self::ClosingPosition,
            ProtectionEstablishmentState::Orphaned => Self::Orphaned,
            ProtectionEstablishmentState::Terminal => Self::Terminal,
        }
    }
}

/// Where a dispatched command stands. `UNKNOWN_AFTER_DISPATCH` is an unfinished answer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JournalStateDto {
    NotDispatched,
    Dispatching,
    Acknowledged,
    Rejected,
    UnknownAfterDispatch,
    Reconciled,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationKindDto {
    PostOrder,
    PostOrderAsync,
    ReplaceOrder,
    CancelOrder,
    PostStopOrder,
    CancelStopOrder,
    ProtectionLeg,
}

/// Whether the client may submit, must reconcile first, or must not submit at all.
/// Decided by the backend; the browser never derives it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationDecisionDto {
    Submit,
    Reconcile,
    DoNotSubmit,
}

/// A trailing distance: an exact value plus the mode it is measured in.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct TrailingDistanceDto {
    pub value: Decimal,
    pub mode: TrailingModeDto,
}

/// Stop loss and take profit are independent and both optional.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ProtectionPlanDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_trigger_price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_loss_trailing: Option<TrailingDistanceDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_trigger_price: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take_profit_limit_price: Option<Decimal>,
}

/// Submit a regular order. Quantity is in lots; price is exact and optional for market orders.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct SubmitOrderRequest {
    /// The immutable target. A submitted command is never retargeted by a later UI change.
    pub scope: ExecutionScope,
    pub instrument_uid: String,
    /// Client-generated identity of this command, used for idempotency and reconciliation.
    #[schema(example = "b4f1c2a0-6d18-4f0e-9a37-2d5c1f0b7e44")]
    pub client_request_id: String,
    pub side: OrderSideDto,
    pub order_type: OrderTypeDto,
    pub quantity_lots: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<Decimal>,
    pub price_convention: PriceConventionDto,
    pub time_in_force: TimeInForceDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<ProtectionPlanDto>,
}

/// Cancel a regular order by broker identity or by the logical request that created it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CancelOrderRequest {
    pub scope: ExecutionScope,
    pub client_request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_request_id: Option<String>,
}

/// The receipt of a capital-affecting command.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct MutationReceiptDto {
    /// Vox-side identity of the command, stable across retries and reconciliation.
    pub logical_request_id: String,
    /// The frozen target this command was dispatched to.
    pub scope: ExecutionScope,
    pub kind: MutationKindDto,
    pub state: JournalStateDto,
    /// What the backend says the client may do next.
    pub decision: MutationDecisionDto,
    pub correlation_id: String,
    /// Set only once the broker has answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_stop_order_id: Option<String>,
    /// Runtime disposition after reconciliation, when it has run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconciliation_disposition: Option<String>,
    pub runtime_epoch: u64,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_lifecycle_has_no_stale_state() -> Result<(), serde_json::Error> {
        assert!(serde_json::from_str::<ProtectionStateDto>("\"STALE\"").is_err());
        let partial =
            ProtectionStateDto::from(ProtectionEstablishmentState::EntryPartiallyFilled {
                filled_lots: 6,
                protected_lots: 4,
            });
        let json = serde_json::to_value(&partial)?;
        assert_eq!(json["ENTRY_PARTIALLY_FILLED"]["filled_lots"], 6);
        assert_eq!(json["ENTRY_PARTIALLY_FILLED"]["protected_lots"], 4);
        assert_eq!(
            serde_json::to_string(&ProtectionStateDto::UnknownAfterDispatch)?,
            "\"UNKNOWN_AFTER_DISPATCH\""
        );
        Ok(())
    }

    #[test]
    fn journal_state_spelling_matches_the_contract_map() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/design/BACKEND_CONTRACTS.md");
        let map = std::fs::read_to_string(path).expect("the contract map is committed");
        for spelling in [
            "NOT_DISPATCHED",
            "DISPATCHING",
            "ACKNOWLEDGED",
            "REJECTED",
            "UNKNOWN_AFTER_DISPATCH",
            "RECONCILED",
        ] {
            assert!(
                map.contains(spelling),
                "{spelling} is not in the recorded contract map"
            );
        }
    }
}
