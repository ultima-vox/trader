//! The provider-neutral market-data read model.
//!
//! #8 owns acquiring market data from the provider. This module owns the Vox-side projection
//! the frontend consumes: provider-neutral, exact, and explicit about how fresh it is. No
//! provider wire type appears here, and the browser never reaches the provider.
//!
//! Every value that can move money is a [`Decimal`] string. Every record states when it was
//! observed and how fresh the stream behind it is, because a price without an age is a claim
//! the operator cannot check.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::instrument::InstrumentIdentityDto;
use super::money::Decimal;
use super::runtime::StreamStateDto;

/// How current a market-data record is.
///
/// Freshness is a property of the feed, not of the instrument, and it is never folded into
/// the price: a stale quote stays visible with its age rather than disappearing or pretending.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct MarketFreshness {
    /// Connectivity of the feed that produced this record.
    pub stream: StreamStateDto,
    /// Event time of the record itself, milliseconds since the Unix epoch, UTC.
    pub observed_at_unix_ms: i64,
    /// Age of the record at the moment the API answered.
    pub age_ms: i64,
}

/// Whether the venue is currently accepting orders for an instrument.
///
/// This is the venue's session state, not a Vox permission: execution authorization is a
/// separate fact carried by runtime health.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradingStatusDto {
    /// The venue is closed for this instrument.
    Closed,
    /// An auction or another non-continuous phase is running.
    Auction,
    /// Continuous trading.
    Open,
    /// The venue suspended the instrument.
    Suspended,
    /// The provider did not tell us. Not a failure, and never rendered as one.
    Unknown,
}

/// The current session for one instrument.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct SessionDto {
    pub instrument_uid: String,
    pub status: TradingStatusDto,
    /// Whether the venue accepts limit orders right now, when the provider says so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_orders_available: Option<bool>,
    /// Whether the venue accepts market orders right now, when the provider says so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market_orders_available: Option<bool>,
    pub freshness: MarketFreshness,
}

/// Last price and the top of book, with the day's range.
///
/// Absent fields mean the provider did not supply them for this instrument, which is a
/// different thing from a zero and must not be rendered as one.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct QuoteDto {
    pub instrument_uid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bid: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Decimal>,
    /// Absolute change against the previous close.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_absolute: Option<Decimal>,
    /// Relative change against the previous close, as an exact percentage value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_high: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub day_low: Option<Decimal>,
    /// Traded volume in instrument units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_units: Option<i64>,
    pub freshness: MarketFreshness,
}

/// One level of the book.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct DepthLevelDto {
    pub price: Decimal,
    /// Size at this level, in instrument units.
    pub size_units: i64,
    /// Cumulative size from the top of book down to this level.
    pub cumulative_units: i64,
}

/// A book snapshot at one depth.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct OrderBookDto {
    pub instrument_uid: String,
    /// Number of levels the provider returned per side.
    pub depth: u16,
    /// Best ask first.
    pub asks: Vec<DepthLevelDto>,
    /// Best bid first.
    pub bids: Vec<DepthLevelDto>,
    /// Absolute spread between best bid and best ask, when both sides exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread_absolute: Option<Decimal>,
    pub freshness: MarketFreshness,
}

/// Which side initiated a public trade, where the provider reports it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradeDirectionDto {
    Buy,
    Sell,
    Unknown,
}

/// One public trade on the tape.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct TradeTickDto {
    pub instrument_uid: String,
    pub price: Decimal,
    pub size_units: i64,
    pub direction: TradeDirectionDto,
    /// Exchange time of the trade, milliseconds since the Unix epoch, UTC.
    pub traded_at_unix_ms: i64,
    /// True when this trade is one of the operator's own, matched by broker fill identity.
    pub own: bool,
}

/// Candle interval for the Vox read model.
///
/// `#8` `CanonicalCandle.interval` is the already-accepted historic GetCandles
/// integer (`candle_request_constraint`, 1..=16). This enum names that integer;
/// it does not invent a second wire table. Stream support is the separate
/// MarketDataStream `SubscriptionInterval` surface (1..=13).
/// 5s/10s/30s exist on GetCandles only; they are not stream-subscribable.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandleIntervalDto {
    FiveSeconds,
    TenSeconds,
    ThirtySeconds,
    OneMinute,
    TwoMinutes,
    ThreeMinutes,
    FiveMinutes,
    TenMinutes,
    FifteenMinutes,
    ThirtyMinutes,
    OneHour,
    TwoHours,
    FourHours,
    OneDay,
    OneWeek,
    OneMonth,
}

