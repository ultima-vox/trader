//! REST surface under `/api/v1`.
//!
//! Handlers only translate: they take a typed request, call an application port and shape a
//! typed response. No business rule, no risk decision, no precedence arithmetic lives here.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::application::{AppState, AuthenticatedActor, ConnectionRequestContext};
use crate::contract::account::{
    BrokerAccountDto, OperationsPageDto, OrderDto, PortfolioDto, PositionDto, ReconciliationDto,
    StopOrderDto,
};
use crate::contract::auth::{AuthSessionDto, CreateSessionRequest};
use crate::contract::capability::CapabilitySet;
use crate::contract::connections::{
    BindBrokerAccountRequest, BrokerAccountBindingDto, BrokerConnectionMetadataDto,
    ChangeExecutionAuthorizationRequest, ConnectionDetailsDto, CreateBrokerConnectionRequest,
    CredentialRotationResultDto, DiscoveredBrokerAccountDto, ExecutionAuthorizationDto,
    RotateCredentialRequest,
};
use crate::contract::execution::{
    CancelOrderRequest, JournalStateDto, MutationReceiptDto, ReplaceOrderRequest,
    SubmitOrderRequest, SubmitProtectionRequest, SubmitStopOrderRequest,
};
use crate::contract::market::{
    CandleIntervalCapability, CandleIntervalDto, CandlesDto, InstrumentSummaryDto, OrderBookDto,
    QuoteDto, SessionDto, TradeTickDto,
};
use crate::contract::runtime::{RuntimeHealthDto, SystemHealthDto};
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto, TradingMode};
use crate::error::{ApiError, ErrorCategory, FieldError};

/// The account-scoped query string every read on the account side requires.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct ScopeQuery {
    /// Provider of the connection, for example `T_INVEST`.
    pub provider: ProviderDto,
    /// Broker environment: `SANDBOX` or `PRODUCTION`.
    pub environment: BrokerEnvironment,
    /// Application connection identity. Never a credential.
    pub broker_connection_id: String,
    /// Canonical Vox account/binding identity.
    pub account_id: String,
}

impl ScopeQuery {
    fn into_scope(self) -> Result<ExecutionScope, ApiError> {
        ExecutionScope::new(
            self.provider,
            self.environment,
            self.broker_connection_id,
            self.account_id,
            TradingMode::Live,
        )
        .map_err(|error| {
            ApiError::validation(
                error.to_string(),
                vec![FieldError {
                    field: "account_id".to_owned(),
                    message: error.to_string(),
                }],
            )
        })
    }
}

/// Paging for the operations history.
///
/// The scope fields are spelled out rather than flattened: a flattened struct becomes one
/// opaque query parameter in the schema, and the generated client would lose the shape.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct OperationsQuery {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    pub broker_connection_id: String,
    pub account_id: String,
    /// Cursor returned by the previous page.
    pub cursor: Option<String>,
    /// Page size; the server caps it.
    pub limit: Option<u16>,
}

impl OperationsQuery {
    fn into_parts(self) -> Result<(ExecutionScope, Option<String>, u16), ApiError> {
        let limit = self.limit.unwrap_or(100).clamp(1, 500);
        let cursor = self.cursor;
        let scope = ScopeQuery {
            provider: self.provider,
            environment: self.environment,
            broker_connection_id: self.broker_connection_id,
            account_id: self.account_id,
        }
        .into_scope()?;
        Ok((scope, cursor, limit))
    }
}

/// Liveness of the API process itself.
#[utoipa::path(
    get, path = "/api/v1/system/health", tag = "system",
    responses((status = 200, description = "The process can serve requests", body = SystemHealthDto))
)]
pub async fn system_health() -> Json<SystemHealthDto> {
    Json(SystemHealthDto {
        status: "ok".to_owned(),
        api_version: "v1".to_owned(),
        server_time_unix_ms: now_unix_ms(),
    })
}

