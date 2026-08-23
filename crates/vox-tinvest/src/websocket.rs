use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    AUTHORIZATION, HeaderValue, SEC_WEBSOCKET_PROTOCOL,
};
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig as TungsteniteConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async_with_config};
use url::Url;
use uuid::Uuid;

use crate::{RetryPolicy, RetryPolicyError, SecretToken};

pub const DEFAULT_MARKET_DATA_STREAM_URL: &str = concat!(
    "wss://invest-public-api.tbank.ru/ws/",
    "tinkoff.public.invest.api.contract.v1.MarketDataStreamService/MarketDataStream"
);
const REQUEST_ID_HEADER: &str = "x-request-id";
const DEFAULT_MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_COMMAND_CAPACITY: usize = 8;
const DEFAULT_ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_LABEL_BYTES: usize = 256;

type ProviderSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificatePolicy {
    /// Roots loaded from host operating-system trust store; verification remains enabled.
    NativeRoots,
}

#[derive(Clone, Debug)]
pub struct WebSocketConfig {
    endpoint: Url,
    connect_timeout: Duration,
    close_timeout: Duration,
    max_message_bytes: usize,
    certificate_policy: CertificatePolicy,
}

impl WebSocketConfig {
    pub fn production() -> Self {
        let endpoint = match Url::parse(DEFAULT_MARKET_DATA_STREAM_URL) {
            Ok(url) => url,
            Err(error) => unreachable!("built-in T-Invest WebSocket URL is invalid: {error}"),
        };
        Self {
            endpoint,
            connect_timeout: Duration::from_secs(20),
            close_timeout: Duration::from_secs(5),
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
            certificate_policy: CertificatePolicy::NativeRoots,
        }
    }

    pub fn with_endpoint(mut self, endpoint: Url) -> Result<Self, WebSocketError> {
        validate_wss_url(&endpoint)?;
        self.endpoint = endpoint;
        Ok(self)
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        close_timeout: Duration,
    ) -> Result<Self, WebSocketError> {
        if connect_timeout.is_zero() || close_timeout.is_zero() {
            return Err(WebSocketError::InvalidConfig(
                "WebSocket timeouts must be positive".to_owned(),
            ));
        }
        self.connect_timeout = connect_timeout;
        self.close_timeout = close_timeout;
        Ok(self)
    }

    pub fn with_max_message_bytes(mut self, limit: usize) -> Result<Self, WebSocketError> {
        if limit == 0 {
            return Err(WebSocketError::InvalidConfig(
                "WebSocket message limit must be positive".to_owned(),
            ));
        }
        self.max_message_bytes = limit;
        Ok(self)
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub const fn certificate_policy(&self) -> CertificatePolicy {
        self.certificate_policy
    }
}

fn validate_wss_url(url: &Url) -> Result<(), WebSocketError> {
    if url.scheme() != "wss" {
        return Err(WebSocketError::InvalidConfig(
            "T-Invest WebSocket endpoint must use WSS".to_owned(),
        ));
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(WebSocketError::InvalidConfig(
            "T-Invest WebSocket endpoint must contain a host and no credentials".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Clone)]
pub struct TInvestWebSocket {
    token: SecretToken,
    config: WebSocketConfig,
}

impl TInvestWebSocket {
    pub fn new(token: SecretToken, config: WebSocketConfig) -> Result<Self, WebSocketError> {
        validate_wss_url(&config.endpoint)?;
        Ok(Self { token, config })
    }

    pub fn production(token: SecretToken) -> Result<Self, WebSocketError> {
        Self::new(token, WebSocketConfig::production())
    }

    pub async fn connect(&self) -> Result<WebSocketSession, WebSocketError> {
        let connection_id = Uuid::new_v4();
        let mut request = self
            .config
            .endpoint
            .as_str()
            .into_client_request()
            .map_err(|error| WebSocketError::Handshake(error.to_string()))?;
        let mut authorization =
            HeaderValue::from_str(&format!("Bearer {}", self.token.expose_secret()))
                .map_err(|_| WebSocketError::InvalidCredential)?;
        authorization.set_sensitive(true);
        request.headers_mut().insert(AUTHORIZATION, authorization);
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("json-proto"),
        );
        request.headers_mut().insert(
            REQUEST_ID_HEADER,
            HeaderValue::from_str(&connection_id.to_string())
                .map_err(|_| WebSocketError::Handshake("invalid request ID".to_owned()))?,
        );

        let mut protocol_config = TungsteniteConfig::default();
        protocol_config.max_message_size = Some(self.config.max_message_bytes);
        protocol_config.max_frame_size = Some(self.config.max_message_bytes);
        let connect = connect_async_with_config(request, Some(protocol_config), false);
        let (socket, _response) = tokio::time::timeout(self.config.connect_timeout, connect)
            .await
            .map_err(|_| WebSocketError::ConnectTimeout(self.config.connect_timeout))?
            .map_err(|error| WebSocketError::Connect(error.to_string()))?;
        Ok(WebSocketSession {
            connection_id,
            socket,
            max_message_bytes: self.config.max_message_bytes,
            close_timeout: self.config.close_timeout,
        })
    }

    pub fn stream_supervisor(
        self,
        registry: SubscriptionRegistry,
        reconnect_policy: ReconnectPolicy,
        event_capacity: usize,
    ) -> Result<StreamSupervisor, WebSocketError> {
        StreamSupervisor::new(self, registry, reconnect_policy, event_capacity)
    }
}

impl fmt::Debug for TInvestWebSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TInvestWebSocket")
            .field("token", &self.token)
            .field("config", &self.config)
            .finish()
    }
}

