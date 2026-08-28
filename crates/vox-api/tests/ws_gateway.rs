//! Live WebSocket path: connect, SUBSCRIBE, SNAPSHOT, then a later application event
//! must arrive as UPDATE without polling. These tests speak the real socket.

use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use vox_api::application::RuntimeQueries;
use vox_api::contract::market::OrderBookDto;
use vox_api::contract::runtime::{
    ReasonCodeDto, RuntimeHealthDto, RuntimeStateDto, StreamStateDto,
};
use vox_api::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto};
use vox_api::contract::stream::{EventPayload, ServerEvent, SubscriptionStatus};
use vox_api::error::ApiError;
use vox_api::events::ApplicationEventBus;
use vox_api::market_project::{
    SnapshotMarketProjection, order_book_from_levels, quote_from_last_price, trade_from_canonical,
};
use vox_api::transport::ws::OUTBOUND_QUEUE_CAPACITY;
use vox_api::{AppState, router};
use vox_domain::FixedPoint;

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

struct MutableRuntime {
    health: Mutex<RuntimeHealthDto>,
}

#[async_trait::async_trait]
impl RuntimeQueries for MutableRuntime {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError> {
        Ok(self.health.lock().await.clone())
    }

    async fn scopes(&self) -> Result<Vec<ExecutionScope>, ApiError> {
        Ok(Vec::new())
    }
}

fn sample_health(state: RuntimeStateDto, epoch: u64, reason: &str) -> RuntimeHealthDto {
    RuntimeHealthDto {
        state,
        reason_code: ReasonCodeDto::ReconciliationComplete,
        reason: reason.to_owned(),
        provider: ProviderDto::TInvest,
        environment: BrokerEnvironment::Sandbox,
        account_display: "sandbox account".to_owned(),
        runtime_epoch: epoch,
        connected: true,
        last_successful_reconciliation_at_unix_ms: Some(1),
        reconciliation_age_ms: Some(10),
        unresolved_unknown_count: 0,
        open_order_count: 0,
        active_stop_count: 0,
        stream_states: Vec::new(),
        persistence_healthy: true,
        execution_authorized: false,
        new_exposure_allowed: false,
    }
}

fn fp(units: i64, nano: i32) -> FixedPoint {
    FixedPoint::from_units_nano(units, nano).expect("fixed point")
}

async fn serve(state: AppState) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.expect("serve");
    });
    format!("ws://{addr}/api/v1/stream")
}

async fn connect(url: &str) -> Ws {
    let addr = url
        .trim_start_matches("ws://")
        .split('/')
        .next()
        .expect("host:port");
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("tcp connect");
    let (ws, _) = tokio_tungstenite::client_async(url, stream)
        .await
        .expect("websocket handshake");
    ws
}

async fn send_text(ws: &mut Ws, text: &str) {
    ws.send(Message::Text(text.into()))
        .await
        .expect("client send");
}

async fn recv_event(ws: &mut Ws) -> ServerEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let message = ws.next().await.expect("socket closed").expect("frame");
            let Message::Text(text) = message else {
                continue;
            };
            let event: ServerEvent = serde_json::from_str(&text).expect("server event json");
            if matches!(event, ServerEvent::Heartbeat { .. }) {
                continue;
            }
            return event;
        }
    })
    .await
    .expect("timed out waiting for a server event")
}

async fn recv_update(ws: &mut Ws) -> ServerEvent {
    let event = recv_event(ws).await;
    assert!(
        matches!(event, ServerEvent::Update { .. }),
        "expected UPDATE, got {event:?}"
    );
    event
}

fn market_state(
    projection: Arc<SnapshotMarketProjection>,
    events: ApplicationEventBus,
) -> AppState {
    AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox)
        .with_market_data(projection)
        .with_events(events)
}

