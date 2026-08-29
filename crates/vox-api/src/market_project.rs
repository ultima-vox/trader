//! Vox-side market-data projection over accepted #8 canonical facts.
//!
//! This is not a broker client. Callers publish already-acquired quotes, books, trades,
//! candles, sessions and catalogue entries. The projection stores them and republishes
//! provider-neutrally through [`MarketDataQueries`]. An empty store is not attached to
//! `vox-server`; tests and composition attach it when a source exists.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use vox_domain::FixedPoint;

use crate::application::MarketDataQueries;
use crate::contract::market::{
    CandleDto, CandleIntervalDto, CandleStateDto, CandlesDto, DepthLevelDto, InstrumentSummaryDto,
    MarketFreshness, OrderBookDto, QuoteDto, SessionDto, TradeDirectionDto, TradeTickDto,
    TradingStatusDto,
};
use crate::contract::money::Decimal;
use crate::contract::runtime::StreamStateDto;
use crate::contract::scope::ProviderDto;
use crate::error::{ApiError, ErrorCategory};
use crate::events::{ApplicationEvent, ApplicationEventBus};
use crate::transport::http::now_unix_ms;

#[derive(Clone, Debug)]
struct StoredQuote {
    quote: QuoteDto,
}

#[derive(Clone, Debug)]
struct StoredBook {
    book: OrderBookDto,
}

#[derive(Clone, Debug)]
struct StoredTape {
    trades: Vec<TradeTickDto>,
}

#[derive(Clone, Debug, Default)]
struct StoredCandles {
    by_interval: BTreeMap<CandleIntervalDto, CandlesDto>,
}

#[derive(Clone, Debug)]
struct StoredSession {
    session: SessionDto,
}

#[derive(Default)]
struct Inner {
    quotes: BTreeMap<String, StoredQuote>,
    books: BTreeMap<String, StoredBook>,
    tapes: BTreeMap<String, StoredTape>,
    candles: BTreeMap<String, StoredCandles>,
    sessions: BTreeMap<String, StoredSession>,
    instruments: Vec<InstrumentSummaryDto>,
}

/// In-memory projection of #8 facts. Publish is explicit; nothing is inferred.
#[derive(Default)]
pub struct SnapshotMarketProjection {
    inner: Mutex<Inner>,
    events: Option<ApplicationEventBus>,
}

impl SnapshotMarketProjection {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Live publishes also fan out on this bus. REST reads still work without it.
    #[must_use]
    pub fn with_events(mut self, events: ApplicationEventBus) -> Self {
        self.events = Some(events);
        self
    }

    pub fn publish_quote(&self, quote: QuoteDto) {
        {
            let mut inner = lock(&self.inner);
            inner.quotes.insert(
                quote.instrument_uid.clone(),
                StoredQuote {
                    quote: quote.clone(),
                },
            );
        }
        self.emit(ApplicationEvent::Quote(quote));
    }

    pub fn publish_order_book(&self, book: OrderBookDto) {
        {
            let mut inner = lock(&self.inner);
            inner.books.insert(
                book.instrument_uid.clone(),
                StoredBook { book: book.clone() },
            );
        }
        self.emit(ApplicationEvent::OrderBook(book));
    }

    pub fn publish_trades(&self, instrument_uid: String, trades: Vec<TradeTickDto>) {
        {
            let mut inner = lock(&self.inner);
            inner.tapes.insert(
                instrument_uid.clone(),
                StoredTape {
                    trades: trades.clone(),
                },
            );
        }
        self.emit(ApplicationEvent::Trades {
            instrument_uid,
            ticks: trades,
        });
    }

    pub fn publish_candles(&self, mut candles: CandlesDto) {
        let changed = {
            let mut inner = lock(&self.inner);
            let prior_page = inner
                .candles
                .get(&candles.instrument_uid)
                .and_then(|row| row.by_interval.get(&candles.interval))
                .cloned();
            let mut changed = prior_page.is_none();
            if let Some(existing) = &prior_page {
                if existing.candles.len() != candles.candles.len() {
                    changed = true;
                }
                for candle in &mut candles.candles {
                    if let Some(prior) = existing
                        .candles
                        .iter()
                        .find(|row| row.opened_at_unix_ms == candle.opened_at_unix_ms)
                    {
                        apply_candle_lifecycle(prior, candle);
                        if prior.revision != candle.revision || prior.state != candle.state {
                            changed = true;
                        }
                    } else {
                        changed = true;
                    }
                }
            }
            inner
                .candles
                .entry(candles.instrument_uid.clone())
                .or_default()
                .by_interval
                .insert(candles.interval, candles.clone());
            changed
        };
        if changed {
            self.emit(ApplicationEvent::Candles(candles));
        }
    }