impl CandleIntervalDto {
    /// Every interval this API names. Order matches the public capability list.
    pub const ALL: [Self; 16] = [
        Self::FiveSeconds,
        Self::TenSeconds,
        Self::ThirtySeconds,
        Self::OneMinute,
        Self::TwoMinutes,
        Self::ThreeMinutes,
        Self::FiveMinutes,
        Self::TenMinutes,
        Self::FifteenMinutes,
        Self::ThirtyMinutes,
        Self::OneHour,
        Self::TwoHours,
        Self::FourHours,
        Self::OneDay,
        Self::OneWeek,
        Self::OneMonth,
    ];

    /// Historic GetCandles integer stored on `#8` `CanonicalCandle.interval`.
    #[must_use]
    pub const fn historic_wire(self) -> i32 {
        match self {
            Self::OneMinute => 1,
            Self::FiveMinutes => 2,
            Self::FifteenMinutes => 3,
            Self::OneHour => 4,
            Self::OneDay => 5,
            Self::TwoMinutes => 6,
            Self::ThreeMinutes => 7,
            Self::TenMinutes => 8,
            Self::ThirtyMinutes => 9,
            Self::TwoHours => 10,
            Self::FourHours => 11,
            Self::OneWeek => 12,
            Self::OneMonth => 13,
            Self::FiveSeconds => 14,
            Self::TenSeconds => 15,
            Self::ThirtySeconds => 16,
        }
    }

    /// Names an `#8` `CanonicalCandle.interval`. `0` and unknown values are rejected.
    #[must_use]
    pub const fn from_canonical_interval(interval: i32) -> Option<Self> {
        Self::from_historic_wire(interval)
    }

    /// Inverse of [`historic_wire`]. `0` and unknown values are rejected.
    #[must_use]
    pub const fn from_historic_wire(interval: i32) -> Option<Self> {
        match interval {
            1 => Some(Self::OneMinute),
            2 => Some(Self::FiveMinutes),
            3 => Some(Self::FifteenMinutes),
            4 => Some(Self::OneHour),
            5 => Some(Self::OneDay),
            6 => Some(Self::TwoMinutes),
            7 => Some(Self::ThreeMinutes),
            8 => Some(Self::TenMinutes),
            9 => Some(Self::ThirtyMinutes),
            10 => Some(Self::TwoHours),
            11 => Some(Self::FourHours),
            12 => Some(Self::OneWeek),
            13 => Some(Self::OneMonth),
            14 => Some(Self::FiveSeconds),
            15 => Some(Self::TenSeconds),
            16 => Some(Self::ThirtySeconds),
            _ => None,
        }
    }

    /// Whether MarketDataStream `SubscriptionInterval` accepts this bar size.
    ///
    /// Official stream intervals are 1m through month (wire 1..=13). Second-resolution
    /// bars (5s/10s/30s) are GetCandles-only.
    #[must_use]
    pub const fn market_data_stream_supported(self) -> bool {
        !matches!(
            self,
            Self::FiveSeconds | Self::TenSeconds | Self::ThirtySeconds
        )
    }

    /// Public provenance for this interval. Every named variant is historic-supported.
    #[must_use]
    pub const fn capability(self) -> CandleIntervalCapability {
        CandleIntervalCapability {
            interval: self,
            historical_supported: true,
            streaming_supported: self.market_data_stream_supported(),
        }
    }
}

/// Whether one interval is historic-only, stream-capable, or (by absence) unsupported.
///
/// Unsupported provider integers never appear here. An unknown integer fails as
/// `UNSUPPORTED_CANDLE_INTERVAL`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct CandleIntervalCapability {
    pub interval: CandleIntervalDto,
    /// Accepted by historic GetCandles / `#8` `CanonicalCandle.interval`.
    pub historical_supported: bool,
    /// Accepted by MarketDataStream `SubscriptionInterval`. False for 5s/10s/30s.
    pub streaming_supported: bool,
}

