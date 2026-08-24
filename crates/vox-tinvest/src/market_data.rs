//! T-Invest market-data policy boundary.
//!
//! Generated protobuf messages enter here. Vox types retain provider facts, exact decimal values,
//! event timestamps, lots, enum wire numbers, and consistency state. No Nautilus policy lives here.

use std::collections::{BTreeMap, BTreeSet};

use prost_types::Timestamp;
use thiserror::Error;
use uuid::Uuid;
use vox_domain::{FixedPoint, FixedPointError};

use crate::generated::v1;

pub const MARKET_DATA_SERVICE_METHODS: [&str; 9] = [
    "GetCandles",
    "GetLastPrices",
    "GetOrderBook",
    "GetTradingStatus",
    "GetTradingStatuses",
    "GetLastTrades",
    "GetClosePrices",
    "GetTechAnalysis",
    "GetMarketValues",
];
pub const MARKET_DATA_STREAM_METHODS: [&str; 2] =
    ["MarketDataStream", "MarketDataServerSideStream"];
pub const MAX_LIMITED_SUBSCRIPTIONS: usize = 300;
pub const MAX_SUBSCRIPTION_REQUESTS_PER_MINUTE: u32 = 100;
pub const MIN_PING_DELAY_MS: i64 = 5_000;
pub const MAX_PING_DELAY_MS: i64 = 180_000;
pub const DEFAULT_PING_DELAY_MS: i64 = 120_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalCandle {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub interval: i32,
    pub open: FixedPoint,
    pub high: FixedPoint,
    pub low: FixedPoint,
    pub close: FixedPoint,
    pub volume_lots: i64,
    pub volume_buy_lots: i64,
    pub volume_sell_lots: i64,
    pub event_time_ns: u64,
    pub last_trade_time_ns: Option<u64>,
    pub candle_source: i32,
    pub is_complete: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTrade {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub direction: i32,
    pub price: FixedPoint,
    pub quantity_lots: i64,
    pub event_time_ns: u64,
    pub trade_source: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalLastPrice {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub price: FixedPoint,
    pub event_time_ns: u64,
    pub last_price_type: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTradingStatus {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub status: i32,
    pub event_time_ns: u64,
    pub limit_order_available: bool,
    pub market_order_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOpenInterest {
    pub instrument_uid: String,
    pub ticker: String,
    pub class_code: String,
    pub value: i64,
    pub event_time_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalBookLevel {
    pub price: FixedPoint,
    pub quantity_lots: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOrderBook {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub depth: i32,
    pub bids: Vec<CanonicalBookLevel>,
    pub asks: Vec<CanonicalBookLevel>,
    pub event_time_ns: u64,
    pub is_consistent: bool,
    pub order_book_type: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalUnaryOrderBook {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub depth: i32,
    pub bids: Vec<CanonicalBookLevel>,
    pub asks: Vec<CanonicalBookLevel>,
    pub last_price: Option<FixedPoint>,
    pub close_price: Option<FixedPoint>,
    pub limit_up: Option<FixedPoint>,
    pub limit_down: Option<FixedPoint>,
    pub last_price_time_ns: Option<u64>,
    pub close_price_time_ns: Option<u64>,
    pub orderbook_time_ns: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalTradingStatusFact {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub status: i32,
    pub limit_order_available: bool,
    pub market_order_available: bool,
    pub api_trade_available: bool,
    pub bestprice_order_available: bool,
    pub only_best_price: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalClosePrice {
    pub instrument_uid: String,
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub price: Option<FixedPoint>,
    pub evening_session_price: Option<FixedPoint>,
    pub time_ns: Option<u64>,
    pub evening_session_time_ns: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalTechAnalysisValue {
    pub timestamp_ns: u64,
    pub middle_band: Option<FixedPoint>,
    pub upper_band: Option<FixedPoint>,
    pub lower_band: Option<FixedPoint>,
    pub signal: Option<FixedPoint>,
    pub macd: Option<FixedPoint>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalMarketValue {
    pub value_type: Option<i32>,
    pub value: Option<FixedPoint>,
    pub time_ns: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalMarketValueInstrument {
    pub instrument_uid: String,
    pub ticker: String,
    pub class_code: String,
    pub values: Vec<CanonicalMarketValue>,
}

impl CanonicalOrderBook {
    pub fn require_authoritative(&self) -> Result<(), MarketDataError> {
        if self.is_consistent {
            Ok(())
        } else {
            Err(MarketDataError::InconsistentOrderBook)
        }
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MarketDataError {
    #[error("missing required provider field {0}")]
    Missing(&'static str),
    #[error("invalid provider decimal in {field}: {source}")]
    Decimal {
        field: &'static str,
        source: FixedPointError,
    },
    #[error("invalid provider timestamp in {field}")]
    Timestamp { field: &'static str },
    #[error("{field} must be positive")]
    NonPositive { field: &'static str },
    #[error("lot conversion overflow")]
    LotOverflow,
    #[error("provider order book is inconsistent")]
    InconsistentOrderBook,
    #[error("unsupported candle interval wire value {0}")]
    UnsupportedCandleInterval(i32),
    #[error("history range must be increasing")]
    InvalidHistoryRange,
    #[error("conflicting historical candles share timestamp {timestamp_ns}")]
    ConflictingHistoricalCandle { timestamp_ns: u64 },
    #[error("instrument identifier must be non-empty")]
    EmptyInstrumentId,
    #[error("order-book depth must be one of 1, 10, 20, 30, 40, 50")]
    InvalidOrderBookDepth,
    #[error("limited stream subscriptions exceed provider maximum {MAX_LIMITED_SUBSCRIPTIONS}")]
    SubscriptionLimit,
    #[error("ping delay must be in {MIN_PING_DELAY_MS}..={MAX_PING_DELAY_MS} ms")]
    InvalidPingDelay,
    #[error("acknowledgement does not match desired subscription")]
    UnknownAcknowledgement,
    #[error("acknowledgement action {0} does not match subscribe request")]
    UnexpectedAcknowledgementAction(i32),
    #[error("successful acknowledgement has invalid stream/subscription identity")]
    InvalidAcknowledgementIdentity,
}

fn quotation(
    value: Option<v1::Quotation>,
    field: &'static str,
) -> Result<FixedPoint, MarketDataError> {
    let value = value.ok_or(MarketDataError::Missing(field))?;
    FixedPoint::from_units_nano(value.units, value.nano)
        .map_err(|source| MarketDataError::Decimal { field, source })
}

fn optional_quotation(
    value: Option<v1::Quotation>,
    field: &'static str,
) -> Result<Option<FixedPoint>, MarketDataError> {
    value
        .map(|value| {
            FixedPoint::from_units_nano(value.units, value.nano)
                .map_err(|source| MarketDataError::Decimal { field, source })
        })
        .transpose()
}

fn optional_timestamp_ns(
    value: Option<Timestamp>,
    field: &'static str,
) -> Result<Option<u64>, MarketDataError> {
    value
        .map(|value| timestamp_ns(Some(value), field))
        .transpose()
}

pub fn timestamp_ns(value: Option<Timestamp>, field: &'static str) -> Result<u64, MarketDataError> {
    let value = value.ok_or(MarketDataError::Missing(field))?;
    if !(0..=253_402_300_799).contains(&value.seconds) || !(0..1_000_000_000).contains(&value.nanos)
    {
        return Err(MarketDataError::Timestamp { field });
    }
    let seconds = u64::try_from(value.seconds).map_err(|_| MarketDataError::Timestamp { field })?;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|base| base.checked_add(u64::try_from(value.nanos).ok()?))
        .ok_or(MarketDataError::Timestamp { field })
}

pub fn lots_to_units(lots: i64, lot_size: u64) -> Result<u64, MarketDataError> {
    if lots <= 0 {
        return Err(MarketDataError::NonPositive { field: "lots" });
    }
    if lot_size == 0 {
        return Err(MarketDataError::NonPositive { field: "lot_size" });
    }
    u64::try_from(lots)
        .ok()
        .and_then(|lots| lots.checked_mul(lot_size))
        .ok_or(MarketDataError::LotOverflow)
}

/// Derives compatibility ID because official public `Trade` has no venue trade ID.
/// Full exposed provider tuple feeds stable FNV-1a-128; `TI-` marks non-provider provenance.
#[must_use]
pub fn derived_trade_compatibility_id(trade: &CanonicalTrade) -> String {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    };
    for value in [
        trade.instrument_uid.as_bytes(),
        trade.figi.as_bytes(),
        trade.ticker.as_bytes(),
        trade.class_code.as_bytes(),
    ] {
        feed(value);
    }
    feed(&trade.direction.to_le_bytes());
    feed(&trade.price.total_nanos().to_le_bytes());
    feed(&trade.quantity_lots.to_le_bytes());
    feed(&trade.event_time_ns.to_le_bytes());
    feed(&trade.trade_source.to_le_bytes());
    format!("TI-{hash:032x}")
}

impl TryFrom<v1::Trade> for CanonicalTrade {
    type Error = MarketDataError;

    fn try_from(value: v1::Trade) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("trade.instrument_uid"));
        }
        if value.quantity <= 0 {
            return Err(MarketDataError::NonPositive {
                field: "trade.quantity",
            });
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            direction: value.direction,
            price: quotation(value.price, "trade.price")?,
            quantity_lots: value.quantity,
            event_time_ns: timestamp_ns(value.time, "trade.time")?,
            trade_source: value.trade_source,
        })
    }
}

impl TryFrom<v1::LastPrice> for CanonicalLastPrice {
    type Error = MarketDataError;

    fn try_from(value: v1::LastPrice) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("last_price.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            price: quotation(value.price, "last_price.price")?,
            event_time_ns: timestamp_ns(value.time, "last_price.time")?,
            last_price_type: value.last_price_type,
        })
    }
}

impl TryFrom<v1::TradingStatus> for CanonicalTradingStatus {
    type Error = MarketDataError;

    fn try_from(value: v1::TradingStatus) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("trading_status.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            status: value.trading_status,
            event_time_ns: timestamp_ns(value.time, "trading_status.time")?,
            limit_order_available: value.limit_order_available_flag,
            market_order_available: value.market_order_available_flag,
        })
    }
}

impl TryFrom<v1::OpenInterest> for CanonicalOpenInterest {
    type Error = MarketDataError;

    fn try_from(value: v1::OpenInterest) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("open_interest.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            ticker: value.ticker,
            class_code: value.class_code,
            value: value.open_interest,
            event_time_ns: timestamp_ns(value.time, "open_interest.time")?,
        })
    }
}

impl TryFrom<v1::Candle> for CanonicalCandle {
    type Error = MarketDataError;

    fn try_from(value: v1::Candle) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("candle.instrument_uid"));
        }
        if value.volume < 0 || value.volume_buy < 0 || value.volume_sell < 0 {
            return Err(MarketDataError::NonPositive {
                field: "candle.volume",
            });
        }
        let last_trade_time_ns = value
            .last_trade_ts
            .map(|time| timestamp_ns(Some(time), "candle.last_trade_ts"))
            .transpose()?;
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            interval: value.interval,
            open: quotation(value.open, "candle.open")?,
            high: quotation(value.high, "candle.high")?,
            low: quotation(value.low, "candle.low")?,
            close: quotation(value.close, "candle.close")?,
            volume_lots: value.volume,
            volume_buy_lots: value.volume_buy,
            volume_sell_lots: value.volume_sell,
            event_time_ns: timestamp_ns(value.time, "candle.time")?,
            last_trade_time_ns,
            candle_source: value.candle_source_type,
            is_complete: None,
        })
    }
}