/// Exchanges trusted bootstrap credential for browser session cookie and CSRF state.
#[utoipa::path(
    post, path = "/api/v1/auth/session", tag = "auth",
    request_body = CreateSessionRequest,
    responses(
        (status = 200, description = "Session established; authentication is in HttpOnly cookie", body = AuthSessionDto),
        (status = 401, description = "Bootstrap credential rejected", body = ApiError),
        (status = 503, description = "Authentication persistence unavailable", body = ApiError),
    )
)]
pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<(HeaderMap, Json<AuthSessionDto>), ApiError> {
    let session = state
        .authentication_port()?
        .establish_session(request)
        .await?;
    let max_age = ((session.expires_at_unix_ms - now_unix_ms()) / 1_000).max(0);
    let secure = if session.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "vox_session={}; HttpOnly{}; SameSite=Strict; Path=/; Max-Age={max_age}",
        session.session_token, secure
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| {
            ApiError::new(
                ErrorCategory::Internal,
                "SESSION_COOKIE_ENCODING_FAILED",
                "server could not encode session cookie",
            )
        })?,
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(AuthSessionDto {
            user_id: session.user_id,
            effective_permissions: session.effective_permissions,
            csrf_token: session.csrf_token,
            expires_at_unix_ms: session.expires_at_unix_ms,
        }),
    ))
}

/// Runtime state, readiness and stream health.
#[utoipa::path(
    get, path = "/api/v1/runtime", tag = "runtime",
    responses(
        (status = 200, description = "Current runtime health", body = RuntimeHealthDto),
        (status = 503, description = "No runtime is attached to this process", body = ApiError),
    )
)]
pub async fn runtime_health(
    State(state): State<AppState>,
) -> Result<Json<RuntimeHealthDto>, ApiError> {
    Ok(Json(state.runtime_port()?.health().await?))
}

/// Scopes the operator may select. Empty until #17 bindings exist.
#[utoipa::path(
    get, path = "/api/v1/runtime/scopes", tag = "runtime",
    responses(
        (status = 200, description = "Selectable execution scopes", body = Vec<ExecutionScope>),
        (status = 503, body = ApiError),
    )
)]
pub async fn runtime_scopes(
    State(state): State<AppState>,
) -> Result<Json<Vec<ExecutionScope>>, ApiError> {
    Ok(Json(state.runtime_port()?.scopes().await?))
}

/// What this deployment can actually do.
#[utoipa::path(
    get, path = "/api/v1/capabilities", tag = "system",
    params(("account_id" = Option<String>, Query, description = "Scope the capability set to one canonical account identity")),
    responses((status = 200, description = "Capability set", body = CapabilitySet))
)]
pub async fn capabilities(
    State(state): State<AppState>,
    Query(params): Query<CapabilityQuery>,
) -> Result<Json<CapabilitySet>, ApiError> {
    if let Some(runtime) = state.runtime.as_ref()
        && let Some(capabilities) = runtime.capabilities(params.account_id.as_deref()).await?
    {
        return Ok(Json(capabilities));
    }
    Ok(Json(state.capabilities(params.account_id)))
}

fn connection_context(
    actor: Option<Extension<AuthenticatedActor>>,
) -> Result<ConnectionRequestContext, ApiError> {
    let actor = actor.map(|Extension(value)| value).ok_or_else(|| {
        ApiError::new(
            ErrorCategory::Authentication,
            "AUTHENTICATED_ACTOR_REQUIRED",
            "authenticated actor context is required",
        )
    })?;
    Ok(ConnectionRequestContext {
        actor,
        correlation_id: uuid::Uuid::new_v4().to_string(),
        now_unix_ms: now_unix_ms(),
    })
}

#[utoipa::path(
    get, path = "/api/v1/broker-connections", tag = "connections",
    responses((status = 200, body = Vec<BrokerConnectionMetadataDto>), (status = 401, body = ApiError), (status = 403, body = ApiError))
)]
pub async fn broker_connections(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
) -> Result<Json<Vec<BrokerConnectionMetadataDto>>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state.connections_port()?.list_connections(&context).await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/broker-connections", tag = "connections",
    request_body = CreateBrokerConnectionRequest,
    responses((status = 201, body = BrokerConnectionMetadataDto), (status = 400, body = ApiError), (status = 401, body = ApiError), (status = 403, body = ApiError))
)]
pub async fn create_broker_connection(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Json(request): Json<CreateBrokerConnectionRequest>,
) -> Result<(StatusCode, Json<BrokerConnectionMetadataDto>), ApiError> {
    let context = connection_context(actor)?;
    let result = state
        .connections_port()?
        .create_connection(&context, request)
        .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

#[utoipa::path(
    get, path = "/api/v1/broker-connections/{connection_id}", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 200, body = ConnectionDetailsDto), (status = 401, body = ApiError), (status = 404, body = ApiError))
)]
pub async fn broker_connection_details(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<Json<ConnectionDetailsDto>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .connection_details(&context, &connection_id)
            .await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/broker-connections/{connection_id}/validate", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 200, body = BrokerConnectionMetadataDto), (status = 401, body = ApiError), (status = 409, body = ApiError), (status = 503, body = ApiError))
)]
pub async fn validate_broker_connection(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<Json<BrokerConnectionMetadataDto>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .revalidate_connection(&context, &connection_id)
            .await?,
    ))
}

