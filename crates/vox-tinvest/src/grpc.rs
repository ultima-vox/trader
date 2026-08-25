use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::{AsciiMetadataValue, MetadataMap};
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
use tonic::{Code, Request, Response, Status};
use url::Url;
use uuid::Uuid;
use vox_domain::{Environment, MutationAuthorization};

use crate::generated::v1;
use crate::market_data::{MarketDataRequestError, validate_get_candles_request};
use crate::{
    GrpcCredential, NoopRetryObserver, RestOperation, RetryEvent, RetryObserver, RetryPolicy,
    RetryReason, SecretToken,
};

pub const DEFAULT_GRPC_ENDPOINT: &str = "https://invest-public-api.tbank.ru:443";
pub const DEFAULT_SANDBOX_GRPC_ENDPOINT: &str = "https://sandbox-invest-public-api.tbank.ru:443";

const DEFAULT_REQUEST_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

type GeneratedClient = v1::instruments_service_client::InstrumentsServiceClient<Channel>;
type MarketGeneratedClient = v1::market_data_service_client::MarketDataServiceClient<Channel>;
type MarketStreamGeneratedClient =
    v1::market_data_stream_service_client::MarketDataStreamServiceClient<Channel>;
type UsersGeneratedClient = v1::users_service_client::UsersServiceClient<Channel>;
type OperationsGeneratedClient = v1::operations_service_client::OperationsServiceClient<Channel>;
type OperationsStreamGeneratedClient =
    v1::operations_stream_service_client::OperationsStreamServiceClient<Channel>;
type SandboxGeneratedClient = v1::sandbox_service_client::SandboxServiceClient<Channel>;
type GrpcFuture<'a, T> = Pin<Box<dyn Future<Output = Result<Response<T>, Status>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrpcCertificatePolicy {
    /// Roots loaded from host operating-system trust store. Verification stays enabled.
    NativeRoots,
}

#[derive(Clone, Debug)]
pub struct GrpcConfig {
    endpoint: Url,
    environment: Environment,
    connect_timeout: Duration,
    request_timeout: Duration,
    max_request_bytes: usize,
    max_response_bytes: usize,
    retry_policy: RetryPolicy,
    certificate_policy: GrpcCertificatePolicy,
}

impl GrpcConfig {
    #[must_use]
    pub fn production() -> Self {
        Self::built_in(DEFAULT_GRPC_ENDPOINT, Environment::Live)
    }

    #[must_use]
    pub fn sandbox() -> Self {
        Self::built_in(DEFAULT_SANDBOX_GRPC_ENDPOINT, Environment::Sandbox)
    }

    fn built_in(endpoint: &str, environment: Environment) -> Self {
        let endpoint = Url::parse(endpoint)
            .unwrap_or_else(|error| unreachable!("built-in T-Invest URL is invalid: {error}"));
        Self {
            endpoint,
            environment,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(20),
            max_request_bytes: DEFAULT_REQUEST_LIMIT,
            max_response_bytes: DEFAULT_RESPONSE_LIMIT,
            retry_policy: RetryPolicy::default(),
            certificate_policy: GrpcCertificatePolicy::NativeRoots,
        }
    }