impl CanonicalCandle {
    pub fn from_historic(
        value: v1::HistoricCandle,
        instrument_uid: String,
        interval: i32,
    ) -> Result<Self, MarketDataError> {
        if instrument_uid.is_empty() {
            return Err(MarketDataError::EmptyInstrumentId);
        }
        if value.volume < 0 || value.volume_buy < 0 || value.volume_sell < 0 {
            return Err(MarketDataError::NonPositive {
                field: "candle.volume",
            });
        }
        Ok(Self {
            instrument_uid,
            figi: String::new(),
            ticker: String::new(),
            class_code: String::new(),
            interval,
            open: quotation(value.open, "candle.open")?,
            high: quotation(value.high, "candle.high")?,
            low: quotation(value.low, "candle.low")?,
            close: quotation(value.close, "candle.close")?,
            volume_lots: value.volume,
            volume_buy_lots: value.volume_buy,
            volume_sell_lots: value.volume_sell,
            event_time_ns: timestamp_ns(value.time, "candle.time")?,
            last_trade_time_ns: None,
            candle_source: value.candle_source,
            is_complete: Some(value.is_complete),
        })
    }
}

impl TryFrom<v1::GetOrderBookResponse> for CanonicalUnaryOrderBook {
    type Error = MarketDataError;