#[utoipa::path(
    put, path = "/api/v1/broker-connections/{connection_id}/credential", tag = "connections",
    params(("connection_id" = String, Path)), request_body = RotateCredentialRequest,
    responses((status = 200, body = CredentialRotationResultDto), (status = 400, body = ApiError), (status = 401, body = ApiError), (status = 409, body = ApiError))
)]
pub async fn rotate_broker_credential(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
    Json(request): Json<RotateCredentialRequest>,
) -> Result<Json<CredentialRotationResultDto>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .rotate_credential(&context, &connection_id, request)
            .await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/broker-connections/{connection_id}/disable", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 200, body = BrokerConnectionMetadataDto), (status = 401, body = ApiError), (status = 403, body = ApiError))
)]
pub async fn disable_broker_connection(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<Json<BrokerConnectionMetadataDto>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .disable_connection(&context, &connection_id)
            .await?,
    ))
}

#[utoipa::path(
    delete, path = "/api/v1/broker-connections/{connection_id}", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 204), (status = 401, body = ApiError), (status = 409, body = ApiError))
)]
pub async fn delete_broker_connection(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let context = connection_context(actor)?;
    state
        .connections_port()?
        .delete_connection(&context, &connection_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get, path = "/api/v1/broker-connections/{connection_id}/accounts", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 200, body = Vec<DiscoveredBrokerAccountDto>), (status = 401, body = ApiError), (status = 404, body = ApiError))
)]
pub async fn discovered_broker_accounts(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<Json<Vec<DiscoveredBrokerAccountDto>>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .accounts(&context, &connection_id)
            .await?,
    ))
}

#[utoipa::path(
    get, path = "/api/v1/broker-connections/{connection_id}/bindings", tag = "connections",
    params(("connection_id" = String, Path)),
    responses((status = 200, body = Vec<BrokerAccountBindingDto>), (status = 401, body = ApiError))
)]
pub async fn broker_account_bindings(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
) -> Result<Json<Vec<BrokerAccountBindingDto>>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .bindings(&context, &connection_id)
            .await?,
    ))
}

#[utoipa::path(
    post, path = "/api/v1/broker-connections/{connection_id}/bindings", tag = "connections",
    params(("connection_id" = String, Path)), request_body = BindBrokerAccountRequest,
    responses((status = 201, body = BrokerAccountBindingDto), (status = 400, body = ApiError), (status = 401, body = ApiError), (status = 409, body = ApiError))
)]
pub async fn bind_broker_account(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
    Json(request): Json<BindBrokerAccountRequest>,
) -> Result<(StatusCode, Json<BrokerAccountBindingDto>), ApiError> {
    let context = connection_context(actor)?;
    let result = state
        .connections_port()?
        .bind_account(&context, &connection_id, request)
        .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

#[utoipa::path(
    delete, path = "/api/v1/broker-bindings/{binding_id}", tag = "connections",
    params(("binding_id" = String, Path)),
    responses((status = 204), (status = 401, body = ApiError), (status = 404, body = ApiError))
)]
pub async fn unbind_broker_account(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(binding_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let context = connection_context(actor)?;
    state
        .connections_port()?
        .unbind_account(&context, &binding_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put, path = "/api/v1/broker-connections/{connection_id}/execution-authorization", tag = "connections",
    params(("connection_id" = String, Path)), request_body = ChangeExecutionAuthorizationRequest,
    responses((status = 200, body = ExecutionAuthorizationDto), (status = 401, body = ApiError), (status = 403, body = ApiError), (status = 409, body = ApiError))
)]
pub async fn change_execution_authorization(
    State(state): State<AppState>,
    actor: Option<Extension<AuthenticatedActor>>,
    Path(connection_id): Path<String>,
    Json(request): Json<ChangeExecutionAuthorizationRequest>,
) -> Result<Json<ExecutionAuthorizationDto>, ApiError> {
    let context = connection_context(actor)?;
    Ok(Json(
        state
            .connections_port()?
            .change_execution_authorization(&context, &connection_id, request)
            .await?,
    ))
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct CapabilityQuery {
    pub account_id: Option<String>,
}

/// Accounts discovered through the connection.
#[utoipa::path(
    get, path = "/api/v1/accounts", tag = "accounts", params(ScopeQuery),
    responses(
        (status = 200, body = Vec<BrokerAccountDto>),
        (status = 503, description = "No account read side is attached", body = ApiError),
    )
)]
pub async fn accounts(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<BrokerAccountDto>>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.accounts(&scope).await?))
}