#[tokio::test]
async fn subscribe_quote_then_publish_yields_snapshot_then_update() {
    let events = ApplicationEventBus::new();
    let projection = Arc::new(SnapshotMarketProjection::new().with_events(events.clone()));
    projection.publish_quote(quote_from_last_price(
        "uid-sber",
        fp(272, 550_000_000),
        1_000,
        StreamStateDto::Active,
    ));
    let url = serve(market_state(projection.clone(), events)).await;
    let mut ws = connect(&url).await;
    send_text(
        &mut ws,
        r#"{"type":"SUBSCRIBE","subscription_id":"q-1","topic":"QUOTES","instrument_uid":"uid-sber"}"#,
    )
    .await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status {
            status: SubscriptionStatus::Active,
            ..
        }
    ));
    match recv_event(&mut ws).await {
        ServerEvent::Snapshot {
            payload: EventPayload::Quote(quote),
            ..
        } => assert_eq!(
            quote
                .last
                .as_ref()
                .map(vox_api::contract::money::Decimal::as_str),
            Some("272.550000000")
        ),
        other => panic!("expected quote SNAPSHOT, got {other:?}"),
    }

    projection.publish_quote(quote_from_last_price(
        "uid-sber",
        fp(273, 0),
        2_000,
        StreamStateDto::Active,
    ));
    match recv_update(&mut ws).await {
        ServerEvent::Update {
            subscription_id,
            sequence,
            payload: EventPayload::Quote(quote),
            ..
        } => {
            assert_eq!(subscription_id, "q-1");
            assert_eq!(sequence, 1);
            assert_eq!(
                quote
                    .last
                    .as_ref()
                    .map(vox_api::contract::money::Decimal::as_str),
                Some("273.000000000")
            );
        }
        other => panic!("expected quote UPDATE, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_order_book_then_publish_yields_update() {
    let events = ApplicationEventBus::new();
    let projection = Arc::new(SnapshotMarketProjection::new().with_events(events.clone()));
    let first = order_book_from_levels(
        "uid-sber",
        &[(fp(1, 0), 1)],
        &[(fp(2, 0), 1)],
        1_000,
        StreamStateDto::Active,
        true,
    )
    .expect("book");
    projection.publish_order_book(first);
    let url = serve(market_state(projection.clone(), events)).await;
    let mut ws = connect(&url).await;
    send_text(
        &mut ws,
        r#"{"type":"SUBSCRIBE","subscription_id":"b-1","topic":"ORDER_BOOK","instrument_uid":"uid-sber"}"#,
    )
    .await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status { .. }
    ));
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Snapshot {
            payload: EventPayload::OrderBook(_),
            ..
        }
    ));

    let second = order_book_from_levels(
        "uid-sber",
        &[(fp(1, 100_000_000), 2)],
        &[(fp(2, 100_000_000), 2)],
        2_000,
        StreamStateDto::Active,
        true,
    )
    .expect("book");
    projection.publish_order_book(second);
    match recv_update(&mut ws).await {
        ServerEvent::Update {
            subscription_id,
            sequence,
            payload: EventPayload::OrderBook(OrderBookDto { depth, .. }),
            ..
        } => {
            assert_eq!(subscription_id, "b-1");
            assert_eq!(sequence, 1);
            assert_eq!(depth, 1);
        }
        other => panic!("expected book UPDATE, got {other:?}"),
    }
}

#[tokio::test]
async fn subscribe_trades_then_publish_yields_update() {
    let events = ApplicationEventBus::new();
    let projection = Arc::new(SnapshotMarketProjection::new().with_events(events.clone()));
    projection.publish_trades(
        "uid-sber".into(),
        vec![trade_from_canonical("uid-sber", fp(272, 0), 1, 1, 1_000)],
    );
    let url = serve(market_state(projection.clone(), events)).await;
    let mut ws = connect(&url).await;
    send_text(
        &mut ws,
        r#"{"type":"SUBSCRIBE","subscription_id":"t-1","topic":"TRADES","instrument_uid":"uid-sber"}"#,
    )
    .await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status { .. }
    ));
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Snapshot {
            payload: EventPayload::Trades(_),
            ..
        }
    ));

    projection.publish_trades(
        "uid-sber".into(),
        vec![
            trade_from_canonical("uid-sber", fp(272, 0), 1, 1, 1_000),
            trade_from_canonical("uid-sber", fp(273, 0), 2, 2, 2_000),
        ],
    );
    match recv_update(&mut ws).await {
        ServerEvent::Update {
            subscription_id,
            sequence,
            payload: EventPayload::Trades(ticks),
            ..
        } => {
            assert_eq!(subscription_id, "t-1");
            assert_eq!(sequence, 1);
            assert_eq!(ticks.len(), 2);
            assert_eq!(ticks[1].price.as_str(), "273.000000000");
        }
        other => panic!("expected trades UPDATE, got {other:?}"),
    }
}

#[tokio::test]
async fn runtime_health_change_after_subscribe_yields_update() {
    let runtime = Arc::new(MutableRuntime {
        health: Mutex::new(sample_health(RuntimeStateDto::Starting, 1, "starting")),
    });
    let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox)
        .with_runtime(runtime.clone());
    let url = serve(state).await;
    let mut ws = connect(&url).await;
    send_text(
        &mut ws,
        r#"{"type":"SUBSCRIBE","subscription_id":"r-1","topic":"RUNTIME_HEALTH"}"#,
    )
    .await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status {
            status: SubscriptionStatus::Active,
            ..
        }
    ));
    match recv_event(&mut ws).await {
        ServerEvent::Snapshot {
            payload: EventPayload::RuntimeHealth(health),
            ..
        } => assert_eq!(health.state, RuntimeStateDto::Starting),
        other => panic!("expected runtime SNAPSHOT, got {other:?}"),
    }

    // Watcher treats the first observe as baseline. Wait one interval so STARTING is stored.
    tokio::time::sleep(Duration::from_millis(250)).await;
    *runtime.health.lock().await = sample_health(RuntimeStateDto::Degraded, 2, "degraded");
    match recv_update(&mut ws).await {
        ServerEvent::Update {
            subscription_id,
            sequence,
            runtime_epoch,
            payload: EventPayload::RuntimeHealth(health),
            ..
        } => {
            assert_eq!(subscription_id, "r-1");
            assert_eq!(sequence, 1);
            assert_eq!(runtime_epoch, 2);
            assert_eq!(health.state, RuntimeStateDto::Degraded);
        }
        other => panic!("expected runtime UPDATE, got {other:?}"),
    }
}