    fn emit(&self, event: ApplicationEvent) {
        if let Some(events) = &self.events {
            events.publish(event);
        }
    }

    pub fn publish_session(&self, session: SessionDto) {
        let changed = {
            let mut inner = lock(&self.inner);
            let changed = inner
                .sessions
                .get(&session.instrument_uid)
                .is_none_or(|stored| {
                    stored.session.status != session.status
                        || stored.session.limit_orders_available != session.limit_orders_available
                        || stored.session.market_orders_available != session.market_orders_available
                });
            inner.sessions.insert(
                session.instrument_uid.clone(),
                StoredSession {
                    session: session.clone(),
                },
            );
            changed
        };
        if changed {
            self.emit(ApplicationEvent::Session(session));
        }
    }

    pub fn publish_instrument(&self, instrument: InstrumentSummaryDto) {
        let mut inner = lock(&self.inner);
        let uid = instrument.identity.uid.clone();
        inner.instruments.retain(|row| row.identity.uid != uid);
        inner.instruments.push(instrument);
    }
}

#[async_trait]
impl MarketDataQueries for SnapshotMarketProjection {
    async fn search_instruments(
        &self,
        _provider: ProviderDto,
        query: &str,
        limit: u16,
    ) -> Result<Vec<InstrumentSummaryDto>, ApiError> {
        let needle = query.trim().to_ascii_lowercase();
        let inner = lock(&self.inner);
        Ok(inner
            .instruments
            .iter()
            .filter(|row| {
                if needle.is_empty() {
                    true
                } else {
                    row.identity.ticker.to_ascii_lowercase().contains(&needle)
                        || row
                            .identity
                            .class_code
                            .to_ascii_lowercase()
                            .contains(&needle)
                        || row.identity.uid.to_ascii_lowercase().contains(&needle)
                }
            })
            .take(usize::from(limit))
            .cloned()
            .collect())
    }

    async fn quote(
        &self,
        _provider: ProviderDto,
        instrument_uid: &str,
    ) -> Result<QuoteDto, ApiError> {
        let inner = lock(&self.inner);
        let stored = inner
            .quotes
            .get(instrument_uid)
            .ok_or_else(|| missing("quote", instrument_uid))?;
        Ok(refresh_quote(&stored.quote))
    }

    async fn order_book(
        &self,
        _provider: ProviderDto,
        instrument_uid: &str,
        depth: u16,
    ) -> Result<OrderBookDto, ApiError> {
        let inner = lock(&self.inner);
        let stored = inner
            .books
            .get(instrument_uid)
            .ok_or_else(|| missing("order book", instrument_uid))?;
        Ok(trim_book(&stored.book, depth))
    }

    async fn trades(
        &self,
        _provider: ProviderDto,
        instrument_uid: &str,
        limit: u16,
    ) -> Result<Vec<TradeTickDto>, ApiError> {
        let inner = lock(&self.inner);
        let stored = inner
            .tapes
            .get(instrument_uid)
            .ok_or_else(|| missing("tape", instrument_uid))?;
        let limit = usize::from(limit);
        let start = stored.trades.len().saturating_sub(limit);
        Ok(stored.trades[start..].to_vec())
    }

    async fn candles(
        &self,
        _provider: ProviderDto,
        instrument_uid: &str,
        interval: CandleIntervalDto,
        from_unix_ms: i64,
        to_unix_ms: i64,
    ) -> Result<CandlesDto, ApiError> {
        let inner = lock(&self.inner);
        let stored = inner
            .candles
            .get(instrument_uid)
            .and_then(|row| row.by_interval.get(&interval))
            .ok_or_else(|| missing("candles", instrument_uid))?;
        let candles = stored
            .candles
            .iter()
            .filter(|candle| {
                candle.opened_at_unix_ms >= from_unix_ms && candle.opened_at_unix_ms < to_unix_ms
            })
            .cloned()
            .collect();
        Ok(CandlesDto {
            instrument_uid: stored.instrument_uid.clone(),
            interval: stored.interval,
            candles,
            freshness: age_freshness(&stored.freshness),
        })
    }