/// Currency balances as the broker reports them.
#[utoipa::path(
    get, path = "/api/v1/portfolio", tag = "accounts", params(ScopeQuery),
    responses((status = 200, body = PortfolioDto), (status = 503, body = ApiError))
)]
pub async fn portfolio(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<PortfolioDto>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.portfolio(&scope).await?))
}

/// Positions and their quantities.
#[utoipa::path(
    get, path = "/api/v1/positions", tag = "accounts", params(ScopeQuery),
    responses((status = 200, body = Vec<PositionDto>), (status = 503, body = ApiError))
)]
pub async fn positions(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<PositionDto>>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.positions(&scope).await?))
}

/// Active orders.
#[utoipa::path(
    get, path = "/api/v1/orders", tag = "accounts", params(ScopeQuery),
    responses((status = 200, body = Vec<OrderDto>), (status = 503, body = ApiError))
)]
pub async fn orders(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<OrderDto>>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.orders(&scope).await?))
}

/// Stop orders.
#[utoipa::path(
    get, path = "/api/v1/stop-orders", tag = "accounts", params(ScopeQuery),
    responses((status = 200, body = Vec<StopOrderDto>), (status = 503, body = ApiError))
)]
pub async fn stop_orders(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<StopOrderDto>>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.stop_orders(&scope).await?))
}

/// Operations history, paged by cursor.
#[utoipa::path(
    get, path = "/api/v1/operations", tag = "accounts", params(OperationsQuery),
    responses((status = 200, body = OperationsPageDto), (status = 503, body = ApiError))
)]
pub async fn operations(
    State(state): State<AppState>,
    Query(query): Query<OperationsQuery>,
) -> Result<Json<OperationsPageDto>, ApiError> {
    let (scope, cursor, limit) = query.into_parts()?;
    Ok(Json(
        state
            .accounts_port()?
            .operations(&scope, cursor.as_deref(), limit)
            .await?,
    ))
}

/// How complete the last reconciliation was.
#[utoipa::path(
    get, path = "/api/v1/reconciliation", tag = "runtime", params(ScopeQuery),
    responses((status = 200, body = ReconciliationDto), (status = 503, body = ApiError))
)]
pub async fn reconciliation(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<ReconciliationDto>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.reconciliation(&scope).await?))
}

/// Mutation journal for one scope. Empty when nothing has been dispatched.
#[utoipa::path(
    get, path = "/api/v1/mutations", tag = "execution", params(ScopeQuery),
    responses((status = 200, body = Vec<MutationReceiptDto>), (status = 503, body = ApiError))
)]
pub async fn mutations(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> Result<Json<Vec<MutationReceiptDto>>, ApiError> {
    let scope = query.into_scope()?;
    Ok(Json(state.accounts_port()?.mutations(&scope).await?))
}

/// Submit a regular order. The scope in the body is the frozen target of the command.
#[utoipa::path(
    post, path = "/api/v1/commands/order", tag = "execution", request_body = SubmitOrderRequest,
    responses(
        (status = 200, description = "Mutation receipt", body = MutationReceiptDto),
        (status = 202, description = "Dispatched, outcome not yet known; body is the mutation receipt", body = MutationReceiptDto),
        (status = 400, body = ApiError),
        (status = 403, description = "Execution is not authorized for this scope", body = ApiError),
        (status = 503, description = "No execution side is attached", body = ApiError),
    )
)]
pub async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    validate_submit(&request)?;
    Ok(MutationHttpResponse(
        state.execution_port()?.submit_order(request).await?,
    ))
}

