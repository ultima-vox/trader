//! Generated gRPC market-data stream supervision.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use prost::Message;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

use crate::generated::v1;
use crate::market_data::{
    CanonicalCandle, CanonicalLastPrice, CanonicalOpenInterest, CanonicalOrderBook, CanonicalTrade,
    CanonicalTradingStatus, DEFAULT_PING_DELAY_MS, MAX_SUBSCRIPTION_REQUESTS_PER_MINUTE,
    MarketDataError, MarketSubscriptionRegistry, SubscriptionKind, validate_ping_delay,
};
use crate::{GrpcError, GrpcStreamError, TInvestGrpcClient};

#[derive(Clone, Debug)]
pub struct MarketDataSupervisorConfig {
    pub outbound_capacity: usize,
    pub event_capacity: usize,
    pub stale_timeout: Duration,
    pub reconnect_initial_delay: Duration,
    pub reconnect_max_delay: Duration,
    pub max_reconnect_attempts: u32,
    pub ping_delay_ms: i64,
}

impl Default for MarketDataSupervisorConfig {
    fn default() -> Self {
        Self {
            outbound_capacity: 64,
            event_capacity: 1_024,
            stale_timeout: Duration::from_secs(150),
            reconnect_initial_delay: Duration::from_millis(250),
            reconnect_max_delay: Duration::from_secs(15),
            max_reconnect_attempts: 8,
            ping_delay_ms: DEFAULT_PING_DELAY_MS,
        }
    }
}

