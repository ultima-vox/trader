use nautilus_model::{enums::AssetClass, instruments::Instrument, types::fixed::FIXED_PRECISION};
use vox_domain::{FixedPoint, FuturesEconomics, FuturesEconomicsError, InstrumentIdentity};
use vox_nautilus::{
    EquitySpec, FutureAssetClass, FutureSpec, InstrumentSpec, MappingError, future_money_per_point,
    to_nautilus_equity, to_nautilus_future,
};

fn fixed(units: i64, nano: i32) -> FixedPoint {
    match FixedPoint::from_units_nano(units, nano) {
        Ok(value) => value,
        Err(error) => panic!("invalid test fixed point: {error}"),
    }
}

fn common(id: &str, symbol: &str, lot_size: u64, tick: FixedPoint) -> InstrumentSpec {
    let class_code = if symbol == "SBER" { "TQBR" } else { "SPBFUT" };
    let identity = match InstrumentIdentity::new(
        "tinvest",
        format!("uid-{symbol}"),
        Some(format!("figi-{symbol}")),
        symbol,
        class_code,
    ) {
        Ok(identity) => identity,
        Err(error) => panic!("invalid test identity: {error}"),
    };
    InstrumentSpec {
        identity,
        instrument_id: id.to_string(),
        raw_symbol: symbol.to_string(),
        currency: "RUB".to_string(),
        lot_size,
        price_increment: tick,
        ts_event_ns: 1,
        ts_init_ns: 2,
    }
}

fn q1_future() -> Result<FutureSpec, FuturesEconomicsError> {
    let tick = fixed(0, 1_000_000);
    let tick_amount = fixed(0, 833_550_000);
    Ok(FutureSpec {
        instrument: common("KCQ6-SPBFUT.TINVEST", "KCQ6", 1, fixed(0, 1_000_000)),
        asset_class: FutureAssetClass::Commodity,
        exchange: Some("MISX".to_string()),
        underlying: "COFFEE".to_string(),
        activation_ns: 1_767_225_600_000_000_000,
        expiration_ns: 1_788_134_400_000_000_000,
        economics: FuturesEconomics::new(tick, tick, tick_amount)?,
    })
}

#[test]
fn high_precision_feature_is_active() {
    assert_eq!(FIXED_PRECISION, 16);
}

#[test]
fn maps_captured_sber_identity_lot_and_tick_exactly() -> Result<(), MappingError> {
    let spec = EquitySpec {
        instrument: common("SBER-TQBR.TINVEST", "SBER", 1, fixed(0, 10_000_000)),
    };

    let equity = to_nautilus_equity(&spec)?;

    assert_eq!(equity.instrument.id.to_string(), "SBER-TQBR.TINVEST");
    assert_eq!(equity.instrument.raw_symbol.to_string(), "SBER");
    assert_eq!(equity.instrument.currency.to_string(), "RUB");
    assert_eq!(equity.instrument.price_precision, 2);
    assert_eq!(equity.instrument.price_increment.to_string(), "0.01");
    assert_eq!(equity.instrument.price_increment.raw, 100_000_000_000_000);
    assert_eq!(equity.identity.uid(), "uid-SBER");
    assert_eq!(equity.identity.figi(), Some("figi-SBER"));
    assert_eq!(equity.identity.class_code(), "TQBR");
    assert_eq!(
        equity.instrument.lot_size.map(|value| value.to_string()),
        Some("1".to_string())
    );
    Ok(())
}

#[test]
fn maps_captured_kcq6_economics_without_approximation() -> Result<(), Box<dyn std::error::Error>> {
    let spec = q1_future()?;

    let money_per_point = future_money_per_point(&spec)?;
    let mapped = to_nautilus_future(&spec)?;

    assert_eq!(money_per_point.coefficient(), 83_355);
    assert_eq!(money_per_point.scale(), 2);
    assert_eq!(money_per_point.to_string(), "833.55");
    assert_eq!(mapped.money_per_point, money_per_point);
    assert_eq!(mapped.price_increment_amount, fixed(0, 833_550_000));
    assert_eq!(mapped.instrument.id.to_string(), "KCQ6-SPBFUT.TINVEST");
    assert_eq!(mapped.identity.uid(), "uid-KCQ6");
    assert_eq!(mapped.identity.class_code(), "SPBFUT");
    assert_eq!(mapped.instrument.asset_class(), AssetClass::Commodity);
    assert_eq!(mapped.instrument.price_increment.to_string(), "0.001");
    assert_eq!(mapped.instrument.price_increment.raw, 10_000_000_000_000);
    assert_eq!(mapped.instrument.multiplier.to_string(), "833.55");
    assert_eq!(mapped.instrument.multiplier.raw, 8_335_500_000_000_000_000);
    assert_eq!(mapped.instrument.lot_size.to_string(), "1");
    assert_eq!(mapped.instrument.underlying.to_string(), "COFFEE");
    Ok(())
}

#[test]
fn rejects_inconsistent_reference_and_economics_ticks() {
    let Ok(mut spec) = q1_future() else {
        panic!("Q1 fixture must contain valid economics");
    };
    spec.instrument.price_increment = fixed(0, 10_000_000);

    assert!(matches!(
        to_nautilus_future(&spec),
        Err(MappingError::TickMismatch {
            instrument_tick_nanos: 10_000_000,
            economics_tick_nanos: 1_000_000,
        })
    ));
}

#[test]
fn invalid_raw_economics_cannot_enter_future_spec() {
    assert_eq!(
        FuturesEconomics::new(
            fixed(0, 1_000_000),
            fixed(0, 10_000_000),
            fixed(0, 833_550_000),
        ),
        Err(FuturesEconomicsError::TickMismatch)
    );
    assert_eq!(
        FuturesEconomics::new(fixed(0, 3), fixed(0, 3), fixed(0, 1)),
        Err(FuturesEconomicsError::InexactMoneyPerPoint)
    );
}

#[test]
fn rejects_zero_tick_amount_and_invalid_lifecycle() {
    assert_eq!(
        FuturesEconomics::new(fixed(0, 1), fixed(0, 1), fixed(0, 0)),
        Err(FuturesEconomicsError::NonPositive(
            "min_price_increment_amount"
        ))
    );

    let Ok(mut spec) = q1_future() else {
        panic!("Q1 fixture must contain valid economics");
    };
    spec.expiration_ns = spec.activation_ns;
    assert!(matches!(
        to_nautilus_future(&spec),
        Err(MappingError::InvalidLifecycle { .. })
    ));
}

#[test]
fn rejects_invalid_common_metadata_without_panicking() {
    let zero_lot = EquitySpec {
        instrument: common("SBER.TINVEST", "SBER", 0, fixed(0, 10_000_000)),
    };
    assert!(matches!(
        to_nautilus_equity(&zero_lot),
        Err(MappingError::NonPositive {
            field: "lot size",
            total_nanos: 0,
        })
    ));

    let invalid_id = EquitySpec {
        instrument: common("SBER", "SBER", 1, fixed(0, 10_000_000)),
    };
    assert!(matches!(
        to_nautilus_equity(&invalid_id),
        Err(MappingError::InvalidNautilusValue {
            field: "instrument ID",
            ..
        })
    ));

    let mut unknown_currency = common("SBER.TINVEST", "SBER", 1, fixed(0, 10_000_000));
    unknown_currency.currency = "NOT_A_CURRENCY".to_string();
    assert!(matches!(
        to_nautilus_equity(&EquitySpec {
            instrument: unknown_currency,
        }),
        Err(MappingError::InvalidNautilusValue {
            field: "currency",
            ..
        })
    ));
}
