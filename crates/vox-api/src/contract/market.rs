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

/// Candle interval. Only intervals the provider actually serves appear here.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandleIntervalDto {
    OneMinute,
    FiveMinutes,
    FifteenMinutes,
    OneHour,
    OneDay,
}

/// One candle. `closed` distinguishes a settled bar from the one still forming.
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
    /// False while the bar is still forming. An open bar may be revised.
    pub closed: bool,
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
            closed: false,
        };
        let json = serde_json::to_value(&candle)?;
        assert_eq!(json["closed"], false);
        assert_eq!(json["interval"], "FIVE_MINUTES");
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
