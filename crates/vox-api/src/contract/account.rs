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
    BrokerAccount, MoneyFact, OperationFact, OperationsPage, OrderExecutionStatus, OrderFact,
    PortfolioFact, PositionFact, ReconciliationCheckpoint, StopExecutionStatus, StopFact,
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

/// One aggregate broker valuation, exact. Not a spendable cash balance.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct MoneyValuationDto {
    #[schema(example = "rub")]
    pub currency: String,
    pub amount: Decimal,
}

/// Portfolio aggregates and authoritative cash balances remain separate.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct PortfolioDto {
    pub account_id: String,
    pub total_portfolio_valuation: Option<MoneyValuationDto>,
    pub total_currency_valuation: Option<MoneyValuationDto>,
    /// Actual currency balances from GetPositions, never inferred from portfolio aggregates.
    pub balances: Vec<CurrencyBalanceDto>,
    pub broker_observed_at_unix_ms: Option<i64>,
}

impl PortfolioDto {
    pub fn from_bound_fact(
        binding: &AccountBinding,
        value: &PortfolioFact,
    ) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        let mut balances = Vec::with_capacity(value.cash_balances.len());
        for (currency, amount_nanos) in &value.cash_balances {
            balances.push(CurrencyBalanceDto {
                currency: currency.clone(),
                amount: decimal_from_nanos(amount_nanos, "cash balance", currency)?,
            });
        }
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            total_portfolio_valuation: value
                .total_portfolio_valuation
                .as_ref()
                .map(money_valuation)
                .transpose()?,
            total_currency_valuation: value
                .total_currency_valuation
                .as_ref()
                .map(money_valuation)
                .transpose()?,
            balances,
            broker_observed_at_unix_ms: value.broker_observed_at_unix_ms,
        })
    }
}

fn money_valuation(value: &MoneyFact) -> Result<MoneyValuationDto, ApiError> {
    Ok(MoneyValuationDto {
        currency: value.currency.clone(),
        amount: decimal_from_nanos(&value.amount_nanos, "valuation", &value.currency)?,
    })
}

