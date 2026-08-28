//! Transport: axum HTTP handlers and the WebSocket gateway.

pub mod http;
pub mod ws;

use axum::Router;

use crate::application::AppState;

/// The whole public surface: REST, the stream gateway and the OpenAPI document.
pub fn router(state: AppState) -> Router {
    http::router(state.clone())
        .merge(ws::router(state))
        .merge(crate::schema::router())
        .fallback(http::not_found)
}
