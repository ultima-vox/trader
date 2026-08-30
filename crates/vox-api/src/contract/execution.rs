//! Execution commands and their receipts.
//!
//! Every capital-affecting command carries its immutable target scope and a client-supplied
//! logical request id, and returns a receipt whose journal state is the runtime's own. The
//! browser never learns whether it may retry from its own reasoning: `MutationDecision`
//! comes from the backend.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_domain::{
    ExecutionPriceConvention, OrderSide, PositionSide, ProtectionEstablishmentState,
    RegularOrderType, TimeInForce, TrailingDistanceMode,
};

use super::money::Decimal;
use super::scope::ExecutionScope;
use crate::error::{ApiError, FieldError};

/// Which side of the market a command takes. The side is the action, never a mode.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSideDto {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PositionSideDto {
    Long,
    Short,
}

impl From<PositionSideDto> for PositionSide {
    fn from(value: PositionSideDto) -> Self {
        match value {
            PositionSideDto::Long => Self::Long,
            PositionSideDto::Short => Self::Short,
        }
    }
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

impl From<vox_runtime::JournalState> for JournalStateDto {
    fn from(value: vox_runtime::JournalState) -> Self {
        match value {
            vox_runtime::JournalState::NotDispatched => Self::NotDispatched,
            vox_runtime::JournalState::Dispatching => Self::Dispatching,
            vox_runtime::JournalState::Acknowledged => Self::Acknowledged,
            vox_runtime::JournalState::Rejected => Self::Rejected,
            vox_runtime::JournalState::UnknownAfterDispatch => Self::UnknownAfterDispatch,
            vox_runtime::JournalState::Reconciled => Self::Reconciled,
        }
    }
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

impl From<vox_runtime::MutationKind> for MutationKindDto {
    fn from(value: vox_runtime::MutationKind) -> Self {
        match value {
            vox_runtime::MutationKind::PostOrder => Self::PostOrder,
            vox_runtime::MutationKind::PostOrderAsync => Self::PostOrderAsync,
            vox_runtime::MutationKind::ReplaceOrder => Self::ReplaceOrder,
            vox_runtime::MutationKind::CancelOrder => Self::CancelOrder,
            vox_runtime::MutationKind::PostStopOrder => Self::PostStopOrder,
            vox_runtime::MutationKind::CancelStopOrder => Self::CancelStopOrder,
            vox_runtime::MutationKind::ProtectionLeg => Self::ProtectionLeg,
        }
    }
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
    /// Opaque canonical instrument identity. Provider UID/FIGI mapping stays inside adapters.
    #[schema(example = "instrument:sber")]
    pub instrument_id: String,
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
    /// Explicit broker margin acknowledgement. Never inferred by server.
    pub confirm_margin_trade: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<ProtectionPlanDto>,
}

/// Which existing command a cancel names. Exactly one variant is valid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CancelTarget {
    BrokerOrder { broker_order_id: String },
    LogicalRequest { logical_request_id: String },
}

/// Cancel a regular order. Exactly one of `broker_order_id` or `logical_request_id` names
/// the command being cancelled. `client_request_id` is the identity of *this* cancel.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CancelOrderRequest {
    pub scope: ExecutionScope,
    pub client_request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_request_id: Option<String>,
}