fn decimal_from_nanos(value: &str, kind: &str, currency: &str) -> Result<Decimal, ApiError> {
    Decimal::from_total_nanos_string(value).map_err(|error| {
        ApiError::new(
            ErrorCategory::Internal,
            "INVALID_BROKER_DECIMAL",
            format!("{kind} for {currency} is not exact total nanos: {error}"),
        )
    })
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(
    tag = "status",
    content = "wire_value",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum OrderExecutionStatusDto {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    UnknownProviderStatus(i32),
}

impl From<OrderExecutionStatus> for OrderExecutionStatusDto {
    fn from(value: OrderExecutionStatus) -> Self {
        match value {
            OrderExecutionStatus::New => Self::New,
            OrderExecutionStatus::PartiallyFilled => Self::PartiallyFilled,
            OrderExecutionStatus::Filled => Self::Filled,
            OrderExecutionStatus::Cancelled => Self::Cancelled,
            OrderExecutionStatus::Rejected => Self::Rejected,
            OrderExecutionStatus::UnknownProviderStatus(wire) => Self::UnknownProviderStatus(wire),
        }
    }
}

/// Order identity and exact provider execution status.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OrderDto {
    pub account_id: String,
    pub broker_order_id: String,
    /// Vox-side identity of the command that created it, when it was ours.
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    pub status: OrderExecutionStatusDto,
    pub status_cause_code: Option<i32>,
}

impl OrderDto {
    pub fn from_bound_fact(binding: &AccountBinding, value: &OrderFact) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            broker_order_id: value.broker_order_id.clone(),
            logical_request_id: value.logical_request_id.clone(),
            instrument_uid: value.instrument_uid.clone(),
            status: value.status.into(),
            status_cause_code: value.status_cause.as_ref().map(|cause| cause.code),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(
    tag = "status",
    content = "wire_value",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum StopExecutionStatusDto {
    Active,
    Executed,
    Canceled,
    Expired,
    UnknownProviderStatus(i32),
}

impl From<StopExecutionStatus> for StopExecutionStatusDto {
    fn from(value: StopExecutionStatus) -> Self {
        match value {
            StopExecutionStatus::Active => Self::Active,
            StopExecutionStatus::Executed => Self::Executed,
            StopExecutionStatus::Canceled => Self::Canceled,
            StopExecutionStatus::Expired => Self::Expired,
            StopExecutionStatus::UnknownProviderStatus(wire) => Self::UnknownProviderStatus(wire),
        }
    }
}

/// Stop identity and exact provider status. Provider readback has no logical request identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct StopOrderDto {
    pub account_id: String,
    pub broker_stop_order_id: String,
    pub instrument_uid: String,
    pub status: StopExecutionStatusDto,
    pub status_cause_code: Option<i32>,
}

impl StopOrderDto {
    pub fn from_bound_fact(binding: &AccountBinding, value: &StopFact) -> Result<Self, ApiError> {
        require_broker_account(binding, &value.account_id)?;
        Ok(Self {
            account_id: binding.account_id().to_owned(),
            broker_stop_order_id: value.broker_stop_order_id.clone(),
            instrument_uid: value.instrument_uid.clone(),
            status: value.status.into(),
            status_cause_code: value.status_cause.as_ref().map(|cause| cause.code),
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vox_runtime::{ProviderStatusCause, StopExecutionStatus};

    use super::*;

    fn binding() -> Result<AccountBinding, crate::binding::BindingError> {
        AccountBinding::new("account:one", "connection:one", "broker-one")
    }

    #[test]
    fn portfolio_valuations_never_become_fake_cash_balances()
    -> Result<(), Box<dyn std::error::Error>> {
        let dto = PortfolioDto::from_bound_fact(
            &binding()?,
            &PortfolioFact {
                account_id: "broker-one".into(),
                total_portfolio_valuation: Some(MoneyFact {
                    currency: "RUB".into(),
                    amount_nanos: "100000000001".into(),
                }),
                total_currency_valuation: Some(MoneyFact {
                    currency: "RUB".into(),
                    amount_nanos: "25000000002".into(),
                }),
                cash_balances: BTreeMap::new(),
                broker_observed_at_unix_ms: Some(1),
            },
        )?;
        assert_eq!(
            dto.total_portfolio_valuation
                .as_ref()
                .map(|value| value.amount.as_str()),
            Some("100.000000001")
        );
        assert_eq!(
            dto.total_currency_valuation
                .as_ref()
                .map(|value| value.amount.as_str()),
            Some("25.000000002")
        );
        assert!(dto.balances.is_empty());
        Ok(())
    }

    #[test]
    fn public_status_projection_keeps_terminal_meaning_and_unknown_wire_value()
    -> Result<(), Box<dyn std::error::Error>> {
        let order = OrderDto::from_bound_fact(
            &binding()?,
            &OrderFact {
                account_id: "broker-one".into(),
                broker_order_id: "order-one".into(),
                logical_request_id: Some("request-one".into()),
                instrument_uid: "instrument-one".into(),
                status: OrderExecutionStatus::Rejected,
                status_cause: Some(ProviderStatusCause { code: 15 }),
            },
        )?;
        assert_eq!(order.status, OrderExecutionStatusDto::Rejected);
        assert_eq!(order.status_cause_code, Some(15));

        let stop = StopOrderDto::from_bound_fact(
            &binding()?,
            &StopFact {
                account_id: "broker-one".into(),
                broker_stop_order_id: "stop-one".into(),
                instrument_uid: "instrument-one".into(),
                status: StopExecutionStatus::UnknownProviderStatus(88_888),
                status_cause: None,
            },
        )?;
        assert_eq!(
            stop.status,
            StopExecutionStatusDto::UnknownProviderStatus(88_888)
        );
        Ok(())
    }
}
