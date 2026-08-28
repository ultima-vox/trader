//! Account read models.
//!
//! These carry exactly what the runtime read side knows today. Where a field a trading
//! screen wants does not exist in the contract — average price, current price, P&L, margin,
//! operation amounts — it is absent here rather than invented, and the capability set says
//! who owns it. See `docs/design/BACKEND_CONTRACTS.md`.

use super::money::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_runtime::{
    BrokerAccount, OperationFact, OperationsPage, OrderFact, PortfolioFact, PositionFact,
    ReconciliationCheckpoint, StopFact,
};

use crate::binding::AccountBinding;
use crate::error::{ApiError, ErrorCategory};

/// A broker account discovered through a connection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct BrokerAccountDto {
    /// Canonical Vox account/binding identity.
    pub account_id: String,
    /// Provider broker-account identifier. Metadata, not the capital-command target key.
    pub broker_account_id: String,
    pub open: bool,
    /// Whether the runtime can currently read this account.
    pub accessible: bool,
}

impl BrokerAccountDto {
    pub fn from_bound_fact(
        binding: &AccountBinding,
        value: &BrokerAccount,
    ) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            broker_account_id: binding.broker_account_id().to_owned(),
            open: value.open,
            accessible: value.accessible,
        })
    }
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

impl PortfolioDto {
    pub fn from_bound_fact(
        binding: &AccountBinding,
        value: &PortfolioFact,
    ) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        let mut balances = Vec::with_capacity(value.currencies.len());
        for (currency, amount) in &value.currencies {
            balances.push(CurrencyBalanceDto {
                currency: currency.clone(),
                amount: Decimal::from_exact_string(amount).map_err(|error| {
                    ApiError::new(
                        ErrorCategory::Internal,
                        "INVALID_BROKER_DECIMAL",
                        format!("portfolio amount for {currency} is not an exact decimal: {error}"),
                    )
                })?,
            });
        }
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            balances,
            broker_observed_at_unix_ms: value.broker_observed_at_unix_ms,
        })
    }
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

impl PositionDto {
    pub fn from_bound_fact(
        binding: &AccountBinding,
        value: &PositionFact,
    ) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            instrument_uid: value.instrument_uid.clone(),
            quantity_units: value.quantity_units,
            broker_observed_at_unix_ms: value.broker_observed_at_unix_ms,
        })
    }
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

impl OrderDto {
    pub fn from_bound_fact(binding: &AccountBinding, value: &OrderFact) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            broker_order_id: value.broker_order_id.clone(),
            logical_request_id: value.logical_request_id.clone(),
            instrument_uid: value.instrument_uid.clone(),
            active: value.active,
            terminal: value.terminal,
        })
    }
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

impl StopOrderDto {
    pub fn from_bound_fact(binding: &AccountBinding, value: &StopFact) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            broker_stop_order_id: value.broker_stop_order_id.clone(),
            logical_request_id: value.logical_request_id.clone(),
            instrument_uid: value.instrument_uid.clone(),
            active: value.active,
            terminal: value.terminal,
        })
    }
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

impl OperationDto {
    pub fn from_bound_fact(
        binding: &AccountBinding,
        value: &OperationFact,
    ) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            cursor: value.cursor.clone(),
            provider_operation_id: value.provider_operation_id.clone(),
            broker_order_id: value.broker_order_id.clone(),
            logical_request_id: value.logical_request_id.clone(),
            broker_fill_ids: value.broker_fill_ids.iter().cloned().collect(),
        })
    }
}

/// A page of operations with the cursor to continue from.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OperationsPageDto {
    pub items: Vec<OperationDto>,
    pub next_cursor: Option<String>,
}

impl OperationsPageDto {
    pub fn from_bound_page(
        binding: &AccountBinding,
        value: &OperationsPage,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            items: value
                .items
                .iter()
                .map(|item| OperationDto::from_bound_fact(binding, item))
                .collect::<Result<Vec<_>, _>>()?,
            next_cursor: value.next_cursor.clone(),
        })
    }
}

fn require_broker_account(binding: &AccountBinding, fact_account_id: &str) -> Result<(), ApiError> {
    if fact_account_id != binding.broker_account_id() {
        return Err(ApiError::new(
            ErrorCategory::Internal,
            "IDENTITY_MISMATCH",
            "broker fact account id does not match the resolved binding",
        ));
    }
    Ok(())
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

impl From<&ReconciliationCheckpoint> for ReconciliationDto {
    fn from(value: &ReconciliationCheckpoint) -> Self {
        Self {
            scope_key: value.scope_key.clone(),
            reconciliation_id: value.reconciliation_id.clone(),
            operations_cursor: value.operations_cursor.clone(),
            snapshot_observed_at_unix_ms: value.snapshot_observed_at_unix_ms,
            completed_at_unix_ms: value.completed_at_unix_ms,
            runtime_epoch: value.runtime_epoch,
            accounts_complete: value.accounts_complete,
            portfolio_complete: value.portfolio_complete,
            positions_complete: value.positions_complete,
            orders_complete: value.orders_complete,
            stops_complete: value.stops_complete,
            operations_complete: value.operations_complete,
            complete: value.complete(),
        }
    }
}
