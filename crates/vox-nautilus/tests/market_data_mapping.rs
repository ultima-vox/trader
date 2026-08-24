use nautilus_model::enums::{AggressorSide, BookAction, RecordFlag};
use vox_domain::FixedPoint;
use vox_nautilus::{
    BarSpec, BookLevelSpec, MappingError, OrderBookSnapshotSpec, TimeBarInterval, TradeAggressor,
    TradeTickSpec, to_nautilus_bar, to_nautilus_order_book_snapshot, to_nautilus_trade,
};

fn price(units: i64, nano: i32) -> FixedPoint {
    FixedPoint::from_units_nano(units, nano).expect("valid fixture decimal")
}

#[test]
fn trade_maps_lots_to_units_and_keeps_event_time() {
    let tick = to_nautilus_trade(&TradeTickSpec {
        instrument_id: "SBER.TINKOFF".into(),
        price: price(321, 500_000_000),
        quantity_lots: 2,
        lot_size: 10,
        aggressor: TradeAggressor::Buy,
        trade_id: "TI-0123456789abcdef0123456789abcdef".into(),
        ts_event_ns: 100,
        ts_init_ns: 200,
    })
    .expect("trade mapping");
    assert_eq!(tick.size.to_string(), "20");
    assert_eq!(tick.aggressor_side, AggressorSide::Buy);
    assert_eq!(tick.ts_event.as_u64(), 100);
    assert_eq!(tick.ts_init.as_u64(), 200);
}

#[test]
fn only_complete_candle_maps_to_external_bar() {
    let mut spec = BarSpec {
        instrument_id: "SBER.TINKOFF".into(),
        interval: TimeBarInterval::Minute(1),
        open: price(100, 0),
        high: price(102, 0),
        low: price(99, 0),
        close: price(101, 0),
        volume_lots: 3,
        lot_size: 10,
        is_complete: false,
        ts_event_ns: 100,
        ts_init_ns: 200,
    };
    assert_eq!(to_nautilus_bar(&spec), Err(MappingError::IncompleteBar));
    spec.is_complete = true;
    let bar = to_nautilus_bar(&spec).expect("complete bar mapping");
    assert_eq!(bar.volume.to_string(), "30");
    assert!(bar.bar_type.is_externally_aggregated());
}

#[test]
fn consistent_snapshot_maps_to_clear_plus_mbp_levels() {
    let mut spec = OrderBookSnapshotSpec {
        instrument_id: "SBER.TINKOFF".into(),
        bids: vec![BookLevelSpec {
            price: price(100, 0),
            quantity_lots: 2,
        }],
        asks: vec![BookLevelSpec {
            price: price(101, 0),
            quantity_lots: 3,
        }],
        lot_size: 10,
        is_consistent: false,
        ts_event_ns: 100,
        ts_init_ns: 200,
    };
    assert!(matches!(
        to_nautilus_order_book_snapshot(&spec),
        Err(MappingError::InconsistentOrderBook)
    ));
    spec.is_consistent = true;
    let deltas = to_nautilus_order_book_snapshot(&spec).expect("snapshot mapping");
    assert_eq!(deltas.deltas.len(), 3);
    assert_eq!(deltas.deltas[0].action, BookAction::Clear);
    assert!(RecordFlag::F_SNAPSHOT.matches(deltas.deltas[1].flags));
    assert!(RecordFlag::F_MBP.matches(deltas.deltas[1].flags));
    assert!(RecordFlag::F_LAST.matches(deltas.deltas[2].flags));
    assert_eq!(deltas.deltas[1].order.size.to_string(), "20");
    assert_eq!(deltas.deltas[2].order.size.to_string(), "30");
    assert_eq!(deltas.sequence, 0);
}
