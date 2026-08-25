//! Vox-owned execution boundary over generated T-Invest contracts.

use prost_types::Timestamp;
use thiserror::Error;
use uuid::Uuid;
use vox_domain::{
    CancelOrderCommand, CancelStopOrderCommand, FixedPoint, OrderSide, PositionSide,
    ProtectionCapability, ProtectionCapabilityError, ProtectionPlan, ProviderOrderIdentityKind,
    RegularOrderCommand, RegularOrderType, ReplaceOrderCommand, StopLossProtection,
    TakeProfitProtection, TimeInForce, TrailingDistance, TrailingDistanceMode, UnitsNano,
};

use crate::account::ProviderTimestamp;
use crate::canonical::CanonicalMoney;
use crate::generated::v1;

pub const ORDERS_SERVICE_METHODS: [&str; 8] = [
    "PostOrder",
    "PostOrderAsync",
    "CancelOrder",
    "GetOrderState",
    "GetOrders",
    "ReplaceOrder",
    "GetMaxLots",
    "GetOrderPrice",
];
pub const ORDERS_STREAM_METHODS: [&str; 2] = ["TradesStream", "OrderStateStream"];
pub const STOP_ORDERS_SERVICE_METHODS: [&str; 3] =
    ["PostStopOrder", "GetStopOrders", "CancelStopOrder"];
pub const EXECUTION_STREAM_PAYLOADS: [&str; 7] = [
    "TradesStream.OrderTrades",
    "TradesStream.Ping",
    "TradesStream.Subscription",
    "OrderStateStream.OrderState",
    "OrderStateStream.Ping",
    "OrderStateStream.Subscription",
    "OrderStateStream.StopOrderState",
];

