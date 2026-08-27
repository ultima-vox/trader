//! REST surface under `/api/v1`.
//!
//! Handlers only translate: they take a typed request, call an application port and shape a
//! typed response. No business rule, no risk decision, no precedence arithmetic lives here.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::application::AppState;
use crate::contract::account::{
    BrokerAccountDto, OperationsPageDto, OrderDto, PortfolioDto, PositionDto, ReconciliationDto,
    StopOrderDto,
};
use crate::contract::capability::CapabilitySet;
use crate::contract::execution::{CancelOrderRequest, MutationReceiptDto, SubmitOrderRequest};
use crate::contract::market::{
    CandleIntervalDto, CandlesDto, InstrumentSummaryDto, OrderBookDto, QuoteDto, SessionDto,
    TradeTickDto,
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
    /// Broker account identifier.
    pub broker_account_id: String,
    /// Opaque connection reference. Never a credential.
    pub connection_ref: String,
}

impl ScopeQuery {
    fn into_scope(self) -> Result<ExecutionScope, ApiError> {
        if self.broker_account_id.trim().is_empty() {
            return Err(ApiError::validation(
                "the account-scoped read needs a broker account id",
                vec![FieldError {
                    field: "broker_account_id".to_owned(),
                    message: "must not be empty".to_owned(),
                }],
            ));
        }
        if self.connection_ref.trim().is_empty() {
            return Err(ApiError::validation(
                "the account-scoped read needs a connection reference",
                vec![FieldError {
                    field: "connection_ref".to_owned(),
                    message: "must not be empty".to_owned(),
                }],
            ));
        }
        Ok(ExecutionScope {
            provider: self.provider,
            environment: self.environment,
            broker_account_id: self.broker_account_id,
            connection_ref: self.connection_ref,
            trading_mode: TradingMode::Live,
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
    pub broker_account_id: String,
    pub connection_ref: String,
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
            broker_account_id: self.broker_account_id,
            connection_ref: self.connection_ref,
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

/// Runtime state, readiness and stream health.
#[utoipa::path(
    get, path = "/api/v1/runtime", tag = "runtime",
    responses(
        (status = 200, description = "Current runtime health", body = RuntimeHealthDto),
        (status = 503, description = "No runtime is attached to this process", body = ApiError),
    )
)]
pub async fn runtime_health(State(state): State<AppState>) -> Result<Json<RuntimeHealthDto>, ApiError> {
    Ok(Json(state.runtime_port()?.health().await?))
}

/// What this deployment can actually do.
#[utoipa::path(
    get, path = "/api/v1/capabilities", tag = "system",
    params(("broker_account_id" = Option<String>, Query, description = "Scope the capability set to one account")),
    responses((status = 200, description = "Capability set", body = CapabilitySet))
)]
pub async fn capabilities(
    State(state): State<AppState>,
    Query(params): Query<CapabilityQuery>,
) -> Json<CapabilitySet> {
    Json(state.capabilities(params.broker_account_id))
}

#[derive(Clone, Debug, Deserialize, IntoParams, ToSchema)]
pub struct CapabilityQuery {
    pub broker_account_id: Option<String>,
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

/// Submit a regular order. The scope in the body is the frozen target of the command.
#[utoipa::path(
    post, path = "/api/v1/commands/order", tag = "execution", request_body = SubmitOrderRequest,
    responses(
        (status = 200, description = "Mutation receipt", body = MutationReceiptDto),
        (status = 202, description = "Dispatched, outcome not yet known", body = ApiError),
        (status = 400, body = ApiError),
        (status = 403, description = "Execution is not authorized for this scope", body = ApiError),
        (status = 503, description = "No execution side is attached", body = ApiError),
    )
)]
pub async fn submit_order(
    State(state): State<AppState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<MutationReceiptDto>, ApiError> {
    validate_submit(&request)?;
    Ok(Json(state.execution_port()?.submit_order(request).await?))
}

/// Cancel a regular order.
#[utoipa::path(
    post, path = "/api/v1/commands/cancel-order", tag = "execution", request_body = CancelOrderRequest,
    responses(
        (status = 200, body = MutationReceiptDto),
        (status = 400, body = ApiError),
        (status = 503, body = ApiError),
    )
)]
pub async fn cancel_order(
    State(state): State<AppState>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<Json<MutationReceiptDto>, ApiError> {
    if request.broker_order_id.is_none() && request.logical_request_id.is_none() {
        return Err(ApiError::validation(
            "a cancel needs the order it cancels",
            vec![FieldError {
                field: "broker_order_id".to_owned(),
                message: "provide either broker_order_id or logical_request_id".to_owned(),
            }],
        ));
    }
    Ok(Json(state.execution_port()?.cancel_order(request).await?))
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
    if request.instrument_uid.trim().is_empty() {
        errors.push(FieldError {
            field: "instrument_uid".to_owned(),
            message: "must not be empty".to_owned(),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ApiError::validation("the order command is not valid", errors))
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
        state.market_data_port()?.quote(query.provider, &query.instrument_uid).await?,
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
        state.market_data_port()?.session(query.provider, &query.instrument_uid).await?,
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
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/runtime", get(runtime_health))
        .route("/api/v1/reconciliation", get(reconciliation))
        .route("/api/v1/accounts", get(accounts))
        .route("/api/v1/portfolio", get(portfolio))
        .route("/api/v1/positions", get(positions))
        .route("/api/v1/orders", get(orders))
        .route("/api/v1/stop-orders", get(stop_orders))
        .route("/api/v1/operations", get(operations))
        .route("/api/v1/commands/order", post(submit_order))
        .route("/api/v1/commands/cancel-order", post(cancel_order))
        .route("/api/v1/market/instruments", get(instruments))
        .route("/api/v1/market/quote", get(quote))
        .route("/api/v1/market/order-book", get(order_book))
        .route("/api/v1/market/trades", get(trades))
        .route("/api/v1/market/candles", get(candles))
        .route("/api/v1/market/session", get(session))
        .with_state(state)
}

pub(crate) fn now_unix_ms() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as i64 / 1_000_000
}
