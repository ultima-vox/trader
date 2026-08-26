//! Supervised long-lived OperationsStream with bounded delivery and resubscription.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use uuid::Uuid;

use crate::account::{
    AccountDataError, ProviderTimestamp, optional_money, optional_text, optional_timestamp,
};
use crate::canonical::CanonicalMoney;
use crate::generated::v1;
use crate::{GrpcError, GrpcStreamError, RetryPolicy, TInvestGrpcClient};

pub const DEFAULT_OPERATIONS_PING_DELAY_MS: i32 = 120_000;
pub const MIN_OPERATIONS_PING_DELAY_MS: i32 = 5_000;
pub const MAX_OPERATIONS_PING_DELAY_MS: i32 = 180_000;

#[derive(Clone, Debug)]
pub struct OperationsStreamConfig {
    pub event_capacity: usize,
    pub stale_timeout: Duration,
    pub reconnect_policy: RetryPolicy,
    pub ping_delay_ms: i32,
}

impl Default for OperationsStreamConfig {
    fn default() -> Self {
        Self {
            event_capacity: 1_024,
            stale_timeout: Duration::from_secs(150),
            reconnect_policy: RetryPolicy::default(),
            ping_delay_ms: DEFAULT_OPERATIONS_PING_DELAY_MS,
        }
    }
}

