//! The versioned WebSocket gateway at `/api/v1/stream`.
//!
//! One socket carries every topic. The outbound queue is bounded: when a browser stops
//! reading, the server sends a typed `DROPPED_SLOW_CONSUMER` status and closes that socket
//! rather than buffering without limit or blocking the runtime that feeds it.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::application::AppState;
use crate::contract::scope::ExecutionScope;
use crate::contract::stream::{
    ClientMessage, EventPayload, ServerEvent, SubscriptionStatus, Topic, STREAM_SCHEMA_VERSION,
};

use super::http::now_unix_ms;

/// How many events may wait for one browser before it is dropped.
pub const OUTBOUND_QUEUE_CAPACITY: usize = 256;

/// How often the server proves it is alive.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// A subscription this socket currently holds.
///
/// The scope and the sequence are carried for the live projection that #11 owns: when a
/// topic starts producing updates, every event must state which scope it belongs to and
/// carry a gap-detectable sequence. Until that projection exists only the tests drive them.
#[allow(dead_code, reason = "consumed by the live projection that #11 owns; exercised by tests")]
struct Subscription {
    topic: Topic,
    scope: Option<ExecutionScope>,
    /// Monotonic per-subscription sequence, so a client can detect a gap.
    sequence: u64,
}

#[allow(dead_code, reason = "consumed by the live projection that #11 owns; exercised by tests")]
impl Subscription {
    /// Builds the next update for this subscription and advances its sequence.
    fn next_update(&mut self, runtime_epoch: u64, payload: EventPayload, subscription_id: &str) -> ServerEvent {
        self.sequence = self.sequence.saturating_add(1);
        ServerEvent::Update {
            schema_version: STREAM_SCHEMA_VERSION,
            subscription_id: subscription_id.to_owned(),
            as_of_unix_ms: now_unix_ms(),
            runtime_epoch,
            sequence: self.sequence,
            scope: self.scope.clone(),
            payload,
        }
    }
}

/// The stream router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/stream", get(upgrade))
        .with_state(state)
}

async fn upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| serve(socket, state))
}

async fn serve(mut socket: WebSocket, state: AppState) {
    let (tx, mut rx) = mpsc::channel::<ServerEvent>(OUTBOUND_QUEUE_CAPACITY);
    let mut subscriptions: HashMap<String, Subscription> = HashMap::new();
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await; // the first tick fires immediately

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
                };
                let events = handle_client_message(&text, &state, &mut subscriptions).await;
                for event in events {
                    if tx.try_send(event).is_err() {
                        let _ = send(&mut socket, &slow_consumer_status()).await;
                        return;
                    }
                }
            }
            queued = rx.recv() => {
                let Some(event) = queued else { break };
                if send(&mut socket, &event).await.is_err() { break }
            }
            _ = heartbeat.tick() => {
                let beat = ServerEvent::Heartbeat {
                    schema_version: STREAM_SCHEMA_VERSION,
                    server_time_unix_ms: now_unix_ms(),
                    nonce: None,
                };
                if send(&mut socket, &beat).await.is_err() { break }
            }
        }
    }
}

/// Turns one client message into the events it produces.
async fn handle_client_message(
    text: &str,
    state: &AppState,
    subscriptions: &mut HashMap<String, Subscription>,
) -> Vec<ServerEvent> {
    let message: ClientMessage = match serde_json::from_str(text) {
        Ok(message) => message,
        Err(error) => {
            return vec![ServerEvent::Error {
                schema_version: STREAM_SCHEMA_VERSION,
                subscription_id: None,
                code: "MALFORMED_MESSAGE".to_owned(),
                message: format!("the message is not a known client envelope: {error}"),
                correlation_id: uuid::Uuid::new_v4().to_string(),
            }];
        }
    };

    match message {
        ClientMessage::Ping { nonce } => vec![ServerEvent::Heartbeat {
            schema_version: STREAM_SCHEMA_VERSION,
            server_time_unix_ms: now_unix_ms(),
            nonce,
        }],
        ClientMessage::Unsubscribe { subscription_id } => {
            subscriptions.remove(&subscription_id);
            vec![ServerEvent::Status {
                schema_version: STREAM_SCHEMA_VERSION,
                subscription_id,
                status: SubscriptionStatus::Cancelled,
                detail: None,
            }]
        }
        ClientMessage::Subscribe { subscription_id, topic, scope } => {
            subscribe(state, subscriptions, subscription_id, topic, scope).await
        }
    }
}