/// Cancel a regular order.
#[utoipa::path(
    post, path = "/api/v1/commands/cancel-order", tag = "execution", request_body = CancelOrderRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 202, description = "Dispatched, outcome not yet known; body is the mutation receipt", body = MutationReceiptDto),
        (status = 400, body = ApiError),
        (status = 503, body = ApiError),
    )
)]
pub async fn cancel_order(
    State(state): State<AppState>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    request.target()?;
    Ok(MutationHttpResponse(
        state.execution_port()?.cancel_order(request).await?,
    ))
}

/// Replace a live regular order.
#[utoipa::path(
    post, path = "/api/v1/commands/replace-order", tag = "execution",
    request_body = ReplaceOrderRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 202, body = MutationReceiptDto),
        (status = 400, body = ApiError),
        (status = 503, body = ApiError),
    )
)]
pub async fn replace_order(
    State(state): State<AppState>,
    Json(request): Json<ReplaceOrderRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    request.target()?;
    if request.instrument_id.trim().is_empty() || request.quantity_lots <= 0 {
        return Err(ApiError::validation(
            "the replace command is not valid",
            vec![FieldError {
                field: "quantity_lots".to_owned(),
                message: "instrument_id must be set and quantity_lots must be positive".to_owned(),
            }],
        ));
    }
    Ok(MutationHttpResponse(
        state.execution_port()?.replace_order(request).await?,
    ))
}

/// Submit a stop order.
#[utoipa::path(
    post, path = "/api/v1/commands/stop-order", tag = "execution",
    request_body = SubmitStopOrderRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 202, body = MutationReceiptDto),
        (status = 503, body = ApiError),
    )
)]
pub async fn submit_stop_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitStopOrderRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    if request.instrument_id.trim().is_empty() || request.quantity_lots <= 0 {
        return Err(ApiError::validation(
            "the stop command is not valid",
            vec![FieldError {
                field: "quantity_lots".to_owned(),
                message: "instrument_id must be set and quantity_lots must be positive".to_owned(),
            }],
        ));
    }
    Ok(MutationHttpResponse(
        state.execution_port()?.submit_stop_order(request).await?,
    ))
}

/// Cancel a stop order. Exactly one target identity.
#[utoipa::path(
    post, path = "/api/v1/commands/cancel-stop-order", tag = "execution",
    request_body = CancelOrderRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 202, body = MutationReceiptDto),
        (status = 400, body = ApiError),
        (status = 503, body = ApiError),
    )
)]
pub async fn cancel_stop_order(
    State(state): State<AppState>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    request.target()?;
    Ok(MutationHttpResponse(
        state.execution_port()?.cancel_stop_order(request).await?,
    ))
}

/// Establish protection legs on a position. Not a bulk migration.
#[utoipa::path(
    post, path = "/api/v1/commands/protection", tag = "execution",
    request_body = SubmitProtectionRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 202, body = MutationReceiptDto),
        (status = 503, body = ApiError),
    )
)]
pub async fn submit_protection(
    State(state): State<AppState>,
    Json(request): Json<SubmitProtectionRequest>,
) -> Result<MutationHttpResponse, ApiError> {
    if request.instrument_id.trim().is_empty() {
        return Err(ApiError::validation(
            "protection needs the instrument it protects",
            vec![FieldError {
                field: "instrument_id".to_owned(),
                message: "must not be empty".to_owned(),
            }],
        ));
    }
    Ok(MutationHttpResponse(
        state.execution_port()?.submit_protection(request).await?,
    ))
}

/// HTTP envelope for a mutation receipt. UNKNOWN after dispatch is 202 with the receipt
/// body, never an `ApiError`.
pub struct MutationHttpResponse(pub MutationReceiptDto);

