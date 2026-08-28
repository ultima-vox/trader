//! The Vox application API.
//!
//! This crate is the boundary between the Vox backend and every client. It owns transport
//! and public contracts; it owns no business rule. Broker adapters stay behind the
//! application ports in [`application`], and no provider wire type is reachable from here.
//!
//! Three invariants hold everywhere in this crate:
//!
//! 1. money and quantities cross as exact decimal strings, never as JSON numbers;
//! 2. a capability without a backend owner answers `CAPABILITY_UNAVAILABLE`, never a
//!    plausible success;
//! 3. every capital-affecting command carries its immutable target scope and a logical
//!    request identity, and returns a receipt whose state comes from the runtime.
#![forbid(unsafe_code)]
#![allow(
    clippy::result_large_err,
    reason = "ApiError is the public envelope: it carries the code, category, correlation id and field errors by value so a handler returns one thing, not a pointer to it"
)]

pub mod application;
pub mod binding;
pub mod contract;
pub mod error;
pub mod runtime_attach;
pub mod schema;
pub mod transport;

pub use application::AppState;
pub use binding::{
    AccountBinding, AccountBindingResolver, StaticAccountBindingResolver,
    broker_connection_id_from_connection_ref, connection_ref_from_broker_connection_id,
};
pub use runtime_attach::{
    AccountReadAdapter, ProcessRuntime, RuntimeHealthAdapter, runtime_scope_from_binding,
};
pub use transport::router;
