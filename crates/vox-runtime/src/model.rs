use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EXECUTION_QUEUE_CAPACITY: usize = 256;
pub const STREAM_QUEUE_CAPACITY: usize = 1_024;
pub const RECONCILIATION_CONCURRENCY: usize = 8;
pub const SQLITE_CONNECTION_LIMIT: usize = 4;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Provider {
    TInvest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuntimeEnvironment {
    Sandbox,
    Production,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RuntimeScope {
    pub provider: Provider,
    pub environment: RuntimeEnvironment,
    pub broker_account_id: String,
    pub connection_ref: OpaqueRef,
    pub credential_ref: OpaqueRef,
}

impl RuntimeScope {
    pub fn new(
        provider: Provider,
        environment: RuntimeEnvironment,
        broker_account_id: impl Into<String>,
        connection_ref: OpaqueRef,
        credential_ref: OpaqueRef,
    ) -> Result<Self, ModelError> {
        let broker_account_id = required(broker_account_id.into(), "broker_account_id")?;
        Ok(Self {
            provider,
            environment,
            broker_account_id,
            connection_ref,
            credential_ref,
        })
    }

    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{:?}:{:?}:{}",
            self.provider, self.environment, self.broker_account_id
        )
    }

    #[must_use]
    pub fn redacted_account_id(&self) -> String {
        let suffix = self
            .broker_account_id
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("***{suffix}")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct OpaqueRef(String);

impl OpaqueRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelError> {
        let value = required(value.into(), "opaque_ref")?;
        if value.len() > 256
            || value.chars().any(char::is_whitespace)
            || ["Bearer", "token=", "secret=", "t."].iter().any(|needle| {
                value
                    .to_ascii_lowercase()
                    .contains(&needle.to_ascii_lowercase())
            })
        {
            return Err(ModelError::SecretLikeOpaqueReference);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for OpaqueRef {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("[opaque-ref]")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuntimeState {
    Starting,
    Connecting,
    Reconciling,
    Ready,
    Degraded,
    Halted,
    Stopping,
    Stopped,
}

impl RuntimeState {
    #[must_use]
    pub const fn new_exposure_allowed(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCode {
    Startup,
    Connecting,
    ReconciliationStarted,
    ReconciliationComplete,
    ReconciliationIncomplete,
    UnknownMutation,
    BrokerPositionConflict,
    BrokerOrderConflict,
    BrokerStopConflict,
    RequiredReadUnavailable,
    AccountUnavailable,
    CredentialRejected,
    ExecutionUnauthorized,
    StreamDisconnected,
    StreamGap,
    StreamQueueOverflow,
    OptionalCapabilityUnavailable,
    CheckpointRebuild,
    PersistenceFailure,
    OwnershipFailure,
    StaleEpoch,
    CorruptMutationEvidence,
    ShutdownRequested,
    ShutdownComplete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StateTransition {
    pub scope_key: String,
    pub from: RuntimeState,
    pub to: RuntimeState,
    pub reason_code: ReasonCode,
    pub detail: String,
    pub correlation_id: String,
    pub observed_at_unix_ms: i64,
    pub runtime_epoch: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationKind {
    PostOrder,
    PostOrderAsync,
    ReplaceOrder,
    CancelOrder,
    PostStopOrder,
    CancelStopOrder,
    ProtectionLeg,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JournalState {
    NotDispatched,
    Dispatching,
    Acknowledged,
    Rejected,
    UnknownAfterDispatch,
    Reconciled,
}

impl JournalState {
    #[must_use]
    pub const fn safety_unresolved(self) -> bool {
        matches!(self, Self::Dispatching | Self::UnknownAfterDispatch)
    }

    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Acknowledged | Self::Rejected | Self::Reconciled)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MutationRecord {
    pub scope_key: String,
    pub logical_request_id: String,
    pub kind: MutationKind,
    pub state: JournalState,
    pub redacted_request_evidence: String,
    pub broker_evidence_ref: Option<String>,
    pub correlation_id: String,
    pub reconciliation_disposition: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub runtime_epoch: u64,
}

impl MutationRecord {
    pub fn prepared(
        scope: &RuntimeScope,
        logical_request_id: impl Into<String>,
        kind: MutationKind,
        redacted_request_evidence: impl Into<String>,
        correlation_id: impl Into<String>,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            scope_key: scope.key(),
            logical_request_id: required(logical_request_id.into(), "logical_request_id")?,
            kind,
            state: JournalState::NotDispatched,
            redacted_request_evidence: validate_redacted(redacted_request_evidence.into())?,
            broker_evidence_ref: None,
            correlation_id: required(correlation_id.into(), "correlation_id")?,
            reconciliation_disposition: None,
            created_at_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            runtime_epoch,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerIdentityLinks {
    pub logical_request_id: String,
    pub broker_order_id: Option<String>,
    pub replacement_broker_order_id: Option<String>,
    pub broker_stop_order_id: Option<String>,
    pub provider_operation_ids: BTreeSet<String>,
    pub broker_fill_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerAccount {
    pub account_id: String,
    pub open: bool,
    pub accessible: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PositionFact {
    pub account_id: String,
    pub instrument_uid: String,
    pub quantity_units: i64,
    pub broker_observed_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortfolioFact {
    pub account_id: String,
    pub currencies: BTreeMap<String, String>,
    pub broker_observed_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderFact {
    pub account_id: String,
    pub broker_order_id: String,
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    pub active: bool,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StopFact {
    pub account_id: String,
    pub broker_stop_order_id: String,
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    pub active: bool,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationFact {
    pub account_id: String,
    pub cursor: String,
    pub provider_operation_id: Option<String>,
    pub broker_order_id: Option<String>,
    pub logical_request_id: Option<String>,
    pub broker_fill_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperationsPage {
    pub items: Vec<OperationFact>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerSnapshot {
    pub accounts: Vec<BrokerAccount>,
    pub portfolio: PortfolioFact,
    pub positions: Vec<PositionFact>,
    pub active_orders: Vec<OrderFact>,
    pub stop_orders: Vec<StopFact>,
    pub operations: Vec<OperationFact>,
    pub stream_evidence: Vec<BrokerEvent>,
    pub observed_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerEventClass {
    Order,
    Stop,
    Fill,
    Operation,
    Position,
    Portfolio,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerEvent {
    pub account_id: String,
    pub event_class: BrokerEventClass,
    pub stable_event_id: String,
    pub broker_order_id: Option<String>,
    pub broker_stop_order_id: Option<String>,
    pub logical_request_id: Option<String>,
    pub runtime_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconciliationCheckpoint {
    pub scope_key: String,
    pub reconciliation_id: String,
    pub operations_cursor: Option<String>,
    pub snapshot_observed_at_unix_ms: i64,
    pub completed_at_unix_ms: i64,
    pub runtime_epoch: u64,
    pub accounts_complete: bool,
    pub portfolio_complete: bool,
    pub positions_complete: bool,
    pub orders_complete: bool,
    pub stops_complete: bool,
    pub operations_complete: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DerivedPositionExpectation {
    pub scope_key: String,
    pub instrument_uid: String,
    pub expected_quantity_units: i64,
    pub based_on_logical_request_id: String,
    pub runtime_epoch: u64,
    pub updated_at_unix_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeAuditRecord {
    pub scope_key: String,
    pub runtime_epoch: u64,
    pub event_type: String,
    pub reason_code: ReasonCode,
    pub correlation_id: String,
    pub redacted_detail: String,
    pub observed_at_unix_ms: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreCounts {
    pub unresolved_unknown_count: u64,
    pub linked_open_order_count: u64,
    pub linked_active_stop_count: u64,
}

impl ReconciliationCheckpoint {
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.accounts_complete
            && self.portfolio_complete
            && self.positions_complete
            && self.orders_complete
            && self.stops_complete
            && self.operations_complete
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamKind {
    OrderState,
    Trades,
    Positions,
    Portfolio,
    Operations,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamState {
    Disconnected,
    Connecting,
    Active,
    Stale,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamHealth {
    pub stream: StreamKind,
    pub state: StreamState,
    pub queue_depth: usize,
    pub last_event_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeHealth {
    pub state: RuntimeState,
    pub reason_code: ReasonCode,
    pub reason: String,
    pub provider: Provider,
    pub environment: RuntimeEnvironment,
    pub account_display: String,
    pub runtime_epoch: u64,
    pub connected: bool,
    pub last_successful_reconciliation_at_unix_ms: Option<i64>,
    pub reconciliation_age_ms: Option<u64>,
    pub unresolved_unknown_count: u64,
    pub open_order_count: u64,
    pub active_stop_count: u64,
    pub stream_states: Vec<StreamHealth>,
    pub persistence_healthy: bool,
    pub execution_authorized: bool,
    pub new_exposure_allowed: bool,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ModelError {
    #[error("{0} cannot be empty")]
    Empty(&'static str),
    #[error("opaque reference resembles secret material")]
    SecretLikeOpaqueReference,
    #[error("request evidence must be bounded, structured and redacted")]
    UnsafeRequestEvidence,
}

fn required(value: String, field: &'static str) -> Result<String, ModelError> {
    if value.trim().is_empty() {
        Err(ModelError::Empty(field))
    } else {
        Ok(value)
    }
}

fn validate_redacted(value: String) -> Result<String, ModelError> {
    let lower = value.to_ascii_lowercase();
    if value.trim().is_empty()
        || value.len() > 2_048
        || lower.contains("authorization")
        || lower.contains("bearer ")
        || lower.contains("token=")
        || lower.contains("secret=")
    {
        return Err(ModelError::UnsafeRequestEvidence);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_refs_and_evidence_reject_secret_material() -> Result<(), ModelError> {
        assert!(OpaqueRef::new("Bearer abc").is_err());
        assert!(OpaqueRef::new("token=abc").is_err());
        let scope = RuntimeScope::new(
            Provider::TInvest,
            RuntimeEnvironment::Sandbox,
            "account-1234",
            OpaqueRef::new("connection:primary")?,
            OpaqueRef::new("credential:primary")?,
        )?;
        assert_eq!(scope.redacted_account_id(), "***1234");
        assert!(
            MutationRecord::prepared(
                &scope,
                "request-1",
                MutationKind::PostOrder,
                "authorization: Bearer abc",
                "correlation-1",
                1,
                1,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn capacities_match_bounded_resource_contract() {
        assert_eq!(EXECUTION_QUEUE_CAPACITY, 256);
        assert_eq!(STREAM_QUEUE_CAPACITY, 1_024);
        assert_eq!(RECONCILIATION_CONCURRENCY, 8);
        const { assert!(SQLITE_CONNECTION_LIMIT <= 4) };
    }

    #[test]
    fn committed_contract_and_qualification_matrices_are_complete() -> Result<(), serde_json::Error>
    {
        let contracts: serde_json::Value = serde_json::from_str(include_str!(
            "../../../qualification/tinvest_runtime_reconciliation_contracts.json"
        ))?;
        assert_eq!(
            contracts["official_contract_revision"],
            "762e720e27164213f41cac0b226c5698c2ae8199"
        );
        assert_eq!(contracts["runtime_facts"].as_array().map(Vec::len), Some(8));
        assert_eq!(
            contracts["stream_policy"]["documented_ping_interval_seconds"]["minimum"],
            5
        );

        let live: serde_json::Value = serde_json::from_str(include_str!(
            "../../../qualification/tinvest_runtime_qualification_rows.json"
        ))?;
        assert_eq!(live["rows"].as_array().map(Vec::len), Some(16));
        let chaos: serde_json::Value = serde_json::from_str(include_str!(
            "../../../qualification/tinvest_runtime_chaos_matrix.json"
        ))?;
        assert_eq!(chaos["scenarios"].as_array().map(Vec::len), Some(29));
        Ok(())
    }
}
