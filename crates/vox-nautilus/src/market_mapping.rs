use core::str::FromStr;

use nautilus_model::{
    data::{
        Bar, BarSpecification, BarType, OrderBookDelta, OrderBookDeltas, TradeTick,
        order::BookOrder,
    },
    enums::{
        AggregationSource, AggressorSide, BarAggregation, BookAction, OrderSide, PriceType,
        RecordFlag,
    },
    identifiers::{InstrumentId, TradeId},
};

use crate::{
    BarSpec, BookLevelSpec, MappingError, OrderBookSnapshotSpec, TimeBarInterval, TradeAggressor,
    TradeTickSpec,
    exact::{quantity_from_nonnegative_whole, quantity_from_whole, to_nautilus_positive_price},
};

pub fn to_nautilus_trade(spec: &TradeTickSpec) -> Result<TradeTick, MappingError> {
    let instrument_id = instrument_id(&spec.instrument_id)?;
    let price = to_nautilus_positive_price(spec.price, "trade price")?;
    let units = lot_units(spec.quantity_lots, spec.lot_size, "trade quantity")?;
    let size = quantity_from_whole(units, "trade quantity")?;
    let aggressor_side = match spec.aggressor {
        TradeAggressor::Buy => AggressorSide::Buy,
        TradeAggressor::Sell => AggressorSide::Sell,
    };
    let trade_id = TradeId::new_checked(&spec.trade_id).map_err(|error| {
        MappingError::InvalidNautilusValue {
            field: "trade ID",
            reason: error.to_string(),
        }
    })?;
    TradeTick::new_checked(
        instrument_id,
        price,
        size,
        aggressor_side,
        trade_id,
        spec.ts_event_ns.into(),
        spec.ts_init_ns.into(),
    )
    .map_err(|error| MappingError::InvalidNautilusValue {
        field: "trade tick",
        reason: error.to_string(),
    })
}

pub fn to_nautilus_bar(spec: &BarSpec) -> Result<Bar, MappingError> {
    if !spec.is_complete {
        return Err(MappingError::IncompleteBar);
    }
    let instrument_id = instrument_id(&spec.instrument_id)?;
    let (step, aggregation) = bar_interval(spec.interval)?;
    let bar_specification = BarSpecification::new_checked(step, aggregation, PriceType::Last)
        .map_err(|error| MappingError::InvalidNautilusValue {
            field: "bar interval",
            reason: error.to_string(),
        })?;
    let bar_type = BarType::new(
        instrument_id,
        bar_specification,
        AggregationSource::External,
    );
    let volume_lots = u64::try_from(spec.volume_lots).map_err(|_| MappingError::NonPositive {
        field: "bar volume",
        total_nanos: i128::from(spec.volume_lots),
    })?;
    let volume_units =
        volume_lots
            .checked_mul(spec.lot_size)
            .ok_or(MappingError::ArithmeticOverflow {
                field: "bar volume",
            })?;
    Bar::new_checked(
        bar_type,
        to_nautilus_positive_price(spec.open, "bar open")?,
        to_nautilus_positive_price(spec.high, "bar high")?,
        to_nautilus_positive_price(spec.low, "bar low")?,
        to_nautilus_positive_price(spec.close, "bar close")?,
        quantity_from_nonnegative_whole(volume_units, "bar volume")?,
        spec.ts_event_ns.into(),
        spec.ts_init_ns.into(),
    )
    .map_err(|error| MappingError::InvalidNautilusValue {
        field: "bar",
        reason: error.to_string(),
    })
}

pub fn to_nautilus_order_book_snapshot(
    spec: &OrderBookSnapshotSpec,
) -> Result<OrderBookDeltas, MappingError> {
    if !spec.is_consistent {
        return Err(MappingError::InconsistentOrderBook);
    }
    let instrument_id = instrument_id(&spec.instrument_id)?;
    let mut clear = OrderBookDelta::clear(
        instrument_id,
        0,
        spec.ts_event_ns.into(),
        spec.ts_init_ns.into(),
    );
    let mut deltas = Vec::with_capacity(1 + spec.bids.len() + spec.asks.len());
    if spec.bids.is_empty() && spec.asks.is_empty() {
        clear.flags |= RecordFlag::F_LAST as u8;
    }
    deltas.push(clear);
    for (side, levels) in [(OrderSide::Buy, &spec.bids), (OrderSide::Sell, &spec.asks)] {
        for (index, level) in levels.iter().enumerate() {
            let is_last = side == OrderSide::Sell && index + 1 == levels.len()
                || spec.asks.is_empty() && side == OrderSide::Buy && index + 1 == levels.len();
            deltas.push(book_level_delta(
                instrument_id,
                side,
                *level,
                spec.lot_size,
                is_last,
                spec.ts_event_ns,
                spec.ts_init_ns,
            )?);
        }
    }
    OrderBookDeltas::new_checked(instrument_id, deltas).map_err(|error| {
        MappingError::InvalidNautilusValue {
            field: "order-book snapshot",
            reason: error.to_string(),
        }
    })
}

fn book_level_delta(
    instrument_id: InstrumentId,
    side: OrderSide,
    level: BookLevelSpec,
    lot_size: u64,
    is_last: bool,
    ts_event_ns: u64,
    ts_init_ns: u64,
) -> Result<OrderBookDelta, MappingError> {
    let units = lot_units(level.quantity_lots, lot_size, "book quantity")?;
    let order = BookOrder::new(
        side,
        to_nautilus_positive_price(level.price, "book price")?,
        quantity_from_whole(units, "book quantity")?,
        0,
    );
    let mut flags = RecordFlag::F_SNAPSHOT as u8 | RecordFlag::F_MBP as u8;
    if is_last {
        flags |= RecordFlag::F_LAST as u8;
    }
    OrderBookDelta::new_checked(
        instrument_id,
        BookAction::Add,
        order,
        flags,
        0,
        ts_event_ns.into(),
        ts_init_ns.into(),
    )
    .map_err(|error| MappingError::InvalidNautilusValue {
        field: "book level",
        reason: error.to_string(),
    })
}

fn instrument_id(value: &str) -> Result<InstrumentId, MappingError> {
    InstrumentId::from_str(value).map_err(|error| MappingError::InvalidNautilusValue {
        field: "instrument ID",
        reason: error.to_string(),
    })
}

fn lot_units(lots: i64, lot_size: u64, field: &'static str) -> Result<u64, MappingError> {
    if lots <= 0 || lot_size == 0 {
        return Err(MappingError::NonPositive {
            field,
            total_nanos: i128::from(lots),
        });
    }
    u64::try_from(lots)
        .ok()
        .and_then(|lots| lots.checked_mul(lot_size))
        .ok_or(MappingError::ArithmeticOverflow { field })
}

fn bar_interval(value: TimeBarInterval) -> Result<(usize, BarAggregation), MappingError> {
    let (step, aggregation) = match value {
        TimeBarInterval::Second(step) => (step, BarAggregation::Second),
        TimeBarInterval::Minute(step) => (step, BarAggregation::Minute),
        TimeBarInterval::Hour(step) => (step, BarAggregation::Hour),
        TimeBarInterval::Day(step) => (step, BarAggregation::Day),
        TimeBarInterval::Week(step) => (step, BarAggregation::Week),
        TimeBarInterval::Month(step) => (step, BarAggregation::Month),
    };
    usize::try_from(step)
        .map(|step| (step, aggregation))
        .map_err(|_| MappingError::ArithmeticOverflow {
            field: "bar interval",
        })
}
