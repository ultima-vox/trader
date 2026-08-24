#![forbid(unsafe_code)]

pub mod qualification;
pub mod reference;

mod rest;
mod retry;
mod secret;
mod websocket;

pub use rest::{
    DEFAULT_REST_BASE_URL, DEFAULT_SANDBOX_REST_BASE_URL, DispatchCertainty, ProviderError,
    ProviderResponse, RequestMetadata, ResponseMetadata, RestCertificatePolicy, RestConfig,
    RestConfigError, RestError, RestErrorKind, RestOperation, TInvestRestClient,
};
pub use retry::{
    NoopRetryObserver, RetryEvent, RetryObserver, RetryPolicy, RetryPolicyError, RetryReason,
};
pub use secret::{SecretToken, SecretTokenError};
pub use websocket::{
    AckKey, AcknowledgementError, AcknowledgementTracker, CertificatePolicy, ConnectionMetadata,
    DEFAULT_MARKET_DATA_STREAM_URL, DesiredSubscription, ProviderMessage, ReconnectPolicy,
    StreamControl, StreamEvent, StreamEvents, StreamHandle, StreamStopReason, StreamSupervisor,
    SubscriptionId, SubscriptionRegistry, SubscriptionRegistryError, TInvestWebSocket,
    WebSocketConfig, WebSocketConnection, WebSocketConnector, WebSocketError, WebSocketSession,
};