#[tokio::test]
async fn unsubscribe_stops_further_updates() {
    let events = ApplicationEventBus::new();
    let projection = Arc::new(SnapshotMarketProjection::new().with_events(events.clone()));
    projection.publish_quote(quote_from_last_price(
        "uid-sber",
        fp(1, 0),
        1_000,
        StreamStateDto::Active,
    ));
    let url = serve(market_state(projection.clone(), events)).await;
    let mut ws = connect(&url).await;
    send_text(
        &mut ws,
        r#"{"type":"SUBSCRIBE","subscription_id":"q-1","topic":"QUOTES","instrument_uid":"uid-sber"}"#,
    )
    .await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status { .. }
    ));
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Snapshot { .. }
    ));
    send_text(&mut ws, r#"{"type":"UNSUBSCRIBE","subscription_id":"q-1"}"#).await;
    assert!(matches!(
        recv_event(&mut ws).await,
        ServerEvent::Status {
            status: SubscriptionStatus::Cancelled,
            ..
        }
    ));
    projection.publish_quote(quote_from_last_price(
        "uid-sber",
        fp(2, 0),
        2_000,
        StreamStateDto::Active,
    ));
    let late = tokio::time::timeout(Duration::from_millis(400), recv_event(&mut ws)).await;
    assert!(late.is_err(), "UNSUBSCRIBE must stop UPDATEs, got {late:?}");
}

#[tokio::test]
async fn slow_consumer_is_dropped_and_does_not_block_a_second_client() {
    let events = ApplicationEventBus::new();
    let projection = Arc::new(SnapshotMarketProjection::new().with_events(events.clone()));
    projection.publish_quote(quote_from_last_price(
        "uid-sber",
        fp(1, 0),
        1_000,
        StreamStateDto::Active,
    ));
    let url = serve(market_state(projection.clone(), events)).await;
    let mut slow = connect(&url).await;
    let mut fast = connect(&url).await;
    let subscribe = r#"{"type":"SUBSCRIBE","subscription_id":"q-1","topic":"QUOTES","instrument_uid":"uid-sber"}"#;
    send_text(&mut slow, subscribe).await;
    send_text(&mut fast, subscribe).await;
    assert!(matches!(
        recv_event(&mut slow).await,
        ServerEvent::Status { .. }
    ));
    assert!(matches!(
        recv_event(&mut slow).await,
        ServerEvent::Snapshot { .. }
    ));
    assert!(matches!(
        recv_event(&mut fast).await,
        ServerEvent::Status { .. }
    ));
    assert!(matches!(
        recv_event(&mut fast).await,
        ServerEvent::Snapshot { .. }
    ));

    let fast_reader = tokio::spawn(async move {
        let mut updates = 0_u32;
        for _ in 0..256 {
            match tokio::time::timeout(Duration::from_secs(2), recv_event(&mut fast)).await {
                Ok(ServerEvent::Update {
                    payload: EventPayload::Quote(_),
                    ..
                }) => {
                    updates += 1;
                    if updates >= 3 {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        updates
    });

    for i in 0..(OUTBOUND_QUEUE_CAPACITY * 4) {
        projection.publish_quote(quote_from_last_price(
            "uid-sber",
            fp(i64::try_from(i).expect("i"), 0),
            2_000 + i64::try_from(i).expect("i"),
            StreamStateDto::Active,
        ));
        tokio::task::yield_now().await;
    }

    let fast_updates = fast_reader.await.expect("fast reader");
    assert!(
        fast_updates >= 3,
        "fast client must keep receiving UPDATEs, got {fast_updates}"
    );

    let slow_end = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match slow.next().await {
                None => return "closed",
                Some(Ok(Message::Text(text))) => {
                    if let Ok(ServerEvent::Status {
                        status: SubscriptionStatus::DroppedSlowConsumer,
                        ..
                    }) = serde_json::from_str::<ServerEvent>(&text)
                    {
                        return "dropped";
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) => return "closed",
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(
        matches!(slow_end, Ok("dropped" | "closed")),
        "slow consumer must drop, got {slow_end:?}"
    );
}