    async fn session(
        &self,
        _provider: ProviderDto,
        instrument_uid: &str,
    ) -> Result<SessionDto, ApiError> {
        let inner = lock(&self.inner);
        let stored = inner
            .sessions
            .get(instrument_uid)
            .ok_or_else(|| missing("session", instrument_uid))?;
        let mut session = stored.session.clone();
        session.freshness = age_freshness(&session.freshness);
        Ok(session)
    }
}

/// Maps a #8 last-price fact into the public quote. Bid/ask stay absent until a book is published.
#[must_use]
pub fn quote_from_last_price(
    instrument_uid: impl Into<String>,
    last: FixedPoint,
    observed_at_unix_ms: i64,
    stream: StreamStateDto,
) -> QuoteDto {
    QuoteDto {
        instrument_uid: instrument_uid.into(),
        last: Some(Decimal::from_fixed_point(last)),
        bid: None,
        ask: None,
        change_absolute: None,
        change_percent: None,
        day_high: None,
        day_low: None,
        volume_units: None,
        freshness: MarketFreshness {
            stream,
            observed_at_unix_ms,
            age_ms: 0,
        },
    }
}

/// Maps a #8 order book. Levels use lots as units; no invented lot-to-unit conversion.
pub fn order_book_from_levels(
    instrument_uid: impl Into<String>,
    bids: &[(FixedPoint, i64)],
    asks: &[(FixedPoint, i64)],
    observed_at_unix_ms: i64,
    stream: StreamStateDto,
    consistent: bool,
) -> Result<OrderBookDto, ApiError> {
    if !consistent {
        return Err(ApiError::new(
            ErrorCategory::Conflict,
            "INCONSISTENT_ORDER_BOOK",
            "provider order book is inconsistent; refusing to project it",
        ));
    }
    let spread_absolute = match (bids.first(), asks.first()) {
        (Some((bid, _)), Some((ask, _))) => Some(Decimal::from_fixed_point(
            FixedPoint::from_total_nanos(ask.total_nanos() - bid.total_nanos()),
        )),
        _ => None,
    };
    let bids = accumulate(bids)?;
    let asks = accumulate(asks)?;
    let depth = u16::try_from(bids.len().max(asks.len())).unwrap_or(u16::MAX);
    Ok(OrderBookDto {
        instrument_uid: instrument_uid.into(),
        depth,
        asks,
        bids,
        spread_absolute,
        freshness: MarketFreshness {
            stream,
            observed_at_unix_ms,
            age_ms: 0,
        },
    })
}

/// Maps a #8 public trade. Direction wire values: 1 buy, 2 sell, anything else unknown.
#[must_use]
pub fn trade_from_canonical(
    instrument_uid: impl Into<String>,
    price: FixedPoint,
    size_lots: i64,
    direction: i32,
    traded_at_unix_ms: i64,
) -> TradeTickDto {
    TradeTickDto {
        instrument_uid: instrument_uid.into(),
        price: Decimal::from_fixed_point(price),
        size_units: size_lots,
        direction: match direction {
            1 => TradeDirectionDto::Buy,
            2 => TradeDirectionDto::Sell,
            _ => TradeDirectionDto::Unknown,
        },
        traded_at_unix_ms,
        own: false,
    }
}

/// Maps a #8 candle. `is_complete` is the historic fact; stream candles leave it `None`
/// and stay [`CandleStateDto::Open`] until a later complete publication.
#[allow(
    clippy::too_many_arguments,
    reason = "maps one canonical candle field-for-field; grouping would invent a second DTO"
)]
pub fn candle_from_canonical(
    instrument_uid: impl Into<String>,
    interval: CandleIntervalDto,
    opened_at_unix_ms: i64,
    open: FixedPoint,
    high: FixedPoint,
    low: FixedPoint,
    close: FixedPoint,
    volume_lots: i64,
    is_complete: Option<bool>,
) -> CandleDto {
    CandleDto {
        instrument_uid: instrument_uid.into(),
        interval,
        opened_at_unix_ms,
        open: Decimal::from_fixed_point(open),
        high: Decimal::from_fixed_point(high),
        low: Decimal::from_fixed_point(low),
        close: Decimal::from_fixed_point(close),
        volume_units: volume_lots,
        state: match is_complete {
            Some(true) => CandleStateDto::Closed,
            Some(false) | None => CandleStateDto::Open,
        },
        revision: 0,
    }
}