/// Lifecycle of one bar. Replaces a boolean `closed` so OPEN / CLOSED / CORRECTED
/// do not have to be inferred.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandleStateDto {
    /// The bar is still forming. Later publishes of the same open time stay `OPEN`.
    Open,
    /// The bar has settled. First complete publication of this open time.
    Closed,
    /// A previously closed bar was republished with a different body.
    Corrected,
}

/// One candle. `state` is explicit: forming, settled, or a post-close correction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CandleDto {
    pub instrument_uid: String,
    pub interval: CandleIntervalDto,
    /// Opening time of the bar, milliseconds since the Unix epoch, UTC.
    pub opened_at_unix_ms: i64,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume_units: i64,
    pub state: CandleStateDto,
    /// Zero at first publication of this open time. Increments on each later publish.
    pub revision: u64,
}

/// A page of candles for one instrument and interval.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CandlesDto {
    pub instrument_uid: String,
    pub interval: CandleIntervalDto,
    pub candles: Vec<CandleDto>,
    pub freshness: MarketFreshness,
}

/// A catalogue entry: the canonical identity plus what the ticket needs to validate an order.
///
/// Lot size and price step are metadata, not UI rules: without them a quantity field would be
/// guessing, and this API refuses to make the browser guess.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct InstrumentSummaryDto {
    pub identity: InstrumentIdentityDto,
    /// Human-readable instrument name from the provider catalogue.
    pub name: String,
    /// Human-readable instrument kind, for example share, bond or futures.
    pub instrument_type: String,
    /// Instrument units in one lot.
    pub lot_size: i64,
    /// Minimum price increment.
    pub min_price_increment: Decimal,
    /// Settlement currency code.
    #[schema(example = "rub")]
    pub currency: String,
    /// Whether the provider currently lists the instrument as tradable.
    pub tradable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_states_its_age_and_carries_no_float() -> Result<(), Box<dyn std::error::Error>> {
        let quote = QuoteDto {
            instrument_uid: "e6123145-9665-43e0-8413-cd61b8aa9b13".to_owned(),
            last: Some(Decimal::from_units_nano(272, 550_000_000)?),
            bid: Some(Decimal::from_units_nano(272, 540_000_000)?),
            ask: Some(Decimal::from_units_nano(272, 560_000_000)?),
            change_absolute: None,
            change_percent: None,
            day_high: None,
            day_low: None,
            volume_units: Some(18_400_000),
            freshness: MarketFreshness {
                stream: StreamStateDto::Active,
                observed_at_unix_ms: 1_787_000_000_000,
                age_ms: 42,
            },
        };
        let json = serde_json::to_value(&quote)?;
        assert_eq!(
            json["last"], "272.550000000",
            "price must be an exact string"
        );
        assert_eq!(json["freshness"]["age_ms"], 42);
        assert!(
            json.get("change_absolute").is_none(),
            "an absent field is absent, not zero"
        );
        Ok(())
    }

    #[test]
    fn an_open_candle_is_marked_as_still_forming() -> Result<(), Box<dyn std::error::Error>> {
        let candle = CandleDto {
            instrument_uid: "uid".to_owned(),
            interval: CandleIntervalDto::FiveMinutes,
            opened_at_unix_ms: 1,
            open: Decimal::from_units_nano(1, 0)?,
            high: Decimal::from_units_nano(2, 0)?,
            low: Decimal::from_units_nano(1, 0)?,
            close: Decimal::from_units_nano(2, 0)?,
            volume_units: 10,
            state: CandleStateDto::Open,
            revision: 0,
        };
        let json = serde_json::to_value(&candle)?;
        assert_eq!(json["state"], "OPEN");
        assert_eq!(json["revision"], 0);
        assert_eq!(json["interval"], "FIVE_MINUTES");
        Ok(())
    }

    #[test]
    fn historic_get_candles_wire_covers_the_accepted_set() {
        let expected = [
            (1, CandleIntervalDto::OneMinute, true),
            (2, CandleIntervalDto::FiveMinutes, true),
            (3, CandleIntervalDto::FifteenMinutes, true),
            (4, CandleIntervalDto::OneHour, true),
            (5, CandleIntervalDto::OneDay, true),
            (6, CandleIntervalDto::TwoMinutes, true),
            (7, CandleIntervalDto::ThreeMinutes, true),
            (8, CandleIntervalDto::TenMinutes, true),
            (9, CandleIntervalDto::ThirtyMinutes, true),
            (10, CandleIntervalDto::TwoHours, true),
            (11, CandleIntervalDto::FourHours, true),
            (12, CandleIntervalDto::OneWeek, true),
            (13, CandleIntervalDto::OneMonth, true),
            (14, CandleIntervalDto::FiveSeconds, false),
            (15, CandleIntervalDto::TenSeconds, false),
            (16, CandleIntervalDto::ThirtySeconds, false),
        ];
        for (wire, interval, stream) in expected {
            assert_eq!(
                CandleIntervalDto::from_historic_wire(wire),
                Some(interval),
                "historic wire {wire}"
            );
            assert_eq!(interval.historic_wire(), wire);
            assert_eq!(
                interval.market_data_stream_supported(),
                stream,
                "{interval:?} stream provenance"
            );
        }
        assert_eq!(CandleIntervalDto::from_historic_wire(0), None);
        assert_eq!(CandleIntervalDto::from_historic_wire(17), None);
    }

    #[test]
    fn five_second_candles_serialize_as_historic_not_stream() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_string(&CandleIntervalDto::FiveSeconds)?,
            "\"FIVE_SECONDS\""
        );
        assert_eq!(
            serde_json::to_string(&CandleStateDto::Corrected)?,
            "\"CORRECTED\""
        );
        assert!(
            !CandleIntervalDto::FiveSeconds.market_data_stream_supported(),
            "5s is GetCandles-only"
        );
        Ok(())
    }

    #[test]
    fn every_named_interval_has_explicit_historic_vs_stream_capability()
    -> Result<(), serde_json::Error> {
        assert_eq!(CandleIntervalDto::ALL.len(), 16);
        for interval in CandleIntervalDto::ALL {
            let capability = interval.capability();
            assert!(capability.historical_supported, "{interval:?} is historic");
            assert_eq!(
                capability.streaming_supported,
                interval.market_data_stream_supported()
            );
            assert_eq!(
                CandleIntervalDto::from_canonical_interval(interval.historic_wire()),
                Some(interval)
            );
        }
        let json = serde_json::to_value(CandleIntervalDto::FiveSeconds.capability())?;
        assert_eq!(json["interval"], "FIVE_SECONDS");
        assert_eq!(json["historical_supported"], true);
        assert_eq!(json["streaming_supported"], false);
        let streamable = serde_json::to_value(CandleIntervalDto::OneMinute.capability())?;
        assert_eq!(streamable["streaming_supported"], true);
        Ok(())
    }

    #[test]
    fn candle_prices_and_volumes_are_never_floats() -> Result<(), Box<dyn std::error::Error>> {
        let candle = CandleDto {
            instrument_uid: "uid".to_owned(),
            interval: CandleIntervalDto::TenSeconds,
            opened_at_unix_ms: 1,
            open: Decimal::from_units_nano(1, 250_000_000)?,
            high: Decimal::from_units_nano(1, 500_000_000)?,
            low: Decimal::from_units_nano(1, 0)?,
            close: Decimal::from_units_nano(1, 250_000_000)?,
            volume_units: 42,
            state: CandleStateDto::Closed,
            revision: 0,
        };
        let json = serde_json::to_value(&candle)?;
        for field in ["open", "high", "low", "close"] {
            assert!(json[field].is_string(), "{field} must be an exact string");
        }
        assert!(json["volume_units"].is_number());
        assert!(!json["volume_units"].is_f64() || json["volume_units"].as_i64() == Some(42));
        assert_eq!(json["state"], "CLOSED");
        assert_eq!(json["revision"], 0);
        Ok(())
    }

    #[test]
    fn unknown_trading_status_is_its_own_value() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_string(&TradingStatusDto::Unknown)?,
            "\"UNKNOWN\""
        );
        Ok(())
    }
}