impl CancelOrderRequest {
    /// Returns the single cancellation target. Two identifiers at once, or none, is invalid.
    pub fn target(&self) -> Result<CancelTarget, ApiError> {
        let broker = nonempty(self.broker_order_id.as_deref());
        let logical = nonempty(self.logical_request_id.as_deref());
        match (broker, logical) {
            (Some(broker_order_id), None) => Ok(CancelTarget::BrokerOrder {
                broker_order_id: broker_order_id.to_owned(),
            }),
            (None, Some(logical_request_id)) => Ok(CancelTarget::LogicalRequest {
                logical_request_id: logical_request_id.to_owned(),
            }),
            (Some(_), Some(_)) => Err(ApiError::validation(
                "a cancel cannot name two targets; prove they are the same command first",
                vec![
                    FieldError {
                        field: "broker_order_id".to_owned(),
                        message: "must be omitted when logical_request_id is set".to_owned(),
                    },
                    FieldError {
                        field: "logical_request_id".to_owned(),
                        message: "must be omitted when broker_order_id is set".to_owned(),
                    },
                ],
            )),
            (None, None) => Err(ApiError::validation(
                "a cancel needs exactly one identifier for the order it cancels",
                vec![FieldError {
                    field: "broker_order_id".to_owned(),
                    message: "provide exactly one of broker_order_id or logical_request_id"
                        .to_owned(),
                }],
            )),
        }
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Replace a live regular order. Identifies the original with exactly one target.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ReplaceOrderRequest {
    pub scope: ExecutionScope,
    pub instrument_id: String,
    pub client_request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_request_id: Option<String>,
    pub quantity_lots: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<Decimal>,
    pub price_convention: PriceConventionDto,
    /// Explicit broker margin acknowledgement. Never inferred by server.
    pub confirm_margin_trade: bool,
}

impl ReplaceOrderRequest {
    pub fn target(&self) -> Result<CancelTarget, ApiError> {
        CancelOrderRequest {
            scope: self.scope.clone(),
            client_request_id: self.client_request_id.clone(),
            broker_order_id: self.broker_order_id.clone(),
            logical_request_id: self.logical_request_id.clone(),
        }
        .target()
    }
}

/// Submit a stop order. Trigger is exact; limit price is optional.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct SubmitStopOrderRequest {
    pub scope: ExecutionScope,
    pub instrument_id: String,
    pub client_request_id: String,
    pub side: OrderSideDto,
    /// Position protected by this stop. Order side alone is not authoritative proof.
    pub position_side: PositionSideDto,
    pub quantity_lots: i64,
    /// Exact current/reference price used by broker validation.
    pub reference_price: Decimal,
    pub trigger_price: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<Decimal>,
    pub price_convention: PriceConventionDto,
    pub confirm_margin_trade: bool,
}

/// Establish or replace protection legs on an existing position. Not a bulk migration.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct SubmitProtectionRequest {
    pub scope: ExecutionScope,
    pub instrument_id: String,
    pub client_request_id: String,
    pub quantity_lots: i64,
    pub position_side: PositionSideDto,
    pub reference_price: Decimal,
    pub price_convention: PriceConventionDto,
    pub confirm_margin_trade: bool,
    pub plan: ProtectionPlanDto,
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

impl MutationReceiptDto {
    /// UNKNOWN after dispatch is a valid unfinished mutation, never an API error.
    #[must_use]
    pub fn unknown_after_dispatch(
        logical_request_id: impl Into<String>,
        scope: ExecutionScope,
        kind: MutationKindDto,
        correlation_id: impl Into<String>,
        runtime_epoch: u64,
        created_at_unix_ms: i64,
        updated_at_unix_ms: i64,
    ) -> Self {
        Self {
            logical_request_id: logical_request_id.into(),
            scope,
            kind,
            state: JournalStateDto::UnknownAfterDispatch,
            decision: MutationDecisionDto::Reconcile,
            correlation_id: correlation_id.into(),
            broker_order_id: None,
            broker_stop_order_id: None,
            reconciliation_disposition: None,
            runtime_epoch,
            created_at_unix_ms,
            updated_at_unix_ms,
        }
    }

    #[must_use]
    pub const fn decision_for(state: JournalStateDto) -> MutationDecisionDto {
        match state {
            JournalStateDto::NotDispatched => MutationDecisionDto::Submit,
            JournalStateDto::Dispatching | JournalStateDto::UnknownAfterDispatch => {
                MutationDecisionDto::Reconcile
            }
            JournalStateDto::Acknowledged
            | JournalStateDto::Rejected
            | JournalStateDto::Reconciled => MutationDecisionDto::DoNotSubmit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::scope::{BrokerEnvironment, ProviderDto, TradingMode};

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

    fn sample_scope() -> Result<ExecutionScope, crate::contract::scope::ScopeError> {
        ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:primary",
            "account:primary",
            TradingMode::Live,
        )
    }

    #[test]
    fn unknown_after_dispatch_is_a_receipt_that_forbids_blind_retry()
    -> Result<(), Box<dyn std::error::Error>> {
        let receipt = MutationReceiptDto::unknown_after_dispatch(
            "req-1",
            sample_scope()?,
            MutationKindDto::PostOrder,
            "corr-1",
            7,
            1,
            2,
        );
        assert_eq!(receipt.state, JournalStateDto::UnknownAfterDispatch);
        assert_eq!(receipt.decision, MutationDecisionDto::Reconcile);
        assert_ne!(receipt.decision, MutationDecisionDto::Submit);
        let json = serde_json::to_value(&receipt)?;
        assert_eq!(json["state"], "UNKNOWN_AFTER_DISPATCH");
        assert_eq!(json["decision"], "RECONCILE");
        assert_eq!(json["logical_request_id"], "req-1");
        assert_eq!(json["scope"]["account_id"], "account:primary");
        assert_eq!(json["scope"]["broker_connection_id"], "connection:primary");
        assert!(
            json.get("code").is_none(),
            "UNKNOWN after dispatch must not be an ApiError envelope"
        );
        Ok(())
    }

    #[test]
    fn cancel_requires_exactly_one_target_identity()
    -> Result<(), crate::contract::scope::ScopeError> {
        let both = CancelOrderRequest {
            scope: sample_scope()?,
            client_request_id: "cancel-1".to_owned(),
            broker_order_id: Some("broker-1".to_owned()),
            logical_request_id: Some("req-1".to_owned()),
        };
        assert!(both.target().is_err(), "two targets are ambiguous");

        let neither = CancelOrderRequest {
            scope: sample_scope()?,
            client_request_id: "cancel-1".to_owned(),
            broker_order_id: None,
            logical_request_id: None,
        };
        assert!(neither.target().is_err());

        let broker = CancelOrderRequest {
            scope: sample_scope()?,
            client_request_id: "cancel-1".to_owned(),
            broker_order_id: Some("broker-1".to_owned()),
            logical_request_id: None,
        };
        assert_eq!(
            broker.target().ok(),
            Some(CancelTarget::BrokerOrder {
                broker_order_id: "broker-1".to_owned()
            })
        );
        Ok(())
    }
}