impl MarketDataSupervisorConfig {
    pub fn validate(&self) -> Result<(), MarketDataSupervisorError> {
        if self.outbound_capacity == 0 || self.event_capacity == 0 {
            return Err(MarketDataSupervisorError::ZeroCapacity);
        }
        if self.stale_timeout.is_zero()
            || self.reconnect_initial_delay.is_zero()
            || self.reconnect_max_delay < self.reconnect_initial_delay
            || self.max_reconnect_attempts == 0
        {
            return Err(MarketDataSupervisorError::InvalidReconnectPolicy);
        }
        validate_ping_delay(self.ping_delay_ms).map_err(MarketDataSupervisorError::MarketData)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MarketDataSupervisorError {
    #[error("stream queue capacities must be positive")]
    ZeroCapacity,
    #[error("invalid reconnect policy")]
    InvalidReconnectPolicy,
    #[error("desired subscriptions need more than provider maximum 100 requests per minute")]
    SubscriptionRequestRate,
    #[error("{0}")]
    MarketData(MarketDataError),
    #[error("{0}")]
    Connect(GrpcError),
    #[error("{0}")]
    Stream(GrpcStreamError),
    #[error("market-data stream became stale")]
    Stale,
    #[error("market-data stream closed")]
    Closed,
    #[error("market-data event does not match adapter desired state")]
    UndesiredEvent,
    #[error("reconnect attempts exhausted")]
    ReconnectExhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarketDataStreamEvent {
    Connected,
    Reconnecting {
        attempt: u32,
        delay: Duration,
    },
    Acknowledged {
        count: usize,
    },
    Candle(CanonicalCandle),
    Trade(CanonicalTrade),
    OrderBook(CanonicalOrderBook),
    TradingStatus(CanonicalTradingStatus),
    LastPrice(CanonicalLastPrice),
    OpenInterest(CanonicalOpenInterest),
    Ping {
        stream_id: String,
        event_time_ns: Option<u64>,
    },
    Dropped {
        instrument_uid: String,
        family: &'static str,
        out_of_order: bool,
    },
    Fault(MarketDataSupervisorError),
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum EventFamily {
    Candle,
    Trade,
    TradingStatus,
    LastPrice,
    OpenInterest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EventDisposition {
    Accept,
    Duplicate,
    OutOfOrder,
}

#[derive(Default)]
struct EventSequenceGate {
    seen: BTreeMap<EventKey, EventFingerprints>,
}

type EventKey = (String, EventFamily);
type EventFingerprints = (u64, BTreeSet<Vec<u8>>);

impl EventSequenceGate {
    fn observe(
        &mut self,
        instrument_uid: &str,
        family: EventFamily,
        event_time_ns: u64,
        fingerprint: Vec<u8>,
    ) -> EventDisposition {
        let key = (instrument_uid.to_owned(), family);
        match self.seen.get_mut(&key) {
            None => {
                self.seen
                    .insert(key, (event_time_ns, BTreeSet::from([fingerprint])));
                EventDisposition::Accept
            }
            Some((latest, _)) if event_time_ns < *latest => EventDisposition::OutOfOrder,
            Some((latest, fingerprints)) if event_time_ns == *latest => {
                if fingerprints.insert(fingerprint) {
                    EventDisposition::Accept
                } else {
                    EventDisposition::Duplicate
                }
            }
            Some((latest, fingerprints)) => {
                *latest = event_time_ns;
                fingerprints.clear();
                fingerprints.insert(fingerprint);
                EventDisposition::Accept
            }
        }
    }
}

pub struct MarketDataStreamHandle {
    events: mpsc::Receiver<MarketDataStreamEvent>,
    stop: watch::Sender<bool>,
}

impl MarketDataStreamHandle {
    pub async fn recv(&mut self) -> Option<MarketDataStreamEvent> {
        self.events.recv().await
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

#[derive(Clone)]
pub struct MarketDataStreamSupervisor {
    connector: Arc<dyn MarketDataStreamConnector>,
    config: MarketDataSupervisorConfig,
}

#[async_trait]
pub trait MarketDataStreamConnection: Send {
    async fn send(
        &mut self,
        request: v1::MarketDataRequest,
    ) -> Result<(), MarketDataSupervisorError>;
    async fn message(
        &mut self,
    ) -> Result<Option<v1::MarketDataResponse>, MarketDataSupervisorError>;
}

#[async_trait]
pub trait MarketDataStreamConnector: Send + Sync {
    async fn connect(
        &self,
        outbound_capacity: usize,
    ) -> Result<Box<dyn MarketDataStreamConnection>, MarketDataSupervisorError>;
}

struct TonicMarketDataStreamConnector(TInvestGrpcClient);

#[async_trait]
impl MarketDataStreamConnector for TonicMarketDataStreamConnector {
    async fn connect(
        &self,
        outbound_capacity: usize,
    ) -> Result<Box<dyn MarketDataStreamConnection>, MarketDataSupervisorError> {
        self.0
            .open_market_data_stream(outbound_capacity)
            .await
            .map(|stream| Box::new(stream) as Box<dyn MarketDataStreamConnection>)
            .map_err(MarketDataSupervisorError::Connect)
    }
}

#[async_trait]
impl MarketDataStreamConnection for crate::GrpcMarketDataStream {
    async fn send(
        &mut self,
        request: v1::MarketDataRequest,
    ) -> Result<(), MarketDataSupervisorError> {
        crate::GrpcMarketDataStream::send(self, request)
            .await
            .map_err(MarketDataSupervisorError::Stream)
    }

    async fn message(
        &mut self,
    ) -> Result<Option<v1::MarketDataResponse>, MarketDataSupervisorError> {
        crate::GrpcMarketDataStream::message(self)
            .await
            .map_err(MarketDataSupervisorError::Stream)
    }
}

impl MarketDataStreamSupervisor {
    pub fn new(
        client: TInvestGrpcClient,
        config: MarketDataSupervisorConfig,
    ) -> Result<Self, MarketDataSupervisorError> {
        config.validate()?;
        Ok(Self {
            connector: Arc::new(TonicMarketDataStreamConnector(client)),
            config,
        })
    }

    pub fn with_connector<C>(
        connector: C,
        config: MarketDataSupervisorConfig,
    ) -> Result<Self, MarketDataSupervisorError>
    where
        C: MarketDataStreamConnector + 'static,
    {
        config.validate()?;
        Ok(Self {
            connector: Arc::new(connector),
            config,
        })
    }

    pub fn start(&self, registry: MarketSubscriptionRegistry) -> MarketDataStreamHandle {
        let (events_tx, events) = mpsc::channel(self.config.event_capacity);
        let (stop, stop_rx) = watch::channel(false);
        let supervisor = self.clone();
        tokio::spawn(async move {
            supervisor.run(registry, events_tx, stop_rx).await;
        });
        MarketDataStreamHandle { events, stop }
    }

    async fn run(
        self,
        mut registry: MarketSubscriptionRegistry,
        events: mpsc::Sender<MarketDataStreamEvent>,
        mut stop: watch::Receiver<bool>,
    ) {
        let mut reconnect_attempt = 0;
        let mut sequence_gate = EventSequenceGate::default();
        loop {
            if *stop.borrow() {
                let _ = events.send(MarketDataStreamEvent::Stopped).await;
                return;
            }
            if reconnect_attempt > 0 {
                if reconnect_attempt > self.config.max_reconnect_attempts {
                    let _ = events
                        .send(MarketDataStreamEvent::Fault(
                            MarketDataSupervisorError::ReconnectExhausted,
                        ))
                        .await;
                    return;
                }
                let delay = reconnect_delay(
                    self.config.reconnect_initial_delay,
                    self.config.reconnect_max_delay,
                    reconnect_attempt,
                );
                if events
                    .send(MarketDataStreamEvent::Reconnecting {
                        attempt: reconnect_attempt,
                        delay,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    () = tokio::time::sleep(delay) => {},
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            let _ = events.send(MarketDataStreamEvent::Stopped).await;
                            return;
                        }
                    }
                }
            }

            registry.disconnected();
            match self
                .run_connection(
                    &mut registry,
                    &mut sequence_gate,
                    &events,
                    &mut stop,
                    &mut reconnect_attempt,
                )
                .await
            {
                Ok(()) => {
                    let _ = events.send(MarketDataStreamEvent::Stopped).await;
                    return;
                }
                Err(error) => {
                    reconnect_attempt += 1;
                    if reconnect_attempt > self.config.max_reconnect_attempts {
                        let _ = events.send(MarketDataStreamEvent::Fault(error)).await;
                        let _ = events
                            .send(MarketDataStreamEvent::Fault(
                                MarketDataSupervisorError::ReconnectExhausted,
                            ))
                            .await;
                        return;
                    }
                }
            }
        }
    }

    async fn run_connection(
        &self,
        registry: &mut MarketSubscriptionRegistry,
        sequence_gate: &mut EventSequenceGate,
        events: &mpsc::Sender<MarketDataStreamEvent>,
        stop: &mut watch::Receiver<bool>,
        reconnect_attempt: &mut u32,
    ) -> Result<(), MarketDataSupervisorError> {
        let mut stream = self
            .connector
            .connect(self.config.outbound_capacity)
            .await?;
        events
            .send(MarketDataStreamEvent::Connected)
            .await
            .map_err(|_| MarketDataSupervisorError::Closed)?;
        // Only consecutive connection failures consume reconnect budget.
        *reconnect_attempt = 0;

        let requests = registry.subscribe_requests();
        if requests.len() > MAX_SUBSCRIPTION_REQUESTS_PER_MINUTE as usize {
            return Err(MarketDataSupervisorError::SubscriptionRequestRate);
        }
        let ping_delay_ms = i32::try_from(self.config.ping_delay_ms).map_err(|_| {
            MarketDataSupervisorError::MarketData(MarketDataError::InvalidPingDelay)
        })?;
        stream
            .send(v1::MarketDataRequest {
                payload: Some(v1::market_data_request::Payload::PingSettings(
                    v1::PingDelaySettings {
                        ping_delay_ms: Some(ping_delay_ms),
                    },
                )),
            })
            .await?;
        for request in requests {
            stream.send(request).await?;
        }

        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        return Ok(());
                    }
                }
                response = timeout(self.config.stale_timeout, stream.message()) => {
                    let response = response
                        .map_err(|_| MarketDataSupervisorError::Stale)?
                        ?
                        .ok_or(MarketDataSupervisorError::Closed)?;
                    if let Some(event) = process_response(registry, sequence_gate, response)? {
                        events.send(event).await.map_err(|_| MarketDataSupervisorError::Closed)?;
                    }
                }
            }
        }
    }
}

fn process_response(
    registry: &mut MarketSubscriptionRegistry,
    sequence_gate: &mut EventSequenceGate,
    response: v1::MarketDataResponse,
) -> Result<Option<MarketDataStreamEvent>, MarketDataSupervisorError> {
    use v1::market_data_response::Payload;

    let ack_count = registry
        .apply_ack_response(&response)
        .map_err(MarketDataSupervisorError::MarketData)?;
    if ack_count > 0 {
        return Ok(Some(MarketDataStreamEvent::Acknowledged {
            count: ack_count,
        }));
    }
    let event = match response.payload {
        Some(Payload::Candle(value)) => {
            require_desired(
                registry,
                &value.instrument_uid,
                &SubscriptionKind::Candle {
                    interval: value.interval,
                    waiting_close: false,
                    source: Some(value.candle_source_type),
                },
            )?;
            let fingerprint = value.encode_to_vec();
            let canonical =
                CanonicalCandle::try_from(value).map_err(MarketDataSupervisorError::MarketData)?;
            if let Some(dropped) = dropped_event(
                sequence_gate.observe(
                    &canonical.instrument_uid,
                    EventFamily::Candle,
                    canonical.event_time_ns,
                    fingerprint,
                ),
                &canonical.instrument_uid,
                "candle",
            ) {
                return Ok(Some(dropped));
            }
            MarketDataStreamEvent::Candle(canonical)
        }
        Some(Payload::Trade(value)) => {
            require_desired(
                registry,
                &value.instrument_uid,
                &SubscriptionKind::Trade {
                    source: value.trade_source,
                    with_open_interest: false,
                },
            )?;
            let fingerprint = value.encode_to_vec();
            let canonical =
                CanonicalTrade::try_from(value).map_err(MarketDataSupervisorError::MarketData)?;
            if let Some(dropped) = dropped_event(
                sequence_gate.observe(
                    &canonical.instrument_uid,
                    EventFamily::Trade,
                    canonical.event_time_ns,
                    fingerprint,
                ),
                &canonical.instrument_uid,
                "trade",
            ) {
                return Ok(Some(dropped));
            }
            MarketDataStreamEvent::Trade(canonical)
        }
        Some(Payload::Orderbook(value)) => {
            require_desired(
                registry,
                &value.instrument_uid,
                &SubscriptionKind::OrderBook {
                    depth: value.depth,
                    order_book_type: value.order_book_type,
                },
            )?;
            registry.observe_book(&value.instrument_uid, value.is_consistent);
            MarketDataStreamEvent::OrderBook(
                CanonicalOrderBook::try_from(value)
                    .map_err(MarketDataSupervisorError::MarketData)?,
            )
        }
        Some(Payload::TradingStatus(value)) => {
            require_desired(registry, &value.instrument_uid, &SubscriptionKind::Info)?;
            let fingerprint = value.encode_to_vec();
            let canonical = CanonicalTradingStatus::try_from(value)
                .map_err(MarketDataSupervisorError::MarketData)?;
            if let Some(dropped) = dropped_event(
                sequence_gate.observe(
                    &canonical.instrument_uid,
                    EventFamily::TradingStatus,
                    canonical.event_time_ns,
                    fingerprint,
                ),
                &canonical.instrument_uid,
                "trading_status",
            ) {
                return Ok(Some(dropped));
            }
            MarketDataStreamEvent::TradingStatus(canonical)
        }
        Some(Payload::LastPrice(value)) => {
            require_desired(
                registry,
                &value.instrument_uid,
                &SubscriptionKind::LastPrice,
            )?;
            let fingerprint = value.encode_to_vec();
            let canonical = CanonicalLastPrice::try_from(value)
                .map_err(MarketDataSupervisorError::MarketData)?;
            if let Some(dropped) = dropped_event(
                sequence_gate.observe(
                    &canonical.instrument_uid,
                    EventFamily::LastPrice,
                    canonical.event_time_ns,
                    fingerprint,
                ),
                &canonical.instrument_uid,
                "last_price",
            ) {
                return Ok(Some(dropped));
            }
            MarketDataStreamEvent::LastPrice(canonical)
        }
        Some(Payload::OpenInterest(value)) => {
            require_desired(
                registry,
                &value.instrument_uid,
                &SubscriptionKind::Trade {
                    source: 0,
                    with_open_interest: true,
                },
            )?;
            let fingerprint = value.encode_to_vec();
            let canonical = CanonicalOpenInterest::try_from(value)
                .map_err(MarketDataSupervisorError::MarketData)?;
            if let Some(dropped) = dropped_event(
                sequence_gate.observe(
                    &canonical.instrument_uid,
                    EventFamily::OpenInterest,
                    canonical.event_time_ns,
                    fingerprint,
                ),
                &canonical.instrument_uid,
                "open_interest",
            ) {
                return Ok(Some(dropped));
            }
            MarketDataStreamEvent::OpenInterest(canonical)
        }
        Some(Payload::Ping(value)) => MarketDataStreamEvent::Ping {
            stream_id: value.stream_id,
            event_time_ns: value
                .time
                .map(|time| crate::market_data::timestamp_ns(Some(time), "ping.time"))
                .transpose()
                .map_err(MarketDataSupervisorError::MarketData)?,
        },
        None => {
            return Err(MarketDataSupervisorError::MarketData(
                MarketDataError::Missing("market_data_response.payload"),
            ));
        }
        _ => return Ok(None),
    };
    Ok(Some(event))
}

fn dropped_event(
    disposition: EventDisposition,
    instrument_uid: &str,
    family: &'static str,
) -> Option<MarketDataStreamEvent> {
    match disposition {
        EventDisposition::Accept => None,
        EventDisposition::Duplicate | EventDisposition::OutOfOrder => {
            Some(MarketDataStreamEvent::Dropped {
                instrument_uid: instrument_uid.to_owned(),
                family,
                out_of_order: disposition == EventDisposition::OutOfOrder,
            })
        }
    }
}

fn require_desired(
    registry: &MarketSubscriptionRegistry,
    instrument_uid: &str,
    kind: &SubscriptionKind,
) -> Result<(), MarketDataSupervisorError> {
    if registry.accepts_event_before_ack(instrument_uid, kind) {
        Ok(())
    } else {
        Err(MarketDataSupervisorError::UndesiredEvent)
    }
}

fn reconnect_delay(initial: Duration, maximum: Duration, attempt: u32) -> Duration {
    let multiplier = 1_u32
        .checked_shl(attempt.saturating_sub(1).min(20))
        .unwrap_or(u32::MAX);
    initial.saturating_mul(multiplier).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Default)]
    struct FakeConnector {
        connects: Arc<AtomicUsize>,
        sends: Arc<AtomicUsize>,
    }