async fn subscribe(
    state: &AppState,
    subscriptions: &mut HashMap<String, Subscription>,
    subscription_id: String,
    topic: Topic,
    scope: Option<ExecutionScope>,
) -> Vec<ServerEvent> {
    // Only the topics whose read model exists may be accepted.
    if let Some(existing) = subscriptions.get(&subscription_id) {
        // Re-using a live subscription id would make two streams indistinguishable.
        let detail = format!("subscription id is already live for {:?}", existing.topic);
        return vec![ServerEvent::Error {
            schema_version: STREAM_SCHEMA_VERSION,
            subscription_id: Some(subscription_id),
            code: "SUBSCRIPTION_ID_IN_USE".to_owned(),
            message: detail,
            correlation_id: uuid::Uuid::new_v4().to_string(),
        }];
    }
    match topic {
        Topic::RuntimeHealth => {}
        Topic::Positions | Topic::Orders | Topic::Stops | Topic::Operations | Topic::Portfolio => {
            if state.accounts.is_none() {
                return vec![unavailable(subscription_id, "no account read side is attached to this process")];
            }
            if scope.is_none() {
                return vec![ServerEvent::Error {
                    schema_version: STREAM_SCHEMA_VERSION,
                    subscription_id: Some(subscription_id),
                    code: "SCOPE_REQUIRED".to_owned(),
                    message: "an account-scoped topic needs its execution scope".to_owned(),
                    correlation_id: uuid::Uuid::new_v4().to_string(),
                }];
            }
            // The read models exist but no live projection feeds them yet.
            return vec![unavailable(
                subscription_id,
                "live updates for this topic need the runtime projection that #11 owns",
            )];
        }
    }

    let Ok(runtime) = state.runtime_port() else {
        return vec![unavailable(subscription_id, "no runtime is attached to this process")];
    };
    let health = match runtime.health().await {
        Ok(health) => health,
        Err(error) => {
            return vec![ServerEvent::Error {
                schema_version: STREAM_SCHEMA_VERSION,
                subscription_id: Some(subscription_id),
                code: error.code,
                message: error.message,
                correlation_id: error.correlation_id,
            }];
        }
    };
    let runtime_epoch = health.runtime_epoch;
    subscriptions.insert(subscription_id.clone(), Subscription { topic, scope: scope.clone(), sequence: 0 });
    vec![
        ServerEvent::Status {
            schema_version: STREAM_SCHEMA_VERSION,
            subscription_id: subscription_id.clone(),
            status: SubscriptionStatus::Active,
            detail: None,
        },
        ServerEvent::Snapshot {
            schema_version: STREAM_SCHEMA_VERSION,
            subscription_id,
            as_of_unix_ms: now_unix_ms(),
            runtime_epoch,
            scope,
            payload: EventPayload::RuntimeHealth(health),
        },
    ]
}

fn unavailable(subscription_id: String, detail: &str) -> ServerEvent {
    ServerEvent::Status {
        schema_version: STREAM_SCHEMA_VERSION,
        subscription_id,
        status: SubscriptionStatus::Unavailable,
        detail: Some(detail.to_owned()),
    }
}

fn slow_consumer_status() -> ServerEvent {
    ServerEvent::Status {
        schema_version: STREAM_SCHEMA_VERSION,
        subscription_id: String::new(),
        status: SubscriptionStatus::DroppedSlowConsumer,
        detail: Some(format!(
            "the outbound queue of {OUTBOUND_QUEUE_CAPACITY} events is full; the socket is closed rather than buffered without limit"
        )),
    }
}