impl IntoResponse for MutationHttpResponse {
    fn into_response(self) -> Response {
        let status = if self.0.state == JournalStateDto::UnknownAfterDispatch {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        };
        (status, Json(self.0)).into_response()
    }
}

fn validate_submit(request: &SubmitOrderRequest) -> Result<(), ApiError> {
    let mut errors = Vec::new();
    if request.quantity_lots <= 0 {
        errors.push(FieldError {
            field: "quantity_lots".to_owned(),
            message: "must be a positive number of lots".to_owned(),
        });
    }
    if request.client_request_id.trim().is_empty() {
        errors.push(FieldError {
            field: "client_request_id".to_owned(),
            message: "must not be empty: it is the identity used for reconciliation".to_owned(),
        });
    }
    if request.instrument_id.trim().is_empty() {
        errors.push(FieldError {
            field: "instrument_id".to_owned(),
            message: "must not be empty".to_owned(),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::validation(
            "the order command is not valid",
            errors,
        ))
    }
}

/// Which instrument, in whose namespace.
///
/// A uid is only an identity beside its provider, so both are required. There is no
/// Vox-minted instrument id: the domain identity is the identity.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct InstrumentQuery {
    pub provider: ProviderDto,
    /// The provider's stable instrument identifier.
    pub instrument_uid: String,
}

impl InstrumentQuery {
    fn validated(&self) -> Result<(), ApiError> {
        if self.instrument_uid.trim().is_empty() {
            return Err(ApiError::validation(
                "a market-data read needs the instrument it reads",
                vec![FieldError {
                    field: "instrument_uid".to_owned(),
                    message: "must not be empty".to_owned(),
                }],
            ));
        }
        Ok(())
    }
}

/// Instrument catalogue search.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct InstrumentSearchQuery {
    pub provider: ProviderDto,
    /// Ticker or name fragment.
    pub query: String,
    /// Page size; the server caps it.
    pub limit: Option<u16>,
}

/// Book depth request.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct OrderBookQuery {
    pub provider: ProviderDto,
    pub instrument_uid: String,
    /// Levels per side; the server caps it at what the provider serves.
    pub depth: Option<u16>,
}

/// Tape request.
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct TradesQuery {
    pub provider: ProviderDto,
    pub instrument_uid: String,
    pub limit: Option<u16>,
}

/// Candle request. The window is explicit: a chart never asks for "recent".
#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct CandlesQuery {
    pub provider: ProviderDto,
    pub instrument_uid: String,
    pub interval: CandleIntervalDto,
    /// Window start, milliseconds since the Unix epoch, UTC.
    pub from_unix_ms: i64,
    /// Window end, milliseconds since the Unix epoch, UTC.
    pub to_unix_ms: i64,
}

/// Search the instrument catalogue.
#[utoipa::path(
    get, path = "/api/v1/market/instruments", tag = "market", params(InstrumentSearchQuery),
    responses(
        (status = 200, body = Vec<InstrumentSummaryDto>),
        (status = 503, description = "No market-data projection is attached", body = ApiError),
    )
)]
pub async fn instruments(
    State(state): State<AppState>,
    Query(query): Query<InstrumentSearchQuery>,
) -> Result<Json<Vec<InstrumentSummaryDto>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    Ok(Json(
        state
            .market_data_port()?
            .search_instruments(query.provider, &query.query, limit)
            .await?,
    ))
}

/// Last price and top of book, with the age of the record.
#[utoipa::path(
    get, path = "/api/v1/market/quote", tag = "market", params(InstrumentQuery),
    responses((status = 200, body = QuoteDto), (status = 400, body = ApiError), (status = 503, body = ApiError))
)]
pub async fn quote(
    State(state): State<AppState>,
    Query(query): Query<InstrumentQuery>,
) -> Result<Json<QuoteDto>, ApiError> {
    query.validated()?;
    Ok(Json(
        state
            .market_data_port()?
            .quote(query.provider, &query.instrument_uid)
            .await?,
    ))
}