pub struct WebSocketSession {
    connection_id: Uuid,
    socket: ProviderSocket,
    max_message_bytes: usize,
    close_timeout: Duration,
}

impl WebSocketSession {
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    pub async fn send_json<T>(&mut self, payload: &T) -> Result<(), WebSocketError>
    where
        T: Serialize + Sync,
    {
        let encoded = serde_json::to_string(payload).map_err(WebSocketError::Encode)?;
        if encoded.len() > self.max_message_bytes {
            return Err(WebSocketError::MessageTooLarge {
                actual: encoded.len(),
                limit: self.max_message_bytes,
            });
        }
        self.socket
            .send(Message::Text(encoded.into()))
            .await
            .map_err(|error| WebSocketError::Send(error.to_string()))
    }

    pub async fn receive_json<T>(&mut self) -> Result<T, WebSocketError>
    where
        T: DeserializeOwned,
    {
        loop {
            let message = self
                .socket
                .next()
                .await
                .ok_or(WebSocketError::Closed(None))?
                .map_err(|error| WebSocketError::Receive(error.to_string()))?;
            let bytes = match message {
                Message::Text(text) => text.as_bytes().to_vec(),
                Message::Binary(binary) => binary.to_vec(),
                Message::Close(frame) => {
                    return Err(WebSocketError::Closed(
                        frame.map(|frame| frame.reason.to_string()),
                    ));
                }
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            };
            if bytes.len() > self.max_message_bytes {
                return Err(WebSocketError::MessageTooLarge {
                    actual: bytes.len(),
                    limit: self.max_message_bytes,
                });
            }
            return serde_json::from_slice(&bytes).map_err(WebSocketError::Decode);
        }
    }

    pub async fn close(&mut self) -> Result<(), WebSocketError> {
        let close = self.socket.close(None);
        tokio::time::timeout(self.close_timeout, close)
            .await
            .map_err(|_| WebSocketError::CloseTimeout(self.close_timeout))?
            .map_err(|error| WebSocketError::Close(error.to_string()))
    }
}

