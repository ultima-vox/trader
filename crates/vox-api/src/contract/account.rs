//! Account read models.
//!
//! These carry exactly what the runtime read side knows today. Where a field a trading
//! screen wants does not exist in the contract — average price, current price, P&L, margin,
//! operation amounts — it is absent here rather than invented, and the capability set says
//! who owns it. See `docs/design/BACKEND_CONTRACTS.md`.

use super::money::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A broker account discovered through a connection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct BrokerAccountDto {
    pub account_id: String,
    pub open: bool,
    /// Whether the runtime can currently read this account.
    pub accessible: bool,
}

/// One currency balance, exact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CurrencyBalanceDto {
    #[schema(example = "rub")]
    pub currency: String,
    pub amount: Decimal,
}

/// Portfolio as the broker reports it: currency balances and when they were observed.
///
/// Valuation, P&L, exposure and margin are **not** here. #22 owns them.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct PortfolioDto {
    pub account_id: String,
    pub balances: Vec<CurrencyBalanceDto>,
    pub broker_observed_at_unix_ms: Option<i64>,
}

/// A position as the broker reports it: instrument and quantity, nothing derived.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct PositionDto {
    pub account_id: String,
    pub instrument_uid: String,
    /// Signed quantity in instrument units.
    pub quantity_units: i64,
    pub broker_observed_at_unix_ms: Option<i64>,
}

/// An order identity and its liveness. Price and quantity are not in the read model yet.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OrderDto {
    pub account_id: String,
    pub broker_order_id: String,
    /// Vox-side identity of the command that created it, when it was ours.
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    pub active: bool,
    pub terminal: bool,
}

/// A stop order identity and its liveness. Trigger levels are not in the read model yet.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct StopOrderDto {
    pub account_id: String,
    pub broker_stop_order_id: String,
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    pub active: bool,
    pub terminal: bool,
}

/// An operation identity used for reconciliation. Amounts and kinds are owned by #22.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OperationDto {
    pub account_id: String,
    pub cursor: String,
    pub provider_operation_id: Option<String>,
    pub broker_order_id: Option<String>,
    pub logical_request_id: Option<String>,
    pub broker_fill_ids: Vec<String>,
}

/// A page of operations with the cursor to continue from.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OperationsPageDto {
    pub items: Vec<OperationDto>,
    pub next_cursor: Option<String>,
}

/// How complete the last reconciliation was, per domain.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct ReconciliationDto {
    pub scope_key: String,
    pub reconciliation_id: String,
    pub operations_cursor: Option<String>,
    pub snapshot_observed_at_unix_ms: i64,
    pub completed_at_unix_ms: i64,
    pub runtime_epoch: u64,
    pub accounts_complete: bool,
    pub portfolio_complete: bool,
    pub positions_complete: bool,
    pub orders_complete: bool,
    pub stops_complete: bool,
    pub operations_complete: bool,
    /// True only when every domain above is complete.
    pub complete: bool,
}