/// Maps T-Invest `SecurityTradingStatus` wire numbers from the pinned proto, without
/// importing protobuf types into this crate.
#[must_use]
pub fn trading_status_from_provider(status: i32) -> TradingStatusDto {
    match status {
        5 | 13 | 14 => TradingStatusDto::Open,
        2 | 6 | 7 | 8 | 9 | 10 | 17 => TradingStatusDto::Auction,
        1 | 3 | 12 | 16 => TradingStatusDto::Closed,
        4 | 15 => TradingStatusDto::Suspended,
        _ => TradingStatusDto::Unknown,
    }
}

/// Names an `#8` `CanonicalCandle.interval`. Not a MarketDataStream `SubscriptionInterval`.
pub fn candle_interval_from_provider(interval: i32) -> Result<CandleIntervalDto, ApiError> {
    CandleIntervalDto::from_canonical_interval(interval).ok_or_else(|| {
        ApiError::new(
            ErrorCategory::Validation,
            "UNSUPPORTED_CANDLE_INTERVAL",
            format!("candle interval {interval} is not in the public Vox historic set"),
        )
    })
}

/// Nanoseconds since epoch → milliseconds. Truncates toward zero; no float step.
#[must_use]
pub fn unix_ms_from_ns(event_time_ns: u64) -> i64 {
    i64::try_from(event_time_ns / 1_000_000).unwrap_or(i64::MAX)
}

fn accumulate(levels: &[(FixedPoint, i64)]) -> Result<Vec<DepthLevelDto>, ApiError> {
    let mut out = Vec::with_capacity(levels.len());
    let mut cumulative = 0_i64;
    for (price, size) in levels {
        if *size < 0 {
            return Err(ApiError::new(
                ErrorCategory::Validation,
                "INVALID_BOOK_SIZE",
                "order-book size cannot be negative",
            ));
        }
        cumulative = cumulative.saturating_add(*size);
        out.push(DepthLevelDto {
            price: Decimal::from_fixed_point(*price),
            size_units: *size,
            cumulative_units: cumulative,
        });
    }
    Ok(out)
}

fn refresh_quote(quote: &QuoteDto) -> QuoteDto {
    let mut quote = quote.clone();
    quote.freshness = age_freshness(&quote.freshness);
    quote
}

fn trim_book(book: &OrderBookDto, depth: u16) -> OrderBookDto {
    let take = usize::from(depth);
    let mut book = book.clone();
    book.bids.truncate(take);
    book.asks.truncate(take);
    book.depth = u16::try_from(book.bids.len().max(book.asks.len())).unwrap_or(depth);
    book.freshness = age_freshness(&book.freshness);
    book
}

fn age_freshness(freshness: &MarketFreshness) -> MarketFreshness {
    let now = now_unix_ms();
    MarketFreshness {
        stream: freshness.stream,
        observed_at_unix_ms: freshness.observed_at_unix_ms,
        age_ms: (now - freshness.observed_at_unix_ms).max(0),
    }
}

/// Authoritative candle body. Freshness/age and revision/state are not part of the body.
fn candle_body_eq(left: &CandleDto, right: &CandleDto) -> bool {
    left.instrument_uid == right.instrument_uid
        && left.interval == right.interval
        && left.opened_at_unix_ms == right.opened_at_unix_ms
        && left.open == right.open
        && left.high == right.high
        && left.low == right.low
        && left.close == right.close
        && left.volume_units == right.volume_units
}

/// Revision rule for one bar of the same open time.
///
/// - OPEN → OPEN, same body: keep revision, stay OPEN
/// - OPEN → OPEN, body changed: revision += 1, stay OPEN
/// - OPEN → CLOSED: revision += 1, CLOSED
/// - CLOSED → identical CLOSED: keep revision, stay CLOSED (not CORRECTED)
/// - CLOSED → changed CLOSED: revision += 1, CORRECTED
fn apply_candle_lifecycle(prior: &CandleDto, incoming: &mut CandleDto) {
    let same_body = candle_body_eq(prior, incoming);
    match (prior.state, incoming.state) {
        (CandleStateDto::Open, CandleStateDto::Open) if same_body => {
            incoming.revision = prior.revision;
        }
        (CandleStateDto::Open, CandleStateDto::Open) => {
            incoming.revision = prior.revision.saturating_add(1);
        }
        (CandleStateDto::Open, CandleStateDto::Closed | CandleStateDto::Corrected) => {
            incoming.revision = prior.revision.saturating_add(1);
            incoming.state = CandleStateDto::Closed;
        }
        (
            CandleStateDto::Closed | CandleStateDto::Corrected,
            CandleStateDto::Closed | CandleStateDto::Corrected,
        ) if same_body => {
            incoming.revision = prior.revision;
            incoming.state = prior.state;
        }
        (CandleStateDto::Closed | CandleStateDto::Corrected, _) => {
            incoming.revision = prior.revision.saturating_add(1);
            incoming.state = CandleStateDto::Corrected;
        }
    }
}