impl fmt::Debug for WebSocketSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebSocketSession")
            .field("connection_id", &self.connection_id)
            .field("max_message_bytes", &self.max_message_bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub enum WebSocketError {
    #[error("invalid WebSocket configuration: {0}")]
    InvalidConfig(String),
    #[error("Bearer credential cannot be represented as a WebSocket header")]
    InvalidCredential,
    #[error("failed to construct WebSocket handshake: {0}")]
    Handshake(String),
    #[error("WebSocket connect timed out after {0:?}")]
    ConnectTimeout(Duration),
    #[error("WebSocket connect failed: {0}")]
    Connect(String),
    #[error("failed to encode provider message")]
    Encode(#[source] serde_json::Error),
    #[error("provider message is {actual} bytes; limit is {limit}")]
    MessageTooLarge { actual: usize, limit: usize },
    #[error("WebSocket send failed: {0}")]
    Send(String),
    #[error("WebSocket receive failed: {0}")]
    Receive(String),
    #[error("provider returned invalid WebSocket JSON")]
    Decode(#[source] serde_json::Error),
    #[error("subscription acknowledgement failed: {0}")]
    SubscriptionAcknowledgement(String),
    #[error("subscription acknowledgements timed out after {0:?}")]
    SubscriptionAcknowledgementTimeout(Duration),
    #[error("WebSocket closed; provider reason: {0:?}")]
    Closed(Option<String>),
    #[error("WebSocket close timed out after {0:?}")]
    CloseTimeout(Duration),
    #[error("WebSocket close failed: {0}")]
    Close(String),
    #[error("stream event or command capacity must be positive")]
    ZeroChannelCapacity,
    #[error("stream supervisor requires a Tokio runtime")]
    NoRuntime,
    #[error("stream supervisor task failed: {0}")]
    SupervisorTask(String),
    #[error("stream supervisor has stopped")]
    SupervisorStopped,
}

#[async_trait]
pub trait WebSocketConnection: Send + fmt::Debug {
    fn connection_id(&self) -> Uuid;
    async fn send_value(&mut self, payload: &Value) -> Result<(), WebSocketError>;
    async fn receive_value(&mut self) -> Result<Value, WebSocketError>;
    async fn close(&mut self) -> Result<(), WebSocketError>;
}

#[async_trait]
impl WebSocketConnection for WebSocketSession {
    fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    async fn send_value(&mut self, payload: &Value) -> Result<(), WebSocketError> {
        self.send_json(payload).await
    }

    async fn receive_value(&mut self) -> Result<Value, WebSocketError> {
        self.receive_json().await
    }

    async fn close(&mut self) -> Result<(), WebSocketError> {
        WebSocketSession::close(self).await
    }
}

#[async_trait]
pub trait WebSocketConnector: Send + Sync + fmt::Debug {
    async fn connect(&self) -> Result<Box<dyn WebSocketConnection>, WebSocketError>;
}

#[async_trait]
impl WebSocketConnector for TInvestWebSocket {
    async fn connect(&self) -> Result<Box<dyn WebSocketConnection>, WebSocketError> {
        TInvestWebSocket::connect(self)
            .await
            .map(|session| Box::new(session) as Box<dyn WebSocketConnection>)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderMessage(Value);

impl ProviderMessage {
    pub(crate) const fn new(value: Value) -> Self {
        Self(value)
    }

    pub fn from_serializable<T>(value: &T) -> Result<Self, WebSocketError>
    where
        T: Serialize,
    {
        serde_json::to_value(value)
            .map(Self)
            .map_err(WebSocketError::Encode)
    }

    pub(crate) fn decode<T>(&self) -> Result<T, WebSocketError>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.0.clone()).map_err(WebSocketError::Decode)
    }

    pub(crate) const fn as_value(&self) -> &Value {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(String);

impl SubscriptionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SubscriptionRegistryError> {
        let value = value.into();
        validate_label(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AckKey(String);

impl AckKey {
    /// Top-level T-Invest response field, for example `subscribe_trades_response`.
    pub fn new(value: impl Into<String>) -> Result<Self, SubscriptionRegistryError> {
        let value = value.into();
        validate_label(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_label(value: &str) -> Result<(), SubscriptionRegistryError> {
    if value.trim().is_empty()
        || value.len() > MAX_LABEL_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(SubscriptionRegistryError::InvalidLabel);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct DesiredSubscription {
    pub id: SubscriptionId,
    pub request: ProviderMessage,
    pub acknowledgement: Option<AckKey>,
    pub expected_instrument_uid: Option<String>,
}

impl DesiredSubscription {
    pub const fn new(
        id: SubscriptionId,
        request: ProviderMessage,
        acknowledgement: AckKey,
    ) -> Self {
        Self {
            id,
            request,
            acknowledgement: Some(acknowledgement),
            expected_instrument_uid: None,
        }
    }

    /// Connection setup message which requires replay but has no subscription ACK.
    pub const fn without_ack(id: SubscriptionId, request: ProviderMessage) -> Self {
        Self {
            id,
            request,
            acknowledgement: None,
            expected_instrument_uid: None,
        }
    }

    #[must_use]
    pub fn with_expected_instrument_uid(mut self, instrument_uid: impl Into<String>) -> Self {
        self.expected_instrument_uid = Some(instrument_uid.into());
        self
    }
}

#[derive(Clone, Debug)]
pub struct SubscriptionRegistry {
    capacity: usize,
    inner: Arc<RwLock<RegistryState>>,
}

#[derive(Debug, Default)]
struct RegistryState {
    by_id: BTreeMap<SubscriptionId, DesiredSubscription>,
    ack_owner: BTreeMap<AckKey, SubscriptionId>,
}

impl SubscriptionRegistry {
    pub fn new(capacity: usize) -> Result<Self, SubscriptionRegistryError> {
        if capacity == 0 {
            return Err(SubscriptionRegistryError::ZeroCapacity);
        }
        Ok(Self {
            capacity,
            inner: Arc::new(RwLock::new(RegistryState::default())),
        })
    }

    pub async fn upsert(
        &self,
        subscription: DesiredSubscription,
    ) -> Result<(), SubscriptionRegistryError> {
        let mut state = self.inner.write().await;
        let is_new = !state.by_id.contains_key(&subscription.id);
        if is_new && state.by_id.len() >= self.capacity {
            return Err(SubscriptionRegistryError::CapacityExceeded(self.capacity));
        }
        if let Some(acknowledgement) = &subscription.acknowledgement
            && let Some(owner) = state.ack_owner.get(acknowledgement)
            && owner != &subscription.id
        {
            return Err(SubscriptionRegistryError::DuplicateAcknowledgement(
                acknowledgement.clone(),
            ));
        }
        if let Some(previous) = state.by_id.get(&subscription.id)
            && let Some(previous_ack) = previous.acknowledgement.clone()
        {
            state.ack_owner.remove(&previous_ack);
        }
        if let Some(acknowledgement) = &subscription.acknowledgement {
            state
                .ack_owner
                .insert(acknowledgement.clone(), subscription.id.clone());
        }
        state.by_id.insert(subscription.id.clone(), subscription);
        Ok(())
    }

    pub async fn remove(&self, id: &SubscriptionId) -> Option<DesiredSubscription> {
        let mut state = self.inner.write().await;
        let removed = state.by_id.remove(id);
        if let Some(subscription) = &removed
            && let Some(acknowledgement) = &subscription.acknowledgement
        {
            state.ack_owner.remove(acknowledgement);
        }
        removed
    }

    pub async fn snapshot(&self) -> Vec<DesiredSubscription> {
        self.inner.read().await.by_id.values().cloned().collect()
    }

    pub async fn len(&self) -> usize {
        self.inner.read().await.by_id.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.by_id.is_empty()
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SubscriptionRegistryError {
    #[error("subscription registry capacity must be positive")]
    ZeroCapacity,
    #[error("subscription registry reached bounded capacity {0}")]
    CapacityExceeded(usize),
    #[error("subscription and acknowledgement labels must be 1..=256 bytes without controls")]
    InvalidLabel,
    #[error("acknowledgement key {0:?} already belongs to another desired subscription")]
    DuplicateAcknowledgement(AckKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcknowledgementTracker {
    expected: BTreeSet<AckKey>,
    expected_instrument_uids: BTreeMap<AckKey, String>,
    acknowledged: BTreeSet<AckKey>,
}

impl AcknowledgementTracker {
    pub fn from_subscriptions(subscriptions: &[DesiredSubscription]) -> Self {
        let expected = subscriptions
            .iter()
            .filter_map(|subscription| subscription.acknowledgement.clone())
            .collect();
        let expected_instrument_uids = subscriptions
            .iter()
            .filter_map(|subscription| {
                Some((
                    subscription.acknowledgement.clone()?,
                    subscription.expected_instrument_uid.clone()?,
                ))
            })
            .collect();
        Self {
            expected,
            expected_instrument_uids,
            acknowledged: BTreeSet::new(),
        }
    }

    /// Accepts all matching top-level ACK fields, independent of arrival order.
    pub fn observe(&mut self, message: &Value) -> Vec<AckKey> {
        let Some(object) = message.as_object() else {
            return Vec::new();
        };
        let mut newly_acknowledged = Vec::new();
        for key in &self.expected {
            if object.contains_key(key.as_str()) && self.acknowledged.insert(key.clone()) {
                newly_acknowledged.push(key.clone());
            }
        }
        newly_acknowledged
    }

    /// Validates known T-Invest subscription-status arrays before accepting ACKs.
    /// Unknown extension ACK keys retain top-level-field semantics.
    pub fn observe_checked(
        &mut self,
        message: &Value,
    ) -> Result<Vec<AckKey>, AcknowledgementError> {
        let Some(object) = message.as_object() else {
            return Ok(Vec::new());
        };
        for key in &self.expected {
            let Some(response) = object.get(key.as_str()) else {
                continue;
            };
            let Some(status_field) = known_status_field(key.as_str()) else {
                continue;
            };
            let statuses = response
                .get(status_field)
                .and_then(Value::as_array)
                .ok_or_else(|| AcknowledgementError::MissingStatuses(key.clone()))?;
            if statuses.is_empty() {
                return Err(AcknowledgementError::MissingStatuses(key.clone()));
            }
            if let Some(expected_uid) = self.expected_instrument_uids.get(key) {
                for status in statuses {
                    let reported_uid = status
                        .get("instrument_uid")
                        .or_else(|| status.get("instrumentUid"))
                        .and_then(Value::as_str);
                    if reported_uid != Some(expected_uid.as_str()) {
                        return Err(AcknowledgementError::UnexpectedInstrument {
                            acknowledgement: key.clone(),
                            expected: expected_uid.clone(),
                            reported: reported_uid.map(str::to_owned),
                        });
                    }
                }
            }
            let reported: Vec<String> = statuses
                .iter()
                .map(|status| {
                    status
                        .get("subscription_status")
                        .and_then(Value::as_str)
                        .unwrap_or("<missing>")
                        .to_owned()
                })
                .collect();
            if reported
                .iter()
                .any(|status| status != "SUBSCRIPTION_STATUS_SUCCESS")
            {
                return Err(AcknowledgementError::Rejected {
                    acknowledgement: key.clone(),
                    statuses: reported,
                });
            }
        }
        Ok(self.observe(message))
    }

    pub fn is_complete(&self) -> bool {
        self.acknowledged == self.expected
    }

    pub fn remaining(&self) -> Vec<AckKey> {
        self.expected
            .difference(&self.acknowledged)
            .cloned()
            .collect()
    }
}

fn known_status_field(acknowledgement: &str) -> Option<&'static str> {
    match acknowledgement {
        "subscribe_trades_response" => Some("trade_subscriptions"),
        "subscribe_order_book_response" => Some("order_book_subscriptions"),
        "subscribe_info_response" => Some("info_subscriptions"),
        "subscribe_last_price_response" => Some("last_price_subscriptions"),
        _ => None,
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AcknowledgementError {
    #[error("{0:?} contains no provider subscription statuses")]
    MissingStatuses(AckKey),
    #[error("{acknowledgement:?} contains non-success statuses {statuses:?}")]
    Rejected {
        acknowledgement: AckKey,
        statuses: Vec<String>,
    },
    #[error(
        "{acknowledgement:?} ACK instrument mismatch: expected {expected}, reported {reported:?}"
    )]
    UnexpectedInstrument {
        acknowledgement: AckKey,
        expected: String,
        reported: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconnectPolicy {
    backoff: RetryPolicy,
}

impl ReconnectPolicy {
    pub fn new(
        max_consecutive_failures: u32,
        initial_delay: Duration,
        max_delay: Duration,
        jitter_basis_points: u16,
    ) -> Result<Self, RetryPolicyError> {
        Ok(Self {
            backoff: RetryPolicy::new(
                max_consecutive_failures,
                initial_delay,
                max_delay,
                jitter_basis_points,
            )?,
        })
    }

    pub const fn max_consecutive_failures(self) -> u32 {
        self.backoff.max_attempts()
    }

    pub fn delay_for(self, failure: u32, connection_id: Uuid) -> Duration {
        self.backoff.delay_for(failure, connection_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionMetadata {
    pub connection_id: Uuid,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    Connected {
        connection: ConnectionMetadata,
        desired_subscriptions: usize,
    },
    SubscriptionAcknowledged {
        connection: ConnectionMetadata,
        acknowledgement: AckKey,
        remaining: usize,
    },
    SubscriptionsReady {
        connection: ConnectionMetadata,
    },
    Message {
        connection: ConnectionMetadata,
        message: ProviderMessage,
    },
    Disconnected {
        connection: Option<ConnectionMetadata>,
        reason: String,
    },
    ReconnectScheduled {
        consecutive_failure: u32,
        delay: Duration,
    },
    Stopped {
        reason: StreamStopReason,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamStopReason {
    Shutdown,
    ReceiverDropped,
    ReconnectExhausted,
}

pub struct StreamSupervisor {
    connector: Arc<dyn WebSocketConnector>,
    registry: SubscriptionRegistry,
    reconnect_policy: ReconnectPolicy,
    event_capacity: usize,
    command_capacity: usize,
    acknowledgement_timeout: Duration,
}

impl StreamSupervisor {
    pub fn new<C>(
        connector: C,
        registry: SubscriptionRegistry,
        reconnect_policy: ReconnectPolicy,
        event_capacity: usize,
    ) -> Result<Self, WebSocketError>
    where
        C: WebSocketConnector + 'static,
    {
        if event_capacity == 0 {
            return Err(WebSocketError::ZeroChannelCapacity);
        }
        Ok(Self {
            connector: Arc::new(connector),
            registry,
            reconnect_policy,
            event_capacity,
            command_capacity: DEFAULT_COMMAND_CAPACITY,
            acknowledgement_timeout: DEFAULT_ACKNOWLEDGEMENT_TIMEOUT,
        })
    }

    pub fn with_command_capacity(mut self, capacity: usize) -> Result<Self, WebSocketError> {
        if capacity == 0 {
            return Err(WebSocketError::ZeroChannelCapacity);
        }
        self.command_capacity = capacity;
        Ok(self)
    }

    pub fn with_acknowledgement_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, WebSocketError> {
        if timeout.is_zero() {
            return Err(WebSocketError::InvalidConfig(
                "subscription acknowledgement timeout must be positive".into(),
            ));
        }
        self.acknowledgement_timeout = timeout;
        Ok(self)
    }

    pub fn start(self) -> Result<StreamHandle, WebSocketError> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| WebSocketError::NoRuntime)?;
        let event_capacity = self.event_capacity;
        let (event_sender, event_receiver) = mpsc::channel(event_capacity);
        let (command_sender, command_receiver) = mpsc::channel(self.command_capacity);
        let task = runtime.spawn(self.run(event_sender, command_receiver));
        Ok(StreamHandle {
            control: StreamControl {
                sender: command_sender,
            },
            events: StreamEvents {
                receiver: event_receiver,
                configured_capacity: event_capacity,
            },
            task,
        })
    }

    async fn run(
        self,
        events: mpsc::Sender<StreamEvent>,
        mut commands: mpsc::Receiver<StreamCommand>,
    ) {
        let mut generation = 0_u64;
        let mut consecutive_failures = 0_u32;
        loop {
            let connect = self.connector.connect();
            tokio::pin!(connect);
            let connection_result = tokio::select! {
                command = commands.recv() => {
                    match command {
                        Some(StreamCommand::Shutdown) | None => {
                            let _ = emit(&events, StreamEvent::Stopped { reason: StreamStopReason::Shutdown }).await;
                            return;
                        }
                        Some(StreamCommand::ForceReconnect | StreamCommand::RefreshSubscriptions) => continue,
                    }
                }
                result = &mut connect => result,
            };

            let mut connection = match connection_result {
                Ok(connection) => connection,
                Err(error) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    if !emit(
                        &events,
                        StreamEvent::Disconnected {
                            connection: None,
                            reason: error.to_string(),
                        },
                    )
                    .await
                    {
                        return;
                    }
                    if !self
                        .wait_before_reconnect(
                            consecutive_failures,
                            Uuid::new_v4(),
                            &events,
                            &mut commands,
                        )
                        .await
                    {
                        return;
                    }
                    continue;
                }
            };

            generation = generation.saturating_add(1);
            let metadata = ConnectionMetadata {
                connection_id: connection.connection_id(),
                generation,
            };
            let desired = self.registry.snapshot().await;
            if !emit(
                &events,
                StreamEvent::Connected {
                    connection: metadata,
                    desired_subscriptions: desired.len(),
                },
            )
            .await
            {
                let _ = connection.close().await;
                return;
            }

            let mut send_failure = None;
            for subscription in &desired {
                if let Err(error) = connection.send_value(subscription.request.as_value()).await {
                    send_failure = Some(error);
                    break;
                }
            }
            if let Some(error) = send_failure {
                consecutive_failures = consecutive_failures.saturating_add(1);
                if !emit(
                    &events,
                    StreamEvent::Disconnected {
                        connection: Some(metadata),
                        reason: error.to_string(),
                    },
                )
                .await
                {
                    return;
                }
                if !self
                    .wait_before_reconnect(
                        consecutive_failures,
                        metadata.connection_id,
                        &events,
                        &mut commands,
                    )
                    .await
                {
                    return;
                }
                continue;
            }

            let mut acknowledgements = AcknowledgementTracker::from_subscriptions(&desired);
            let acknowledgement_timeout = tokio::time::sleep(self.acknowledgement_timeout);
            tokio::pin!(acknowledgement_timeout);
            if acknowledgements.is_complete() {
                consecutive_failures = 0;
                if !emit(
                    &events,
                    StreamEvent::SubscriptionsReady {
                        connection: metadata,
                    },
                )
                .await
                {
                    let _ = connection.close().await;
                    return;
                }
            }

            let exit = 'connection: loop {
                tokio::select! {
                    () = &mut acknowledgement_timeout, if !acknowledgements.is_complete() => {
                        break ConnectionExit::Transport(
                            WebSocketError::SubscriptionAcknowledgementTimeout(
                                self.acknowledgement_timeout,
                            ),
                        );
                    }
                    command = commands.recv() => {
                        match command {
                            Some(StreamCommand::Shutdown) | None => break ConnectionExit::Shutdown,
                            Some(StreamCommand::ForceReconnect | StreamCommand::RefreshSubscriptions) => {
                                break ConnectionExit::RequestedReconnect;
                            }
                        }
                    }
                    incoming = connection.receive_value() => {
                        match incoming {
                            Ok(message) => {
                                let was_complete = acknowledgements.is_complete();
                                let observed = match acknowledgements.observe_checked(&message) {
                                    Ok(observed) => observed,
                                    Err(error) => {
                                        break ConnectionExit::Transport(
                                            WebSocketError::SubscriptionAcknowledgement(
                                                error.to_string(),
                                            ),
                                        );
                                    }
                                };
                                for acknowledgement in observed {
                                    if !emit(
                                        &events,
                                        StreamEvent::SubscriptionAcknowledged {
                                            connection: metadata,
                                            acknowledgement,
                                            remaining: acknowledgements.remaining().len(),
                                        },
                                    )
                                    .await
                                    {
                                        break 'connection ConnectionExit::ReceiverDropped;
                                    }
                                }
                                if !was_complete && acknowledgements.is_complete() {
                                    consecutive_failures = 0;
                                    if !emit(
                                        &events,
                                        StreamEvent::SubscriptionsReady { connection: metadata },
                                    )
                                    .await
                                    {
                                        break 'connection ConnectionExit::ReceiverDropped;
                                    }
                                }
                                if !emit(
                                    &events,
                                    StreamEvent::Message {
                                        connection: metadata,
                                        message: ProviderMessage::new(message),
                                    },
                                )
                                .await
                                {
                                    break 'connection ConnectionExit::ReceiverDropped;
                                }
                            }
                            Err(error) => break ConnectionExit::Transport(error),
                        }
                    }
                }
            };

            match exit {
                ConnectionExit::Shutdown => {
                    let _ = connection.close().await;
                    let _ = emit(
                        &events,
                        StreamEvent::Stopped {
                            reason: StreamStopReason::Shutdown,
                        },
                    )
                    .await;
                    return;
                }
                ConnectionExit::RequestedReconnect => {
                    let _ = connection.close().await;
                    if !emit(
                        &events,
                        StreamEvent::Disconnected {
                            connection: Some(metadata),
                            reason: "reconnect requested".to_owned(),
                        },
                    )
                    .await
                    {
                        return;
                    }
                }
                ConnectionExit::Transport(error) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    if !emit(
                        &events,
                        StreamEvent::Disconnected {
                            connection: Some(metadata),
                            reason: error.to_string(),
                        },
                    )
                    .await
                    {
                        return;
                    }
                    if !self
                        .wait_before_reconnect(
                            consecutive_failures,
                            metadata.connection_id,
                            &events,
                            &mut commands,
                        )
                        .await
                    {
                        return;
                    }
                }
                ConnectionExit::ReceiverDropped => {
                    let _ = connection.close().await;
                    return;
                }
            }
        }
    }

    async fn wait_before_reconnect(
        &self,
        consecutive_failures: u32,
        connection_id: Uuid,
        events: &mpsc::Sender<StreamEvent>,
        commands: &mut mpsc::Receiver<StreamCommand>,
    ) -> bool {
        if consecutive_failures >= self.reconnect_policy.max_consecutive_failures() {
            let _ = emit(
                events,
                StreamEvent::Stopped {
                    reason: StreamStopReason::ReconnectExhausted,
                },
            )
            .await;
            return false;
        }
        let delay = self
            .reconnect_policy
            .delay_for(consecutive_failures, connection_id);
        if !emit(
            events,
            StreamEvent::ReconnectScheduled {
                consecutive_failure: consecutive_failures,
                delay,
            },
        )
        .await
        {
            return false;
        }
        tokio::select! {
            () = tokio::time::sleep(delay) => true,
            command = commands.recv() => {
                match command {
                    Some(StreamCommand::ForceReconnect | StreamCommand::RefreshSubscriptions) => true,
                    Some(StreamCommand::Shutdown) | None => {
                        let _ = emit(events, StreamEvent::Stopped { reason: StreamStopReason::Shutdown }).await;
                        false
                    }
                }
            }
        }
    }
}

impl fmt::Debug for StreamSupervisor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamSupervisor")
            .field("connector", &self.connector)
            .field("registry", &self.registry)
            .field("reconnect_policy", &self.reconnect_policy)
            .field("event_capacity", &self.event_capacity)
            .field("command_capacity", &self.command_capacity)
            .finish()
    }
}

enum ConnectionExit {
    Shutdown,
    RequestedReconnect,
    Transport(WebSocketError),
    ReceiverDropped,
}

async fn emit(events: &mpsc::Sender<StreamEvent>, event: StreamEvent) -> bool {
    events.send(event).await.is_ok()
}

#[derive(Clone, Debug)]
pub struct StreamControl {
    sender: mpsc::Sender<StreamCommand>,
}

impl StreamControl {
    pub async fn force_reconnect(&self) -> Result<(), WebSocketError> {
        self.send(StreamCommand::ForceReconnect).await
    }

    /// Reconnects so current desired-subscription registry is replayed atomically.
    pub async fn refresh_subscriptions(&self) -> Result<(), WebSocketError> {
        self.send(StreamCommand::RefreshSubscriptions).await
    }

    pub async fn shutdown(&self) -> Result<(), WebSocketError> {
        self.send(StreamCommand::Shutdown).await
    }

    async fn send(&self, command: StreamCommand) -> Result<(), WebSocketError> {
        self.sender
            .send(command)
            .await
            .map_err(|_| WebSocketError::SupervisorStopped)
    }
}

#[derive(Clone, Copy, Debug)]
enum StreamCommand {
    ForceReconnect,
    RefreshSubscriptions,
    Shutdown,
}

#[derive(Debug)]
pub struct StreamEvents {
    receiver: mpsc::Receiver<StreamEvent>,
    configured_capacity: usize,
}

impl StreamEvents {
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        self.receiver.recv().await
    }

    pub const fn bounded_capacity(&self) -> usize {
        self.configured_capacity
    }
}

#[derive(Debug)]
pub struct StreamHandle {
    control: StreamControl,
    events: StreamEvents,
    task: JoinHandle<()>,
}

impl StreamHandle {
    pub fn control(&self) -> StreamControl {
        self.control.clone()
    }

    pub async fn recv(&mut self) -> Option<StreamEvent> {
        self.events.recv().await
    }

    pub const fn bounded_event_capacity(&self) -> usize {
        self.events.bounded_capacity()
    }

    pub fn split(self) -> (StreamControl, StreamEvents) {
        (self.control, self.events)
    }

    pub async fn shutdown(self) -> Result<(), WebSocketError> {
        let Self {
            control,
            events,
            task,
        } = self;
        let command_result = control.shutdown().await;
        drop(events);
        drop(control);
        task.await
            .map_err(|error| WebSocketError::SupervisorTask(error.to_string()))?;
        command_result
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use serde_json::json;
    use tokio::sync::Mutex;

    fn id(value: &str) -> SubscriptionId {
        match SubscriptionId::new(value) {
            Ok(id) => id,
            Err(error) => panic!("unexpected subscription ID error: {error}"),
        }
    }

    fn ack(value: &str) -> AckKey {
        match AckKey::new(value) {
            Ok(key) => key,
            Err(error) => panic!("unexpected ACK key error: {error}"),
        }
    }

    fn subscription(name: &str, acknowledgement: &str) -> DesiredSubscription {
        DesiredSubscription::new(
            id(name),
            ProviderMessage::new(json!({"subscribe": name})),
            ack(acknowledgement),
        )
    }

    #[test]
    fn websocket_config_uses_native_roots_and_rejects_plaintext() {
        let endpoint = match Url::parse("ws://example.test/stream") {
            Ok(url) => url,
            Err(error) => panic!("unexpected URL error: {error}"),
        };
        assert!(matches!(
            WebSocketConfig::production().with_endpoint(endpoint),
            Err(WebSocketError::InvalidConfig(_))
        ));
        assert_eq!(
            WebSocketConfig::production().certificate_policy(),
            CertificatePolicy::NativeRoots
        );
    }

    #[tokio::test]
    async fn registry_is_bounded_and_rejects_ambiguous_ack_keys() {
        let registry = match SubscriptionRegistry::new(2) {
            Ok(registry) => registry,
            Err(error) => panic!("unexpected registry error: {error}"),
        };
        assert!(
            registry
                .upsert(subscription("trades", "trades_ack"))
                .await
                .is_ok()
        );
        assert!(matches!(
            registry.upsert(subscription("book", "trades_ack")).await,
            Err(SubscriptionRegistryError::DuplicateAcknowledgement(_))
        ));
        assert!(
            registry
                .upsert(subscription("book", "book_ack"))
                .await
                .is_ok()
        );
        assert!(matches!(
            registry.upsert(subscription("info", "info_ack")).await,
            Err(SubscriptionRegistryError::CapacityExceeded(2))
        ));
    }

    #[test]
    fn acknowledgements_are_order_independent_and_idempotent() {
        let subscriptions = vec![
            subscription("trades", "trades_ack"),
            subscription("book", "book_ack"),
            subscription("info", "info_ack"),
        ];
        let mut tracker = AcknowledgementTracker::from_subscriptions(&subscriptions);
        assert_eq!(
            tracker.observe(&json!({"info_ack": {}})),
            vec![ack("info_ack")]
        );
        assert_eq!(
            tracker.observe(&json!({"trades_ack": {}})),
            vec![ack("trades_ack")]
        );
        assert!(tracker.observe(&json!({"info_ack": {}})).is_empty());
        assert!(!tracker.is_complete());
        assert_eq!(
            tracker.observe(&json!({"book_ack": {}})),
            vec![ack("book_ack")]
        );
        assert!(tracker.is_complete());
        assert!(tracker.remaining().is_empty());
    }

    #[test]
    fn known_provider_ack_requires_success_status() {
        let subscriptions = vec![subscription("trades", "subscribe_trades_response")];
        let mut tracker = AcknowledgementTracker::from_subscriptions(&subscriptions);
        let rejected = tracker.observe_checked(&json!({
            "subscribe_trades_response": {
                "trade_subscriptions": [{
                    "subscription_status": "SUBSCRIPTION_STATUS_INSTRUMENT_NOT_FOUND"
                }]
            }
        }));
        assert!(matches!(
            rejected,
            Err(AcknowledgementError::Rejected { .. })
        ));
        assert!(!tracker.is_complete());

        let accepted = tracker.observe_checked(&json!({
            "subscribe_trades_response": {
                "trade_subscriptions": [{
                    "subscription_status": "SUBSCRIPTION_STATUS_SUCCESS"
                }]
            }
        }));
        assert!(
            matches!(accepted, Ok(ref keys) if keys == &vec![ack("subscribe_trades_response")])
        );
        assert!(tracker.is_complete());
    }

    #[test]
    fn provider_ack_must_match_expected_instrument_uid() {
        let subscriptions = vec![
            subscription("trades", "subscribe_trades_response")
                .with_expected_instrument_uid("expected-uid"),
        ];
        let mut tracker = AcknowledgementTracker::from_subscriptions(&subscriptions);
        let result = tracker.observe_checked(&json!({
            "subscribe_trades_response": {
                "trade_subscriptions": [{
                    "instrument_uid": "other-uid",
                    "subscription_status": "SUBSCRIPTION_STATUS_SUCCESS"
                }]
            }
        }));
        assert!(matches!(
            result,
            Err(AcknowledgementError::UnexpectedInstrument { .. })
        ));
        assert!(!tracker.is_complete());
    }

    #[derive(Clone, Debug)]
    struct FakeConnector {
        sessions: Arc<Mutex<VecDeque<VecDeque<FakeAction>>>>,
        sent: Arc<Mutex<Vec<Vec<Value>>>>,
    }

    impl FakeConnector {
        fn new(sessions: Vec<Vec<FakeAction>>) -> Self {
            Self {
                sessions: Arc::new(Mutex::new(
                    sessions
                        .into_iter()
                        .map(VecDeque::from)
                        .collect::<VecDeque<_>>(),
                )),
                sent: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[derive(Clone, Debug)]
    enum FakeAction {
        Message(Value),
        Wait,
    }

    #[derive(Debug)]
    struct FakeConnection {
        connection_id: Uuid,
        actions: VecDeque<FakeAction>,
        sent: Arc<Mutex<Vec<Vec<Value>>>>,
        session_index: usize,
    }

    #[async_trait]
    impl WebSocketConnector for FakeConnector {
        async fn connect(&self) -> Result<Box<dyn WebSocketConnection>, WebSocketError> {
            let mut sessions = self.sessions.lock().await;
            let actions = sessions.pop_front().ok_or_else(|| {
                WebSocketError::Connect("no scripted fake connection remains".to_owned())
            })?;
            let mut sent = self.sent.lock().await;
            let session_index = sent.len();
            sent.push(Vec::new());
            drop(sent);
            let numeric_id = u128::try_from(session_index).unwrap_or(u128::MAX) + 1;
            Ok(Box::new(FakeConnection {
                connection_id: Uuid::from_u128(numeric_id),
                actions,
                sent: self.sent.clone(),
                session_index,
            }))
        }
    }

    #[async_trait]
    impl WebSocketConnection for FakeConnection {
        fn connection_id(&self) -> Uuid {
            self.connection_id
        }

        async fn send_value(&mut self, payload: &Value) -> Result<(), WebSocketError> {
            let mut sent = self.sent.lock().await;
            let Some(session) = sent.get_mut(self.session_index) else {
                return Err(WebSocketError::Send("missing fake session".to_owned()));
            };
            session.push(payload.clone());
            Ok(())
        }

        async fn receive_value(&mut self) -> Result<Value, WebSocketError> {
            match self.actions.pop_front() {
                Some(FakeAction::Message(message)) => Ok(message),
                Some(FakeAction::Wait) | None => std::future::pending().await,
            }
        }

        async fn close(&mut self) -> Result<(), WebSocketError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn supervisor_reconnects_resubscribes_and_keeps_channel_bounded() {
        let registry = match SubscriptionRegistry::new(4) {
            Ok(registry) => registry,
            Err(error) => panic!("unexpected registry error: {error}"),
        };
        assert!(
            registry
                .upsert(subscription("trades", "trades_ack"))
                .await
                .is_ok()
        );
        assert!(
            registry
                .upsert(subscription("book", "book_ack"))
                .await
                .is_ok()
        );

        let connector = FakeConnector::new(vec![
            vec![
                FakeAction::Message(json!({"book_ack": {}})),
                FakeAction::Message(json!({"trades_ack": {}})),
                FakeAction::Wait,
            ],
            vec![
                FakeAction::Message(json!({"trades_ack": {}})),
                FakeAction::Message(json!({"book_ack": {}})),
                FakeAction::Message(json!({"trade": {"instrument_id": "uid"}})),
                FakeAction::Wait,
            ],
        ]);
        let sent = connector.sent.clone();
        let policy =
            match ReconnectPolicy::new(3, Duration::from_millis(1), Duration::from_millis(4), 0) {
                Ok(policy) => policy,
                Err(error) => panic!("unexpected reconnect policy error: {error}"),
            };
        let supervisor = match StreamSupervisor::new(connector, registry, policy, 8) {
            Ok(supervisor) => supervisor,
            Err(error) => panic!("unexpected supervisor error: {error}"),
        };
        let mut handle = match supervisor.start() {
            Ok(handle) => handle,
            Err(error) => panic!("unexpected supervisor start error: {error}"),
        };
        assert_eq!(handle.bounded_event_capacity(), 8);
        let control = handle.control();

        let first_ready = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match handle.recv().await {
                    Some(StreamEvent::SubscriptionsReady { connection })
                        if connection.generation == 1 =>
                    {
                        return;
                    }
                    Some(_) => {}
                    None => panic!("stream stopped before first ready"),
                }
            }
        })
        .await;
        assert!(first_ready.is_ok());
        assert!(control.force_reconnect().await.is_ok());

        let post_reconnect = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match handle.recv().await {
                    Some(StreamEvent::Message {
                        connection,
                        message,
                    }) if connection.generation == 2
                        && message.as_value().get("trade").is_some() =>
                    {
                        return;
                    }
                    Some(_) => {}
                    None => panic!("stream stopped before post-reconnect event"),
                }
            }
        })
        .await;
        assert!(post_reconnect.is_ok());
        drop(control);
        assert!(handle.shutdown().await.is_ok());

        let sent = sent.lock().await;
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].len(), 2);
        assert_eq!(sent[1].len(), 2);
        assert_eq!(sent[0], sent[1]);
    }
}
