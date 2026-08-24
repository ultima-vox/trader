use vox_domain::FixedPoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TradeAggressor {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradeTickSpec {
    pub instrument_id: String,
    pub price: FixedPoint,
    pub quantity_lots: i64,
    pub lot_size: u64,
    pub aggressor: TradeAggressor,
    /// Provider ID or explicitly documented adapter compatibility ID.
    pub trade_id: String,
    pub ts_event_ns: u64,
    pub ts_init_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeBarInterval {
    Second(u32),
    Minute(u32),
    Hour(u32),
    Day(u32),
    Week(u32),
    Month(u32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarSpec {
    pub instrument_id: String,
    pub interval: TimeBarInterval,
    pub open: FixedPoint,
    pub high: FixedPoint,
    pub low: FixedPoint,
    pub close: FixedPoint,
    pub volume_lots: i64,
    pub lot_size: u64,
    pub is_complete: bool,
    pub ts_event_ns: u64,
    pub ts_init_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BookLevelSpec {
    pub price: FixedPoint,
    pub quantity_lots: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderBookSnapshotSpec {
    pub instrument_id: String,
    pub bids: Vec<BookLevelSpec>,
    pub asks: Vec<BookLevelSpec>,
    pub lot_size: u64,
    pub is_consistent: bool,
    pub ts_event_ns: u64,
    pub ts_init_ns: u64,
}
