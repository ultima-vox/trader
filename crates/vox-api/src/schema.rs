//! The OpenAPI document, generated from the Rust contracts.
//!
//! There is one authoritative description of this API and it is produced from the same
//! types the handlers use. A hand-written duplicate would drift; this cannot.

use axum::routing::get;
use axum::{Json, Router};
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDoc;

use crate::contract::account::{
    BrokerAccountDto, CurrencyBalanceDto, MoneyValuationDto, OperationDto, OperationsPageDto,
    OrderDto, OrderExecutionStatusDto, PortfolioDto, PositionDto, ReconciliationDto,
    StopExecutionStatusDto, StopOrderDto,
};
use crate::contract::capability::{Capability, CapabilitySet, UnavailableCapability};
use crate::contract::execution::{
    CancelOrderRequest, JournalStateDto, MutationDecisionDto, MutationKindDto, MutationReceiptDto,
    OrderSideDto, OrderTypeDto, PriceConventionDto, ProtectionPlanDto, ProtectionStateDto,
    ReplaceOrderRequest, SubmitOrderRequest, SubmitProtectionRequest, SubmitStopOrderRequest,
    TimeInForceDto, TrailingDistanceDto, TrailingModeDto,
};
use crate::contract::instrument::InstrumentIdentityDto;
use crate::contract::market::{
    CandleDto, CandleIntervalCapability, CandleIntervalDto, CandleStateDto, CandlesDto,
    DepthLevelDto, InstrumentSummaryDto, MarketFreshness, OrderBookDto, QuoteDto, SessionDto,
    TradeDirectionDto, TradeTickDto, TradingStatusDto,
};
use crate::contract::money::Decimal;
use crate::contract::runtime::{
    ReasonCodeDto, RuntimeHealthDto, RuntimeStateDto, SafetyConditionDto, StreamHealthDto,
    StreamKindDto, StreamStateDto, SystemHealthDto,
};
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto, TradingMode};
use crate::contract::stream::{
    ClientMessage, EventPayload, ServerEvent, SubscriptionStatus, Topic,
};
use crate::error::{ApiError, ErrorCategory, FieldError};
use crate::transport::http;

/// The public API description.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Vox Trader Application API",
        version = "1.0.0",
        description = "The application boundary between the Vox backend and its clients. The browser talks to this API and never to a broker. Money crosses as exact decimal strings, never as JSON numbers. A capability without a backend owner answers CAPABILITY_UNAVAILABLE rather than a plausible success."
    ),
    servers((url = "/", description = "Same-origin deployment")),
    paths(
        http::system_health,
        http::capabilities,
        http::runtime_health,
        http::runtime_scopes,
        http::reconciliation,
        http::mutations,
        http::accounts,
        http::portfolio,
        http::positions,
        http::orders,
        http::stop_orders,
        http::operations,
        http::submit_order,
        http::cancel_order,
        http::replace_order,
        http::submit_stop_order,
        http::cancel_stop_order,
        http::submit_protection,
        http::instruments,
        http::quote,
        http::order_book,
        http::trades,
        http::candles,
        http::session,
        http::candle_intervals,
    ),
    components(schemas(
        ApiError, ErrorCategory, FieldError,
        SystemHealthDto, RuntimeHealthDto, RuntimeStateDto, ReasonCodeDto, SafetyConditionDto,
        StreamHealthDto, StreamKindDto, StreamStateDto,
        ExecutionScope, ProviderDto, BrokerEnvironment, TradingMode,
        BrokerAccountDto, PortfolioDto, CurrencyBalanceDto, MoneyValuationDto, PositionDto,
        OrderDto, OrderExecutionStatusDto, StopOrderDto, StopExecutionStatusDto,
        OperationDto, OperationsPageDto, ReconciliationDto,
        SubmitOrderRequest, CancelOrderRequest, ReplaceOrderRequest, SubmitStopOrderRequest,
        SubmitProtectionRequest, MutationReceiptDto, MutationKindDto,
        JournalStateDto, MutationDecisionDto, OrderSideDto, OrderTypeDto, TimeInForceDto,
        PriceConventionDto, ProtectionPlanDto, ProtectionStateDto, TrailingDistanceDto,
        TrailingModeDto, Decimal,
        Capability, CapabilitySet, UnavailableCapability,
        InstrumentIdentityDto, InstrumentSummaryDto, MarketFreshness, TradingStatusDto, SessionDto,
        QuoteDto, DepthLevelDto, OrderBookDto, TradeDirectionDto, TradeTickDto, CandleIntervalDto,
        CandleIntervalCapability, CandleStateDto, CandleDto, CandlesDto,
        ClientMessage, ServerEvent, EventPayload, Topic, SubscriptionStatus,
    )),
    tags(
        (name = "system", description = "Process liveness and capability discovery"),
        (name = "runtime", description = "Runtime state, readiness and reconciliation"),
        (name = "accounts", description = "Account-scoped read side"),
        (name = "execution", description = "Capital-affecting commands"),
        (name = "market", description = "Provider-neutral market data, projected over the #8 adapter layer"),
    )
)]
pub struct ApiDoc;

/// The document as a value.
#[must_use]
pub fn openapi() -> OpenApiDoc {
    ApiDoc::openapi()
}