/// A book snapshot.
#[utoipa::path(
    get, path = "/api/v1/market/order-book", tag = "market", params(OrderBookQuery),
    responses((status = 200, body = OrderBookDto), (status = 400, body = ApiError), (status = 503, body = ApiError))
)]
pub async fn order_book(
    State(state): State<AppState>,
    Query(query): Query<OrderBookQuery>,
) -> Result<Json<OrderBookDto>, ApiError> {
    let depth = query.depth.unwrap_or(20).clamp(1, 50);
    Ok(Json(
        state
            .market_data_port()?
            .order_book(query.provider, &query.instrument_uid, depth)
            .await?,
    ))
}

/// The public tape.
#[utoipa::path(
    get, path = "/api/v1/market/trades", tag = "market", params(TradesQuery),
    responses((status = 200, body = Vec<TradeTickDto>), (status = 503, body = ApiError))
)]
pub async fn trades(
    State(state): State<AppState>,
    Query(query): Query<TradesQuery>,
) -> Result<Json<Vec<TradeTickDto>>, ApiError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    Ok(Json(
        state
            .market_data_port()?
            .trades(query.provider, &query.instrument_uid, limit)
            .await?,
    ))
}

/// Historic vs streaming provenance for every public candle interval.
///
/// The UI uses this list instead of guessing. Intervals the provider does not name never
/// appear; an unknown integer on a candle query fails as `UNSUPPORTED_CANDLE_INTERVAL`.
#[utoipa::path(
    get, path = "/api/v1/market/candle-intervals", tag = "market",
    responses((status = 200, description = "Named intervals with historic/stream flags", body = Vec<CandleIntervalCapability>))
)]
pub async fn candle_intervals() -> Json<Vec<CandleIntervalCapability>> {
    Json(
        CandleIntervalDto::ALL
            .into_iter()
            .map(CandleIntervalDto::capability)
            .collect(),
    )
}

/// Candles for one interval and window.
#[utoipa::path(
    get, path = "/api/v1/market/candles", tag = "market", params(CandlesQuery),
    responses((status = 200, body = CandlesDto), (status = 400, body = ApiError), (status = 503, body = ApiError))
)]
pub async fn candles(
    State(state): State<AppState>,
    Query(query): Query<CandlesQuery>,
) -> Result<Json<CandlesDto>, ApiError> {
    if query.to_unix_ms <= query.from_unix_ms {
        return Err(ApiError::validation(
            "the candle window is empty",
            vec![FieldError {
                field: "to_unix_ms".to_owned(),
                message: "must be later than from_unix_ms".to_owned(),
            }],
        ));
    }
    Ok(Json(
        state
            .market_data_port()?
            .candles(
                query.provider,
                &query.instrument_uid,
                query.interval,
                query.from_unix_ms,
                query.to_unix_ms,
            )
            .await?,
    ))
}

/// Venue session state for one instrument.
#[utoipa::path(
    get, path = "/api/v1/market/session", tag = "market", params(InstrumentQuery),
    responses((status = 200, body = SessionDto), (status = 400, body = ApiError), (status = 503, body = ApiError))
)]
pub async fn session(
    State(state): State<AppState>,
    Query(query): Query<InstrumentQuery>,
) -> Result<Json<SessionDto>, ApiError> {
    query.validated()?;
    Ok(Json(
        state
            .market_data_port()?
            .session(query.provider, &query.instrument_uid)
            .await?,
    ))
}

/// Anything outside the versioned surface.
pub async fn not_found() -> ApiError {
    ApiError::new(
        ErrorCategory::NotFound,
        "ROUTE_NOT_FOUND",
        "no such route in this API version",
    )
}

