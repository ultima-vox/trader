use vox_domain::{FixedPoint, FuturesEconomics, InstrumentIdentity};

/// Broker-neutral instrument fields required by both supported Nautilus projections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstrumentSpec {
    pub identity: InstrumentIdentity,
    pub instrument_id: String,
    pub raw_symbol: String,
    pub currency: String,
    pub lot_size: u64,
    pub price_increment: FixedPoint,
    pub ts_event_ns: u64,
    pub ts_init_ns: u64,
}

/// Broker-neutral equity projection accepted by the Nautilus boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EquitySpec {
    pub instrument: InstrumentSpec,
}

/// Explicit future asset class. Unknown provider values must be rejected before this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FutureAssetClass {
    Fx,
    Equity,
    Commodity,
    Debt,
    Index,
    Cryptocurrency,
    Alternative,
}

/// Broker-neutral future projection with validated contract economics.
///
/// [`FuturesEconomics`] can only be built when reference and authoritative economics ticks agree
/// and money per point is exactly representable at provider nano precision. The mapper checks the
/// typed economics tick against `instrument.price_increment` again, preventing projections from
/// combining metadata from different observations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FutureSpec {
    pub instrument: InstrumentSpec,
    pub asset_class: FutureAssetClass,
    pub exchange: Option<String>,
    pub underlying: String,
    pub activation_ns: u64,
    pub expiration_ns: u64,
    pub economics: FuturesEconomics,
}