impl OperationsStreamConfig {
    pub fn validate(&self) -> Result<(), OperationsStreamError> {
        if self.event_capacity == 0 {
            return Err(OperationsStreamError::ZeroCapacity);
        }
        if self.stale_timeout.is_zero() {
            return Err(OperationsStreamError::ZeroStaleTimeout);
        }
        if !(MIN_OPERATIONS_PING_DELAY_MS..=MAX_OPERATIONS_PING_DELAY_MS)
            .contains(&self.ping_delay_ms)
        {
            return Err(OperationsStreamError::InvalidPingDelay);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct OperationUpdateKey {
    pub broker_account_id: Option<String>,
    pub parent_operation_id: Option<String>,
    pub instrument_uid: Option<String>,
    pub operation_type: i32,
    pub date: Option<ProviderTimestamp>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamOperation {
    pub update_key: OperationUpdateKey,
    /// Monotonic occurrence count for same reconciliation key. Every duplicate/update is emitted.
    pub revision: u64,
    /// Provider operation ID is mutable provenance, never durable key.
    pub provider_operation_id: Option<String>,
    pub name: Option<String>,
    pub state: i32,
    pub figi: Option<String>,
    pub instrument_type: Option<String>,
    pub instrument_kind: i32,
    pub position_uid: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub payment: Option<CanonicalMoney>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationsStreamEvent {
    Connected,
    Reconnecting {
        attempt: u32,
        delay: Duration,
    },
    Subscribed {
        accounts: Vec<String>,
        tracking_id: Option<String>,
        stream_id: String,
    },
    Operation(Box<StreamOperation>),
    Ping {
        stream_id: Option<String>,
        at: Option<ProviderTimestamp>,
        request_at: Option<ProviderTimestamp>,
    },
    Fault(OperationsStreamError),
    Stopped,
}

pub struct OperationsStreamHandle {
    events: mpsc::Receiver<OperationsStreamEvent>,
    stop: watch::Sender<bool>,
    force_reconnect: watch::Sender<u64>,
}

impl OperationsStreamHandle {
    pub async fn recv(&mut self) -> Option<OperationsStreamEvent> {
        self.events.recv().await
    }

    pub fn force_reconnect(&self) {
        let next = self.force_reconnect.borrow().wrapping_add(1);
        let _ = self.force_reconnect.send(next);
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

#[async_trait]
pub trait OperationsStreamConnection: Send {
    async fn message(
        &mut self,
    ) -> Result<Option<v1::OperationsStreamResponse>, OperationsStreamError>;
}

#[async_trait]
pub trait OperationsStreamConnector: Send + Sync {
    async fn connect(
        &self,
        request: v1::OperationsStreamRequest,
    ) -> Result<Box<dyn OperationsStreamConnection>, OperationsStreamError>;
}

struct TonicOperationsStreamConnector(TInvestGrpcClient);

#[async_trait]
impl OperationsStreamConnector for TonicOperationsStreamConnector {
    async fn connect(
        &self,
        request: v1::OperationsStreamRequest,
    ) -> Result<Box<dyn OperationsStreamConnection>, OperationsStreamError> {
        self.0
            .open_operations_stream(request)
            .await
            .map(|stream| Box::new(stream) as Box<dyn OperationsStreamConnection>)
            .map_err(OperationsStreamError::Connect)
    }
}

#[async_trait]
impl OperationsStreamConnection for crate::GrpcServerStream<v1::OperationsStreamResponse> {
    async fn message(
        &mut self,
    ) -> Result<Option<v1::OperationsStreamResponse>, OperationsStreamError> {
        crate::GrpcServerStream::message(self)
            .await
            .map_err(OperationsStreamError::Stream)
    }
}

#[derive(Clone)]
pub struct OperationsStreamSupervisor {
    connector: Arc<dyn OperationsStreamConnector>,
    config: OperationsStreamConfig,
}

impl OperationsStreamSupervisor {
    pub fn new(
        client: TInvestGrpcClient,
        config: OperationsStreamConfig,
    ) -> Result<Self, OperationsStreamError> {
        Self::with_connector(TonicOperationsStreamConnector(client), config)
    }

    pub fn with_connector<C>(
        connector: C,
        config: OperationsStreamConfig,
    ) -> Result<Self, OperationsStreamError>
    where
        C: OperationsStreamConnector + 'static,
    {
        config.validate()?;
        Ok(Self {
            connector: Arc::new(connector),
            config,
        })
    }

    pub fn start(
        &self,
        accounts: Vec<String>,
    ) -> Result<OperationsStreamHandle, OperationsStreamError> {
        let accounts = validate_accounts(accounts)?;
        let (events_tx, events) = mpsc::channel(self.config.event_capacity);
        let (stop, stop_rx) = watch::channel(false);
        let (force_reconnect, reconnect_rx) = watch::channel(0_u64);
        let supervisor = self.clone();
        tokio::spawn(async move {
            supervisor
                .run(accounts, events_tx, stop_rx, reconnect_rx)
                .await;
        });
        Ok(OperationsStreamHandle {
            events,
            stop,
            force_reconnect,
        })
    }

    async fn run(
        self,
        accounts: Vec<String>,
        events: mpsc::Sender<OperationsStreamEvent>,
        mut stop: watch::Receiver<bool>,
        mut force_reconnect: watch::Receiver<u64>,
    ) {
        let mut failed_attempts = 0_u32;
        let connection_id = Uuid::new_v4();
        let mut revisions = BTreeMap::new();
        loop {
            if *stop.borrow() {
                let _ = events.send(OperationsStreamEvent::Stopped).await;
                return;
            }
            if failed_attempts > 0 {
                if failed_attempts >= self.config.reconnect_policy.max_attempts() {
                    let _ = events
                        .send(OperationsStreamEvent::Fault(
                            OperationsStreamError::ReconnectExhausted,
                        ))
                        .await;
                    return;
                }
                let delay = self
                    .config
                    .reconnect_policy
                    .delay_for(failed_attempts, connection_id);
                if events
                    .send(OperationsStreamEvent::Reconnecting {
                        attempt: failed_attempts,
                        delay,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            let _ = events.send(OperationsStreamEvent::Stopped).await;
                            return;
                        }
                    }
                }
            }

            let request = v1::OperationsStreamRequest {
                accounts: accounts.clone(),
                ping_settings: Some(v1::PingDelaySettings {
                    ping_delay_ms: Some(self.config.ping_delay_ms),
                }),
            };
            let mut stream = match self.connector.connect(request).await {
                Ok(stream) => stream,
                Err(error) => {
                    let reconnect = error.reconnectable();
                    failed_attempts += 1;
                    let _ = events.send(OperationsStreamEvent::Fault(error)).await;
                    if !reconnect {
                        return;
                    }
                    continue;
                }
            };
            if events.send(OperationsStreamEvent::Connected).await.is_err() {
                return;
            }
            let mut subscribed_stream_id = None;
            let expected = accounts.iter().cloned().collect::<BTreeSet<_>>();
            loop {
                tokio::select! {
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            let _ = events.send(OperationsStreamEvent::Stopped).await;
                            return;
                        }
                    }
                    changed = force_reconnect.changed() => {
                        if changed.is_err() {
                            let _ = events.send(OperationsStreamEvent::Stopped).await;
                            return;
                        }
                        failed_attempts = 1;
                        break;
                    }
                    response = timeout(self.config.stale_timeout, stream.message()) => {
                        let result = match response {
                            Err(_) => Err(OperationsStreamError::Stale),
                            Ok(Err(error)) => Err(error),
                            Ok(Ok(None)) => Err(OperationsStreamError::Closed),
                            Ok(Ok(Some(response))) => process_response(
                                response,
                                &expected,
                                &mut subscribed_stream_id,
                                &mut revisions,
                            ),
                        };
                        match result {
                            Ok(Some(event)) => {
                                if matches!(event, OperationsStreamEvent::Subscribed { .. }) {
                                    failed_attempts = 0;
                                }
                                if events.send(event).await.is_err() {
                                    return;
                                }
                            }
                            Ok(None) => {}
                            Err(error) => {
                                failed_attempts += 1;
                                let reconnect = error.reconnectable();
                                let _ = events.send(OperationsStreamEvent::Fault(error)).await;
                                if !reconnect {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn process_response(
    response: v1::OperationsStreamResponse,
    expected: &BTreeSet<String>,
    subscribed_stream_id: &mut Option<String>,
    revisions: &mut BTreeMap<OperationUpdateKey, u64>,
) -> Result<Option<OperationsStreamEvent>, OperationsStreamError> {
    use v1::operations_stream_response::Payload;
    match response.payload {
        Some(Payload::Subscriptions(value)) => {
            let actual = value.accounts.iter().cloned().collect::<BTreeSet<_>>();
            if actual != *expected
                || value.subscription_status
                    != v1::OperationsAccountSubscriptionStatus::OperationsSubscriptionStatusSuccess
                        as i32
                || value.stream_id.trim().is_empty()
            {
                return Err(OperationsStreamError::SubscriptionRejected {
                    status: value.subscription_status,
                });
            }
            subscribed_stream_id.clone_from(&Some(value.stream_id.clone()));
            Ok(Some(OperationsStreamEvent::Subscribed {
                accounts: value.accounts,
                tracking_id: optional_text(value.tracking_id),
                stream_id: value.stream_id,
            }))
        }
        Some(Payload::Operation(value)) => {
            if subscribed_stream_id.is_none() {
                return Err(OperationsStreamError::EventBeforeSubscription);
            }
            if value.broker_account_id.trim().is_empty()
                || !expected.contains(&value.broker_account_id)
            {
                return Err(OperationsStreamError::UnexpectedAccountIdentity);
            }
            let date = optional_timestamp(value.date)?;
            let key = OperationUpdateKey {
                broker_account_id: optional_text(value.broker_account_id.clone()),
                parent_operation_id: optional_text(value.parent_operation_id.clone()),
                instrument_uid: optional_text(value.instrument_uid.clone()),
                operation_type: value.r#type,
                date,
            };
            let revision = revisions.entry(key.clone()).or_default();
            *revision = revision.saturating_add(1);
            Ok(Some(OperationsStreamEvent::Operation(Box::new(
                StreamOperation {
                    update_key: key,
                    revision: *revision,
                    provider_operation_id: optional_text(value.id),
                    name: optional_text(value.name),
                    state: value.state,
                    figi: optional_text(value.figi),
                    instrument_type: optional_text(value.instrument_type),
                    instrument_kind: value.instrument_kind,
                    position_uid: optional_text(value.position_uid),
                    ticker: optional_text(value.ticker),
                    class_code: optional_text(value.class_code),
                    payment: optional_money(value.payment)?,
                },
            ))))
        }
        Some(Payload::Ping(value)) => {
            let Some(expected_stream_id) = subscribed_stream_id.as_ref() else {
                return Err(OperationsStreamError::EventBeforeSubscription);
            };
            if value.stream_id != *expected_stream_id {
                return Err(OperationsStreamError::UnexpectedStreamIdentity);
            }
            Ok(Some(OperationsStreamEvent::Ping {
                stream_id: Some(value.stream_id),
                at: optional_timestamp(value.time)?,
                request_at: optional_timestamp(value.ping_request_time)?,
            }))
        }
        None => Err(OperationsStreamError::MissingPayload),
    }
}

fn validate_accounts(accounts: Vec<String>) -> Result<Vec<String>, OperationsStreamError> {
    if accounts.is_empty() {
        return Err(OperationsStreamError::NoAccounts);
    }
    let mut seen = BTreeSet::new();
    for account in &accounts {
        if account.trim().is_empty() || !seen.insert(account.clone()) {
            return Err(OperationsStreamError::InvalidAccounts);
        }
    }
    Ok(accounts)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum OperationsStreamError {
    #[error("operations stream event capacity must be positive")]
    ZeroCapacity,
    #[error("operations stream stale timeout must be positive")]
    ZeroStaleTimeout,
    #[error("operations stream ping delay must be within 5000..=180000 ms")]
    InvalidPingDelay,
    #[error("operations stream requires at least one account")]
    NoAccounts,
    #[error("operations stream account IDs must be non-empty and unique")]
    InvalidAccounts,
    #[error("{0}")]
    Connect(GrpcError),
    #[error("{0}")]
    Stream(GrpcStreamError),
    #[error("operations stream became stale")]
    Stale,
    #[error("operations stream closed")]
    Closed,
    #[error("operations stream subscription rejected with status {status}")]
    SubscriptionRejected { status: i32 },
    #[error("operations stream delivered event before successful subscription")]
    EventBeforeSubscription,
    #[error("operations stream delivered operation for an unsubscribed account")]
    UnexpectedAccountIdentity,
    #[error("operations stream ping stream_id differs from subscription")]
    UnexpectedStreamIdentity,
    #[error("operations stream response omitted payload")]
    MissingPayload,
    #[error("{0}")]
    Canonical(#[from] AccountDataError),
    #[error("operations stream reconnect attempts exhausted")]
    ReconnectExhausted,
}

impl OperationsStreamError {
    fn reconnectable(&self) -> bool {
        match self {
            Self::Connect(GrpcError {
                kind: crate::GrpcErrorKind::Provider(provider),
                ..
            }) => matches!(
                provider.code,
                tonic::Code::Unavailable
                    | tonic::Code::ResourceExhausted
                    | tonic::Code::DeadlineExceeded
            ),
            Self::Stream(GrpcStreamError::Provider(provider)) => matches!(
                provider.code,
                tonic::Code::Unavailable
                    | tonic::Code::ResourceExhausted
                    | tonic::Code::DeadlineExceeded
            ),
            Self::Stream(GrpcStreamError::NoActiveSubscriptions(_))
            | Self::Stale
            | Self::Closed => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{GrpcErrorKind, GrpcProviderError, GrpcRequestMetadata};

    #[derive(Clone)]
    struct FakeConnector {
        connects: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<v1::OperationsStreamRequest>>>,
    }

    struct FakeConnection {
        messages: VecDeque<v1::OperationsStreamResponse>,
    }

    #[async_trait]
    impl OperationsStreamConnector for FakeConnector {
        async fn connect(
            &self,
            request: v1::OperationsStreamRequest,
        ) -> Result<Box<dyn OperationsStreamConnection>, OperationsStreamError> {
            let attempt = self.connects.fetch_add(1, Ordering::SeqCst);
            self.requests.lock().expect("requests lock").push(request);
            let mut messages = VecDeque::from([subscription(&["one", "two"])]);
            if attempt == 0 {
                messages.push_back(operation("mutable-id"));
            }
            Ok(Box::new(FakeConnection { messages }))
        }
    }

    #[async_trait]
    impl OperationsStreamConnection for FakeConnection {
        async fn message(
            &mut self,
        ) -> Result<Option<v1::OperationsStreamResponse>, OperationsStreamError> {
            Ok(self.messages.pop_front())
        }
    }

    fn subscription(accounts: &[&str]) -> v1::OperationsStreamResponse {
        v1::OperationsStreamResponse {
            payload: Some(v1::operations_stream_response::Payload::Subscriptions(
                v1::OperationsSubscriptionResult {
                    accounts: accounts.iter().map(|value| (*value).to_owned()).collect(),
                    subscription_status:
                        v1::OperationsAccountSubscriptionStatus::OperationsSubscriptionStatusSuccess
                            as i32,
                    tracking_id: "tracking".to_owned(),
                    stream_id: "stream".to_owned(),
                },
            )),
        }
    }

    fn operation(id: &str) -> v1::OperationsStreamResponse {
        v1::OperationsStreamResponse {
            payload: Some(v1::operations_stream_response::Payload::Operation(
                v1::OperationData {
                    broker_account_id: "one".to_owned(),
                    id: id.to_owned(),
                    parent_operation_id: "parent".to_owned(),
                    instrument_uid: "instrument".to_owned(),
                    date: Some(prost_types::Timestamp {
                        seconds: 1,
                        nanos: 0,
                    }),
                    ..Default::default()
                },
            )),
        }
    }

    #[test]
    fn duplicate_operations_emit_increasing_revisions_without_id_keying() {
        let expected = BTreeSet::from(["one".to_owned()]);
        let mut subscribed = None;
        let mut revisions = BTreeMap::new();
        process_response(
            subscription(&["one"]),
            &expected,
            &mut subscribed,
            &mut revisions,
        )
        .expect("subscription");
        let first = process_response(
            operation("old-id"),
            &expected,
            &mut subscribed,
            &mut revisions,
        )
        .expect("first operation");
        let second = process_response(
            operation("changed-id"),
            &expected,
            &mut subscribed,
            &mut revisions,
        )
        .expect("updated operation");
        assert!(
            matches!(first, Some(OperationsStreamEvent::Operation(value)) if value.revision == 1)
        );
        assert!(
            matches!(second, Some(OperationsStreamEvent::Operation(value)) if value.revision == 2)
        );
    }

    #[test]
    fn permanent_provider_errors_do_not_reconnect() {
        let error = OperationsStreamError::Connect(GrpcError {
            metadata: GrpcRequestMetadata {
                request_id: Uuid::nil(),
                method: "OperationsStream",
                attempt: 1,
                mutation: false,
            },
            kind: GrpcErrorKind::Provider(Box::new(GrpcProviderError {
                code: tonic::Code::PermissionDenied,
                message: "permission denied".to_owned(),
                details: Vec::new(),
                tracking_id: None,
                rate_limit: Box::default(),
            })),
        });
        assert!(!error.reconnectable());
    }

    #[test]
    fn events_must_match_subscribed_account_and_stream_identity() {
        let expected = BTreeSet::from(["one".to_owned()]);
        let mut subscribed = None;
        let mut revisions = BTreeMap::new();
        process_response(
            subscription(&["one"]),
            &expected,
            &mut subscribed,
            &mut revisions,
        )
        .expect("subscription");

        let mut wrong_account = operation("id");
        let Some(v1::operations_stream_response::Payload::Operation(value)) =
            wrong_account.payload.as_mut()
        else {
            unreachable!()
        };
        value.broker_account_id = "other".to_owned();
        assert_eq!(
            process_response(wrong_account, &expected, &mut subscribed, &mut revisions),
            Err(OperationsStreamError::UnexpectedAccountIdentity)
        );

        let wrong_ping = v1::OperationsStreamResponse {
            payload: Some(v1::operations_stream_response::Payload::Ping(v1::Ping {
                stream_id: "other-stream".to_owned(),
                ..Default::default()
            })),
        };
        assert_eq!(
            process_response(wrong_ping, &expected, &mut subscribed, &mut revisions),
            Err(OperationsStreamError::UnexpectedStreamIdentity)
        );
    }

    #[tokio::test]
    async fn reconnect_restores_multiple_accounts_and_ping_settings() {
        let connector = FakeConnector {
            connects: Arc::new(AtomicUsize::new(0)),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let evidence = connector.clone();
        let config = OperationsStreamConfig {
            reconnect_policy: RetryPolicy::new(
                3,
                Duration::from_millis(1),
                Duration::from_millis(1),
                0,
            )
            .expect("retry policy"),
            stale_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let supervisor =
            OperationsStreamSupervisor::with_connector(connector, config).expect("supervisor");
        let mut handle = supervisor
            .start(vec!["one".to_owned(), "two".to_owned()])
            .expect("stream");
        let mut subscriptions = 0;
        while subscriptions < 2 {
            let event = timeout(Duration::from_secs(1), handle.recv())
                .await
                .expect("event timeout")
                .expect("event channel");
            if matches!(event, OperationsStreamEvent::Subscribed { .. }) {
                subscriptions += 1;
            }
        }
        handle.stop();
        let requests = evidence.requests.lock().expect("requests lock");
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.accounts == ["one", "two"])
        );
        assert!(requests.iter().all(|request| {
            request
                .ping_settings
                .as_ref()
                .and_then(|value| value.ping_delay_ms)
                == Some(DEFAULT_OPERATIONS_PING_DELAY_MS)
        }));
    }
}
