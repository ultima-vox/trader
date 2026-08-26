use nautilus_model::{
    enums::{OrderSide, OrderType, TimeInForce},
    types::{Price, Quantity},
};
use vox_domain::{
    OrderSide as VoxOrderSide, RegularOrderCommand, RegularOrderType, TimeInForce as VoxTimeInForce,
};

use crate::{MappingError, exact::quantity_from_whole, to_nautilus_price};

/// Faithful Nautilus command projection. Provider/request identities stay outside Nautilus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NautilusRegularOrderCommand {
    pub side: OrderSide,
    pub order_type: OrderType,
    pub quantity: Quantity,
    pub price: Option<Price>,
    pub time_in_force: Option<TimeInForce>,
}

pub fn to_nautilus_regular_order(
    command: &RegularOrderCommand,
) -> Result<NautilusRegularOrderCommand, MappingError> {
    let quantity = u64::try_from(command.quantity_lots).map_err(|_| MappingError::NonPositive {
        field: "order quantity lots",
        total_nanos: i128::from(command.quantity_lots) * 1_000_000_000,
    })?;
    let order_type = match command.order_type {
        RegularOrderType::Limit => OrderType::Limit,
        RegularOrderType::Market => OrderType::Market,
        RegularOrderType::BestPrice => {
            return Err(MappingError::UnsupportedExecutionSemantic {
                semantic: "T-Invest BESTPRICE order",
            });
        }
    };
    let price = command.price.map(to_nautilus_price).transpose()?;
    if matches!(order_type, OrderType::Limit) && price.is_none() {
        return Err(MappingError::InvalidNautilusValue {
            field: "limit order price",
            reason: "missing exact price".into(),
        });
    }
    if matches!(order_type, OrderType::Market) && price.is_some() {
        return Err(MappingError::InvalidNautilusValue {
            field: "market order price",
            reason: "market command must not carry price".into(),
        });
    }
    Ok(NautilusRegularOrderCommand {
        side: match command.side {
            VoxOrderSide::Buy => OrderSide::Buy,
            VoxOrderSide::Sell => OrderSide::Sell,
        },
        order_type,
        quantity: quantity_from_whole(quantity, "order quantity lots")?,
        price,
        time_in_force: command.time_in_force.map(|value| match value {
            VoxTimeInForce::Day => TimeInForce::Day,
            VoxTimeInForce::FillAndKill => TimeInForce::Ioc,
            VoxTimeInForce::FillOrKill => TimeInForce::Fok,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_domain::{ExecutionPriceConvention, FixedPoint};

    fn command(order_type: RegularOrderType) -> RegularOrderCommand {
        RegularOrderCommand {
            account_id: "account".into(),
            instrument_id: "instrument".into(),
            client_request_id: "request".into(),
            quantity_lots: 2,
            price: (order_type == RegularOrderType::Limit)
                .then(|| FixedPoint::from_units_nano(10, 500_000_000).expect("price")),
            price_convention: ExecutionPriceConvention::SettlementCurrency,
            side: VoxOrderSide::Buy,
            order_type,
            time_in_force: (order_type == RegularOrderType::Limit)
                .then_some(VoxTimeInForce::FillAndKill),
            confirm_margin_trade: false,
        }
    }

    #[test]
    fn regular_limit_maps_exactly_without_provider_identity_invention() {
        let mapped = to_nautilus_regular_order(&command(RegularOrderType::Limit)).expect("map");
        assert_eq!(mapped.side, OrderSide::Buy);
        assert_eq!(mapped.order_type, OrderType::Limit);
        assert_eq!(mapped.time_in_force, Some(TimeInForce::Ioc));
        assert_eq!(mapped.price.expect("price").to_string(), "10.5");
        assert_eq!(mapped.quantity.to_string(), "2");
    }

    #[test]
    fn best_price_stays_vox_extension_when_not_faithful() {
        assert!(matches!(
            to_nautilus_regular_order(&command(RegularOrderType::BestPrice)),
            Err(MappingError::UnsupportedExecutionSemantic { .. })
        ));
    }
}