pub const TINVEST_PROTECTION_CAPABILITY: ProtectionCapability = ProtectionCapability {
    fixed_stop: true,
    native_trailing_relative: true,
    native_trailing_absolute: true,
    take_profit: true,
    stop_limit: true,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOrderResult {
    pub broker_order_id: Option<String>,
    pub client_request_id: Option<String>,
    pub execution_status: i32,
    pub lots_requested: i64,
    pub lots_executed: i64,
    pub direction: i32,
    pub order_type: i32,
    pub instrument_uid: Option<String>,
    pub initial_order_price: Option<CanonicalMoney>,
    pub executed_order_price: Option<CanonicalMoney>,
    pub total_order_amount: Option<CanonicalMoney>,
    pub initial_commission: Option<CanonicalMoney>,
    pub executed_commission: Option<CanonicalMoney>,
    pub accrued_interest: Option<CanonicalMoney>,
    pub initial_security_price: Option<CanonicalMoney>,
    pub initial_order_price_points: Option<UnitsNano>,
    pub provider_message: Option<String>,
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub response_metadata: Option<CanonicalResponseMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalResponseMetadata {
    pub tracking_id: Option<String>,
    pub server_time: Option<ProviderTimestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalAsyncOrderResult {
    pub client_request_id: Option<String>,
    pub execution_status: i32,
    pub provider_operation_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalStopOrderResult {
    pub broker_stop_order_id: Option<String>,
    pub client_request_id: Option<String>,
    pub response_metadata: Option<CanonicalResponseMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalCancellation {
    pub cancelled_at: Option<ProviderTimestamp>,
    pub response_metadata: Option<CanonicalResponseMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalMaxLots {
    pub currency: Option<String>,
    pub buy: Option<CanonicalBuyLimits>,
    pub buy_with_margin: Option<CanonicalBuyLimits>,
    pub sell: Option<CanonicalSellLimits>,
    pub sell_with_margin: Option<CanonicalSellLimits>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalBuyLimits {
    pub available_money: Option<UnitsNano>,
    pub max_lots: i64,
    pub max_market_lots: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalSellLimits {
    pub max_lots: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOrderPrice {
    pub total_order_amount: Option<CanonicalMoney>,
    pub initial_order_amount: Option<CanonicalMoney>,
    pub lots_requested: i64,
    pub executed_commission: Option<CanonicalMoney>,
    pub executed_commission_rub: Option<CanonicalMoney>,
    pub service_commission: Option<CanonicalMoney>,
    pub deal_commission: Option<CanonicalMoney>,
    pub instrument_extra: Option<CanonicalOrderPriceExtra>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalOrderPriceExtra {
    Bond {
        accrued_interest: Option<CanonicalMoney>,
        nominal_conversion_rate: Option<UnitsNano>,
    },
    Future {
        initial_margin: Option<CanonicalMoney>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOrderState {
    pub broker_order_id: Option<String>,
    pub client_request_id: Option<String>,
    pub execution_status: i32,
    pub lots_requested: i64,
    pub lots_executed: i64,
    pub direction: i32,
    pub order_type: i32,
    pub stages: Vec<CanonicalOrderStage>,
    pub initial_order_price: Option<CanonicalMoney>,
    pub executed_order_price: Option<CanonicalMoney>,
    pub total_order_amount: Option<CanonicalMoney>,
    pub initial_commission: Option<CanonicalMoney>,
    pub executed_commission: Option<CanonicalMoney>,
    pub average_position_price: Option<CanonicalMoney>,
    pub initial_security_price: Option<CanonicalMoney>,
    pub service_commission: Option<CanonicalMoney>,
    pub currency: Option<String>,
    pub order_date: Option<ProviderTimestamp>,
    pub instrument_uid: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOrderStage {
    pub price: Option<CanonicalMoney>,
    pub quantity: i64,
    pub trade_id: Option<String>,
    pub execution_time: Option<ProviderTimestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalStopOrder {
    pub broker_stop_order_id: Option<String>,
    pub exchange_order_id: Option<String>,
    pub lots_requested: i64,
    pub direction: i32,
    pub stop_order_type: i32,
    pub take_profit_type: i32,
    pub status: i32,
    pub instrument_uid: Option<String>,
    pub figi: Option<String>,
    pub currency: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub exchange_order_type: i32,
    pub price: Option<CanonicalMoney>,
    pub stop_price: Option<CanonicalMoney>,
    pub created_at: Option<ProviderTimestamp>,
    pub activated_at: Option<ProviderTimestamp>,
    pub expires_at: Option<ProviderTimestamp>,
    pub trailing: Option<CanonicalTrailingState>,
    pub instant_execution: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTrailingState {
    pub indent: Option<UnitsNano>,
    pub indent_type: i32,
    pub spread: Option<UnitsNano>,
    pub spread_type: i32,
    pub status: i32,
    pub execution_price: Option<UnitsNano>,
    pub favorable_extreme: Option<UnitsNano>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalExecutionStreamEvent {
    Trades(CanonicalTradeBatch),
    OrderState(CanonicalStreamOrderState),
    StopOrderState(CanonicalStreamStopOrderState),
    Subscription {
        status: i32,
        tracking_id: Option<String>,
        stream_id: Option<String>,
        accounts: Vec<String>,
        provider_error_code: Option<String>,
    },
    Ping(Option<ProviderTimestamp>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTradeBatch {
    pub broker_order_id: Option<String>,
    pub created_at: Option<ProviderTimestamp>,
    pub direction: i32,
    pub account_id: Option<String>,
    pub instrument_uid: Option<String>,
    pub trades: Vec<CanonicalTrade>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTrade {
    pub occurred_at: Option<ProviderTimestamp>,
    pub price: Option<UnitsNano>,
    pub quantity: i64,
    pub broker_fill_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalStreamOrderState {
    pub broker_order_id: Option<String>,
    pub client_request_id: Option<String>,
    pub account_id: Option<String>,
    pub execution_status: i32,
    pub direction: i32,
    pub order_type: i32,
    pub lots_requested: i64,
    pub lots_executed: i64,
    pub lots_left: i64,
    pub lots_cancelled: i64,
    pub instrument_uid: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalStreamStopOrderState {
    pub broker_stop_order_id: Option<String>,
    pub account_id: Option<String>,
    pub direction: i32,
    pub order_type: i32,
    pub status: i32,
    pub instrument_uid: Option<String>,
    pub price: Option<CanonicalMoney>,
    pub stop_price: Option<CanonicalMoney>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectionRequestIds {
    pub stop_loss: Option<String>,
    pub take_profit: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectionRequestContext {
    pub account_id: String,
    pub instrument_id: String,
    pub quantity_lots: i64,
    pub position_side: PositionSide,
    pub expire_at: Option<ProviderTimestamp>,
    pub confirm_margin_trade: bool,
    pub request_ids: ProtectionRequestIds,
}

pub fn regular_order_request(
    command: &RegularOrderCommand,
) -> Result<v1::PostOrderRequest, ExecutionValidationError> {
    validate_text("account_id", &command.account_id)?;
    validate_text("instrument_id", &command.instrument_id)?;
    validate_request_id(&command.client_request_id)?;
    positive_quantity(command.quantity_lots)?;
    validate_regular_price(command.order_type, command.price)?;
    if command.order_type != RegularOrderType::Limit && command.time_in_force.is_some() {
        return Err(ExecutionValidationError::TimeInForceRequiresLimit);
    }
    Ok(v1::PostOrderRequest {
        quantity: command.quantity_lots,
        price: command.price.map(quotation).transpose()?,
        direction: order_direction(command.side),
        account_id: command.account_id.clone(),
        order_type: order_type(command.order_type),
        order_id: command.client_request_id.clone(),
        instrument_id: command.instrument_id.clone(),
        time_in_force: command.time_in_force.map_or(0, time_in_force),
        price_type: v1::PriceType::Unspecified as i32,
        confirm_margin_trade: command.confirm_margin_trade,
        ..Default::default()
    })
}

pub fn async_regular_order_request(
    command: &RegularOrderCommand,
) -> Result<v1::PostOrderAsyncRequest, ExecutionValidationError> {
    let validated = regular_order_request(command)?;
    Ok(v1::PostOrderAsyncRequest {
        instrument_id: validated.instrument_id,
        quantity: validated.quantity,
        price: validated.price,
        direction: validated.direction,
        account_id: validated.account_id,
        order_type: validated.order_type,
        order_id: validated.order_id,
        time_in_force: command.time_in_force.map(time_in_force),
        price_type: Some(v1::PriceType::Unspecified as i32),
        confirm_margin_trade: validated.confirm_margin_trade,
    })
}

pub fn replace_order_request(
    command: &ReplaceOrderCommand,
) -> Result<v1::ReplaceOrderRequest, ExecutionValidationError> {
    validate_text("account_id", &command.account_id)?;
    validate_text("existing_order_id", &command.existing_order_id)?;
    validate_request_id(&command.replacement_request_id)?;
    if matches!(
        command.existing_order_id_kind,
        Some(ProviderOrderIdentityKind::ClientRequest)
    ) {
        validate_request_id(&command.existing_order_id)?;
    }
    positive_quantity(command.quantity_lots)?;
    Ok(v1::ReplaceOrderRequest {
        account_id: command.account_id.clone(),
        order_id_type: command.existing_order_id_kind.map(order_id_type),
        order_id: command.existing_order_id.clone(),
        idempotency_key: command.replacement_request_id.clone(),
        quantity: command.quantity_lots,
        price: Some(quotation(command.price)?),
        price_type: Some(v1::PriceType::Unspecified as i32),
        confirm_margin_trade: command.confirm_margin_trade,
    })
}

pub fn cancel_order_request(
    command: &CancelOrderCommand,
) -> Result<v1::CancelOrderRequest, ExecutionValidationError> {
    validate_text("account_id", &command.account_id)?;
    validate_text("order_id", &command.order_id)?;
    if matches!(
        command.order_id_kind,
        Some(ProviderOrderIdentityKind::ClientRequest)
    ) {
        validate_request_id(&command.order_id)?;
    }
    Ok(v1::CancelOrderRequest {
        account_id: command.account_id.clone(),
        order_id: command.order_id.clone(),
        order_id_type: command.order_id_kind.map(order_id_type),
    })
}

pub fn cancel_stop_order_request(
    command: &CancelStopOrderCommand,
) -> Result<v1::CancelStopOrderRequest, ExecutionValidationError> {
    validate_text("account_id", &command.account_id)?;
    validate_text("broker_stop_order_id", &command.broker_stop_order_id)?;
    Ok(v1::CancelStopOrderRequest {
        account_id: command.account_id.clone(),
        stop_order_id: command.broker_stop_order_id.clone(),
    })
}

pub fn protection_requests(
    plan: &ProtectionPlan,
    context: &ProtectionRequestContext,
) -> Result<Vec<v1::PostStopOrderRequest>, ExecutionValidationError> {
    TINVEST_PROTECTION_CAPABILITY.validate(plan)?;
    validate_text("account_id", &context.account_id)?;
    validate_text("instrument_id", &context.instrument_id)?;
    positive_quantity(context.quantity_lots)?;
    let (expiration_type, expire_date) = match context.expire_at {
        Some(value) if (0..1_000_000_000).contains(&value.nanos) => (
            v1::StopOrderExpirationType::GoodTillDate as i32,
            Some(Timestamp {
                seconds: value.seconds,
                nanos: value.nanos,
            }),
        ),
        Some(_) => return Err(ExecutionValidationError::Timestamp),
        None => (v1::StopOrderExpirationType::GoodTillCancel as i32, None),
    };
    let direction = match context.position_side {
        PositionSide::Long => v1::StopOrderDirection::Sell as i32,
        PositionSide::Short => v1::StopOrderDirection::Buy as i32,
    };
    let mut requests = Vec::new();
    if let Some(stop_loss) = &plan.stop_loss {
        let request_id = context.request_ids.stop_loss.as_deref().ok_or(
            ExecutionValidationError::MissingProtectionRequestId("stop_loss"),
        )?;
        validate_request_id(request_id)?;
        let (
            price,
            stop_price,
            stop_order_type,
            exchange_order_type,
            take_profit_type,
            trailing_data,
            instant_execution,
        ) = match stop_loss {
            StopLossProtection::Fixed {
                trigger_price,
                limit_price,
            } => (
                limit_price.map(quotation).transpose()?,
                Some(quotation(*trigger_price)?),
                if limit_price.is_some() {
                    v1::StopOrderType::StopLimit as i32
                } else {
                    v1::StopOrderType::StopLoss as i32
                },
                if limit_price.is_some() {
                    v1::ExchangeOrderType::Limit as i32
                } else {
                    v1::ExchangeOrderType::Market as i32
                },
                v1::TakeProfitType::Unspecified as i32,
                None,
                None,
            ),
            StopLossProtection::Trailing {
                distance,
                activation_price,
                protective_spread,
                instant_execution,
            } => {
                if activation_price.is_none() && *instant_execution != Some(true) {
                    return Err(ExecutionValidationError::TrailingActivationRequired);
                }
                (
                    None,
                    activation_price.map(quotation).transpose()?,
                    v1::StopOrderType::TakeProfit as i32,
                    v1::ExchangeOrderType::Market as i32,
                    v1::TakeProfitType::Trailing as i32,
                    Some(trailing_request(*distance, *protective_spread)?),
                    *instant_execution,
                )
            }
        };
        requests.push(v1::PostStopOrderRequest {
            quantity: context.quantity_lots,
            price,
            stop_price,
            direction,
            account_id: context.account_id.clone(),
            expiration_type,
            stop_order_type,
            expire_date,
            instrument_id: context.instrument_id.clone(),
            exchange_order_type,
            take_profit_type,
            trailing_data,
            price_type: v1::PriceType::Unspecified as i32,
            order_id: request_id.to_owned(),
            confirm_margin_trade: context.confirm_margin_trade,
            instant_execution,
            ..Default::default()
        });
    }
    if let Some(take_profit) = &plan.take_profit {
        let request_id = context.request_ids.take_profit.as_deref().ok_or(
            ExecutionValidationError::MissingProtectionRequestId("take_profit"),
        )?;
        validate_request_id(request_id)?;
        requests.push(take_profit_request(
            take_profit,
            context,
            request_id,
            direction,
            expiration_type,
            expire_date,
        )?);
    }
    Ok(requests)
}

fn take_profit_request(
    protection: &TakeProfitProtection,
    context: &ProtectionRequestContext,
    request_id: &str,
    direction: i32,
    expiration_type: i32,
    expire_date: Option<Timestamp>,
) -> Result<v1::PostStopOrderRequest, ExecutionValidationError> {
    if protection.trailing.is_none() && protection.trigger_price.is_none() {
        return Err(ExecutionValidationError::MissingTakeProfitTrigger);
    }
    let trailing_data = protection
        .trailing
        .map(|distance| trailing_request(distance, None))
        .transpose()?;
    Ok(v1::PostStopOrderRequest {
        quantity: context.quantity_lots,
        price: protection.limit_price.map(quotation).transpose()?,
        stop_price: protection.trigger_price.map(quotation).transpose()?,
        direction,
        account_id: context.account_id.clone(),
        expiration_type,
        stop_order_type: v1::StopOrderType::TakeProfit as i32,
        expire_date,
        instrument_id: context.instrument_id.clone(),
        exchange_order_type: if protection.limit_price.is_some() {
            v1::ExchangeOrderType::Limit as i32
        } else {
            v1::ExchangeOrderType::Market as i32
        },
        take_profit_type: if protection.trailing.is_some() {
            v1::TakeProfitType::Trailing as i32
        } else {
            v1::TakeProfitType::Regular as i32
        },
        trailing_data,
        price_type: v1::PriceType::Unspecified as i32,
        order_id: request_id.to_owned(),
        confirm_margin_trade: context.confirm_margin_trade,
        instant_execution: None,
        ..Default::default()
    })
}

fn trailing_request(
    distance: TrailingDistance,
    spread: Option<TrailingDistance>,
) -> Result<v1::post_stop_order_request::TrailingData, ExecutionValidationError> {
    positive_fixed("trailing distance", distance.value)?;
    if let Some(spread) = spread {
        positive_fixed("trailing spread", spread.value)?;
    }
    Ok(v1::post_stop_order_request::TrailingData {
        indent: Some(quotation(distance.value)?),
        indent_type: trailing_mode(distance.mode),
        spread: spread.map(|value| quotation(value.value)).transpose()?,
        spread_type: spread.map_or(0, |value| trailing_mode(value.mode)),
    })
}

impl TryFrom<v1::PostOrderResponse> for CanonicalOrderResult {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::PostOrderResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            broker_order_id: optional_text(value.order_id),
            client_request_id: optional_text(value.order_request_id),
            execution_status: value.execution_report_status,
            lots_requested: value.lots_requested,
            lots_executed: value.lots_executed,
            direction: value.direction,
            order_type: value.order_type,
            instrument_uid: optional_text(value.instrument_uid),
            initial_order_price: money(value.initial_order_price)?,
            executed_order_price: money(value.executed_order_price)?,
            total_order_amount: money(value.total_order_amount)?,
            initial_commission: money(value.initial_commission)?,
            executed_commission: money(value.executed_commission)?,
            accrued_interest: money(value.aci_value)?,
            initial_security_price: money(value.initial_security_price)?,
            initial_order_price_points: units_nano(value.initial_order_price_pt)?,
            provider_message: optional_text(value.message),
            figi: optional_text(value.figi),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
            response_metadata: value.response_metadata.map(response_metadata).transpose()?,
        })
    }
}

impl From<v1::PostOrderAsyncResponse> for CanonicalAsyncOrderResult {
    fn from(value: v1::PostOrderAsyncResponse) -> Self {
        Self {
            client_request_id: optional_text(value.order_request_id),
            execution_status: value.execution_report_status,
            provider_operation_id: value
                .trade_intent_id
                .filter(|value| !value.trim().is_empty()),
        }
    }
}

impl TryFrom<v1::PostStopOrderResponse> for CanonicalStopOrderResult {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::PostStopOrderResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            broker_stop_order_id: optional_text(value.stop_order_id),
            client_request_id: optional_text(value.order_request_id),
            response_metadata: value.response_metadata.map(response_metadata).transpose()?,
        })
    }
}

impl TryFrom<v1::CancelOrderResponse> for CanonicalCancellation {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::CancelOrderResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            cancelled_at: timestamp(value.time)?,
            response_metadata: value.response_metadata.map(response_metadata).transpose()?,
        })
    }
}

impl TryFrom<v1::CancelStopOrderResponse> for CanonicalCancellation {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::CancelStopOrderResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            cancelled_at: timestamp(value.time)?,
            response_metadata: None,
        })
    }
}

impl TryFrom<v1::GetMaxLotsResponse> for CanonicalMaxLots {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::GetMaxLotsResponse) -> Result<Self, Self::Error> {
        fn buy(
            value: v1::get_max_lots_response::BuyLimitsView,
        ) -> Result<CanonicalBuyLimits, ExecutionDecodeError> {
            Ok(CanonicalBuyLimits {
                available_money: units_nano(value.buy_money_amount)?,
                max_lots: value.buy_max_lots,
                max_market_lots: value.buy_max_market_lots,
            })
        }
        fn sell(value: v1::get_max_lots_response::SellLimitsView) -> CanonicalSellLimits {
            CanonicalSellLimits {
                max_lots: value.sell_max_lots,
            }
        }
        Ok(Self {
            currency: optional_text(value.currency),
            buy: value.buy_limits.map(buy).transpose()?,
            buy_with_margin: value.buy_margin_limits.map(buy).transpose()?,
            sell: value.sell_limits.map(sell),
            sell_with_margin: value.sell_margin_limits.map(sell),
        })
    }
}

impl TryFrom<v1::GetOrderPriceResponse> for CanonicalOrderPrice {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::GetOrderPriceResponse) -> Result<Self, Self::Error> {
        use v1::get_order_price_response::InstrumentExtra;
        let instrument_extra = match value.instrument_extra {
            Some(InstrumentExtra::ExtraBond(extra)) => Some(CanonicalOrderPriceExtra::Bond {
                accrued_interest: money(extra.aci_value)?,
                nominal_conversion_rate: units_nano(extra.nominal_conversion_rate)?,
            }),
            Some(InstrumentExtra::ExtraFuture(extra)) => Some(CanonicalOrderPriceExtra::Future {
                initial_margin: money(extra.initial_margin)?,
            }),
            None => None,
        };
        Ok(Self {
            total_order_amount: money(value.total_order_amount)?,
            initial_order_amount: money(value.initial_order_amount)?,
            lots_requested: value.lots_requested,
            executed_commission: money(value.executed_commission)?,
            executed_commission_rub: money(value.executed_commission_rub)?,
            service_commission: money(value.service_commission)?,
            deal_commission: money(value.deal_commission)?,
            instrument_extra,
        })
    }
}

pub fn canonical_orders(
    value: v1::GetOrdersResponse,
) -> Result<Vec<CanonicalOrderState>, ExecutionDecodeError> {
    value.orders.into_iter().map(TryInto::try_into).collect()
}

pub fn canonical_stop_orders(
    value: v1::GetStopOrdersResponse,
) -> Result<Vec<CanonicalStopOrder>, ExecutionDecodeError> {
    value
        .stop_orders
        .into_iter()
        .map(TryInto::try_into)
        .collect()
}

impl TryFrom<v1::OrderState> for CanonicalOrderState {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::OrderState) -> Result<Self, Self::Error> {
        Ok(Self {
            broker_order_id: optional_text(value.order_id),
            client_request_id: optional_text(value.order_request_id),
            execution_status: value.execution_report_status,
            lots_requested: value.lots_requested,
            lots_executed: value.lots_executed,
            direction: value.direction,
            order_type: value.order_type,
            stages: value
                .stages
                .into_iter()
                .map(|stage| {
                    Ok(CanonicalOrderStage {
                        price: money(stage.price)?,
                        quantity: stage.quantity,
                        trade_id: optional_text(stage.trade_id),
                        execution_time: timestamp(stage.execution_time)?,
                    })
                })
                .collect::<Result<_, ExecutionDecodeError>>()?,
            initial_order_price: money(value.initial_order_price)?,
            executed_order_price: money(value.executed_order_price)?,
            total_order_amount: money(value.total_order_amount)?,
            initial_commission: money(value.initial_commission)?,
            executed_commission: money(value.executed_commission)?,
            average_position_price: money(value.average_position_price)?,
            initial_security_price: money(value.initial_security_price)?,
            service_commission: money(value.service_commission)?,
            currency: optional_text(value.currency),
            order_date: timestamp(value.order_date)?,
            instrument_uid: optional_text(value.instrument_uid),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
        })
    }
}

impl TryFrom<v1::StopOrder> for CanonicalStopOrder {
    type Error = ExecutionDecodeError;
    fn try_from(value: v1::StopOrder) -> Result<Self, Self::Error> {
        Ok(Self {
            broker_stop_order_id: optional_text(value.stop_order_id),
            exchange_order_id: value
                .exchange_order_id
                .filter(|value| !value.trim().is_empty()),
            lots_requested: value.lots_requested,
            direction: value.direction,
            stop_order_type: value.order_type,
            take_profit_type: value.take_profit_type,
            status: value.status,
            instrument_uid: optional_text(value.instrument_uid),
            figi: optional_text(value.figi),
            currency: optional_text(value.currency),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
            exchange_order_type: value.exchange_order_type,
            price: money(value.price)?,
            stop_price: money(value.stop_price)?,
            created_at: timestamp(value.create_date)?,
            activated_at: timestamp(value.activation_date_time)?,
            expires_at: timestamp(value.expiration_time)?,
            trailing: value.trailing_data.map(trailing_state).transpose()?,
            instant_execution: value.instant_execution,
        })
    }
}

fn response_metadata(
    value: v1::ResponseMetadata,
) -> Result<CanonicalResponseMetadata, ExecutionDecodeError> {
    Ok(CanonicalResponseMetadata {
        tracking_id: optional_text(value.tracking_id),
        server_time: timestamp(value.server_time)?,
    })
}

pub fn decode_trades_stream(
    response: v1::TradesStreamResponse,
) -> Result<CanonicalExecutionStreamEvent, ExecutionDecodeError> {
    use v1::trades_stream_response::Payload;
    match response
        .payload
        .ok_or(ExecutionDecodeError::MissingPayload)?
    {
        Payload::OrderTrades(value) => {
            Ok(CanonicalExecutionStreamEvent::Trades(trade_batch(value)?))
        }
        Payload::Ping(value) => Ok(CanonicalExecutionStreamEvent::Ping(timestamp(value.time)?)),
        Payload::Subscription(value) => Ok(subscription_event(value)),
    }
}

pub fn decode_order_state_stream(
    response: v1::OrderStateStreamResponse,
) -> Result<CanonicalExecutionStreamEvent, ExecutionDecodeError> {
    use v1::order_state_stream_response::Payload;
    match response
        .payload
        .ok_or(ExecutionDecodeError::MissingPayload)?
    {
        Payload::OrderState(value) => Ok(CanonicalExecutionStreamEvent::OrderState(
            stream_order_state(value),
        )),
        Payload::StopOrderState(value) => Ok(CanonicalExecutionStreamEvent::StopOrderState(
            stream_stop_order_state(value)?,
        )),
        Payload::Ping(value) => Ok(CanonicalExecutionStreamEvent::Ping(timestamp(value.time)?)),
        Payload::Subscription(value) => Ok(subscription_event(value)),
    }
}

fn trade_batch(value: v1::OrderTrades) -> Result<CanonicalTradeBatch, ExecutionDecodeError> {
    Ok(CanonicalTradeBatch {
        broker_order_id: optional_text(value.order_id),
        created_at: timestamp(value.created_at)?,
        direction: value.direction,
        account_id: optional_text(value.account_id),
        instrument_uid: optional_text(value.instrument_uid),
        trades: value
            .trades
            .into_iter()
            .map(|trade| {
                Ok(CanonicalTrade {
                    occurred_at: timestamp(trade.date_time)?,
                    price: units_nano(trade.price)?,
                    quantity: trade.quantity,
                    broker_fill_id: optional_text(trade.trade_id),
                })
            })
            .collect::<Result<_, ExecutionDecodeError>>()?,
    })
}

fn stream_order_state(
    value: v1::order_state_stream_response::OrderState,
) -> CanonicalStreamOrderState {
    CanonicalStreamOrderState {
        broker_order_id: optional_text(value.order_id),
        client_request_id: value
            .order_request_id
            .filter(|value| !value.trim().is_empty()),
        account_id: optional_text(value.account_id),
        execution_status: value.execution_report_status,
        direction: value.direction,
        order_type: value.order_type,
        lots_requested: value.lots_requested,
        lots_executed: value.lots_executed,
        lots_left: value.lots_left,
        lots_cancelled: value.lots_cancelled,
        instrument_uid: optional_text(value.instrument_uid),
    }
}

fn stream_stop_order_state(
    value: v1::order_state_stream_response::StopOrderState,
) -> Result<CanonicalStreamStopOrderState, ExecutionDecodeError> {
    Ok(CanonicalStreamStopOrderState {
        broker_stop_order_id: optional_text(value.stop_order_id),
        account_id: optional_text(value.account_id),
        direction: value.direction,
        order_type: value.order_type,
        status: value.status,
        instrument_uid: optional_text(value.instrument_uid),
        price: money(value.price)?,
        stop_price: money(value.stop_price)?,
    })
}

fn subscription_event(value: v1::SubscriptionResponse) -> CanonicalExecutionStreamEvent {
    CanonicalExecutionStreamEvent::Subscription {
        status: value.status,
        tracking_id: optional_text(value.tracking_id),
        stream_id: optional_text(value.stream_id),
        accounts: value.accounts,
        provider_error_code: value.error.and_then(|error| optional_text(error.code)),
    }
}

fn trailing_state(
    value: v1::stop_order::TrailingData,
) -> Result<CanonicalTrailingState, ExecutionDecodeError> {
    Ok(CanonicalTrailingState {
        indent: units_nano(value.indent)?,
        indent_type: value.indent_type,
        spread: units_nano(value.spread)?,
        spread_type: value.spread_type,
        status: value.status,
        execution_price: units_nano(value.price)?,
        favorable_extreme: units_nano(value.extr)?,
    })
}

fn validate_regular_price(
    order_type: RegularOrderType,
    price: Option<FixedPoint>,
) -> Result<(), ExecutionValidationError> {
    match (order_type, price) {
        (RegularOrderType::Limit, None) => Err(ExecutionValidationError::LimitPriceRequired),
        (RegularOrderType::Market | RegularOrderType::BestPrice, Some(_)) => {
            Err(ExecutionValidationError::PriceForbidden)
        }
        (_, Some(price)) => positive_fixed("price", price),
        _ => Ok(()),
    }
}
fn validate_text(field: &'static str, value: &str) -> Result<(), ExecutionValidationError> {
    if value.trim().is_empty() {
        Err(ExecutionValidationError::Missing(field))
    } else {
        Ok(())
    }
}
fn validate_request_id(value: &str) -> Result<(), ExecutionValidationError> {
    if value.len() > 36 || Uuid::parse_str(value).is_err() {
        Err(ExecutionValidationError::InvalidRequestId)
    } else {
        Ok(())
    }
}
fn positive_quantity(value: i64) -> Result<(), ExecutionValidationError> {
    if value > 0 {
        Ok(())
    } else {
        Err(ExecutionValidationError::NonPositiveQuantity)
    }
}
fn positive_fixed(field: &'static str, value: FixedPoint) -> Result<(), ExecutionValidationError> {
    if value.total_nanos() > 0 {
        Ok(())
    } else {
        Err(ExecutionValidationError::NonPositive(field))
    }
}
fn quotation(value: FixedPoint) -> Result<v1::Quotation, ExecutionValidationError> {
    positive_fixed("price/distance", value)?;
    let (units, nano) = value.units_nano();
    Ok(v1::Quotation {
        units: i64::try_from(units).map_err(|_| ExecutionValidationError::NumericOverflow)?,
        nano,
    })
}
fn money(value: Option<v1::MoneyValue>) -> Result<Option<CanonicalMoney>, ExecutionDecodeError> {
    value
        .map(TryInto::try_into)
        .transpose()
        .map_err(|_| ExecutionDecodeError::InvalidEconomics)
}
fn units_nano(value: Option<v1::Quotation>) -> Result<Option<UnitsNano>, ExecutionDecodeError> {
    value
        .map(|value| UnitsNano::new(value.units, value.nano))
        .transpose()
        .map_err(|_| ExecutionDecodeError::InvalidEconomics)
}
fn timestamp(value: Option<Timestamp>) -> Result<Option<ProviderTimestamp>, ExecutionDecodeError> {
    value
        .map(TryInto::try_into)
        .transpose()
        .map_err(|_| ExecutionDecodeError::InvalidTimestamp)
}
fn optional_text(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
const fn order_direction(value: OrderSide) -> i32 {
    match value {
        OrderSide::Buy => v1::OrderDirection::Buy as i32,
        OrderSide::Sell => v1::OrderDirection::Sell as i32,
    }
}
const fn order_type(value: RegularOrderType) -> i32 {
    match value {
        RegularOrderType::Limit => v1::OrderType::Limit as i32,
        RegularOrderType::Market => v1::OrderType::Market as i32,
        RegularOrderType::BestPrice => v1::OrderType::Bestprice as i32,
    }
}
const fn time_in_force(value: TimeInForce) -> i32 {
    match value {
        TimeInForce::Day => v1::TimeInForceType::TimeInForceDay as i32,
        TimeInForce::FillAndKill => v1::TimeInForceType::TimeInForceFillAndKill as i32,
        TimeInForce::FillOrKill => v1::TimeInForceType::TimeInForceFillOrKill as i32,
    }
}
const fn trailing_mode(value: TrailingDistanceMode) -> i32 {
    match value {
        TrailingDistanceMode::AbsolutePrice => v1::TrailingValueType::TrailingValueAbsolute as i32,
        TrailingDistanceMode::RelativePercent => {
            v1::TrailingValueType::TrailingValueRelative as i32
        }
    }
}

const fn order_id_type(value: ProviderOrderIdentityKind) -> i32 {
    match value {
        ProviderOrderIdentityKind::BrokerOrder => v1::OrderIdType::Exchange as i32,
        ProviderOrderIdentityKind::ClientRequest => v1::OrderIdType::Request as i32,
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionValidationError {
    #[error("missing required execution field: {0}")]
    Missing(&'static str),
    #[error("quantity lots must be positive")]
    NonPositiveQuantity,
    #[error("{0} must be positive")]
    NonPositive(&'static str),
    #[error("limit order requires price")]
    LimitPriceRequired,
    #[error("market/best-price order must not carry price")]
    PriceForbidden,
    #[error("time-in-force is valid only for limit orders")]
    TimeInForceRequiresLimit,
    #[error("provider idempotency ID must be UUID and at most 36 characters")]
    InvalidRequestId,
    #[error("protection leg missing distinct request ID: {0}")]
    MissingProtectionRequestId(&'static str),
    #[error("regular take-profit requires trigger price")]
    MissingTakeProfitTrigger,
    #[error("trailing stop requires activation price or instant execution")]
    TrailingActivationRequired,
    #[error("provider timestamp is invalid")]
    Timestamp,
    #[error("exact value exceeds provider int64 units")]
    NumericOverflow,
    #[error("{0}")]
    Capability(#[from] ProtectionCapabilityError),
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ExecutionDecodeError {
    #[error("execution stream response omitted payload")]
    MissingPayload,
    #[error("provider execution economics invalid")]
    InvalidEconomics,
    #[error("provider execution timestamp invalid")]
    InvalidTimestamp,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    fn fp(units: i64) -> FixedPoint {
        FixedPoint::from_units_nano(units, 0).expect("valid")
    }
    fn command(kind: RegularOrderType, price: Option<FixedPoint>) -> RegularOrderCommand {
        RegularOrderCommand {
            account_id: "account".into(),
            instrument_id: "instrument".into(),
            client_request_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            quantity_lots: 1,
            price,
            side: OrderSide::Buy,
            order_type: kind,
            time_in_force: (kind == RegularOrderType::Limit).then_some(TimeInForce::Day),
            confirm_margin_trade: false,
        }
    }

    #[test]
    fn regular_order_constraints_fail_before_dispatch() {
        assert_eq!(
            regular_order_request(&command(RegularOrderType::Limit, None)),
            Err(ExecutionValidationError::LimitPriceRequired)
        );
        assert_eq!(
            regular_order_request(&command(RegularOrderType::Market, Some(fp(10)))),
            Err(ExecutionValidationError::PriceForbidden)
        );
        let request = regular_order_request(&command(RegularOrderType::Limit, Some(fp(10))))
            .expect("valid limit");
        assert_eq!(request.quantity, 1);
        assert_eq!(request.instrument_id, "instrument");
    }

    #[test]
    fn protection_combinations_encode_independent_native_legs() {
        let plan = ProtectionPlan {
            stop_loss: Some(StopLossProtection::Trailing {
                distance: TrailingDistance {
                    value: fp(5),
                    mode: TrailingDistanceMode::RelativePercent,
                },
                activation_price: None,
                protective_spread: None,
                instant_execution: Some(true),
            }),
            take_profit: Some(TakeProfitProtection {
                trigger_price: Some(fp(120)),
                limit_price: None,
                trailing: None,
            }),
        };
        let requests = protection_requests(
            &plan,
            &ProtectionRequestContext {
                account_id: "account".into(),
                instrument_id: "instrument".into(),
                quantity_lots: 2,
                position_side: PositionSide::Long,
                expire_at: None,
                confirm_margin_trade: false,
                request_ids: ProtectionRequestIds {
                    stop_loss: Some("550e8400-e29b-41d4-a716-446655440000".into()),
                    take_profit: Some("550e8400-e29b-41d4-a716-446655440001".into()),
                },
            },
        )
        .expect("native protection");
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].take_profit_type,
            v1::TakeProfitType::Trailing as i32
        );
        assert!(requests[0].trailing_data.is_some());
        assert_eq!(
            requests[1].take_profit_type,
            v1::TakeProfitType::Regular as i32
        );

        let context = ProtectionRequestContext {
            account_id: "account".into(),
            instrument_id: "instrument".into(),
            quantity_lots: 1,
            position_side: PositionSide::Short,
            expire_at: None,
            confirm_margin_trade: false,
            request_ids: ProtectionRequestIds {
                stop_loss: Some("550e8400-e29b-41d4-a716-446655440002".into()),
                take_profit: Some("550e8400-e29b-41d4-a716-446655440003".into()),
            },
        };
        let fixed = protection_requests(
            &ProtectionPlan {
                stop_loss: Some(StopLossProtection::Fixed {
                    trigger_price: fp(105),
                    limit_price: Some(fp(106)),
                }),
                take_profit: None,
            },
            &context,
        )
        .expect("fixed only");
        assert_eq!(fixed.len(), 1);
        assert_eq!(fixed[0].direction, v1::StopOrderDirection::Buy as i32);
        assert_eq!(
            fixed[0].stop_order_type,
            v1::StopOrderType::StopLimit as i32
        );
        assert_eq!(
            fixed[0].take_profit_type,
            v1::TakeProfitType::Unspecified as i32
        );

        let take_profit = protection_requests(
            &ProtectionPlan {
                stop_loss: None,
                take_profit: Some(TakeProfitProtection {
                    trigger_price: Some(fp(90)),
                    limit_price: None,
                    trailing: None,
                }),
            },
            &context,
        )
        .expect("take profit only");
        assert_eq!(take_profit.len(), 1);
        assert_eq!(
            take_profit[0].stop_order_type,
            v1::StopOrderType::TakeProfit as i32
        );

        let invalid_trailing = protection_requests(
            &ProtectionPlan {
                stop_loss: Some(StopLossProtection::Trailing {
                    distance: TrailingDistance {
                        value: fp(1),
                        mode: TrailingDistanceMode::RelativePercent,
                    },
                    activation_price: None,
                    protective_spread: None,
                    instant_execution: Some(false),
                }),
                take_profit: None,
            },
            &context,
        );
        assert_eq!(
            invalid_trailing,
            Err(ExecutionValidationError::TrailingActivationRequired)
        );
    }

    #[test]
    fn replace_cancel_and_async_constraints_validate_before_dispatch() {
        let async_request =
            async_regular_order_request(&command(RegularOrderType::Limit, Some(fp(10))))
                .expect("async request");
        assert_eq!(
            async_request.time_in_force,
            Some(v1::TimeInForceType::TimeInForceDay as i32)
        );
        let replacement = ReplaceOrderCommand {
            account_id: "account".into(),
            existing_order_id: "broker-order".into(),
            existing_order_id_kind: Some(ProviderOrderIdentityKind::BrokerOrder),
            replacement_request_id: "550e8400-e29b-41d4-a716-446655440004".into(),
            quantity_lots: 2,
            price: fp(11),
            confirm_margin_trade: false,
        };
        let request = replace_order_request(&replacement).expect("replace");
        assert_eq!(
            request.order_id_type,
            Some(v1::OrderIdType::Exchange as i32)
        );
        let cancel = cancel_order_request(&CancelOrderCommand {
            account_id: "account".into(),
            order_id: "550e8400-e29b-41d4-a716-446655440005".into(),
            order_id_kind: Some(ProviderOrderIdentityKind::ClientRequest),
        })
        .expect("cancel");
        assert_eq!(cancel.order_id_type, Some(v1::OrderIdType::Request as i32));
    }

    #[test]
    fn generated_optionality_and_unknown_status_survive() {
        let wire = v1::PostOrderResponse {
            execution_report_status: 77_777,
            ..Default::default()
        };
        let decoded = v1::PostOrderResponse::decode(wire.encode_to_vec().as_slice())
            .expect("generated round trip");
        let canonical = CanonicalOrderResult::try_from(decoded).expect("canonical decode");
        assert_eq!(canonical.execution_status, 77_777);
        assert!(canonical.initial_commission.is_none());

        let limits =
            CanonicalMaxLots::try_from(v1::GetMaxLotsResponse::default()).expect("optional limits");
        assert_eq!(limits.buy, None);
        assert_eq!(limits.sell, None);
        let estimate = CanonicalOrderPrice::try_from(v1::GetOrderPriceResponse::default())
            .expect("optional estimate");
        assert_eq!(estimate.instrument_extra, None);
    }

    #[test]
    fn every_stream_oneof_branch_is_typed() {
        use v1::order_state_stream_response::Payload as StatePayload;
        use v1::trades_stream_response::Payload as TradesPayload;
        assert!(matches!(
            decode_trades_stream(v1::TradesStreamResponse {
                payload: Some(TradesPayload::OrderTrades(v1::OrderTrades::default()))
            }),
            Ok(CanonicalExecutionStreamEvent::Trades(_))
        ));
        for payload in [
            StatePayload::OrderState(v1::order_state_stream_response::OrderState::default()),
            StatePayload::StopOrderState(v1::order_state_stream_response::StopOrderState::default()),
        ] {
            assert!(
                decode_order_state_stream(v1::OrderStateStreamResponse {
                    payload: Some(payload)
                })
                .is_ok()
            );
        }
    }
}
