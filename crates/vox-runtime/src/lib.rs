//! Restart-safe broker-authoritative trading runtime.
//!
//! Broker unary reads remain authoritative. Local SQLite persists only Vox-owned
//! mutation uncertainty, typed identity links, fencing epochs, checkpoints,
//! bounded dedupe evidence, audit and derived readiness observations.

pub mod coordinator;
pub mod metrics;
pub mod model;
pub mod policy;
pub mod ports;
pub mod reconcile;
pub mod store;

pub use coordinator::{DispatchReceipt, RuntimeConfig, RuntimeCoordinator, RuntimeError};
pub use metrics::{InMemoryMetrics, MetricValue};
pub use model::*;
pub use policy::{PolicyDecision, RuntimeStateMachine, SafetyCondition, readiness_policy};
pub use ports::*;
pub use reconcile::{Reconciler, ReconciliationConfig, ReconciliationError, ReconciliationReport};
pub use store::{SqliteConfiguration, SqliteRuntimeStore};
