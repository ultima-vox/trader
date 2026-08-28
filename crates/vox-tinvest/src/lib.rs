#![forbid(unsafe_code)]

pub mod qualification;
pub mod reference;

pub mod account;
pub mod account_qualification;
pub mod canonical;
pub mod connection_provider;
pub mod execution;
pub mod execution_dispatch;
pub mod execution_qualification;
pub mod execution_stream;
pub mod generated;
pub mod market_data;
pub mod market_stream;
pub mod operations;
pub mod operations_stream;
pub mod reports;
pub mod runtime_execution;

mod grpc;
mod rest;
mod retry;
mod secret;
mod websocket;

pub use grpc::{
    DEFAULT_GRPC_ENDPOINT, DEFAULT_SANDBOX_GRPC_ENDPOINT, GrpcCertificatePolicy, GrpcConfig,
    GrpcConfigError, GrpcError, GrpcErrorKind, GrpcMarketDataServerStream, GrpcMarketDataStream,
    GrpcProviderError, GrpcRateLimitMetadata, GrpcRequestMetadata, GrpcResponse,
    GrpcResponseMetadata, GrpcServerStream, GrpcStreamError, TInvestGrpcClient,
};
pub use rest::{
    DEFAULT_REST_BASE_URL, DEFAULT_SANDBOX_REST_BASE_URL, DispatchCertainty, ProviderError,
    ProviderResponse, RequestMetadata, ResponseMetadata, RestCertificatePolicy, RestConfig,
    RestConfigError, RestError, RestErrorKind, RestOperation, TInvestRestClient,
};
pub use retry::{
    NoopRetryObserver, RetryEvent, RetryObserver, RetryPolicy, RetryPolicyError, RetryReason,
};
pub use secret::{GrpcCredential, SecretToken, SecretTokenError};
pub use websocket::{
    AckKey, AcknowledgementError, AcknowledgementTracker, CertificatePolicy, ConnectionMetadata,
    DEFAULT_MARKET_DATA_STREAM_URL, DesiredSubscription, ProviderMessage, ReconnectPolicy,
    StreamControl, StreamEvent, StreamEvents, StreamHandle, StreamStopReason, StreamSupervisor,
    SubscriptionId, SubscriptionRegistry, SubscriptionRegistryError, TInvestWebSocket,
    WebSocketConfig, WebSocketConnection, WebSocketConnector, WebSocketError, WebSocketSession,
};