/// The document as deterministic pretty JSON, for serving and for the committed artefact.
///
/// # Errors
/// Fails only if the generated document cannot be serialized, which would be a bug.
pub fn openapi_json() -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&openapi())
}

/// Serves the document beside the API it describes.
pub fn router() -> Router {
    Router::new().route("/api/v1/openapi.json", get(|| async { Json(openapi()) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic() -> Result<(), serde_json::Error> {
        assert_eq!(openapi_json()?, openapi_json()?);
        Ok(())
    }

    #[test]
    fn every_route_is_versioned() {
        for path in openapi().paths.paths.keys() {
            assert!(path.starts_with("/api/v1/"), "unversioned route: {path}");
        }
    }

    #[test]
    fn no_provider_wire_type_reaches_the_public_schema() -> Result<(), serde_json::Error> {
        let json = openapi_json()?.to_lowercase();
        for forbidden in ["protobuf", "prost", "tinkoff", "grpc"] {
            assert!(
                !json.contains(forbidden),
                "provider wire vocabulary leaked: {forbidden}"
            );
        }
        Ok(())
    }

    #[test]
    fn money_is_a_string_schema_not_a_number() -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let decimal = &doc["components"]["schemas"]["Decimal"];
        assert_eq!(
            decimal["type"], "string",
            "Decimal must be a string schema: {decimal}"
        );
        Ok(())
    }

    #[test]
    fn no_secret_shaped_field_is_describable() -> Result<(), serde_json::Error> {
        let json = openapi_json()?.to_lowercase();
        for forbidden in [
            "\"token\"",
            "\"secret\"",
            "\"password\"",
            "\"authorization\"",
        ] {
            assert!(
                !json.contains(forbidden),
                "a secret-shaped field is in the public schema: {forbidden}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_market_read_model_is_published_under_the_versioned_surface() {
        let doc = openapi();
        for path in [
            "/api/v1/market/instruments",
            "/api/v1/market/quote",
            "/api/v1/market/order-book",
            "/api/v1/market/trades",
            "/api/v1/market/candles",
            "/api/v1/market/candle-intervals",
            "/api/v1/market/session",
        ] {
            assert!(
                doc.paths.paths.contains_key(path),
                "missing market route: {path}"
            );
        }
    }

    #[test]
    fn candle_contract_covers_historic_five_seconds_and_explicit_state()
    -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let intervals = doc["components"]["schemas"]["CandleIntervalDto"]["enum"]
            .as_array()
            .expect("CandleIntervalDto enum");
        for required in [
            "FIVE_SECONDS",
            "TEN_SECONDS",
            "THIRTY_SECONDS",
            "TWO_MINUTES",
            "ONE_WEEK",
            "ONE_MONTH",
        ] {
            assert!(
                intervals.iter().any(|value| value == required),
                "missing interval {required}: {intervals:?}"
            );
        }
        let states = doc["components"]["schemas"]["CandleStateDto"]["enum"]
            .as_array()
            .expect("CandleStateDto enum");
        for required in ["OPEN", "CLOSED", "CORRECTED"] {
            assert!(
                states.iter().any(|value| value == required),
                "missing candle state {required}: {states:?}"
            );
        }
        let candle = &doc["components"]["schemas"]["CandleDto"]["properties"];
        assert!(candle.get("state").is_some());
        assert!(candle.get("revision").is_some());
        assert!(
            candle.get("closed").is_none(),
            "boolean closed was replaced by explicit state"
        );
        let capability = &doc["components"]["schemas"]["CandleIntervalCapability"]["properties"];
        assert!(capability.get("interval").is_some());
        assert!(capability.get("historical_supported").is_some());
        assert!(capability.get("streaming_supported").is_some());
        Ok(())
    }

    #[test]
    fn execution_commands_use_canonical_instrument_id_not_provider_uid()
    -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let submit = &doc["components"]["schemas"]["SubmitOrderRequest"]["properties"];
        assert!(
            submit.get("instrument_id").is_some(),
            "public execution identity is instrument_id"
        );
        assert!(
            submit.get("instrument_uid").is_none(),
            "provider UID must not be the public execution identity"
        );
        let scope = &doc["components"]["schemas"]["ExecutionScope"]["properties"];
        assert!(scope.get("broker_connection_id").is_some());
        assert!(scope.get("account_id").is_some());
        assert!(
            scope.get("broker_account_id").is_none(),
            "provider broker-account id is not the command target key"
        );
        assert!(
            scope.get("connection_ref").is_none(),
            "connection_ref is not the public connection identity"
        );
        Ok(())
    }

    #[test]
    fn decimal_schema_is_a_validated_string() -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let decimal = &doc["components"]["schemas"]["Decimal"];
        assert_eq!(decimal["type"], "string");
        assert!(
            decimal["pattern"]
                .as_str()
                .is_some_and(|pattern| pattern.contains(r"[0-9]{9}")),
            "Decimal must advertise the canonical nano grammar: {decimal}"
        );
        Ok(())
    }

    #[test]
    fn a_market_price_crosses_as_a_string() -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let last = &doc["components"]["schemas"]["QuoteDto"]["properties"]["last"];
        let rendered = serde_json::to_string(last)?;
        assert!(
            rendered.contains("Decimal"),
            "a quoted price must be the exact Decimal type: {rendered}"
        );
        Ok(())
    }
}
