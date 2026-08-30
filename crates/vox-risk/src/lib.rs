//! Broker-neutral production risk boundary.
//!
//! #21 owns policy, risk decisions, reservations and risk state. Broker/runtime truth
//! remains owned by the existing provider/runtime foundations and is supplied as an
//! explicit snapshot with provenance.

pub mod engine;
pub mod model;
pub mod store;

pub use engine::{RiskEngine, RiskEngineError};
pub use model::*;
pub use store::{RiskStore, RiskStoreError, SqliteRiskStore};