    fn try_from(value: v1::GetOrderBookResponse) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("orderbook.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            depth: value.depth,
            bids: levels(value.bids, "orderbook.bid")?,
            asks: levels(value.asks, "orderbook.ask")?,
            last_price: optional_quotation(value.last_price, "orderbook.last_price")?,
            close_price: optional_quotation(value.close_price, "orderbook.close_price")?,
            limit_up: optional_quotation(value.limit_up, "orderbook.limit_up")?,
            limit_down: optional_quotation(value.limit_down, "orderbook.limit_down")?,
            last_price_time_ns: optional_timestamp_ns(
                value.last_price_ts,
                "orderbook.last_price_ts",
            )?,
            close_price_time_ns: optional_timestamp_ns(
                value.close_price_ts,
                "orderbook.close_price_ts",
            )?,
            orderbook_time_ns: optional_timestamp_ns(value.orderbook_ts, "orderbook.orderbook_ts")?,
        })
    }
}

impl TryFrom<v1::GetTradingStatusResponse> for CanonicalTradingStatusFact {
    type Error = MarketDataError;

    fn try_from(value: v1::GetTradingStatusResponse) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("trading_status.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            status: value.trading_status,
            limit_order_available: value.limit_order_available_flag,
            market_order_available: value.market_order_available_flag,
            api_trade_available: value.api_trade_available_flag,
            bestprice_order_available: value.bestprice_order_available_flag,
            only_best_price: value.only_best_price,
        })
    }
}

