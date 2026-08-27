//! The OpenAPI document, generated from the Rust contracts.
//!
//! There is one authoritative description of this API and it is produced from the same
//! types the handlers use. A hand-written duplicate would drift; this cannot.

use axum::routing::get;
use axum::{Json, Router};
use utoipa::openapi::OpenApi as OpenApiDoc;
use utoipa::OpenApi;

use crate::contract::account::{
    BrokerAccountDto, CurrencyBalanceDto, OperationDto, OperationsPageDto, OrderDto, PortfolioDto,
    PositionDto, ReconciliationDto, StopOrderDto,
};
use crate::contract::capability::{Capability, CapabilitySet, UnavailableCapability};
use crate::contract::execution::{
    CancelOrderRequest, JournalStateDto, MutationDecisionDto, MutationKindDto, MutationReceiptDto,
    OrderSideDto, OrderTypeDto, PriceConventionDto, ProtectionPlanDto, ProtectionStateDto,
    SubmitOrderRequest, TimeInForceDto, TrailingDistanceDto, TrailingModeDto,
};
use crate::contract::money::Decimal;
use crate::contract::runtime::{
    ReasonCodeDto, RuntimeHealthDto, RuntimeStateDto, SafetyConditionDto, StreamHealthDto,
    StreamKindDto, StreamStateDto, SystemHealthDto,
};
use crate::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto, TradingMode};
use crate::contract::stream::{ClientMessage, EventPayload, ServerEvent, SubscriptionStatus, Topic};
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
        http::reconciliation,
        http::accounts,
        http::portfolio,
        http::positions,
        http::orders,
        http::stop_orders,
        http::operations,
        http::submit_order,
        http::cancel_order,
    ),
    components(schemas(
        ApiError, ErrorCategory, FieldError,
        SystemHealthDto, RuntimeHealthDto, RuntimeStateDto, ReasonCodeDto, SafetyConditionDto,
        StreamHealthDto, StreamKindDto, StreamStateDto,
        ExecutionScope, ProviderDto, BrokerEnvironment, TradingMode,
        BrokerAccountDto, PortfolioDto, CurrencyBalanceDto, PositionDto, OrderDto, StopOrderDto,
        OperationDto, OperationsPageDto, ReconciliationDto,
        SubmitOrderRequest, CancelOrderRequest, MutationReceiptDto, MutationKindDto,
        JournalStateDto, MutationDecisionDto, OrderSideDto, OrderTypeDto, TimeInForceDto,
        PriceConventionDto, ProtectionPlanDto, ProtectionStateDto, TrailingDistanceDto,
        TrailingModeDto, Decimal,
        Capability, CapabilitySet, UnavailableCapability,
        ClientMessage, ServerEvent, EventPayload, Topic, SubscriptionStatus,
    )),
    tags(
        (name = "system", description = "Process liveness and capability discovery"),
        (name = "runtime", description = "Runtime state, readiness and reconciliation"),
        (name = "accounts", description = "Account-scoped read side"),
        (name = "execution", description = "Capital-affecting commands"),
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
            assert!(!json.contains(forbidden), "provider wire vocabulary leaked: {forbidden}");
        }
        Ok(())
    }

    #[test]
    fn money_is_a_string_schema_not_a_number() -> Result<(), serde_json::Error> {
        let doc: serde_json::Value = serde_json::from_str(&openapi_json()?)?;
        let decimal = &doc["components"]["schemas"]["Decimal"];
        assert_eq!(decimal["type"], "string", "Decimal must be a string schema: {decimal}");
        Ok(())
    }

    #[test]
    fn no_secret_shaped_field_is_describable() -> Result<(), serde_json::Error> {
        let json = openapi_json()?.to_lowercase();
        for forbidden in ["\"token\"", "\"secret\"", "\"password\"", "\"authorization\""] {
            assert!(!json.contains(forbidden), "a secret-shaped field is in the public schema: {forbidden}");
        }
        Ok(())
    }
}
