use core::str::FromStr;

use nautilus_model::{
    enums::AssetClass,
    identifiers::{InstrumentId, Symbol},
    instruments::{Equity, FuturesContract},
    types::Currency,
};
use ustr::Ustr;

use crate::{
    EquitySpec, ExactDecimal, FutureAssetClass, FutureSpec, MappingError,
    exact::{future_money_per_point, quantity_from_whole, to_nautilus_positive_price},
};

/// Equity projection plus provider identity retained outside Nautilus.
#[derive(Clone, Debug)]
pub struct MappedEquity {
    pub instrument: Equity,
    pub identity: vox_domain::InstrumentIdentity,
}

/// Future mapping plus provider identity and exact economics evidence.
#[derive(Clone, Debug)]
pub struct MappedFuture {
    /// Checked Nautilus future projection.
    pub instrument: FuturesContract,
    /// Provider aliases retained for reconciliation and collision-free lookup.
    pub identity: vox_domain::InstrumentIdentity,
    /// Exact value of one quoted point, also used as Nautilus multiplier.
    pub money_per_point: ExactDecimal,
    /// Authoritative settlement-currency value of one minimum price increment.
    pub price_increment_amount: vox_domain::FixedPoint,
}

/// Maps broker-neutral Vox equity fields into Nautilus using checked constructors only.
pub fn to_nautilus_equity(spec: &EquitySpec) -> Result<MappedEquity, MappingError> {
    let common = &spec.instrument;
    let instrument_id = parse_instrument_id(&common.instrument_id)?;
    let raw_symbol = parse_symbol(&common.raw_symbol)?;
    let currency = parse_currency(&common.currency)?;
    let price_increment = to_nautilus_positive_price(common.price_increment, "price increment")?;
    let lot_size = quantity_from_whole(common.lot_size, "lot size")?;

    let instrument = Equity::new_checked(
        instrument_id,
        raw_symbol,
        None,
        currency,
        price_increment.precision,
        price_increment,
        Some(lot_size),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        common.ts_event_ns.into(),
        common.ts_init_ns.into(),
    )
    .map_err(|error| MappingError::InvalidNautilusValue {
        field: "equity",
        reason: error.to_string(),
    })?;
    Ok(MappedEquity {
        instrument,
        identity: common.identity.clone(),
    })
}

/// Maps broker-neutral Vox future fields and retains exact economics evidence.
pub fn to_nautilus_future(spec: &FutureSpec) -> Result<MappedFuture, MappingError> {
    if spec.expiration_ns <= spec.activation_ns {
        return Err(MappingError::InvalidLifecycle {
            activation_ns: spec.activation_ns,
            expiration_ns: spec.expiration_ns,
        });
    }

    let common = &spec.instrument;
    let instrument_id = parse_instrument_id(&common.instrument_id)?;
    let raw_symbol = parse_symbol(&common.raw_symbol)?;
    let currency = parse_currency(&common.currency)?;
    let price_increment = to_nautilus_positive_price(common.price_increment, "price increment")?;
    let lot_size = quantity_from_whole(common.lot_size, "lot size")?;
    let money_per_point = future_money_per_point(spec)?;
    let multiplier = money_per_point.to_nautilus_quantity()?;
    let exchange = spec.exchange.as_deref().map(Ustr::from);
    let underlying = Ustr::from(spec.underlying.as_str());

    let instrument = FuturesContract::new_checked(
        instrument_id,
        raw_symbol,
        map_asset_class(spec.asset_class),
        exchange,
        underlying,
        spec.activation_ns.into(),
        spec.expiration_ns.into(),
        currency,
        price_increment.precision,
        price_increment,
        multiplier,
        lot_size,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        common.ts_event_ns.into(),
        common.ts_init_ns.into(),
    )
    .map_err(|error| MappingError::InvalidNautilusValue {
        field: "future",
        reason: error.to_string(),
    })?;

    Ok(MappedFuture {
        instrument,
        identity: common.identity.clone(),
        money_per_point,
        price_increment_amount: spec.economics.min_price_increment_amount(),
    })
}

fn parse_instrument_id(value: &str) -> Result<InstrumentId, MappingError> {
    InstrumentId::from_str(value).map_err(|error| MappingError::InvalidNautilusValue {
        field: "instrument ID",
        reason: error.to_string(),
    })
}

fn parse_symbol(value: &str) -> Result<Symbol, MappingError> {
    Symbol::new_checked(value).map_err(|error| MappingError::InvalidNautilusValue {
        field: "raw symbol",
        reason: error.to_string(),
    })
}

fn parse_currency(value: &str) -> Result<Currency, MappingError> {
    Currency::from_str(value).map_err(|error| MappingError::InvalidNautilusValue {
        field: "currency",
        reason: error.to_string(),
    })
}

const fn map_asset_class(value: FutureAssetClass) -> AssetClass {
    match value {
        FutureAssetClass::Fx => AssetClass::FX,
        FutureAssetClass::Equity => AssetClass::Equity,
        FutureAssetClass::Commodity => AssetClass::Commodity,
        FutureAssetClass::Debt => AssetClass::Debt,
        FutureAssetClass::Index => AssetClass::Index,
        FutureAssetClass::Cryptocurrency => AssetClass::Cryptocurrency,
        FutureAssetClass::Alternative => AssetClass::Alternative,
    }
}