async fn send(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), axum::Error> {
    let text = serde_json::to_string(event).unwrap_or_else(|_| {
        // Serializing our own envelope cannot fail on valid data; if it ever does, say so
        // in the same typed shape rather than closing silently.
        "{\"type\":\"ERROR\",\"schema_version\":1,\"code\":\"ENCODE_FAILED\",\"message\":\"the server could not encode an event\",\"correlation_id\":\"\"}".to_owned()
    });
    socket.send(Message::Text(Utf8Bytes::from(text))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::scope::{BrokerEnvironment, ProviderDto};

    #[tokio::test]
    async fn a_ping_is_answered_with_a_heartbeat() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let mut subs = HashMap::new();
        let events = handle_client_message(
            "{\"type\":\"PING\",\"nonce\":\"n-1\"}",
            &state,
            &mut subs,
        )
        .await;
        assert!(matches!(events.as_slice(), [ServerEvent::Heartbeat { nonce: Some(n), .. }] if n == "n-1"));
    }

    #[tokio::test]
    async fn an_unknown_envelope_is_a_typed_error_not_a_close() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let mut subs = HashMap::new();
        let events = handle_client_message("{\"type\":\"NONSENSE\"}", &state, &mut subs).await;
        assert!(matches!(events.as_slice(), [ServerEvent::Error { code, .. }] if code == "MALFORMED_MESSAGE"));
    }

    #[tokio::test]
    async fn a_topic_without_a_read_model_is_refused_explicitly() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let mut subs = HashMap::new();
        let events = handle_client_message(
            "{\"type\":\"SUBSCRIBE\",\"subscription_id\":\"s-1\",\"topic\":\"POSITIONS\"}",
            &state,
            &mut subs,
        )
        .await;
        assert!(matches!(
            events.as_slice(),
            [ServerEvent::Status { status: SubscriptionStatus::Unavailable, .. }]
        ));
        assert!(subs.is_empty(), "a refused subscription must not be registered");
    }

    #[tokio::test]
    async fn a_reused_subscription_id_is_refused() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let mut subs = HashMap::new();
        subs.insert(
            "s-1".to_owned(),
            Subscription { topic: Topic::RuntimeHealth, scope: None, sequence: 0 },
        );
        let events = handle_client_message(
            "{\"type\":\"SUBSCRIBE\",\"subscription_id\":\"s-1\",\"topic\":\"RUNTIME_HEALTH\"}",
            &state,
            &mut subs,
        )
        .await;
        assert!(matches!(events.as_slice(), [ServerEvent::Error { code, .. }] if code == "SUBSCRIPTION_ID_IN_USE"));
    }

    #[test]
    fn updates_carry_a_monotonic_sequence() {
        let mut subscription = Subscription { topic: Topic::RuntimeHealth, scope: None, sequence: 0 };
        let payload = || EventPayload::RuntimeHealth(sample_health());
        let first = subscription.next_update(7, payload(), "s-1");
        let second = subscription.next_update(7, payload(), "s-1");
        let sequence_of = |event: &ServerEvent| match event {
            ServerEvent::Update { sequence, .. } => *sequence,
            _ => panic!("expected an update"),
        };
        assert_eq!(sequence_of(&first), 1);
        assert_eq!(sequence_of(&second), 2);
    }

    fn sample_health() -> crate::contract::runtime::RuntimeHealthDto {
        use crate::contract::runtime::{ReasonCodeDto, RuntimeHealthDto, RuntimeStateDto};
        RuntimeHealthDto {
            state: RuntimeStateDto::Ready,
            reason_code: ReasonCodeDto::ReconciliationComplete,
            reason: "reconciled".to_owned(),
            provider: ProviderDto::TInvest,
            environment: BrokerEnvironment::Sandbox,
            account_display: "sandbox account".to_owned(),
            runtime_epoch: 7,
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
}