fn missing(kind: &str, instrument_uid: &str) -> ApiError {
    ApiError::new(
        ErrorCategory::NotFound,
        "MARKET_FACT_NOT_FOUND",
        format!("no {kind} in the projection for {instrument_uid}"),
    )
}

fn lock(inner: &Mutex<Inner>) -> std::sync::MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::instrument::InstrumentIdentityDto;

    fn fp(units: i64, nano: i32) -> Result<FixedPoint, vox_domain::FixedPointError> {
        FixedPoint::from_units_nano(units, nano)
    }

    #[tokio::test]
    async fn projection_republishes_published_facts_and_ages_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let projection = SnapshotMarketProjection::new();
        projection.publish_quote(quote_from_last_price(
            "uid-sber",
            fp(272, 550_000_000)?,
            1_000,
            StreamStateDto::Active,
        ));
        let quote = projection.quote(ProviderDto::TInvest, "uid-sber").await?;
        assert_eq!(
            quote.last.as_ref().map(Decimal::as_str),
            Some("272.550000000")
        );
        assert!(quote.bid.is_none(), "absent bid stays absent");
        assert!(quote.freshness.age_ms >= 0);
        Ok(())
    }

    #[tokio::test]
    async fn missing_instrument_is_not_found_not_a_zero_price() {
        let projection = SnapshotMarketProjection::new();
        let error = match projection.quote(ProviderDto::TInvest, "missing").await {
            Err(error) => error,
            Ok(_) => panic!("missing quote must not invent a price"),
        };
        assert_eq!(error.code, "MARKET_FACT_NOT_FOUND");
    }

    #[test]
    fn inconsistent_book_is_refused() -> Result<(), vox_domain::FixedPointError> {
        let error = match order_book_from_levels(
            "uid",
            &[(fp(1, 0)?, 1)],
            &[(fp(2, 0)?, 1)],
            1,
            StreamStateDto::Active,
            false,
        ) {
            Err(error) => error,
            Ok(_) => panic!("inconsistent book must be refused"),
        };
        assert_eq!(error.code, "INCONSISTENT_ORDER_BOOK");
        Ok(())
    }

    #[test]
    fn provider_trading_status_wire_numbers_map_without_protobuf() {
        assert_eq!(trading_status_from_provider(5), TradingStatusDto::Open);
        assert_eq!(trading_status_from_provider(9), TradingStatusDto::Auction);
        assert_eq!(trading_status_from_provider(1), TradingStatusDto::Closed);
        assert_eq!(trading_status_from_provider(4), TradingStatusDto::Suspended);
        assert_eq!(trading_status_from_provider(0), TradingStatusDto::Unknown);
    }

    #[tokio::test]
    async fn catalogue_search_is_projection_not_a_second_broker_client()
    -> Result<(), Box<dyn std::error::Error>> {
        let projection = SnapshotMarketProjection::new();
        projection.publish_instrument(InstrumentSummaryDto {
            identity: InstrumentIdentityDto {
                provider: "tinvest".into(),
                uid: "uid-sber".into(),
                figi: Some("BBG004730N88".into()),
                ticker: "SBER".into(),
                class_code: "TQBR".into(),
            },
            lot_size: 10,
            min_price_increment: Decimal::from_units_nano(0, 10_000_000)?,
            currency: "rub".into(),
            tradable: true,
        });
        let hits = projection
            .search_instruments(ProviderDto::TInvest, "sber", 10)
            .await?;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].identity.ticker, "SBER");
        Ok(())
    }

    #[test]
    fn unix_ms_from_ns_is_exact_truncation() {
        assert_eq!(
            unix_ms_from_ns(1_787_000_000_000_000_000),
            1_787_000_000_000
        );
    }

    #[test]
    fn historic_interval_mapping_covers_five_seconds_and_rejects_unknown() {
        assert_eq!(
            candle_interval_from_provider(14).expect("5s"),
            CandleIntervalDto::FiveSeconds
        );
        assert_eq!(
            candle_interval_from_provider(16).expect("30s"),
            CandleIntervalDto::ThirtySeconds
        );
        assert_eq!(
            candle_interval_from_provider(1).expect("1m"),
            CandleIntervalDto::OneMinute
        );
        let error = candle_interval_from_provider(0).expect_err("unspecified");
        assert_eq!(error.code, "UNSUPPORTED_CANDLE_INTERVAL");
    }

    fn publish_bar(
        projection: &SnapshotMarketProjection,
        bar: CandleDto,
        observed_at_unix_ms: i64,
    ) {
        projection.publish_candles(CandlesDto {
            instrument_uid: bar.instrument_uid.clone(),
            interval: bar.interval,
            candles: vec![bar],
            freshness: MarketFreshness {
                stream: StreamStateDto::Active,
                observed_at_unix_ms,
                age_ms: 0,
            },
        });
    }

    fn stored_bar(projection: &SnapshotMarketProjection) -> CandleDto {
        lock(&projection.inner)
            .candles
            .get("uid")
            .and_then(|row| row.by_interval.get(&CandleIntervalDto::FiveSeconds))
            .and_then(|page| page.candles.first().cloned())
            .expect("stored candle")
    }

    fn bar(
        high: i64,
        close_nano: i32,
        volume: i64,
        complete: Option<bool>,
    ) -> Result<CandleDto, Box<dyn std::error::Error>> {
        Ok(candle_from_canonical(
            "uid",
            CandleIntervalDto::FiveSeconds,
            1_000,
            fp(1, 0)?,
            fp(high, 0)?,
            fp(1, 0)?,
            fp(1, close_nano)?,
            volume,
            complete,
        ))
    }

    #[test]
    fn candle_lifecycle_is_idempotent_and_corrects_only_on_body_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let projection = SnapshotMarketProjection::new();

        publish_bar(&projection, bar(2, 0, 10, Some(false))?, 1_000);
        let open = stored_bar(&projection);
        assert_eq!(open.state, CandleStateDto::Open);
        assert_eq!(open.revision, 0);

        publish_bar(&projection, bar(2, 0, 10, Some(false))?, 1_010);
        let open_same = stored_bar(&projection);
        assert_eq!(open_same.state, CandleStateDto::Open);
        assert_eq!(open_same.revision, 0, "identical OPEN republish is a no-op");

        publish_bar(&projection, bar(3, 250_000_000, 11, Some(false))?, 1_020);
        let open_changed = stored_bar(&projection);
        assert_eq!(open_changed.state, CandleStateDto::Open);
        assert_eq!(
            open_changed.revision, 1,
            "forming body change advances revision"
        );

        publish_bar(&projection, bar(3, 250_000_000, 11, Some(true))?, 1_030);
        let closed = stored_bar(&projection);
        assert_eq!(closed.state, CandleStateDto::Closed);
        assert_eq!(closed.revision, 2);

        publish_bar(&projection, bar(3, 250_000_000, 11, Some(true))?, 1_040);
        let closed_same = stored_bar(&projection);
        assert_eq!(closed_same.state, CandleStateDto::Closed);
        assert_eq!(
            closed_same.revision, 2,
            "identical CLOSED republish is not CORRECTED"
        );

        publish_bar(&projection, bar(4, 500_000_000, 12, Some(true))?, 1_050);
        let corrected = stored_bar(&projection);
        assert_eq!(corrected.state, CandleStateDto::Corrected);
        assert_eq!(corrected.revision, 3);
        Ok(())
    }

    #[test]
    fn identical_closed_republish_does_not_emit_a_live_event()
    -> Result<(), Box<dyn std::error::Error>> {
        let events = crate::events::ApplicationEventBus::new();
        let mut live = events.subscribe();
        let projection = SnapshotMarketProjection::new().with_events(events);
        publish_bar(&projection, bar(2, 0, 10, Some(true))?, 1_000);
        assert!(matches!(
            live.try_recv(),
            Ok(crate::events::ApplicationEvent::Candles(_))
        ));
        publish_bar(&projection, bar(2, 0, 10, Some(true))?, 1_100);
        assert!(
            live.try_recv().is_err(),
            "duplicate historic page must not look like a correction"
        );
        Ok(())
    }
}