    pub fn with_endpoint(
        mut self,
        endpoint: Url,
        environment: Environment,
    ) -> Result<Self, GrpcConfigError> {
        validate_endpoint(&endpoint)?;
        self.endpoint = endpoint;
        self.environment = environment;
        Ok(self)
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, GrpcConfigError> {
        if connect_timeout.is_zero() || request_timeout.is_zero() {
            return Err(GrpcConfigError::ZeroTimeout);
        }
        self.connect_timeout = connect_timeout;
        self.request_timeout = request_timeout;
        Ok(self)
    }

    pub fn with_message_limits(
        mut self,
        max_request_bytes: usize,
        max_response_bytes: usize,
    ) -> Result<Self, GrpcConfigError> {
        if max_request_bytes == 0 || max_response_bytes == 0 {
            return Err(GrpcConfigError::ZeroMessageLimit);
        }
        self.max_request_bytes = max_request_bytes;
        self.max_response_bytes = max_response_bytes;
        Ok(self)
    }

    #[must_use]
    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    #[must_use]
    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    #[must_use]
    pub const fn environment(&self) -> Environment {
        self.environment
    }

    #[must_use]
    pub const fn certificate_policy(&self) -> GrpcCertificatePolicy {
        self.certificate_policy
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<(), GrpcConfigError> {
    if endpoint.scheme() != "https" {
        return Err(GrpcConfigError::InsecureUrl);
    }
    if endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || (endpoint.path() != "/" && !endpoint.path().is_empty())
    {
        return Err(GrpcConfigError::InvalidUrl);
    }
    Ok(())
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GrpcConfigError {
    #[error("T-Invest gRPC endpoint must use HTTPS")]
    InsecureUrl,
    #[error("T-Invest gRPC endpoint must contain only an origin")]
    InvalidUrl,
    #[error("gRPC connect and request timeouts must be positive")]
    ZeroTimeout,
    #[error("gRPC request and response message limits must be positive")]
    ZeroMessageLimit,
    #[error("failed to build verified native-root gRPC channel: {0}")]
    Channel(String),
    #[error(
        "gRPC credential environment {credential:?} does not match endpoint environment {endpoint:?}"
    )]
    CredentialEnvironmentMismatch {
        credential: Environment,
        endpoint: Environment,
    },
}

#[derive(Clone)]
pub struct TInvestGrpcClient {
    token: SecretToken,
    config: GrpcConfig,
    channel: Channel,
    retry_observer: Arc<dyn RetryObserver>,
}

impl TInvestGrpcClient {
    pub fn new(credential: GrpcCredential, config: GrpcConfig) -> Result<Self, GrpcConfigError> {
        validate_endpoint(&config.endpoint)?;
        if credential.environment() != config.environment {
            return Err(GrpcConfigError::CredentialEnvironmentMismatch {
                credential: credential.environment(),
                endpoint: config.environment,
            });
        }
        let endpoint =
            Endpoint::from_shared(config.endpoint.as_str().trim_end_matches('/').to_owned())
                .map_err(|error| GrpcConfigError::Channel(error.to_string()))?
                .connect_timeout(config.connect_timeout)
                .user_agent("vox-trader/0.1 vox-tinvest")
                .map_err(|error| GrpcConfigError::Channel(error.to_string()))?
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(|error| GrpcConfigError::Channel(error.to_string()))?;
        Ok(Self {
            token: credential.token().clone(),
            config,
            channel: endpoint.connect_lazy(),
            retry_observer: Arc::new(NoopRetryObserver),
        })
    }

    pub fn production(credential: GrpcCredential) -> Result<Self, GrpcConfigError> {
        Self::new(credential, GrpcConfig::production())
    }

    pub fn sandbox(credential: GrpcCredential) -> Result<Self, GrpcConfigError> {
        Self::new(credential, GrpcConfig::sandbox())
    }

    #[must_use]
    pub fn with_retry_observer<O>(mut self, observer: O) -> Self
    where
        O: RetryObserver + 'static,
    {
        self.retry_observer = Arc::new(observer);
        self
    }

    fn generated_client(&self) -> GeneratedClient {
        GeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn market_generated_client(&self) -> MarketGeneratedClient {
        MarketGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn market_stream_generated_client(&self) -> MarketStreamGeneratedClient {
        MarketStreamGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn users_generated_client(&self) -> UsersGeneratedClient {
        UsersGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn operations_generated_client(&self) -> OperationsGeneratedClient {
        OperationsGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn operations_stream_generated_client(&self) -> OperationsStreamGeneratedClient {
        OperationsStreamGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn sandbox_generated_client(&self) -> SandboxGeneratedClient {
        SandboxGeneratedClient::new(self.channel.clone())
            .max_encoding_message_size(self.config.max_request_bytes)
            .max_decoding_message_size(self.config.max_response_bytes)
    }

    fn authenticated_request<T>(
        &self,
        body: T,
        request_id: Uuid,
    ) -> Result<Request<T>, GrpcErrorKind> {
        let bearer = format!("Bearer {}", self.token.expose_secret());
        let authorization = AsciiMetadataValue::try_from(bearer)
            .map_err(|_| GrpcErrorKind::InvalidAuthorizationMetadata)?;
        let request_id_value = AsciiMetadataValue::try_from(request_id.to_string())
            .map_err(|_| GrpcErrorKind::InvalidRequestMetadata)?;
        let mut request = Request::new(body);
        request
            .metadata_mut()
            .insert("authorization", authorization);
        request
            .metadata_mut()
            .insert("x-request-id", request_id_value);
        request.metadata_mut().insert(
            "x-app-name",
            AsciiMetadataValue::from_static("ultima-vox.trader"),
        );
        Ok(request)
    }

    fn unary_request<T>(&self, body: T, request_id: Uuid) -> Result<Request<T>, GrpcErrorKind> {
        let mut request = self.authenticated_request(body, request_id)?;
        request.set_timeout(self.config.request_timeout);
        Ok(request)
    }

    fn stream_request<T>(&self, body: T, request_id: Uuid) -> Result<Request<T>, GrpcErrorKind> {
        self.authenticated_request(body, request_id)
    }

    async fn safe_read<Req, Resp, Dispatch>(
        &self,
        method: &'static str,
        body: Req,
        mut dispatch: Dispatch,
    ) -> Result<GrpcResponse<Resp>, GrpcError>
    where
        Req: Clone + Send + Sync + 'static,
        Resp: Send + 'static,
        Dispatch: for<'a> FnMut(&'a mut GeneratedClient, Request<Req>) -> GrpcFuture<'a, Resp>,
    {
        let request_id = Uuid::new_v4();
        for attempt in 1..=self.config.retry_policy.max_attempts() {
            let metadata = GrpcRequestMetadata {
                request_id,
                method,
                attempt,
                mutation: false,
            };
            let request = self
                .unary_request(body.clone(), request_id)
                .map_err(|kind| GrpcError { metadata, kind })?;
            let mut client = self.generated_client();
            match dispatch(&mut client, request).await {
                Ok(response) => return Ok(GrpcResponse::from_tonic(request_id, attempt, response)),
                Err(status)
                    if attempt < self.config.retry_policy.max_attempts()
                        && retryable_status(status.code()) =>
                {
                    let delay = self.config.retry_policy.delay_for(attempt, request_id);
                    self.retry_observer.on_retry(&RetryEvent {
                        operation: RestOperation::SafeRead,
                        request_id,
                        failed_attempt: attempt,
                        next_attempt: attempt + 1,
                        delay,
                        server_retry_after: None,
                        server_retry_after_raw: None,
                        reason: match status.code() {
                            Code::DeadlineExceeded => RetryReason::Timeout,
                            _ => RetryReason::Transport,
                        },
                    });
                    tokio::time::sleep(delay).await;
                }
                Err(status) => {
                    return Err(GrpcError {
                        metadata,
                        kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
                    });
                }
            }
        }
        unreachable!("retry policy always has at least one attempt")
    }

    async fn mutation<Req, Resp, Dispatch>(
        &self,
        authorization: MutationAuthorization,
        method: &'static str,
        body: Req,
        dispatch: Dispatch,
    ) -> Result<GrpcResponse<Resp>, GrpcError>
    where
        Req: Send + Sync + 'static,
        Resp: Send + 'static,
        Dispatch: for<'a> FnOnce(&'a mut GeneratedClient, Request<Req>) -> GrpcFuture<'a, Resp>,
    {
        let request_id = Uuid::new_v4();
        let metadata = GrpcRequestMetadata {
            request_id,
            method,
            attempt: 1,
            mutation: true,
        };
        if authorization.environment() != self.config.environment
            || self.config.environment == Environment::Paper
        {
            return Err(GrpcError {
                metadata,
                kind: GrpcErrorKind::MutationEnvironmentMismatch {
                    authorized: authorization.environment(),
                    configured: self.config.environment,
                },
            });
        }
        let request = self
            .unary_request(body, request_id)
            .map_err(|kind| GrpcError { metadata, kind })?;
        let mut client = self.generated_client();
        dispatch(&mut client, request)
            .await
            .map(|response| GrpcResponse::from_tonic(request_id, 1, response))
            .map_err(|status| GrpcError {
                metadata,
                kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
            })
    }

    async fn safe_market_read<Req, Resp, Dispatch>(
        &self,
        method: &'static str,
        body: Req,
        mut dispatch: Dispatch,
    ) -> Result<GrpcResponse<Resp>, GrpcError>
    where
        Req: Clone + Send + Sync + 'static,
        Resp: Send + 'static,
        Dispatch:
            for<'a> FnMut(&'a mut MarketGeneratedClient, Request<Req>) -> GrpcFuture<'a, Resp>,
    {
        let request_id = Uuid::new_v4();
        for attempt in 1..=self.config.retry_policy.max_attempts() {
            let metadata = GrpcRequestMetadata {
                request_id,
                method,
                attempt,
                mutation: false,
            };
            let request = self
                .unary_request(body.clone(), request_id)
                .map_err(|kind| GrpcError { metadata, kind })?;
            let mut client = self.market_generated_client();
            match dispatch(&mut client, request).await {
                Ok(response) => return Ok(GrpcResponse::from_tonic(request_id, attempt, response)),
                Err(status)
                    if attempt < self.config.retry_policy.max_attempts()
                        && retryable_status(status.code()) =>
                {
                    let delay = self.config.retry_policy.delay_for(attempt, request_id);
                    self.retry_observer.on_retry(&RetryEvent {
                        operation: RestOperation::SafeRead,
                        request_id,
                        failed_attempt: attempt,
                        next_attempt: attempt + 1,
                        delay,
                        server_retry_after: None,
                        server_retry_after_raw: None,
                        reason: match status.code() {
                            Code::DeadlineExceeded => RetryReason::Timeout,
                            _ => RetryReason::Transport,
                        },
                    });
                    tokio::time::sleep(delay).await;
                }
                Err(status) => {
                    return Err(GrpcError {
                        metadata,
                        kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
                    });
                }
            }
        }
        unreachable!("retry policy always has at least one attempt")
    }

    async fn service_read<Req, Resp, Client, Build, Dispatch>(
        &self,
        method: &'static str,
        body: Req,
        mut build: Build,
        mut dispatch: Dispatch,
        retry: bool,
    ) -> Result<GrpcResponse<Resp>, GrpcError>
    where
        Req: Clone + Send + Sync + 'static,
        Resp: Send + 'static,
        Client: Send,
        Build: FnMut() -> Client,
        Dispatch: for<'a> FnMut(&'a mut Client, Request<Req>) -> GrpcFuture<'a, Resp>,
    {
        let request_id = Uuid::new_v4();
        let attempts = if retry {
            self.config.retry_policy.max_attempts()
        } else {
            1
        };
        for attempt in 1..=attempts {
            let metadata = GrpcRequestMetadata {
                request_id,
                method,
                attempt,
                mutation: false,
            };
            let request = self
                .unary_request(body.clone(), request_id)
                .map_err(|kind| GrpcError { metadata, kind })?;
            let mut client = build();
            match dispatch(&mut client, request).await {
                Ok(response) => return Ok(GrpcResponse::from_tonic(request_id, attempt, response)),
                Err(status) if attempt < attempts && retryable_status(status.code()) => {
                    let delay = self.config.retry_policy.delay_for(attempt, request_id);
                    self.retry_observer.on_retry(&RetryEvent {
                        operation: RestOperation::SafeRead,
                        request_id,
                        failed_attempt: attempt,
                        next_attempt: attempt + 1,
                        delay,
                        server_retry_after: None,
                        server_retry_after_raw: None,
                        reason: match status.code() {
                            Code::DeadlineExceeded => RetryReason::Timeout,
                            _ => RetryReason::Transport,
                        },
                    });
                    tokio::time::sleep(delay).await;
                }
                Err(status) => {
                    return Err(GrpcError {
                        metadata,
                        kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
                    });
                }
            }
        }
        unreachable!("read always has at least one attempt")
    }

    /// Opens verified, authenticated generated bidirectional market-data stream.
    pub async fn open_market_data_stream(
        &self,
        outbound_capacity: usize,
        initial_requests: Vec<v1::MarketDataRequest>,
    ) -> Result<GrpcMarketDataStream, GrpcError> {
        let request_id = Uuid::new_v4();
        let metadata = GrpcRequestMetadata {
            request_id,
            method: "MarketDataStream",
            attempt: 1,
            mutation: false,
        };
        if outbound_capacity == 0 {
            return Err(GrpcError {
                metadata,
                kind: GrpcErrorKind::InvalidStreamCapacity,
            });
        }
        if initial_requests.is_empty() || initial_requests.len() > outbound_capacity {
            return Err(GrpcError {
                metadata,
                kind: GrpcErrorKind::InvalidInitialStreamRequests,
            });
        }
        let (outbound, receiver) = mpsc::channel(outbound_capacity);
        for initial in initial_requests {
            outbound.try_send(initial).map_err(|_| GrpcError {
                metadata,
                kind: GrpcErrorKind::InvalidInitialStreamRequests,
            })?;
        }
        let request = self
            .stream_request(ReceiverStream::new(receiver), request_id)
            .map_err(|kind| GrpcError { metadata, kind })?;
        let mut client = self.market_stream_generated_client();
        let response = client
            .market_data_stream(request)
            .await
            .map_err(|status| GrpcError {
                metadata,
                kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
            })?;
        let tracking_id = metadata_text(response.metadata(), "x-tracking-id");
        Ok(GrpcMarketDataStream {
            outbound,
            inbound: response.into_inner(),
            metadata: GrpcResponseMetadata {
                request_id,
                tracking_id,
                attempt: 1,
            },
        })
    }

    /// Opens generated server-side stream. Bidirectional stream remains production default because
    /// only it supports runtime subscription changes and `GetMySubscriptions`.
    pub async fn open_market_data_server_stream(
        &self,
        body: v1::MarketDataServerSideStreamRequest,
    ) -> Result<GrpcMarketDataServerStream, GrpcError> {
        let request_id = Uuid::new_v4();
        let metadata = GrpcRequestMetadata {
            request_id,
            method: "MarketDataServerSideStream",
            attempt: 1,
            mutation: false,
        };
        let request = self
            .stream_request(body, request_id)
            .map_err(|kind| GrpcError { metadata, kind })?;
        let mut client = self.market_stream_generated_client();
        let response = client
            .market_data_server_side_stream(request)
            .await
            .map_err(|status| GrpcError {
                metadata,
                kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
            })?;
        let tracking_id = metadata_text(response.metadata(), "x-tracking-id");
        Ok(GrpcMarketDataServerStream {
            inbound: response.into_inner(),
            metadata: GrpcResponseMetadata {
                request_id,
                tracking_id,
                attempt: 1,
            },
        })
    }
}

macro_rules! market_safe_reads {
    ($(($name:ident, $provider_name:literal, $request:ty, $response:ty)),+ $(,)?) => {
        impl TInvestGrpcClient {
            $(
                pub async fn $name(&self, request: $request) -> Result<GrpcResponse<$response>, GrpcError> {
                    self.safe_market_read($provider_name, request, |client, request| {
                        Box::pin(client.$name(request))
                    }).await
                }
            )+
        }
    };
}

market_safe_reads!(
    (
        get_last_prices,
        "GetLastPrices",
        v1::GetLastPricesRequest,
        v1::GetLastPricesResponse
    ),
    (
        get_order_book,
        "GetOrderBook",
        v1::GetOrderBookRequest,
        v1::GetOrderBookResponse
    ),
    (
        get_trading_status,
        "GetTradingStatus",
        v1::GetTradingStatusRequest,
        v1::GetTradingStatusResponse
    ),
    (
        get_trading_statuses,
        "GetTradingStatuses",
        v1::GetTradingStatusesRequest,
        v1::GetTradingStatusesResponse
    ),
    (
        get_last_trades,
        "GetLastTrades",
        v1::GetLastTradesRequest,
        v1::GetLastTradesResponse
    ),
    (
        get_close_prices,
        "GetClosePrices",
        v1::GetClosePricesRequest,
        v1::GetClosePricesResponse
    ),
    (
        get_tech_analysis,
        "GetTechAnalysis",
        v1::GetTechAnalysisRequest,
        v1::GetTechAnalysisResponse
    ),
    (
        get_market_values,
        "GetMarketValues",
        v1::GetMarketValuesRequest,
        v1::GetMarketValuesResponse
    ),
);

impl TInvestGrpcClient {
    pub async fn get_candles(
        &self,
        request: v1::GetCandlesRequest,
    ) -> Result<GrpcResponse<v1::GetCandlesResponse>, GrpcError> {
        if let Err(error) = validate_get_candles_request(&request) {
            return Err(GrpcError {
                metadata: GrpcRequestMetadata {
                    request_id: Uuid::new_v4(),
                    method: "GetCandles",
                    attempt: 1,
                    mutation: false,
                },
                kind: GrpcErrorKind::MarketDataRequest(error),
            });
        }
        self.safe_market_read("GetCandles", request, |client, request| {
            Box::pin(client.get_candles(request))
        })
        .await
    }
}

macro_rules! service_safe_reads {
    ($builder:ident; $(($name:ident, $provider_name:literal, $request:ty, $response:ty)),+ $(,)?) => {
        impl TInvestGrpcClient {
            $(
                #[allow(deprecated)]
                pub async fn $name(&self, request: $request) -> Result<GrpcResponse<$response>, GrpcError> {
                    self.service_read(
                        $provider_name,
                        request,
                        || self.$builder(),
                        |client, request| Box::pin(client.$name(request)),
                        true,
                    ).await
                }
            )+
        }
    };
}

service_safe_reads!(users_generated_client;
    (get_accounts, "GetAccounts", v1::GetAccountsRequest, v1::GetAccountsResponse),
    (get_margin_attributes, "GetMarginAttributes", v1::GetMarginAttributesRequest, v1::GetMarginAttributesResponse),
    (get_user_tariff, "GetUserTariff", v1::GetUserTariffRequest, v1::GetUserTariffResponse),
    (get_info, "GetInfo", v1::GetInfoRequest, v1::GetInfoResponse),
    (get_bank_accounts, "GetBankAccounts", v1::GetBankAccountsRequest, v1::GetBankAccountsResponse),
    (get_account_values, "GetAccountValues", v1::GetAccountValuesRequest, v1::GetAccountValuesResponse),
);

service_safe_reads!(operations_generated_client;
    (get_operations, "GetOperations", v1::OperationsRequest, v1::OperationsResponse),
    (get_portfolio, "GetPortfolio", v1::PortfolioRequest, v1::PortfolioResponse),
    (get_positions, "GetPositions", v1::PositionsRequest, v1::PositionsResponse),
    (get_withdraw_limits, "GetWithdrawLimits", v1::WithdrawLimitsRequest, v1::WithdrawLimitsResponse),
    (get_operations_by_cursor, "GetOperationsByCursor", v1::GetOperationsByCursorRequest, v1::GetOperationsByCursorResponse),
);

service_safe_reads!(sandbox_generated_client;
    (get_sandbox_accounts, "GetSandboxAccounts", v1::GetAccountsRequest, v1::GetAccountsResponse),
    (get_sandbox_portfolio, "GetSandboxPortfolio", v1::PortfolioRequest, v1::PortfolioResponse),
    (get_sandbox_positions, "GetSandboxPositions", v1::PositionsRequest, v1::PositionsResponse),
    (get_sandbox_withdraw_limits, "GetSandboxWithdrawLimits", v1::WithdrawLimitsRequest, v1::WithdrawLimitsResponse),
    (get_sandbox_operations, "GetSandboxOperations", v1::OperationsRequest, v1::OperationsResponse),
    (get_sandbox_operations_by_cursor, "GetSandboxOperationsByCursor", v1::GetOperationsByCursorRequest, v1::GetOperationsByCursorResponse),
);

impl TInvestGrpcClient {
    pub async fn get_broker_report(
        &self,
        request: v1::BrokerReportRequest,
    ) -> Result<GrpcResponse<v1::BrokerReportResponse>, GrpcError> {
        let retry = matches!(
            request.payload,
            Some(v1::broker_report_request::Payload::GetBrokerReportRequest(
                _
            ))
        );
        self.service_read(
            "GetBrokerReport",
            request,
            || self.operations_generated_client(),
            |client, request| Box::pin(client.get_broker_report(request)),
            retry,
        )
        .await
    }

    pub async fn get_dividends_foreign_issuer(
        &self,
        request: v1::GetDividendsForeignIssuerRequest,
    ) -> Result<GrpcResponse<v1::GetDividendsForeignIssuerResponse>, GrpcError> {
        let retry = matches!(
            request.payload,
            Some(v1::get_dividends_foreign_issuer_request::Payload::GetDivForeignIssuerReport(_))
        );
        self.service_read(
            "GetDividendsForeignIssuer",
            request,
            || self.operations_generated_client(),
            |client, request| Box::pin(client.get_dividends_foreign_issuer(request)),
            retry,
        )
        .await
    }

    async fn open_operations_server_stream<Req, Resp, Dispatch>(
        &self,
        method: &'static str,
        body: Req,
        mut dispatch: Dispatch,
    ) -> Result<GrpcServerStream<Resp>, GrpcError>
    where
        Req: Send + Sync + 'static,
        Resp: Send + 'static,
        Dispatch: for<'a> FnMut(
            &'a mut OperationsStreamGeneratedClient,
            Request<Req>,
        ) -> GrpcFuture<'a, tonic::codec::Streaming<Resp>>,
    {
        let request_id = Uuid::new_v4();
        let metadata = GrpcRequestMetadata {
            request_id,
            method,
            attempt: 1,
            mutation: false,
        };
        let request = self
            .stream_request(body, request_id)
            .map_err(|kind| GrpcError { metadata, kind })?;
        let mut client = self.operations_stream_generated_client();
        let response = dispatch(&mut client, request)
            .await
            .map_err(|status| GrpcError {
                metadata,
                kind: GrpcErrorKind::Provider(GrpcProviderError::from_status(status)),
            })?;
        let tracking_id = metadata_text(response.metadata(), "x-tracking-id");
        Ok(GrpcServerStream {
            inbound: response.into_inner(),
            metadata: GrpcResponseMetadata {
                request_id,
                tracking_id,
                attempt: 1,
            },
        })
    }

    pub async fn open_portfolio_stream(
        &self,
        request: v1::PortfolioStreamRequest,
    ) -> Result<GrpcServerStream<v1::PortfolioStreamResponse>, GrpcError> {
        self.open_operations_server_stream("PortfolioStream", request, |client, request| {
            Box::pin(client.portfolio_stream(request))
        })
        .await
    }

    pub async fn open_positions_stream(
        &self,
        request: v1::PositionsStreamRequest,
    ) -> Result<GrpcServerStream<v1::PositionsStreamResponse>, GrpcError> {
        self.open_operations_server_stream("PositionsStream", request, |client, request| {
            Box::pin(client.positions_stream(request))
        })
        .await
    }

    pub async fn open_operations_stream(
        &self,
        request: v1::OperationsStreamRequest,
    ) -> Result<GrpcServerStream<v1::OperationsStreamResponse>, GrpcError> {
        self.open_operations_server_stream("OperationsStream", request, |client, request| {
            Box::pin(client.operations_stream(request))
        })
        .await
    }
}

fn retryable_status(code: Code) -> bool {
    matches!(
        code,
        Code::Unavailable | Code::ResourceExhausted | Code::DeadlineExceeded
    )
}

macro_rules! safe_reads {
    ($(($name:ident, $provider_name:literal, $request:ty, $response:ty)),+ $(,)?) => {
        impl TInvestGrpcClient {
            $(
                #[allow(deprecated)]
                pub async fn $name(&self, request: $request) -> Result<GrpcResponse<$response>, GrpcError> {
                    self.safe_read($provider_name, request, |client, request| {
                        Box::pin(client.$name(request))
                    }).await
                }
            )+
        }
    };
}

safe_reads!(
    (
        trading_schedules,
        "TradingSchedules",
        v1::TradingSchedulesRequest,
        v1::TradingSchedulesResponse
    ),
    (bond_by, "BondBy", v1::InstrumentRequest, v1::BondResponse),
    (bonds, "Bonds", v1::InstrumentsRequest, v1::BondsResponse),
    (
        get_bond_coupons,
        "GetBondCoupons",
        v1::GetBondCouponsRequest,
        v1::GetBondCouponsResponse
    ),
    (
        get_bond_events,
        "GetBondEvents",
        v1::GetBondEventsRequest,
        v1::GetBondEventsResponse
    ),
    (
        currency_by,
        "CurrencyBy",
        v1::InstrumentRequest,
        v1::CurrencyResponse
    ),
    (
        currencies,
        "Currencies",
        v1::InstrumentsRequest,
        v1::CurrenciesResponse
    ),
    (etf_by, "EtfBy", v1::InstrumentRequest, v1::EtfResponse),
    (etfs, "Etfs", v1::InstrumentsRequest, v1::EtfsResponse),
    (
        future_by,
        "FutureBy",
        v1::InstrumentRequest,
        v1::FutureResponse
    ),
    (
        futures,
        "Futures",
        v1::InstrumentsRequest,
        v1::FuturesResponse
    ),
    (
        option_by,
        "OptionBy",
        v1::InstrumentRequest,
        v1::OptionResponse
    ),
    (
        options,
        "Options",
        v1::InstrumentsRequest,
        v1::OptionsResponse
    ),
    (
        options_by,
        "OptionsBy",
        v1::FilterOptionsRequest,
        v1::OptionsResponse
    ),
    (
        share_by,
        "ShareBy",
        v1::InstrumentRequest,
        v1::ShareResponse
    ),
    (shares, "Shares", v1::InstrumentsRequest, v1::SharesResponse),
    (dfa_by, "DfaBy", v1::InstrumentRequest, v1::DfaResponse),
    (dfas, "Dfas", v1::DfasRequest, v1::DfasResponse),
    (
        indicatives,
        "Indicatives",
        v1::IndicativesRequest,
        v1::IndicativesResponse
    ),
    (
        get_accrued_interests,
        "GetAccruedInterests",
        v1::GetAccruedInterestsRequest,
        v1::GetAccruedInterestsResponse
    ),
    (
        get_futures_margin,
        "GetFuturesMargin",
        v1::GetFuturesMarginRequest,
        v1::GetFuturesMarginResponse
    ),
    (
        get_instrument_by,
        "GetInstrumentBy",
        v1::InstrumentRequest,
        v1::InstrumentResponse
    ),
    (
        get_dividends,
        "GetDividends",
        v1::GetDividendsRequest,
        v1::GetDividendsResponse
    ),
    (
        get_asset_by,
        "GetAssetBy",
        v1::AssetRequest,
        v1::AssetResponse
    ),
    (
        get_assets,
        "GetAssets",
        v1::AssetsRequest,
        v1::AssetsResponse
    ),
    (
        get_favorites,
        "GetFavorites",
        v1::GetFavoritesRequest,
        v1::GetFavoritesResponse
    ),
    (
        get_favorite_groups,
        "GetFavoriteGroups",
        v1::GetFavoriteGroupsRequest,
        v1::GetFavoriteGroupsResponse
    ),
    (
        get_countries,
        "GetCountries",
        v1::GetCountriesRequest,
        v1::GetCountriesResponse
    ),
    (
        find_instrument,
        "FindInstrument",
        v1::FindInstrumentRequest,
        v1::FindInstrumentResponse
    ),
    (
        get_brands,
        "GetBrands",
        v1::GetBrandsRequest,
        v1::GetBrandsResponse
    ),
    (get_brand_by, "GetBrandBy", v1::GetBrandRequest, v1::Brand),
    (
        get_asset_fundamentals,
        "GetAssetFundamentals",
        v1::GetAssetFundamentalsRequest,
        v1::GetAssetFundamentalsResponse
    ),
    (
        get_asset_reports,
        "GetAssetReports",
        v1::GetAssetReportsRequest,
        v1::GetAssetReportsResponse
    ),
    (
        get_consensus_forecasts,
        "GetConsensusForecasts",
        v1::GetConsensusForecastsRequest,
        v1::GetConsensusForecastsResponse
    ),
    (
        get_forecast_by,
        "GetForecastBy",
        v1::GetForecastRequest,
        v1::GetForecastResponse
    ),
    (
        get_risk_rates,
        "GetRiskRates",
        v1::RiskRatesRequest,
        v1::RiskRatesResponse
    ),
    (
        get_insider_deals,
        "GetInsiderDeals",
        v1::GetInsiderDealsRequest,
        v1::GetInsiderDealsResponse
    ),
    (
        structured_note_by,
        "StructuredNoteBy",
        v1::InstrumentRequest,
        v1::StructuredNoteResponse
    ),
    (
        structured_notes,
        "StructuredNotes",
        v1::InstrumentsRequest,
        v1::StructuredNotesResponse
    ),
    (news, "News", v1::NewsRequest, v1::NewsResponse),
);

macro_rules! mutations {
    ($(($name:ident, $provider_name:literal, $request:ty, $response:ty)),+ $(,)?) => {
        impl TInvestGrpcClient {
            $(
                pub async fn $name(
                    &self,
                    authorization: MutationAuthorization,
                    request: $request,
                ) -> Result<GrpcResponse<$response>, GrpcError> {
                    self.mutation(authorization, $provider_name, request, |client, request| {
                        Box::pin(client.$name(request))
                    }).await
                }
            )+
        }
    };
}

mutations!(
    (
        edit_favorites,
        "EditFavorites",
        v1::EditFavoritesRequest,
        v1::EditFavoritesResponse
    ),
    (
        create_favorite_group,
        "CreateFavoriteGroup",
        v1::CreateFavoriteGroupRequest,
        v1::CreateFavoriteGroupResponse
    ),
    (
        delete_favorite_group,
        "DeleteFavoriteGroup",
        v1::DeleteFavoriteGroupRequest,
        v1::DeleteFavoriteGroupResponse
    ),
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrpcRequestMetadata {
    pub request_id: Uuid,
    pub method: &'static str,
    pub attempt: u32,
    pub mutation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrpcResponseMetadata {
    pub request_id: Uuid,
    pub tracking_id: Option<String>,
    pub attempt: u32,
}

#[derive(Clone, Debug)]
pub struct GrpcResponse<T> {
    pub body: T,
    pub metadata: GrpcResponseMetadata,
}

/// Live generated gRPC stream with bounded outbound backpressure.
pub struct GrpcMarketDataStream {
    outbound: mpsc::Sender<v1::MarketDataRequest>,
    inbound: tonic::Streaming<v1::MarketDataResponse>,
    pub metadata: GrpcResponseMetadata,
}

pub struct GrpcMarketDataServerStream {
    inbound: tonic::Streaming<v1::MarketDataResponse>,
    pub metadata: GrpcResponseMetadata,
}

pub struct GrpcServerStream<T> {
    inbound: tonic::Streaming<T>,
    pub metadata: GrpcResponseMetadata,
}

impl<T> GrpcServerStream<T> {
    pub async fn message(&mut self) -> Result<Option<T>, GrpcStreamError> {
        self.inbound
            .message()
            .await
            .map_err(GrpcStreamError::from_status)
    }
}

impl GrpcMarketDataServerStream {
    pub async fn message(&mut self) -> Result<Option<v1::MarketDataResponse>, GrpcStreamError> {
        self.inbound
            .message()
            .await
            .map_err(GrpcStreamError::from_status)
    }
}

impl GrpcMarketDataStream {
    pub async fn send(&mut self, request: v1::MarketDataRequest) -> Result<(), GrpcStreamError> {
        self.outbound
            .send(request)
            .await
            .map_err(|_| GrpcStreamError::OutboundClosed)
    }

    pub async fn message(&mut self) -> Result<Option<v1::MarketDataResponse>, GrpcStreamError> {
        self.inbound
            .message()
            .await
            .map_err(GrpcStreamError::from_status)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GrpcStreamError {
    #[error("market-data stream outbound channel closed")]
    OutboundClosed,
    #[error("provider error 80004: no active market-data subscriptions: {0}")]
    NoActiveSubscriptions(GrpcProviderError),
    #[error("{0}")]
    Provider(GrpcProviderError),
}

impl GrpcStreamError {
    fn from_status(status: Status) -> Self {
        let provider = GrpcProviderError::from_status(status);
        if provider.code == Code::ResourceExhausted && provider.has_provider_code("80004") {
            Self::NoActiveSubscriptions(provider)
        } else {
            Self::Provider(provider)
        }
    }
}

impl<T> GrpcResponse<T> {
    fn from_tonic(request_id: Uuid, attempt: u32, response: Response<T>) -> Self {
        let tracking_id = metadata_text(response.metadata(), "x-tracking-id");
        Self {
            body: response.into_inner(),
            metadata: GrpcResponseMetadata {
                request_id,
                tracking_id,
                attempt,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrpcProviderError {
    pub code: Code,
    pub message: String,
    pub details: Vec<u8>,
    pub tracking_id: Option<String>,
}

impl GrpcProviderError {
    fn from_status(status: Status) -> Self {
        Self {
            code: status.code(),
            message: status.message().to_owned(),
            details: status.details().to_vec(),
            tracking_id: metadata_text(status.metadata(), "x-tracking-id"),
        }
    }

    pub fn has_provider_code(&self, expected: &str) -> bool {
        if digit_tokens(&self.message).any(|token| token == expected) {
            return true;
        }
        if let Ok(detail) = v1::ErrorDetail::decode(self.details.as_slice())
            && detail.code == expected
        {
            return true;
        }
        let details = String::from_utf8_lossy(&self.details);
        digit_tokens(&details).any(|token| token == expected)
    }
}

fn digit_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| !token.is_empty())
}

impl fmt::Display for GrpcProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "provider gRPC {:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for GrpcProviderError {}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum GrpcErrorKind {
    #[error("failed to encode authorization metadata")]
    InvalidAuthorizationMetadata,
    #[error("failed to encode request correlation metadata")]
    InvalidRequestMetadata,
    #[error("market-data stream outbound capacity must be positive")]
    InvalidStreamCapacity,
    #[error("initial market-data stream requests must fit non-empty outbound capacity")]
    InvalidInitialStreamRequests,
    #[error("invalid market-data request: {0}")]
    MarketDataRequest(MarketDataRequestError),
    #[error(
        "mutation authorization environment {authorized:?} does not match gRPC client environment {configured:?}"
    )]
    MutationEnvironmentMismatch {
        authorized: Environment,
        configured: Environment,
    },
    #[error("{0}")]
    Provider(GrpcProviderError),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("T-Invest gRPC {method} failed on attempt {attempt}: {kind}", method = metadata.method, attempt = metadata.attempt)]
pub struct GrpcError {
    pub metadata: GrpcRequestMetadata,
    pub kind: GrpcErrorKind,
}

fn metadata_text(metadata: &MetadataMap, key: &'static str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl fmt::Debug for TInvestGrpcClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TInvestGrpcClient")
            .field("token", &self.token)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_domain::MutationGuard;

    fn token() -> SecretToken {
        SecretToken::new("secret").unwrap_or_else(|error| panic!("token failed: {error}"))
    }

    #[tokio::test]
    async fn production_uses_verified_native_roots() {
        let config = GrpcConfig::production();
        assert_eq!(
            config.certificate_policy(),
            GrpcCertificatePolicy::NativeRoots
        );
        assert_eq!(config.environment(), Environment::Live);
        assert!(TInvestGrpcClient::new(GrpcCredential::Production(token()), config).is_ok());
    }

    #[test]
    fn credentials_cannot_cross_production_and_sandbox_routes() {
        assert!(matches!(
            TInvestGrpcClient::new(GrpcCredential::Sandbox(token()), GrpcConfig::production()),
            Err(GrpcConfigError::CredentialEnvironmentMismatch {
                credential: Environment::Sandbox,
                endpoint: Environment::Live,
            })
        ));
        assert!(matches!(
            TInvestGrpcClient::new(GrpcCredential::Production(token()), GrpcConfig::sandbox()),
            Err(GrpcConfigError::CredentialEnvironmentMismatch {
                credential: Environment::Live,
                endpoint: Environment::Sandbox,
            })
        ));
        assert!(matches!(
            TInvestGrpcClient::production(GrpcCredential::Sandbox(token())),
            Err(GrpcConfigError::CredentialEnvironmentMismatch { .. })
        ));
        assert!(matches!(
            TInvestGrpcClient::sandbox(GrpcCredential::Production(token())),
            Err(GrpcConfigError::CredentialEnvironmentMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn unary_requests_have_deadline_but_stream_requests_do_not() {
        let client = TInvestGrpcClient::new(
            GrpcCredential::Production(token()),
            GrpcConfig::production(),
        )
        .unwrap_or_else(|error| panic!("client failed: {error}"));
        let unary = client
            .unary_request((), Uuid::nil())
            .expect("unary request metadata");
        let stream = client
            .stream_request((), Uuid::nil())
            .expect("stream request metadata");

        assert_eq!(
            unary
                .metadata()
                .get("grpc-timeout")
                .and_then(|value| value.to_str().ok()),
            Some("20000000u")
        );
        assert!(stream.metadata().get("grpc-timeout").is_none());
    }

    #[test]
    fn insecure_or_credentialed_endpoint_is_rejected() {
        let insecure = Url::parse("http://example.test").expect("static URL");
        assert!(matches!(
            GrpcConfig::production().with_endpoint(insecure, Environment::Live),
            Err(GrpcConfigError::InsecureUrl)
        ));
        let credentialed = Url::parse("https://user@example.test").expect("static URL");
        assert!(matches!(
            GrpcConfig::production().with_endpoint(credentialed, Environment::Live),
            Err(GrpcConfigError::InvalidUrl)
        ));
    }

    #[tokio::test]
    async fn mutation_environment_mismatch_fails_before_dispatch() {
        let client =
            TInvestGrpcClient::new(GrpcCredential::Sandbox(token()), GrpcConfig::sandbox())
                .unwrap_or_else(|error| panic!("client failed: {error}"));
        let authorization = MutationGuard::with_live_mutations_enabled(Environment::Live)
            .authorize_mutation()
            .expect("explicit live authorization");
        let error = client
            .edit_favorites(authorization, v1::EditFavoritesRequest::default())
            .await
            .expect_err("mismatch must fail");
        assert!(matches!(
            error.kind,
            GrpcErrorKind::MutationEnvironmentMismatch { .. }
        ));
    }

    #[tokio::test]
    async fn candle_source_with_limit_fails_before_dispatch() {
        let client = TInvestGrpcClient::new(
            GrpcCredential::Production(token()),
            GrpcConfig::production(),
        )
        .unwrap_or_else(|error| panic!("client failed: {error}"));
        let error = client
            .get_candles(v1::GetCandlesRequest {
                candle_source_type: Some(v1::get_candles_request::CandleSource::Exchange as i32),
                limit: Some(60),
                ..Default::default()
            })
            .await
            .expect_err("provider error 30220 must fail locally");
        assert_eq!(error.metadata.method, "GetCandles");
        assert_eq!(
            error.kind,
            GrpcErrorKind::MarketDataRequest(MarketDataRequestError::CandleSourceWithLimit)
        );
    }

    #[test]
    fn provider_80004_has_typed_stream_classification() {
        let error = GrpcStreamError::from_status(Status::resource_exhausted(
            "80004: No active subscriptions",
        ));
        assert!(matches!(
            error,
            GrpcStreamError::NoActiveSubscriptions(GrpcProviderError {
                code: Code::ResourceExhausted,
                ..
            })
        ));
        assert!(matches!(
            GrpcStreamError::from_status(Status::resource_exhausted("quota exhausted")),
            GrpcStreamError::Provider(_)
        ));
    }

    #[tokio::test]
    async fn bidirectional_stream_requires_seed_requests_before_dispatch() {
        let client = TInvestGrpcClient::new(
            GrpcCredential::Production(token()),
            GrpcConfig::production(),
        )
        .unwrap_or_else(|error| panic!("client failed: {error}"));
        let error = match client.open_market_data_stream(16, Vec::new()).await {
            Err(error) => error,
            Ok(_) => panic!("empty bootstrap would trigger provider 80004"),
        };
        assert_eq!(error.kind, GrpcErrorKind::InvalidInitialStreamRequests);
    }
}
