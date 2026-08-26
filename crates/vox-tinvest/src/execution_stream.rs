//! Supervised T-Invest execution streams. Provider events remain ordered evidence.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep_until, timeout};
use uuid::Uuid;

use crate::execution::{
    CanonicalExecutionStreamEvent, ExecutionDecodeError, decode_order_state_stream,
    decode_trades_stream,
};
use crate::generated::v1;
use crate::{GrpcError, GrpcResponseMetadata, GrpcStreamError, RetryPolicy, TInvestGrpcClient};

pub const DEFAULT_EXECUTION_PING_DELAY_MS: i32 = 120_000;
pub const MIN_EXECUTION_PING_DELAY_MS: i32 = 5_000;
pub const MAX_EXECUTION_PING_DELAY_MS: i32 = 180_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionStreamKind {
    Trades,
    OrderState,
}

#[derive(Clone, Debug)]
pub struct ExecutionStreamConfig {
    pub event_capacity: usize,
    pub stale_timeout: Duration,
    pub subscription_ack_timeout: Duration,
    pub reconnect_policy: RetryPolicy,
    pub ping_delay_ms: i32,
}

impl Default for ExecutionStreamConfig {
    fn default() -> Self {
        Self {
            event_capacity: 1_024,
            stale_timeout: Duration::from_secs(150),
            subscription_ack_timeout: Duration::from_secs(30),
            reconnect_policy: RetryPolicy::default(),
            ping_delay_ms: DEFAULT_EXECUTION_PING_DELAY_MS,
        }
    }
}

