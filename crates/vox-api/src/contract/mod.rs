//! Public request, response and event contracts.
//!
//! Everything the browser can see is defined here. Provider wire types never appear: the
//! adapters keep their protobuf to themselves, and this layer speaks only in canonical
//! domain and runtime vocabulary.

pub mod account;
pub mod capability;
pub mod connections;
pub mod execution;
pub mod instrument;
pub mod market;
pub mod money;
pub mod risk;
pub mod runtime;
pub mod scope;
pub mod stream;