    struct FakeConnection {
        sends: Arc<AtomicUsize>,
        close_after_ping: bool,
        emitted_ping: bool,
    }

    #[async_trait]
    impl MarketDataStreamConnector for FakeConnector {
        async fn connect(
            &self,
            _outbound_capacity: usize,
        ) -> Result<Box<dyn MarketDataStreamConnection>, MarketDataSupervisorError> {
            let attempt = self.connects.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeConnection {
                sends: Arc::clone(&self.sends),
                close_after_ping: attempt == 0,
                emitted_ping: false,
            }))
        }
    }

    #[async_trait]
    impl MarketDataStreamConnection for FakeConnection {
        async fn send(
            &mut self,
            _request: v1::MarketDataRequest,
        ) -> Result<(), MarketDataSupervisorError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn message(
            &mut self,
        ) -> Result<Option<v1::MarketDataResponse>, MarketDataSupervisorError> {
            if !self.emitted_ping {
                self.emitted_ping = true;
                return Ok(Some(v1::MarketDataResponse {
                    payload: Some(v1::market_data_response::Payload::Ping(v1::Ping {
                        time: Some(prost_types::Timestamp {
                            seconds: 1,
                            nanos: 0,
                        }),
                        stream_id: "fake".into(),
                        ping_request_time: None,
                    })),
                }));
            }
            if self.close_after_ping {
                Ok(None)
            } else {
                std::future::pending().await
            }
        }
    }

    #[test]
    fn reconnect_delay_is_bounded() {
        let initial = Duration::from_millis(100);
        let maximum = Duration::from_secs(1);
        assert_eq!(reconnect_delay(initial, maximum, 1), initial);
        assert_eq!(reconnect_delay(initial, maximum, 8), maximum);
    }

    #[test]
    fn config_rejects_busy_loop_and_invalid_ping() {
        let invalid = MarketDataSupervisorConfig {
            reconnect_initial_delay: Duration::ZERO,
            ..Default::default()
        };
        assert_eq!(
            invalid.validate(),
            Err(MarketDataSupervisorError::InvalidReconnectPolicy)
        );
        let invalid = MarketDataSupervisorConfig {
            ping_delay_ms: 1,
            ..Default::default()
        };
        assert!(matches!(
            invalid.validate(),
            Err(MarketDataSupervisorError::MarketData(_))
        ));
    }

    #[test]
    fn sequence_gate_drops_duplicates_and_out_of_order_but_allows_same_time_distinct_trade() {
        let mut gate = EventSequenceGate::default();
        assert_eq!(
            gate.observe("uid", EventFamily::Trade, 10, vec![1]),
            EventDisposition::Accept
        );
        assert_eq!(
            gate.observe("uid", EventFamily::Trade, 10, vec![1]),
            EventDisposition::Duplicate
        );
        assert_eq!(
            gate.observe("uid", EventFamily::Trade, 10, vec![2]),
            EventDisposition::Accept
        );
        assert_eq!(
            gate.observe("uid", EventFamily::Trade, 9, vec![3]),
            EventDisposition::OutOfOrder
        );
        assert_eq!(
            gate.observe("uid", EventFamily::Trade, 11, vec![4]),
            EventDisposition::Accept
        );
    }

    #[tokio::test]
    async fn forced_disconnect_reconnects_and_replays_desired_subscriptions() {
        let connector = FakeConnector::default();
        let evidence = connector.clone();
        let config = MarketDataSupervisorConfig {
            reconnect_initial_delay: Duration::from_millis(1),
            reconnect_max_delay: Duration::from_millis(2),
            stale_timeout: Duration::from_secs(5),
            ..Default::default()
        };
        let supervisor = MarketDataStreamSupervisor::with_connector(connector, config)
            .expect("valid fake supervisor");
        let mut registry = MarketSubscriptionRegistry::default();
        registry
            .insert(crate::market_data::MarketSubscription {
                instrument_id: "uid".into(),
                kind: SubscriptionKind::LastPrice,
            })
            .expect("valid desired state");
        let mut handle = supervisor.start(registry);
        let mut connected = 0;
        while connected < 2 {
            let event = tokio::time::timeout(Duration::from_secs(1), handle.recv())
                .await
                .expect("supervisor event timeout")
                .expect("supervisor event channel closed");
            if event == MarketDataStreamEvent::Connected {
                connected += 1;
            }
        }
        assert_eq!(evidence.connects.load(Ordering::SeqCst), 2);
        assert_eq!(evidence.sends.load(Ordering::SeqCst), 4);
        handle.stop();
    }
}