/// The versioned REST router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v1/system/health", get(system_health))
        .route("/api/v1/auth/session", post(create_session))
        .route("/api/v1/capabilities", get(capabilities))
        .route(
            "/api/v1/broker-connections",
            get(broker_connections).post(create_broker_connection),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}",
            get(broker_connection_details).delete(delete_broker_connection),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/validate",
            post(validate_broker_connection),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/credential",
            put(rotate_broker_credential),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/disable",
            post(disable_broker_connection),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/accounts",
            get(discovered_broker_accounts),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/bindings",
            get(broker_account_bindings).post(bind_broker_account),
        )
        .route(
            "/api/v1/broker-connections/{connection_id}/execution-authorization",
            put(change_execution_authorization),
        )
        .route(
            "/api/v1/broker-bindings/{binding_id}",
            delete(unbind_broker_account),
        )
        .route("/api/v1/runtime", get(runtime_health))
        .route("/api/v1/runtime/scopes", get(runtime_scopes))
        .route("/api/v1/reconciliation", get(reconciliation))
        .route("/api/v1/accounts", get(accounts))
        .route("/api/v1/portfolio", get(portfolio))
        .route("/api/v1/positions", get(positions))
        .route("/api/v1/orders", get(orders))
        .route("/api/v1/stop-orders", get(stop_orders))
        .route("/api/v1/operations", get(operations))
        .route("/api/v1/mutations", get(mutations))
        .route("/api/v1/commands/order", post(submit_order))
        .route("/api/v1/commands/cancel-order", post(cancel_order))
        .route("/api/v1/commands/replace-order", post(replace_order))
        .route("/api/v1/commands/stop-order", post(submit_stop_order))
        .route(
            "/api/v1/commands/cancel-stop-order",
            post(cancel_stop_order),
        )
        .route("/api/v1/commands/protection", post(submit_protection))
        .route("/api/v1/market/instruments", get(instruments))
        .route("/api/v1/market/quote", get(quote))
        .route("/api/v1/market/order-book", get(order_book))
        .route("/api/v1/market/trades", get(trades))
        .route("/api/v1/market/candle-intervals", get(candle_intervals))
        .route("/api/v1/market/candles", get(candles))
        .route("/api/v1/market/session", get(session))
        .with_state(state)
}

pub(crate) fn now_unix_ms() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::execution::{MutationDecisionDto, MutationKindDto};
    use crate::contract::scope::TradingMode;
    use crate::error::ErrorCategory;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn candle_interval_capabilities_distinguish_historic_only_seconds() {
        let Json(list) = candle_intervals().await;
        assert_eq!(list.len(), 16);
        let five = list
            .iter()
            .find(|row| row.interval == CandleIntervalDto::FiveSeconds)
            .expect("5s");
        assert!(five.historical_supported);
        assert!(!five.streaming_supported);
        let minute = list
            .iter()
            .find(|row| row.interval == CandleIntervalDto::OneMinute)
            .expect("1m");
        assert!(minute.historical_supported);
        assert!(minute.streaming_supported);
    }

    #[tokio::test]
    async fn connection_administration_requires_authenticated_actor() {
        let state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
        let error = broker_connections(State(state), None)
            .await
            .expect_err("anonymous connection metadata must fail closed");
        assert_eq!(error.category, ErrorCategory::Authentication);
        assert_eq!(error.code, "AUTHENTICATED_ACTOR_REQUIRED");
    }

    fn sample_scope() -> Result<ExecutionScope, crate::contract::scope::ScopeError> {
        ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:primary",
            "account:primary",
            TradingMode::Live,
        )
    }

    #[test]
    fn unknown_after_dispatch_is_accepted_as_a_receipt_not_an_api_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let receipt = MutationReceiptDto::unknown_after_dispatch(
            "req-1",
            sample_scope()?,
            MutationKindDto::PostOrder,
            "corr-1",
            7,
            1,
            2,
        );
        assert_eq!(receipt.decision, MutationDecisionDto::Reconcile);
        let response = MutationHttpResponse(receipt.clone()).into_response();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let json = serde_json::to_value(&receipt)?;
        assert_eq!(json["state"], "UNKNOWN_AFTER_DISPATCH");
        assert_eq!(json["decision"], "RECONCILE");
        assert!(json.get("code").is_none());
        assert_ne!(
            ErrorCategory::UnresolvedUnknown.status(),
            StatusCode::OK,
            "the unused error category must not be how UNKNOWN is returned"
        );
        Ok(())
    }

    #[test]
    fn acknowledged_receipts_are_ok() -> Result<(), Box<dyn std::error::Error>> {
        let mut receipt = MutationReceiptDto::unknown_after_dispatch(
            "req-1",
            sample_scope()?,
            MutationKindDto::PostOrder,
            "corr-1",
            7,
            1,
            2,
        );
        receipt.state = JournalStateDto::Acknowledged;
        receipt.decision = MutationDecisionDto::DoNotSubmit;
        let response = MutationHttpResponse(receipt).into_response();
        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }
}