impl ExecutionStreamConfig {
    pub fn validate(&self) -> Result<(), ExecutionStreamError> {
        if self.event_capacity == 0 {
            return Err(ExecutionStreamError::ZeroCapacity);
        }
        if self.stale_timeout.is_zero() {
            return Err(ExecutionStreamError::ZeroStaleTimeout);
        }
        if self.subscription_ack_timeout.is_zero() {
            return Err(ExecutionStreamError::ZeroSubscriptionAckTimeout);
        }
        if !(MIN_EXECUTION_PING_DELAY_MS..=MAX_EXECUTION_PING_DELAY_MS)
            .contains(&self.ping_delay_ms)
        {
            return Err(ExecutionStreamError::InvalidPingDelay);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionStreamEvent {
    Connected(ExecutionStreamConnectionEvidence),
    Reconnecting { attempt: u32, delay: Duration },
    Evidence(CanonicalExecutionStreamEvent),
    Fault(ExecutionStreamError),
    Stopped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionStreamConnectionEvidence {
    pub kind: ExecutionStreamKind,
    pub account_count: usize,
    pub ping_delay_ms: i32,
    pub request_id: Option<Uuid>,
    pub tracking_id: Option<String>,
}

pub struct ExecutionStreamHandle {
    events: mpsc::Receiver<ExecutionStreamEvent>,
    stop: watch::Sender<bool>,
    force_reconnect: watch::Sender<u64>,
}

impl ExecutionStreamHandle {
    pub async fn recv(&mut self) -> Option<ExecutionStreamEvent> {
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

pub enum ExecutionWireResponse {
    Trades(Box<v1::TradesStreamResponse>),
    OrderState(Box<v1::OrderStateStreamResponse>),
}

#[async_trait]
pub trait ExecutionStreamConnection: Send {
    async fn message(&mut self) -> Result<Option<ExecutionWireResponse>, ExecutionStreamError>;

    fn metadata(&self) -> Option<&GrpcResponseMetadata> {
        None
    }
}

#[async_trait]
pub trait ExecutionStreamConnector: Send + Sync {
    async fn connect(
        &self,
        kind: ExecutionStreamKind,
        accounts: Vec<String>,
        ping_delay_ms: i32,
    ) -> Result<Box<dyn ExecutionStreamConnection>, ExecutionStreamError>;
}

struct TonicExecutionStreamConnector(TInvestGrpcClient);

struct TradesConnection(crate::GrpcServerStream<v1::TradesStreamResponse>);
struct OrderStateConnection(crate::GrpcServerStream<v1::OrderStateStreamResponse>);

#[async_trait]
impl ExecutionStreamConnection for TradesConnection {
    async fn message(&mut self) -> Result<Option<ExecutionWireResponse>, ExecutionStreamError> {
        self.0
            .message()
            .await
            .map(|value| value.map(|response| ExecutionWireResponse::Trades(Box::new(response))))
            .map_err(ExecutionStreamError::Stream)
    }

    fn metadata(&self) -> Option<&GrpcResponseMetadata> {
        Some(&self.0.metadata)
    }
}

#[async_trait]
impl ExecutionStreamConnection for OrderStateConnection {
    async fn message(&mut self) -> Result<Option<ExecutionWireResponse>, ExecutionStreamError> {
        self.0
            .message()
            .await
            .map(|value| {
                value.map(|response| ExecutionWireResponse::OrderState(Box::new(response)))
            })
            .map_err(ExecutionStreamError::Stream)
    }

    fn metadata(&self) -> Option<&GrpcResponseMetadata> {
        Some(&self.0.metadata)
    }
}

#[async_trait]
impl ExecutionStreamConnector for TonicExecutionStreamConnector {
    async fn connect(
        &self,
        kind: ExecutionStreamKind,
        accounts: Vec<String>,
        ping_delay_ms: i32,
    ) -> Result<Box<dyn ExecutionStreamConnection>, ExecutionStreamError> {
        match kind {
            ExecutionStreamKind::Trades => self
                .0
                .open_trades_stream(trades_stream_request(accounts, ping_delay_ms))
                .await
                .map(|stream| {
                    Box::new(TradesConnection(stream)) as Box<dyn ExecutionStreamConnection>
                }),
            ExecutionStreamKind::OrderState => self
                .0
                .open_order_state_stream(v1::OrderStateStreamRequest {
                    accounts,
                    ping_delay_millis: Some(ping_delay_ms),
                })
                .await
                .map(|stream| {
                    Box::new(OrderStateConnection(stream)) as Box<dyn ExecutionStreamConnection>
                }),
        }
        .map_err(ExecutionStreamError::Connect)
    }
}

fn trades_stream_request(accounts: Vec<String>, ping_delay_ms: i32) -> v1::TradesStreamRequest {
    v1::TradesStreamRequest {
        accounts,
        ping_delay_ms: Some(ping_delay_ms),
    }
}

#[derive(Clone)]
pub struct ExecutionStreamSupervisor {
    connector: Arc<dyn ExecutionStreamConnector>,
    config: ExecutionStreamConfig,
}

impl ExecutionStreamSupervisor {
    pub fn new(
        client: TInvestGrpcClient,
        config: ExecutionStreamConfig,
    ) -> Result<Self, ExecutionStreamError> {
        Self::with_connector(TonicExecutionStreamConnector(client), config)
    }

    pub fn with_connector<C>(
        connector: C,
        config: ExecutionStreamConfig,
    ) -> Result<Self, ExecutionStreamError>
    where
        C: ExecutionStreamConnector + 'static,
    {
        config.validate()?;
        Ok(Self {
            connector: Arc::new(connector),
            config,
        })
    }

    pub fn start(
        &self,
        kind: ExecutionStreamKind,
        accounts: Vec<String>,
    ) -> Result<ExecutionStreamHandle, ExecutionStreamError> {
        let accounts = validate_accounts(accounts)?;
        let (events_tx, events) = mpsc::channel(self.config.event_capacity);
        let (stop, stop_rx) = watch::channel(false);
        let (force_reconnect, reconnect_rx) = watch::channel(0_u64);
        let supervisor = self.clone();
        tokio::spawn(async move {
            supervisor
                .run(kind, accounts, events_tx, stop_rx, reconnect_rx)
                .await;
        });
        Ok(ExecutionStreamHandle {
            events,
            stop,
            force_reconnect,
        })
    }

    async fn run(
        self,
        kind: ExecutionStreamKind,
        accounts: Vec<String>,
        events: mpsc::Sender<ExecutionStreamEvent>,
        mut stop: watch::Receiver<bool>,
        mut force_reconnect: watch::Receiver<u64>,
    ) {
        let connection_id = Uuid::new_v4();
        let expected = accounts.iter().cloned().collect::<BTreeSet<_>>();
        let mut failed_attempts = 0_u32;
        loop {
            if *stop.borrow() {
                let _ = events.send(ExecutionStreamEvent::Stopped).await;
                return;
            }
            if failed_attempts > 0 {
                if failed_attempts >= self.config.reconnect_policy.max_attempts() {
                    let _ = events
                        .send(ExecutionStreamEvent::Fault(
                            ExecutionStreamError::ReconnectExhausted,
                        ))
                        .await;
                    return;
                }
                let delay = self
                    .config
                    .reconnect_policy
                    .delay_for(failed_attempts, connection_id);
                if events
                    .send(ExecutionStreamEvent::Reconnecting {
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
                            let _ = events.send(ExecutionStreamEvent::Stopped).await;
                            return;
                        }
                    }
                }
            }
            let mut stream = match self
                .connector
                .connect(kind, accounts.clone(), self.config.ping_delay_ms)
                .await
            {
                Ok(stream) => stream,
                Err(error) => {
                    let reconnect = error.reconnectable();
                    failed_attempts = failed_attempts.saturating_add(1);
                    let _ = events.send(ExecutionStreamEvent::Fault(error)).await;
                    if !reconnect {
                        return;
                    }
                    continue;
                }
            };
            let metadata = stream.metadata();
            let connected = ExecutionStreamConnectionEvidence {
                kind,
                account_count: accounts.len(),
                ping_delay_ms: self.config.ping_delay_ms,
                request_id: metadata.map(|value| value.request_id),
                tracking_id: metadata.and_then(|value| value.tracking_id.clone()),
            };
            if events
                .send(ExecutionStreamEvent::Connected(connected))
                .await
                .is_err()
            {
                return;
            }
            let mut subscribed = false;
            let subscription_deadline = Instant::now() + self.config.subscription_ack_timeout;
            loop {
                tokio::select! {
                    () = sleep_until(subscription_deadline), if !subscribed => {
                        failed_attempts = failed_attempts.saturating_add(1);
                        let error = ExecutionStreamError::SubscriptionAckTimeout;
                        let _ = events.send(ExecutionStreamEvent::Fault(error)).await;
                        break;
                    }
                    changed = stop.changed() => {
                        if changed.is_err() || *stop.borrow() {
                            let _ = events.send(ExecutionStreamEvent::Stopped).await;
                            return;
                        }
                    }
                    changed = force_reconnect.changed() => {
                        if changed.is_err() {
                            let _ = events.send(ExecutionStreamEvent::Stopped).await;
                            return;
                        }
                        failed_attempts = 1;
                        break;
                    }
                    response = timeout(self.config.stale_timeout, stream.message()) => {
                        let result = match response {
                            Err(_) => Err(ExecutionStreamError::Stale),
                            Ok(Err(error)) => Err(error),
                            Ok(Ok(None)) => Err(ExecutionStreamError::Closed),
                            Ok(Ok(Some(response))) => process_response(
                                kind,
                                response,
                                &expected,
                                &mut subscribed,
                            ),
                        };
                        match result {
                            Ok(event) => {
                                if matches!(event, CanonicalExecutionStreamEvent::Subscription { .. }) {
                                    failed_attempts = 0;
                                }
                                if events.send(ExecutionStreamEvent::Evidence(event)).await.is_err() {
                                    return;
                                }
                            }
                            Err(error) => {
                                failed_attempts = failed_attempts.saturating_add(1);
                                let reconnect = error.reconnectable();
                                let _ = events.send(ExecutionStreamEvent::Fault(error)).await;
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
    kind: ExecutionStreamKind,
    response: ExecutionWireResponse,
    expected: &BTreeSet<String>,
    subscribed: &mut bool,
) -> Result<CanonicalExecutionStreamEvent, ExecutionStreamError> {
    let event = match (kind, response) {
        (ExecutionStreamKind::Trades, ExecutionWireResponse::Trades(response)) => {
            decode_trades_stream(*response)?
        }
        (ExecutionStreamKind::OrderState, ExecutionWireResponse::OrderState(response)) => {
            decode_order_state_stream(*response)?
        }
        _ => return Err(ExecutionStreamError::WrongWireStream),
    };
    match &event {
        CanonicalExecutionStreamEvent::Subscription {
            status, accounts, ..
        } => {
            let actual = accounts.iter().cloned().collect::<BTreeSet<_>>();
            if *status != v1::ResultSubscriptionStatus::Ok as i32 || actual != *expected {
                return Err(ExecutionStreamError::SubscriptionRejected { status: *status });
            }
            *subscribed = true;
        }
        CanonicalExecutionStreamEvent::Trades(batch) => {
            if batch
                .account_id
                .as_ref()
                .is_none_or(|account| !expected.contains(account))
            {
                return Err(ExecutionStreamError::UnexpectedAccountIdentity);
            }
        }
        CanonicalExecutionStreamEvent::OrderState(state) => {
            if state
                .account_id
                .as_ref()
                .is_none_or(|account| !expected.contains(account))
            {
                return Err(ExecutionStreamError::UnexpectedAccountIdentity);
            }
        }
        CanonicalExecutionStreamEvent::StopOrderState(state) => {
            if state
                .account_id
                .as_ref()
                .is_none_or(|account| !expected.contains(account))
            {
                return Err(ExecutionStreamError::UnexpectedAccountIdentity);
            }
        }
        CanonicalExecutionStreamEvent::Ping(_) => {}
    }
    Ok(event)
}

fn validate_accounts(accounts: Vec<String>) -> Result<Vec<String>, ExecutionStreamError> {
    if accounts.is_empty() {
        return Err(ExecutionStreamError::NoAccounts);
    }
    let mut seen = BTreeSet::new();
    for account in &accounts {
        if account.trim().is_empty() || !seen.insert(account.clone()) {
            return Err(ExecutionStreamError::InvalidAccounts);
        }
    }
    Ok(accounts)
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ExecutionStreamError {
    #[error("execution stream event capacity must be positive")]
    ZeroCapacity,
    #[error("execution stream stale timeout must be positive")]
    ZeroStaleTimeout,
    #[error("execution stream subscription ACK timeout must be positive")]
    ZeroSubscriptionAckTimeout,
    #[error("execution stream ping delay must be within 5000..=180000 ms")]
    InvalidPingDelay,
    #[error("execution stream requires at least one account")]
    NoAccounts,
    #[error("execution stream account IDs must be non-empty and unique")]
    InvalidAccounts,
    #[error("{0}")]
    Connect(GrpcError),
    #[error("{0}")]
    Stream(GrpcStreamError),
    #[error("{0}")]
    Decode(#[from] ExecutionDecodeError),
    #[error("execution stream became stale")]
    Stale,
    #[error("execution stream closed")]
    Closed,
    #[error("execution stream subscription rejected with status {status}")]
    SubscriptionRejected { status: i32 },
    #[error("execution stream subscription ACK timed out")]
    SubscriptionAckTimeout,
    #[error("execution stream delivered an unsubscribed account identity")]
    UnexpectedAccountIdentity,
    #[error("execution connector returned response for wrong stream")]
    WrongWireStream,
    #[error("execution stream reconnect attempts exhausted")]
    ReconnectExhausted,
}

impl ExecutionStreamError {
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
            | Self::Closed
            | Self::SubscriptionAckTimeout => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    struct PendingConnector;
    struct PendingConnection;

    #[async_trait]
    impl ExecutionStreamConnector for PendingConnector {
        async fn connect(
            &self,
            _kind: ExecutionStreamKind,
            _accounts: Vec<String>,
            _ping_delay_ms: i32,
        ) -> Result<Box<dyn ExecutionStreamConnection>, ExecutionStreamError> {
            Ok(Box::new(PendingConnection))
        }
    }

    #[async_trait]
    impl ExecutionStreamConnection for PendingConnection {
        async fn message(&mut self) -> Result<Option<ExecutionWireResponse>, ExecutionStreamError> {
            std::future::pending().await
        }
    }

    fn subscription(accounts: Vec<String>) -> ExecutionWireResponse {
        ExecutionWireResponse::Trades(Box::new(v1::TradesStreamResponse {
            payload: Some(v1::trades_stream_response::Payload::Subscription(
                v1::SubscriptionResponse {
                    status: v1::ResultSubscriptionStatus::Ok as i32,
                    stream_id: "stream".into(),
                    accounts,
                    ..Default::default()
                },
            )),
        }))
    }

    #[test]
    fn validates_bounded_configuration_and_accounts() {
        let config = ExecutionStreamConfig {
            ping_delay_ms: 180_000,
            ..Default::default()
        };
        assert!(config.validate().is_ok());
        let config = ExecutionStreamConfig {
            ping_delay_ms: 180_001,
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ExecutionStreamError::InvalidPingDelay)
        );
        let config = ExecutionStreamConfig {
            event_capacity: 0,
            ..Default::default()
        };
        assert_eq!(config.validate(), Err(ExecutionStreamError::ZeroCapacity));
        let config = ExecutionStreamConfig {
            subscription_ack_timeout: Duration::ZERO,
            ..Default::default()
        };
        assert_eq!(
            config.validate(),
            Err(ExecutionStreamError::ZeroSubscriptionAckTimeout)
        );
        assert_eq!(
            validate_accounts(vec![]),
            Err(ExecutionStreamError::NoAccounts)
        );
        assert_eq!(
            validate_accounts(vec!["a".into(), "a".into()]),
            Err(ExecutionStreamError::InvalidAccounts)
        );
    }

    #[test]
    fn trades_stream_wire_uses_generated_account_and_ping_fields_exactly() {
        let request = trades_stream_request(vec!["sandbox-account".into()], 5_000);
        let bytes = request.encode_to_vec();
        let decoded = v1::TradesStreamRequest::decode(bytes.as_slice()).expect("generated decode");
        assert_eq!(decoded.accounts, vec!["sandbox-account"]);
        assert_eq!(decoded.ping_delay_ms, Some(5_000));
    }

    #[tokio::test]
    async fn missing_ack_faults_within_its_own_bound() {
        let supervisor = ExecutionStreamSupervisor::with_connector(
            PendingConnector,
            ExecutionStreamConfig {
                subscription_ack_timeout: Duration::from_millis(10),
                stale_timeout: Duration::from_secs(1),
                reconnect_policy: RetryPolicy::new(
                    1,
                    Duration::from_millis(1),
                    Duration::from_millis(1),
                    0,
                )
                .expect("retry policy"),
                ..Default::default()
            },
        )
        .expect("supervisor");
        let mut handle = supervisor
            .start(ExecutionStreamKind::Trades, vec!["a".into()])
            .expect("stream");
        assert!(matches!(
            handle.recv().await,
            Some(ExecutionStreamEvent::Connected(
                ExecutionStreamConnectionEvidence {
                    kind: ExecutionStreamKind::Trades,
                    account_count: 1,
                    ping_delay_ms: DEFAULT_EXECUTION_PING_DELAY_MS,
                    request_id: None,
                    tracking_id: None,
                }
            ))
        ));
        let event = timeout(Duration::from_millis(100), handle.recv())
            .await
            .expect("bounded ACK timeout");
        assert_eq!(
            event,
            Some(ExecutionStreamEvent::Fault(
                ExecutionStreamError::SubscriptionAckTimeout
            ))
        );
        handle.stop();
    }

    #[test]
    fn requires_ack_and_exact_restored_account_set() {
        let expected = ["a".to_owned()].into_iter().collect();
        let mut subscribed = false;
        let ping = ExecutionWireResponse::Trades(Box::new(v1::TradesStreamResponse {
            payload: Some(v1::trades_stream_response::Payload::Ping(
                v1::Ping::default(),
            )),
        }));
        assert!(
            process_response(
                ExecutionStreamKind::Trades,
                ping,
                &expected,
                &mut subscribed
            )
            .is_ok()
        );
        assert!(!subscribed);
        let event = ExecutionWireResponse::Trades(Box::new(v1::TradesStreamResponse {
            payload: Some(v1::trades_stream_response::Payload::OrderTrades(
                v1::OrderTrades {
                    account_id: "a".into(),
                    ..Default::default()
                },
            )),
        }));
        assert!(
            process_response(
                ExecutionStreamKind::Trades,
                event,
                &expected,
                &mut subscribed
            )
            .is_ok()
        );
        assert!(!subscribed);
        assert!(
            process_response(
                ExecutionStreamKind::Trades,
                subscription(vec!["a".into()]),
                &expected,
                &mut subscribed,
            )
            .is_ok()
        );
        assert!(subscribed);
        let mut other = false;
        assert_eq!(
            process_response(
                ExecutionStreamKind::Trades,
                subscription(vec!["b".into()]),
                &expected,
                &mut other,
            ),
            Err(ExecutionStreamError::SubscriptionRejected {
                status: v1::ResultSubscriptionStatus::Ok as i32
            })
        );
    }

    #[test]
    fn duplicate_and_out_of_order_events_remain_unmodified_evidence() {
        let expected = ["a".to_owned()].into_iter().collect();
        let mut subscribed = false;
        process_response(
            ExecutionStreamKind::Trades,
            subscription(vec!["a".into()]),
            &expected,
            &mut subscribed,
        )
        .expect("subscription");
        let response = || {
            ExecutionWireResponse::Trades(Box::new(v1::TradesStreamResponse {
                payload: Some(v1::trades_stream_response::Payload::OrderTrades(
                    v1::OrderTrades {
                        order_id: "order".into(),
                        account_id: "a".into(),
                        instrument_uid: "instrument".into(),
                        ..Default::default()
                    },
                )),
            }))
        };
        let first = process_response(
            ExecutionStreamKind::Trades,
            response(),
            &expected,
            &mut subscribed,
        )
        .expect("first");
        let duplicate = process_response(
            ExecutionStreamKind::Trades,
            response(),
            &expected,
            &mut subscribed,
        )
        .expect("duplicate");
        assert_eq!(first, duplicate);
    }
}
