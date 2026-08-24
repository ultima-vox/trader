use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use url::Url;
use uuid::Uuid;
use vox_domain::{Environment, MutationAuthorization};

use crate::SecretToken;
use crate::retry::{NoopRetryObserver, RetryEvent, RetryObserver, RetryPolicy, RetryReason};

pub const DEFAULT_REST_BASE_URL: &str = "https://invest-public-api.tbank.ru/rest";
const DEFAULT_REQUEST_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
const TRACKING_ID_HEADER: HeaderName = HeaderName::from_static("x-tracking-id");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestOperation {
    /// Idempotent provider read. Only this class may use bounded retries.
    SafeRead,
    /// State-changing request. Transport performs exactly one attempt.
    Mutation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestCertificatePolicy {
    /// Roots loaded from host operating-system trust store; verification remains enabled.
    NativeRoots,
}

impl fmt::Display for RestOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SafeRead => formatter.write_str("safe-read"),
            Self::Mutation => formatter.write_str("mutation"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RestConfig {
    base_url: Url,
    connect_timeout: Duration,
    request_timeout: Duration,
    max_request_bytes: usize,
    max_response_bytes: usize,
    retry_policy: RetryPolicy,
    certificate_policy: RestCertificatePolicy,
}

impl RestConfig {
    pub fn production() -> Self {
        let base_url = match Url::parse(DEFAULT_REST_BASE_URL) {
            Ok(url) => url,
            Err(error) => unreachable!("built-in T-Invest URL is invalid: {error}"),
        };
        Self {
            base_url,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(20),
            max_request_bytes: DEFAULT_REQUEST_LIMIT,
            max_response_bytes: DEFAULT_RESPONSE_LIMIT,
            retry_policy: RetryPolicy::default(),
            certificate_policy: RestCertificatePolicy::NativeRoots,
        }
    }

    pub fn with_base_url(mut self, base_url: Url) -> Result<Self, RestConfigError> {
        validate_https_url(&base_url)?;
        self.base_url = base_url;
        Ok(self)
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, RestConfigError> {
        if connect_timeout.is_zero() || request_timeout.is_zero() {
            return Err(RestConfigError::ZeroTimeout);
        }
        self.connect_timeout = connect_timeout;
        self.request_timeout = request_timeout;
        Ok(self)
    }

    pub fn with_body_limits(
        mut self,
        max_request_bytes: usize,
        max_response_bytes: usize,
    ) -> Result<Self, RestConfigError> {
        if max_request_bytes == 0 || max_response_bytes == 0 {
            return Err(RestConfigError::ZeroBodyLimit);
        }
        self.max_request_bytes = max_request_bytes;
        self.max_response_bytes = max_response_bytes;
        Ok(self)
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub const fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy
    }

    pub const fn certificate_policy(&self) -> RestCertificatePolicy {
        self.certificate_policy
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RestConfigError {
    #[error("T-Invest REST base URL must use HTTPS")]
    InsecureUrl,
    #[error("T-Invest REST base URL must contain a host and no credentials")]
    InvalidUrl,
    #[error("REST connect and request timeouts must be positive")]
    ZeroTimeout,
    #[error("REST request and response body limits must be positive")]
    ZeroBodyLimit,
    #[error("failed to build rustls HTTPS client: {0}")]
    Client(String),
}

fn validate_https_url(url: &Url) -> Result<(), RestConfigError> {
    if url.scheme() != "https" {
        return Err(RestConfigError::InsecureUrl);
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(RestConfigError::InvalidUrl);
    }
    Ok(())
}

#[derive(Clone)]
pub struct TInvestRestClient {
    token: SecretToken,
    config: RestConfig,
    client: Client,
    retry_observer: Arc<dyn RetryObserver>,
}

impl TInvestRestClient {
    pub fn new(token: SecretToken, config: RestConfig) -> Result<Self, RestConfigError> {
        validate_https_url(&config.base_url)?;
        let client = Client::builder()
            .https_only(true)
            .tls_built_in_root_certs(false)
            .tls_built_in_native_certs(true)
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("vox-trader/0.1 vox-tinvest")
            .build()
            .map_err(|error| RestConfigError::Client(error.to_string()))?;
        Ok(Self {
            token,
            config,
            client,
            retry_observer: Arc::new(NoopRetryObserver),
        })
    }

    pub fn production(token: SecretToken) -> Result<Self, RestConfigError> {
        Self::new(token, RestConfig::production())
    }

    pub fn with_retry_observer<O>(mut self, observer: O) -> Self
    where
        O: RetryObserver + 'static,
    {
        self.retry_observer = Arc::new(observer);
        self
    }

    pub(crate) async fn post_read<Request, Response>(
        &self,
        method_path: &str,
        payload: &Request,
    ) -> Result<ProviderResponse<Response>, RestError>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        if provider_endpoint(method_path).is_none_or(|(_, method)| !is_safe_read_method(method)) {
            return Err(RestError::new(
                RequestMetadata {
                    request_id: Uuid::new_v4(),
                    operation: RestOperation::SafeRead,
                    method_path: method_path.to_owned(),
                    attempt: 0,
                },
                RestErrorKind::OperationMethodMismatch,
            ));
        }
        self.post(RestOperation::SafeRead, method_path, payload)
            .await
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "sealed mutation primitive awaits typed order adapter"
        )
    )]
    pub(crate) async fn post_mutation<Request, Response>(
        &self,
        authorization: MutationAuthorization,
        method_path: &str,
        payload: &Request,
    ) -> Result<ProviderResponse<Response>, RestError>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        if !mutation_path_matches_environment(authorization.environment(), method_path) {
            return Err(RestError::new(
                RequestMetadata {
                    request_id: Uuid::new_v4(),
                    operation: RestOperation::Mutation,
                    method_path: method_path.to_owned(),
                    attempt: 0,
                },
                RestErrorKind::MutationEnvironmentMismatch {
                    environment: authorization.environment(),
                },
            ));
        }
        self.post(RestOperation::Mutation, method_path, payload)
            .await
    }

    /// Sends a T-Invest REST-over-HTTP POST. Callers must classify operation honestly;
    /// mutations receive exactly one attempt even when retry policy allows more.
    async fn post<Request, Response>(
        &self,
        operation: RestOperation,
        method_path: &str,
        payload: &Request,
    ) -> Result<ProviderResponse<Response>, RestError>
    where
        Request: Serialize + Sync,
        Response: DeserializeOwned,
    {
        let request_id = Uuid::new_v4();
        let initial_metadata = RequestMetadata {
            request_id,
            operation,
            method_path: method_path.to_owned(),
            attempt: 0,
        };
        let url = self
            .method_url(method_path)
            .map_err(|kind| RestError::new(initial_metadata.clone(), kind))?;
        let serialized = serde_json::to_vec(payload).map_err(|error| {
            RestError::new(
                initial_metadata.clone(),
                RestErrorKind::RequestSerialization(error),
            )
        })?;
        if serialized.len() > self.config.max_request_bytes {
            return Err(RestError::new(
                initial_metadata,
                RestErrorKind::RequestTooLarge {
                    actual: serialized.len(),
                    limit: self.config.max_request_bytes,
                },
            ));
        }

        let max_attempts = match operation {
            RestOperation::SafeRead => self.config.retry_policy.max_attempts(),
            RestOperation::Mutation => 1,
        };
        let mut attempt = 1;
        loop {
            let metadata = RequestMetadata {
                request_id,
                operation,
                method_path: method_path.to_owned(),
                attempt,
            };
            tracing::debug!(
                request_id = %request_id,
                operation = %operation,
                method_path,
                attempt,
                "sending T-Invest REST request"
            );
            match self
                .send_attempt::<Response>(&url, &serialized, metadata.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(error) => {
                    if attempt >= max_attempts {
                        return Err(error);
                    }
                    let Some(retry) = error.retry_directive(SystemTime::now()) else {
                        return Err(error);
                    };
                    let backoff = self.config.retry_policy.delay_for(attempt, request_id);
                    let delay = match retry.server_delay {
                        Some(server_delay)
                            if server_delay > self.config.retry_policy.max_delay() =>
                        {
                            // Retrying earlier than provider's bound would violate rate limiting.
                            return Err(error);
                        }
                        Some(server_delay) => backoff.max(server_delay),
                        None => backoff,
                    };
                    let event = RetryEvent {
                        operation,
                        request_id,
                        failed_attempt: attempt,
                        next_attempt: attempt + 1,
                        delay,
                        server_retry_after: retry.server_delay,
                        server_retry_after_raw: retry.server_raw,
                        reason: retry.reason,
                    };
                    self.retry_observer.on_retry(&event);
                    tracing::warn!(
                        request_id = %request_id,
                        operation = %operation,
                        method_path,
                        failed_attempt = attempt,
                        next_attempt = attempt + 1,
                        delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
                        reason = ?event.reason,
                        "retrying safe T-Invest read"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    fn method_url(&self, method_path: &str) -> Result<Url, RestErrorKind> {
        if provider_endpoint(method_path).is_none() {
            return Err(RestErrorKind::InvalidMethodPath);
        }
        let combined = format!(
            "{}{}",
            self.config.base_url.as_str().trim_end_matches('/'),
            method_path
        );
        let url = Url::parse(&combined).map_err(|_| RestErrorKind::InvalidMethodPath)?;
        if url.origin() != self.config.base_url.origin()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(RestErrorKind::InvalidMethodPath);
        }
        Ok(url)
    }

    async fn send_attempt<Response>(
        &self,
        url: &Url,
        body: &[u8],
        metadata: RequestMetadata,
    ) -> Result<ProviderResponse<Response>, RestError>
    where
        Response: DeserializeOwned,
    {
        let request = self
            .client
            .post(url.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .header(REQUEST_ID_HEADER, metadata.request_id.to_string())
            .bearer_auth(self.token.expose_secret())
            .body(body.to_vec())
            .build()
            .map_err(|error| {
                RestError::new(metadata.clone(), RestErrorKind::RequestBuild(error))
            })?;

        let response = self.client.execute(request).await.map_err(|error| {
            let kind = if error.is_timeout() {
                RestErrorKind::Timeout(error)
            } else if error.is_connect() {
                RestErrorKind::Connect(error)
            } else {
                RestErrorKind::Transport(error)
            };
            RestError::new(metadata.clone(), kind)
        })?;

        let status = response.status();
        let headers = response.headers().clone();
        let raw_body = read_bounded_body(response, self.config.max_response_bytes)
            .await
            .map_err(|kind| RestError::new(metadata.clone(), kind))?;
        let response_metadata = ResponseMetadata {
            request: metadata.clone(),
            http_status: status.as_u16(),
            provider_tracking_id: header_text(&headers, &TRACKING_ID_HEADER),
        };

        if !status.is_success() {
            return Err(RestError::new(
                metadata,
                RestErrorKind::Provider(ProviderError::from_response(status, &headers, raw_body)),
            ));
        }

        let decoded = if raw_body.is_empty() {
            serde_json::from_slice(b"null")
        } else {
            serde_json::from_slice(&raw_body)
        }
        .map_err(|error| RestError::new(metadata, RestErrorKind::ResponseDecode(error)))?;

        Ok(ProviderResponse {
            body: decoded,
            metadata: response_metadata,
        })
    }
}

impl fmt::Debug for TInvestRestClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TInvestRestClient")
            .field("token", &self.token)
            .field("config", &self.config)
            .field("client", &"reqwest::Client(rustls, native roots)")
            .field("retry_observer", &self.retry_observer)
            .finish()
    }
}

async fn read_bounded_body(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, RestErrorKind> {
    let status = response.status().as_u16();
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(RestErrorKind::ResponseTooLarge { status, limit });
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| RestErrorKind::ResponseBody {
            status,
            source: error,
        })?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(RestErrorKind::ResponseTooLarge { status, limit });
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMetadata {
    pub request_id: Uuid,
    pub operation: RestOperation,
    pub method_path: String,
    pub attempt: u32,
}

impl fmt::Display for RequestMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {} request_id={} attempt={}",
            self.operation, self.method_path, self.request_id, self.attempt
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseMetadata {
    pub request: RequestMetadata,
    pub http_status: u16,
    pub provider_tracking_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderResponse<T> {
    pub body: T,
    pub metadata: ResponseMetadata,
}

impl<T> ProviderResponse<T> {
    pub fn into_body(self) -> T {
        self.body
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ProviderError {
    http_status: u16,
    provider_tracking_id: Option<String>,
    retry_after_raw: Option<String>,
    raw_body: Vec<u8>,
    json_body: Option<Value>,
}

impl ProviderError {
    fn from_response(status: StatusCode, headers: &HeaderMap, raw_body: Vec<u8>) -> Self {
        let json_body = serde_json::from_slice(&raw_body).ok();
        Self {
            http_status: status.as_u16(),
            provider_tracking_id: header_text(headers, &TRACKING_ID_HEADER),
            retry_after_raw: header_text(headers, &reqwest::header::RETRY_AFTER),
            raw_body,
            json_body,
        }
    }

    pub const fn http_status(&self) -> u16 {
        self.http_status
    }

    pub fn provider_tracking_id(&self) -> Option<&str> {
        self.provider_tracking_id.as_deref()
    }

    pub fn retry_after_raw(&self) -> Option<&str> {
        self.retry_after_raw.as_deref()
    }

    pub fn code(&self) -> Option<String> {
        self.json_field("code")
    }

    pub fn message(&self) -> Option<String> {
        self.json_field("message")
    }

    pub fn description(&self) -> Option<String> {
        self.json_field("description")
    }

    fn json_field(&self, field: &str) -> Option<String> {
        let value = self.json_body.as_ref()?.get(field)?;
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Null => None,
            other => Some(other.to_string()),
        }
    }

    fn retry_after(&self, now: SystemTime) -> Option<Duration> {
        self.retry_after_raw
            .as_deref()
            .and_then(|raw| parse_retry_after(raw, now))
    }
}

impl fmt::Debug for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderError")
            .field("http_status", &self.http_status)
            .field("provider_tracking_id", &self.provider_tracking_id)
            .field("retry_after_raw", &self.retry_after_raw)
            .field("code", &self.code())
            .field("message", &self.message())
            .field("description", &self.description())
            .field("raw_body", &"[REDACTED: retained inside adapter]")
            .finish()
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "provider HTTP {}", self.http_status)?;
        if let Some(code) = self.code() {
            write!(formatter, " code={code}")?;
        }
        if let Some(message) = self.message() {
            write!(formatter, " message={message}")?;
        }
        if let Some(tracking_id) = self.provider_tracking_id() {
            write!(formatter, " tracking_id={tracking_id}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProviderError {}

fn header_text(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[derive(Debug, Error)]
#[error("T-Invest REST {metadata}: {kind}")]
pub struct RestError {
    metadata: RequestMetadata,
    #[source]
    kind: RestErrorKind,
}

impl RestError {
    fn new(metadata: RequestMetadata, kind: RestErrorKind) -> Self {
        Self { metadata, kind }
    }

    pub const fn metadata(&self) -> &RequestMetadata {
        &self.metadata
    }

    pub const fn kind(&self) -> &RestErrorKind {
        &self.kind
    }

    pub const fn dispatch_certainty(&self) -> DispatchCertainty {
        match self.kind {
            RestErrorKind::InvalidMethodPath
            | RestErrorKind::MutationEnvironmentMismatch { .. }
            | RestErrorKind::OperationMethodMismatch
            | RestErrorKind::RequestSerialization(_)
            | RestErrorKind::RequestTooLarge { .. }
            | RestErrorKind::RequestBuild(_)
            | RestErrorKind::Connect(_) => DispatchCertainty::NotDispatched,
            RestErrorKind::Timeout(_)
            | RestErrorKind::Transport(_)
            | RestErrorKind::ResponseBody { .. }
            | RestErrorKind::ResponseTooLarge { .. } => DispatchCertainty::PossiblyDispatched,
            RestErrorKind::Provider(_) | RestErrorKind::ResponseDecode(_) => {
                DispatchCertainty::ProviderResponded
            }
        }
    }

    fn retry_directive(&self, now: SystemTime) -> Option<RetryDirective> {
        if self.metadata.operation != RestOperation::SafeRead {
            return None;
        }
        match &self.kind {
            RestErrorKind::Connect(_) => Some(RetryDirective::new(RetryReason::Connect)),
            RestErrorKind::Timeout(_) => Some(RetryDirective::new(RetryReason::Timeout)),
            RestErrorKind::Transport(_) => Some(RetryDirective::new(RetryReason::Transport)),
            RestErrorKind::ResponseBody { .. } => {
                Some(RetryDirective::new(RetryReason::ResponseBody))
            }
            RestErrorKind::Provider(provider)
                if matches!(provider.http_status(), 429 | 500 | 502 | 503 | 504) =>
            {
                Some(RetryDirective {
                    reason: RetryReason::HttpStatus(provider.http_status()),
                    server_delay: provider.retry_after(now),
                    server_raw: provider.retry_after_raw.clone(),
                })
            }
            _ => None,
        }
    }
}

struct RetryDirective {
    reason: RetryReason,
    server_delay: Option<Duration>,
    server_raw: Option<String>,
}

impl RetryDirective {
    const fn new(reason: RetryReason) -> Self {
        Self {
            reason,
            server_delay: None,
            server_raw: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum RestErrorKind {
    #[error("method path must start with one '/' and contain no query or fragment")]
    InvalidMethodPath,
    #[error("mutation method does not match authorized {environment:?} environment")]
    MutationEnvironmentMismatch { environment: Environment },
    #[error("REST operation classification does not match provider method")]
    OperationMethodMismatch,
    #[error("failed to serialize typed request")]
    RequestSerialization(#[source] serde_json::Error),
    #[error("serialized request is {actual} bytes; limit is {limit}")]
    RequestTooLarge { actual: usize, limit: usize },
    #[error("failed to build HTTPS request")]
    RequestBuild(#[source] reqwest::Error),
    #[error("HTTPS connection failed before dispatch")]
    Connect(#[source] reqwest::Error),
    #[error("HTTPS request timed out after dispatch became possible")]
    Timeout(#[source] reqwest::Error),
    #[error("HTTPS transport failed after dispatch became possible")]
    Transport(#[source] reqwest::Error),
    #[error("failed reading provider HTTP {status} response")]
    ResponseBody {
        status: u16,
        #[source]
        source: reqwest::Error,
    },
    #[error("provider HTTP {status} response exceeded {limit} bytes")]
    ResponseTooLarge { status: u16, limit: usize },
    #[error("{0}")]
    Provider(#[source] ProviderError),
    #[error("provider returned invalid JSON for typed response")]
    ResponseDecode(#[source] serde_json::Error),
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by sealed mutation primitive")
)]
fn mutation_path_matches_environment(environment: Environment, method_path: &str) -> bool {
    let Some((service, method)) = provider_endpoint(method_path) else {
        return false;
    };
    if !is_mutation_method(method) {
        return false;
    }
    let sandbox_method = service == "tinkoff.public.invest.api.contract.v1.SandboxService";
    match environment {
        Environment::Sandbox => sandbox_method,
        Environment::Paper => false,
        Environment::Live => !sandbox_method,
    }
}

fn provider_endpoint(method_path: &str) -> Option<(&str, &str)> {
    let path = method_path.strip_prefix('/')?;
    let mut segments = path.split('/');
    let service = segments.next()?;
    let method = segments.next()?;
    if segments.next().is_some()
        || !service.starts_with("tinkoff.public.invest.api.contract.v1.")
        || !service.ends_with("Service")
        || !service
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.')
        || method.is_empty()
        || !method.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some((service, method))
}

fn is_safe_read_method(method: &str) -> bool {
    method.starts_with("Get")
        || matches!(
            method,
            "Shares" | "Bonds" | "Etfs" | "Currencies" | "Futures" | "OptionsBy"
        )
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by sealed mutation primitive")
)]
fn is_mutation_method(method: &str) -> bool {
    ["Post", "Cancel", "Replace", "Open", "Close", "PayIn"]
        .iter()
        .any(|prefix| method.starts_with(prefix))
        || method == "SandboxPayIn"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchCertainty {
    NotDispatched,
    PossiblyDispatched,
    ProviderResponded,
}

fn parse_retry_after(raw: &str, now: SystemTime) -> Option<Duration> {
    if let Ok(seconds) = raw.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let target = parse_imf_fixdate(raw.trim())?;
    Some(target.duration_since(now).unwrap_or(Duration::ZERO))
}

fn parse_imf_fixdate(raw: &str) -> Option<SystemTime> {
    let fields: Vec<_> = raw.split_ascii_whitespace().collect();
    if fields.len() != 6 || !fields[0].ends_with(',') || fields[5] != "GMT" {
        return None;
    }
    let day = fields[1].parse::<u32>().ok()?;
    let month = match fields[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = fields[3].parse::<i32>().ok()?;
    let time: Vec<_> = fields[4].split(':').collect();
    if time.len() != 3 {
        return None;
    }
    let hour = time[0].parse::<u32>().ok()?;
    let minute = time[1].parse::<u32>().ok()?;
    let second = time[2].parse::<u32>().ok()?;
    if !(1970..=9999).contains(&year)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = u64::try_from(days)
        .ok()?
        .checked_mul(86_400)?
        .checked_add(u64::from(hour) * 3_600 + u64::from(minute) * 60 + u64::from(second))?;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

// Howard Hinnant's civil-date conversion, shifted to the Unix epoch.
fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let leap =
        |candidate: i32| candidate % 4 == 0 && (candidate % 100 != 0 || candidate % 400 == 0);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > max_day {
        return None;
    }
    let adjusted_year = year - i32::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i32::try_from(month).ok()? + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i32::try_from(day).ok()? - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(i64::from(era * 146_097 + day_of_era - 719_468))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn metadata(operation: RestOperation) -> RequestMetadata {
        RequestMetadata {
            request_id: Uuid::nil(),
            operation,
            method_path: "/service/Method".to_owned(),
            attempt: 1,
        }
    }

    #[test]
    fn production_config_uses_native_roots_enforces_https_and_redacts_token() {
        assert_eq!(
            RestConfig::production().certificate_policy(),
            RestCertificatePolicy::NativeRoots
        );
        let insecure = match Url::parse("http://example.test/rest") {
            Ok(url) => url,
            Err(error) => panic!("unexpected URL error: {error}"),
        };
        assert!(matches!(
            RestConfig::production().with_base_url(insecure),
            Err(RestConfigError::InsecureUrl)
        ));

        let token = match SecretToken::new("super-secret-token") {
            Ok(token) => token,
            Err(error) => panic!("unexpected token error: {error}"),
        };
        let client = match TInvestRestClient::production(token) {
            Ok(client) => client,
            Err(error) => panic!("unexpected client error: {error}"),
        };
        let debug = format!("{client:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-token"));
    }

    #[test]
    fn provider_error_preserves_exact_body_and_fields() {
        let raw =
            br#"{"code":30042,"message":"rate limited","description":"quota A","extra":{"n":7}}"#
                .to_vec();
        let mut headers = HeaderMap::new();
        headers.insert(TRACKING_ID_HEADER, HeaderValue::from_static("track-123"));
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("3"));
        let error =
            ProviderError::from_response(StatusCode::TOO_MANY_REQUESTS, &headers, raw.clone());

        assert_eq!(error.http_status(), 429);
        assert_eq!(error.provider_tracking_id(), Some("track-123"));
        assert_eq!(error.retry_after_raw(), Some("3"));
        assert_eq!(error.raw_body, raw);
        assert_eq!(error.code().as_deref(), Some("30042"));
        assert_eq!(error.message().as_deref(), Some("rate limited"));
        assert_eq!(
            error
                .json_body
                .as_ref()
                .and_then(|body| body.get("extra"))
                .and_then(|extra| extra.get("n")),
            Some(&Value::from(7))
        );
    }

    #[test]
    fn mutation_errors_never_produce_retry_directive() {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("1"));
        let provider =
            ProviderError::from_response(StatusCode::TOO_MANY_REQUESTS, &headers, b"{}".to_vec());
        let error = RestError::new(
            metadata(RestOperation::Mutation),
            RestErrorKind::Provider(provider),
        );
        assert!(error.retry_directive(SystemTime::now()).is_none());
    }

    #[tokio::test]
    async fn mutation_authorization_is_bound_to_environment_path() {
        let token = match SecretToken::new("token") {
            Ok(token) => token,
            Err(error) => panic!("unexpected token error: {error}"),
        };
        let client = match TInvestRestClient::production(token) {
            Ok(client) => client,
            Err(error) => panic!("unexpected client error: {error}"),
        };
        let sandbox =
            match vox_domain::MutationGuard::new(Environment::Sandbox).authorize_mutation() {
                Ok(authorization) => authorization,
                Err(error) => panic!("unexpected sandbox authorization error: {error}"),
            };
        let result = client
            .post_mutation::<_, serde_json::Value>(
                sandbox,
                "/tinkoff.public.invest.api.contract.v1.OrdersService/PostOrder",
                &serde_json::json!({}),
            )
            .await;
        assert!(matches!(
            result,
            Err(error)
                if matches!(
                    error.kind(),
                    RestErrorKind::MutationEnvironmentMismatch {
                        environment: Environment::Sandbox
                    }
                )
                && error.dispatch_certainty() == DispatchCertainty::NotDispatched
        ));

        let traversal =
            match vox_domain::MutationGuard::new(Environment::Sandbox).authorize_mutation() {
                Ok(authorization) => authorization,
                Err(error) => panic!("unexpected sandbox authorization error: {error}"),
            };
        let traversal_result = client
            .post_mutation::<_, serde_json::Value>(
                traversal,
                "/tinkoff.public.invest.api.contract.v1.SandboxService/../tinkoff.public.invest.api.contract.v1.OrdersService/PostOrder",
                &serde_json::json!({}),
            )
            .await;
        assert!(matches!(
            traversal_result,
            Err(error) if error.dispatch_certainty() == DispatchCertainty::NotDispatched
        ));

        let read_as_mutation = client
            .post_read::<_, serde_json::Value>(
                "/tinkoff.public.invest.api.contract.v1.OrdersService/PostOrder",
                &serde_json::json!({}),
            )
            .await;
        assert!(matches!(
            read_as_mutation,
            Err(error)
                if matches!(error.kind(), RestErrorKind::OperationMethodMismatch)
                    && error.dispatch_certainty() == DispatchCertainty::NotDispatched
        ));
    }

    #[test]
    fn safe_read_429_honors_delta_and_http_date_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(reqwest::header::RETRY_AFTER, HeaderValue::from_static("7"));
        let provider =
            ProviderError::from_response(StatusCode::TOO_MANY_REQUESTS, &headers, b"{}".to_vec());
        let error = RestError::new(
            metadata(RestOperation::SafeRead),
            RestErrorKind::Provider(provider),
        );
        let directive = match error.retry_directive(UNIX_EPOCH) {
            Some(directive) => directive,
            None => panic!("429 safe read must be retryable"),
        };
        assert_eq!(directive.server_delay, Some(Duration::from_secs(7)));

        let target = parse_retry_after("Thu, 01 Jan 1970 00:00:10 GMT", UNIX_EPOCH);
        assert_eq!(target, Some(Duration::from_secs(10)));
        assert!(parse_retry_after("not-a-date", UNIX_EPOCH).is_none());
    }

    #[test]
    fn method_path_cannot_escape_configured_origin() {
        let token = match SecretToken::new("token") {
            Ok(token) => token,
            Err(error) => panic!("unexpected token error: {error}"),
        };
        let client = match TInvestRestClient::production(token) {
            Ok(client) => client,
            Err(error) => panic!("unexpected client error: {error}"),
        };
        assert!(client.method_url("service/Method").is_err());
        assert!(client.method_url("//evil.test/path").is_err());
        assert!(client.method_url("/service/Method?secret=x").is_err());
        assert!(
            client
                .method_url("/tinkoff.public.invest.api.contract.v1.InstrumentsService/Shares")
                .is_ok()
        );
        assert!(
            client
                .method_url(
                    "/tinkoff.public.invest.api.contract.v1.SandboxService/../tinkoff.public.invest.api.contract.v1.OrdersService/PostOrder"
                )
                .is_err()
        );
    }
}