impl TryFrom<v1::InstrumentClosePriceResponse> for CanonicalClosePrice {
    type Error = MarketDataError;

    fn try_from(value: v1::InstrumentClosePriceResponse) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("close_price.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            price: optional_quotation(value.price, "close_price.price")?,
            evening_session_price: optional_quotation(
                value.evening_session_price,
                "close_price.evening_session_price",
            )?,
            time_ns: optional_timestamp_ns(value.time, "close_price.time")?,
            evening_session_time_ns: optional_timestamp_ns(
                value.evening_session_price_time,
                "close_price.evening_session_price_time",
            )?,
        })
    }
}

impl TryFrom<v1::get_tech_analysis_response::TechAnalysisItem> for CanonicalTechAnalysisValue {
    type Error = MarketDataError;

    fn try_from(
        value: v1::get_tech_analysis_response::TechAnalysisItem,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            timestamp_ns: timestamp_ns(value.timestamp, "technical_analysis.timestamp")?,
            middle_band: optional_quotation(value.middle_band, "technical_analysis.middle_band")?,
            upper_band: optional_quotation(value.upper_band, "technical_analysis.upper_band")?,
            lower_band: optional_quotation(value.lower_band, "technical_analysis.lower_band")?,
            signal: optional_quotation(value.signal, "technical_analysis.signal")?,
            macd: optional_quotation(value.macd, "technical_analysis.macd")?,
        })
    }
}

impl TryFrom<v1::MarketValue> for CanonicalMarketValue {
    type Error = MarketDataError;

    fn try_from(value: v1::MarketValue) -> Result<Self, Self::Error> {
        Ok(Self {
            value_type: value.r#type,
            value: optional_quotation(value.value, "market_value.value")?,
            time_ns: optional_timestamp_ns(value.time, "market_value.time")?,
        })
    }
}

impl TryFrom<v1::MarketValueInstrument> for CanonicalMarketValueInstrument {
    type Error = MarketDataError;

    fn try_from(value: v1::MarketValueInstrument) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("market_value.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            ticker: value.ticker,
            class_code: value.class_code,
            values: value
                .values
                .into_iter()
                .map(CanonicalMarketValue::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

fn levels(
    values: Vec<v1::Order>,
    field: &'static str,
) -> Result<Vec<CanonicalBookLevel>, MarketDataError> {
    values
        .into_iter()
        .map(|value| {
            if value.quantity <= 0 {
                return Err(MarketDataError::NonPositive { field });
            }
            Ok(CanonicalBookLevel {
                price: quotation(value.price, field)?,
                quantity_lots: value.quantity,
            })
        })
        .collect()
}

impl TryFrom<v1::OrderBook> for CanonicalOrderBook {
    type Error = MarketDataError;

    fn try_from(value: v1::OrderBook) -> Result<Self, Self::Error> {
        if value.instrument_uid.is_empty() {
            return Err(MarketDataError::Missing("orderbook.instrument_uid"));
        }
        Ok(Self {
            instrument_uid: value.instrument_uid,
            figi: value.figi,
            ticker: value.ticker,
            class_code: value.class_code,
            depth: value.depth,
            bids: levels(value.bids, "orderbook.bid")?,
            asks: levels(value.asks, "orderbook.ask")?,
            event_time_ns: timestamp_ns(value.time, "orderbook.time")?,
            is_consistent: value.is_consistent,
            order_book_type: value.order_book_type,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CandleRequestConstraint {
    pub max_span_seconds: i64,
    pub max_limit: i32,
}

pub fn candle_request_constraint(
    interval: i32,
) -> Result<CandleRequestConstraint, MarketDataError> {
    const DAY: i64 = 86_400;
    let value = match interval {
        1 => (DAY, 2400),
        2 => (7 * DAY, 2400),
        3 => (21 * DAY, 2400),
        4 => (92 * DAY, 2400),
        5 => (366 * DAY, 2400),
        6 => (DAY, 1200),
        7 => (DAY, 750),
        8 => (7 * DAY, 1200),
        9 => (21 * DAY, 1200),
        10 => (92 * DAY, 2400),
        11 => (92 * DAY, 700),
        12 => (366 * DAY, 300),
        13 => (366 * DAY, 120),
        14 => (200 * 60, 2500),
        15 => (200 * 60, 1250),
        16 => (20 * 60 * 60, 2500),
        other => return Err(MarketDataError::UnsupportedCandleInterval(other)),
    };
    Ok(CandleRequestConstraint {
        max_span_seconds: value.0,
        max_limit: value.1,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HistoryWindow {
    pub from_seconds: i64,
    pub to_seconds: i64,
    pub limit: i32,
}

pub fn plan_candle_history(
    from_seconds: i64,
    to_seconds: i64,
    interval: i32,
) -> Result<Vec<HistoryWindow>, MarketDataError> {
    if from_seconds >= to_seconds {
        return Err(MarketDataError::InvalidHistoryRange);
    }
    let constraint = candle_request_constraint(interval)?;
    let mut windows = Vec::new();
    let mut cursor = from_seconds;
    while cursor < to_seconds {
        let end = cursor
            .checked_add(constraint.max_span_seconds)
            .unwrap_or(i64::MAX)
            .min(to_seconds);
        windows.push(HistoryWindow {
            from_seconds: cursor,
            to_seconds: end,
            limit: constraint.max_limit,
        });
        cursor = end;
    }
    Ok(windows)
}

/// Deterministically merges chunk responses. Exact boundary duplicates collapse; conflicting
/// candles at one provider timestamp fail instead of silently selecting a value.
pub fn merge_historic_candles(
    chunks: impl IntoIterator<Item = Vec<v1::HistoricCandle>>,
) -> Result<Vec<v1::HistoricCandle>, MarketDataError> {
    let mut candles = chunks.into_iter().flatten().collect::<Vec<_>>();
    let mut keyed = candles
        .drain(..)
        .map(|candle| {
            let time = timestamp_ns(candle.time, "candle.time")?;
            Ok((time, candle))
        })
        .collect::<Result<Vec<_>, MarketDataError>>()?;
    keyed.sort_by_key(|(time, _)| *time);
    let mut merged = Vec::<(u64, v1::HistoricCandle)>::with_capacity(keyed.len());
    for (time, candle) in keyed {
        match merged.last() {
            Some((previous_time, previous)) if *previous_time == time && previous == &candle => {}
            Some((previous_time, _)) if *previous_time == time => {
                return Err(MarketDataError::ConflictingHistoricalCandle { timestamp_ns: time });
            }
            _ => merged.push((time, candle)),
        }
    }
    Ok(merged.into_iter().map(|(_, candle)| candle).collect())
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SubscriptionKind {
    Candle {
        interval: i32,
        waiting_close: bool,
        source: Option<i32>,
    },
    OrderBook {
        depth: i32,
        order_book_type: i32,
    },
    Trade {
        source: i32,
        with_open_interest: bool,
    },
    Info,
    LastPrice,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MarketSubscription {
    pub instrument_id: String,
    pub kind: SubscriptionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfirmationState {
    Pending,
    Confirmed {
        stream_id: String,
        subscription_id: String,
    },
    Rejected {
        provider_status: i32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct MarketSubscriptionRegistry {
    entries: BTreeMap<MarketSubscription, ConfirmationState>,
    authoritative_books: BTreeSet<String>,
}

impl MarketSubscriptionRegistry {
    pub fn insert(&mut self, mut subscription: MarketSubscription) -> Result<(), MarketDataError> {
        canonicalize_provider_defaults(&mut subscription.kind);
        validate_subscription(&subscription)?;
        let adding_limited = matches!(
            subscription.kind,
            SubscriptionKind::Candle { .. }
                | SubscriptionKind::OrderBook { .. }
                | SubscriptionKind::Trade { .. }
        ) && !self.entries.contains_key(&subscription);
        let limited = self
            .entries
            .keys()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    SubscriptionKind::Candle { .. }
                        | SubscriptionKind::OrderBook { .. }
                        | SubscriptionKind::Trade { .. }
                )
            })
            .count();
        if adding_limited && limited >= MAX_LIMITED_SUBSCRIPTIONS {
            return Err(MarketDataError::SubscriptionLimit);
        }
        self.entries
            .insert(subscription, ConfirmationState::Pending);
        Ok(())
    }

    #[must_use]
    pub fn state(&self, subscription: &MarketSubscription) -> Option<&ConfirmationState> {
        self.entries.get(subscription)
    }

    #[must_use]
    pub fn accepts_event_before_ack(&self, instrument_id: &str, kind: &SubscriptionKind) -> bool {
        self.entries.keys().any(|entry| {
            entry.instrument_id == instrument_id && same_event_family(&entry.kind, kind)
        })
    }

    pub fn acknowledge(
        &mut self,
        subscription: &MarketSubscription,
        provider_status: i32,
        provider_action: i32,
        stream_id: String,
        subscription_id: String,
    ) -> Result<(), MarketDataError> {
        let state = self
            .entries
            .get_mut(subscription)
            .ok_or(MarketDataError::UnknownAcknowledgement)?;
        if provider_action != v1::SubscriptionAction::Subscribe as i32 {
            return Err(MarketDataError::UnexpectedAcknowledgementAction(
                provider_action,
            ));
        }
        *state = if provider_status == v1::SubscriptionStatus::Success as i32 {
            if stream_id.is_empty() || Uuid::parse_str(&subscription_id).is_err() {
                return Err(MarketDataError::InvalidAcknowledgementIdentity);
            }
            ConfirmationState::Confirmed {
                stream_id,
                subscription_id,
            }
        } else {
            ConfirmationState::Rejected { provider_status }
        };
        Ok(())
    }

    pub fn observe_book(&mut self, instrument_id: &str, is_consistent: bool) {
        if is_consistent {
            self.authoritative_books.insert(instrument_id.to_owned());
        } else {
            self.authoritative_books.remove(instrument_id);
        }
    }

    #[must_use]
    pub fn book_is_authoritative(&self, instrument_id: &str) -> bool {
        self.authoritative_books.contains(instrument_id)
    }

    pub fn disconnected(&mut self) {
        for state in self.entries.values_mut() {
            *state = ConfirmationState::Pending;
        }
        self.authoritative_books.clear();
    }

    /// Builds generated subscribe messages from desired state. Requests with identical provider
    /// options are batched; no message exceeds provider per-stream limited subscription ceiling.
    #[allow(deprecated)]
    #[must_use]
    pub fn subscribe_requests(&self) -> Vec<v1::MarketDataRequest> {
        use v1::market_data_request::Payload;

        let mut groups = BTreeMap::<SubscriptionKind, Vec<String>>::new();
        for entry in self.entries.keys() {
            groups
                .entry(entry.kind.clone())
                .or_default()
                .push(entry.instrument_id.clone());
        }
        let mut requests = Vec::new();
        for (kind, instruments) in groups {
            for chunk in instruments.chunks(MAX_LIMITED_SUBSCRIPTIONS) {
                let action = v1::SubscriptionAction::Subscribe as i32;
                let payload = match kind {
                    SubscriptionKind::Candle {
                        interval,
                        waiting_close,
                        source,
                    } => Payload::SubscribeCandlesRequest(v1::SubscribeCandlesRequest {
                        subscription_action: action,
                        instruments: chunk
                            .iter()
                            .map(|instrument_id| v1::CandleInstrument {
                                figi: String::new(),
                                interval,
                                instrument_id: instrument_id.clone(),
                            })
                            .collect(),
                        waiting_close,
                        candle_source_type: source,
                    }),
                    SubscriptionKind::OrderBook {
                        depth,
                        order_book_type,
                    } => Payload::SubscribeOrderBookRequest(v1::SubscribeOrderBookRequest {
                        subscription_action: action,
                        instruments: chunk
                            .iter()
                            .map(|instrument_id| v1::OrderBookInstrument {
                                figi: String::new(),
                                depth,
                                instrument_id: instrument_id.clone(),
                                order_book_type,
                            })
                            .collect(),
                    }),
                    SubscriptionKind::Trade {
                        source,
                        with_open_interest,
                    } => Payload::SubscribeTradesRequest(v1::SubscribeTradesRequest {
                        subscription_action: action,
                        instruments: chunk
                            .iter()
                            .map(|instrument_id| v1::TradeInstrument {
                                figi: String::new(),
                                instrument_id: instrument_id.clone(),
                            })
                            .collect(),
                        trade_source: source,
                        with_open_interest,
                    }),
                    SubscriptionKind::Info => {
                        Payload::SubscribeInfoRequest(v1::SubscribeInfoRequest {
                            subscription_action: action,
                            instruments: chunk
                                .iter()
                                .map(|instrument_id| v1::InfoInstrument {
                                    figi: String::new(),
                                    instrument_id: instrument_id.clone(),
                                })
                                .collect(),
                        })
                    }
                    SubscriptionKind::LastPrice => {
                        Payload::SubscribeLastPriceRequest(v1::SubscribeLastPriceRequest {
                            subscription_action: action,
                            instruments: chunk
                                .iter()
                                .map(|instrument_id| v1::LastPriceInstrument {
                                    figi: String::new(),
                                    instrument_id: instrument_id.clone(),
                                })
                                .collect(),
                        })
                    }
                };
                requests.push(v1::MarketDataRequest {
                    payload: Some(payload),
                });
            }
        }
        requests
    }

    /// Applies generated provider ACKs in any arrival order. Desired keys remain adapter-owned.
    pub fn apply_ack_response(
        &mut self,
        response: &v1::MarketDataResponse,
    ) -> Result<usize, MarketDataError> {
        use v1::market_data_response::Payload;
        let mut applied = 0;
        match response.payload.as_ref() {
            Some(Payload::SubscribeCandlesResponse(response)) => {
                for ack in &response.candles_subscriptions {
                    let key = MarketSubscription {
                        instrument_id: ack.instrument_uid.clone(),
                        kind: SubscriptionKind::Candle {
                            interval: ack.interval,
                            waiting_close: ack.waiting_close,
                            source: ack.candle_source_type,
                        },
                    };
                    self.acknowledge(
                        &key,
                        ack.subscription_status,
                        ack.subscription_action,
                        ack.stream_id.clone(),
                        ack.subscription_id.clone(),
                    )?;
                    applied += 1;
                }
            }
            Some(Payload::SubscribeOrderBookResponse(response)) => {
                for ack in &response.order_book_subscriptions {
                    let key = MarketSubscription {
                        instrument_id: ack.instrument_uid.clone(),
                        kind: SubscriptionKind::OrderBook {
                            depth: ack.depth,
                            order_book_type: ack.order_book_type,
                        },
                    };
                    self.acknowledge(
                        &key,
                        ack.subscription_status,
                        ack.subscription_action,
                        ack.stream_id.clone(),
                        ack.subscription_id.clone(),
                    )?;
                    applied += 1;
                }
            }
            Some(Payload::SubscribeTradesResponse(response)) => {
                for ack in &response.trade_subscriptions {
                    let key = MarketSubscription {
                        instrument_id: ack.instrument_uid.clone(),
                        kind: SubscriptionKind::Trade {
                            source: response.trade_source,
                            with_open_interest: ack.with_open_interest,
                        },
                    };
                    self.acknowledge(
                        &key,
                        ack.subscription_status,
                        ack.subscription_action,
                        ack.stream_id.clone(),
                        ack.subscription_id.clone(),
                    )?;
                    applied += 1;
                }
            }
            Some(Payload::SubscribeInfoResponse(response)) => {
                for ack in &response.info_subscriptions {
                    let key = MarketSubscription {
                        instrument_id: ack.instrument_uid.clone(),
                        kind: SubscriptionKind::Info,
                    };
                    self.acknowledge(
                        &key,
                        ack.subscription_status,
                        ack.subscription_action,
                        ack.stream_id.clone(),
                        ack.subscription_id.clone(),
                    )?;
                    applied += 1;
                }
            }
            Some(Payload::SubscribeLastPriceResponse(response)) => {
                for ack in &response.last_price_subscriptions {
                    let key = MarketSubscription {
                        instrument_id: ack.instrument_uid.clone(),
                        kind: SubscriptionKind::LastPrice,
                    };
                    self.acknowledge(
                        &key,
                        ack.subscription_status,
                        ack.subscription_action,
                        ack.stream_id.clone(),
                        ack.subscription_id.clone(),
                    )?;
                    applied += 1;
                }
            }
            _ => {}
        }
        Ok(applied)
    }

    pub fn desired(&self) -> impl Iterator<Item = &MarketSubscription> {
        self.entries.keys()
    }
}

fn same_event_family(left: &SubscriptionKind, right: &SubscriptionKind) -> bool {
    matches!(
        (left, right),
        (
            SubscriptionKind::Candle { .. },
            SubscriptionKind::Candle { .. }
        ) | (
            SubscriptionKind::OrderBook { .. },
            SubscriptionKind::OrderBook { .. }
        ) | (
            SubscriptionKind::Trade { .. },
            SubscriptionKind::Trade { .. }
        ) | (SubscriptionKind::Info, SubscriptionKind::Info)
            | (SubscriptionKind::LastPrice, SubscriptionKind::LastPrice)
    )
}

fn validate_subscription(subscription: &MarketSubscription) -> Result<(), MarketDataError> {
    if subscription.instrument_id.trim().is_empty() {
        return Err(MarketDataError::EmptyInstrumentId);
    }
    match subscription.kind {
        SubscriptionKind::Candle { interval, .. } if !(1..=13).contains(&interval) => {
            Err(MarketDataError::UnsupportedCandleInterval(interval))
        }
        SubscriptionKind::OrderBook { depth, .. }
            if !matches!(depth, 1 | 10 | 20 | 30 | 40 | 50) =>
        {
            Err(MarketDataError::InvalidOrderBookDepth)
        }
        _ => Ok(()),
    }
}

fn canonicalize_provider_defaults(kind: &mut SubscriptionKind) {
    match kind {
        SubscriptionKind::Candle { source, .. } if *source == Some(0) => *source = None,
        SubscriptionKind::OrderBook {
            order_book_type, ..
        } if *order_book_type == 0 => {
            *order_book_type = v1::OrderBookType::OrderbookTypeAll as i32;
        }
        _ => {}
    }
}

pub fn validate_ping_delay(delay_ms: i64) -> Result<(), MarketDataError> {
    if (MIN_PING_DELAY_MS..=MAX_PING_DELAY_MS).contains(&delay_ms) {
        Ok(())
    } else {
        Err(MarketDataError::InvalidPingDelay)
    }
}
