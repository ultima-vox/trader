use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;

use crate::model::{
    BrokerAccount, BrokerEvent, BrokerIdentityLinks, DerivedPositionExpectation, JournalState,
    MutationRecord, OperationsPage, OrderFact, PortfolioFact, ReasonCode, ReconciliationCheckpoint,
    ReconciliationDisposition, RuntimeAuditRecord, RuntimeExecutionCommand, RuntimeHealth,
    RuntimeScope, StateTransition, StopFact, StoreCounts, StreamKind,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BrokerResultClass {
    Success,
    RateLimited,
    Credential,
    Permission,
    Transient,
    Permanent,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("broker {class:?} error in {service}/{method}: {message}")]
pub struct BrokerPortError {
    pub service: &'static str,
    pub method: &'static str,
    pub class: BrokerResultClass,
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl BrokerPortError {
    #[must_use]
    pub const fn safe_read_retryable(&self) -> bool {
        matches!(
            self.class,
            BrokerResultClass::RateLimited | BrokerResultClass::Transient
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialResolution {
    pub execution_authorized: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeExecutionPurpose {
    SandboxMutation,
    ProductionManual,
    ProductionAutomated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskAdmission {
    pub decision_id: String,
    /// Present for exposure-bearing actions. Pure cancel/protection-maintenance actions
    /// may be admitted without creating a new capacity reservation.
    pub reservation_id: Option<String>,
    pub policy_revision: u64,
    pub approved_delta_lots: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RiskDispatchOutcome {
    Acknowledged,
    Rejected,
    UnknownAfterDispatch,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RiskAdmissionError {
    #[error("risk admission denied ({code}): {message}")]
    Denied { code: String, message: String },
    #[error("risk admission is stale: {0}")]
    Stale(String),
    #[error("risk admission unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait RiskAdmissionPort: Send + Sync {
    /// Mandatory #21 boundary. This is called before runtime persists a mutation
    /// intent or the UNKNOWN_AFTER_DISPATCH fence. Any error fails closed.
    async fn admit(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        logical_request_id: &str,
    ) -> Result<RiskAdmission, RiskAdmissionError>;

    /// Persist the #21 reservation transition only after #11 durably records broker outcome.
    async fn record_dispatch_outcome(
        &self,
        _scope: &RuntimeScope,
        _logical_request_id: &str,
        _outcome: RiskDispatchOutcome,
    ) -> Result<(), RiskAdmissionError> {
        Ok(())
    }

    /// Transition a protection plan from PLANNED to SUBMITTED when a protection leg
    /// is dispatched to the broker. Called immediately after successful dispatch.
    /// `entry_reservation_id` is the #21 entry reservation whose exposure this
    /// protection covers — used to locate the protection plan without relying on
    /// the protection command's logical_request_id.
    /// `canonical_plan_id` is the #10 canonical protection plan/leg identity
    /// (the client_request_id from the ProtectionLegCommand).
    async fn transition_protection_plan_on_dispatch(
        &self,
        _scope: &RuntimeScope,
        _entry_reservation_id: &str,
        _canonical_plan_id: Option<String>,
        _now_unix_ms: i64,
    ) -> Result<(), RiskAdmissionError> {
        Ok(())
    }

    /// Transition a protection plan from PLANNED to FAILED when the broker rejects
    /// the protection leg. Called immediately after rejection.
    /// `entry_reservation_id` is the #21 entry reservation whose exposure this
    /// protection covers.
    async fn transition_protection_plan_on_reject(
        &self,
        _scope: &RuntimeScope,
        _entry_reservation_id: &str,
        _now_unix_ms: i64,
    ) -> Result<(), RiskAdmissionError> {
        Ok(())
    }

    /// Revalidate correctness-critical facts after queueing and immediately before the
    /// durable dispatch fence. Production adapters reject changed approval watermarks.
    async fn validate_before_dispatch(
        &self,
        _scope: &RuntimeScope,
        _purpose: RuntimeExecutionPurpose,
        _command: &RuntimeExecutionCommand,
        _logical_request_id: &str,
        _admission: &RiskAdmission,
    ) -> Result<(), RiskAdmissionError> {
        Ok(())
    }

    /// Consume broker-authoritative #11 reconciliation without making #11 own risk state.
    async fn reconcile(
        &self,
        _scope: &RuntimeScope,
        _report: &crate::reconcile::ReconciliationReport,
    ) -> Result<(), RiskAdmissionError> {
        Ok(())
    }
}

#[async_trait]
pub trait CredentialResolverPort: Send + Sync {
    async fn resolve(&self, scope: &RuntimeScope) -> Result<CredentialResolution, BrokerPortError>;

    async fn authorize_execution(
        &self,
        scope: &RuntimeScope,
        _purpose: RuntimeExecutionPurpose,
    ) -> Result<(), BrokerPortError> {
        if self.resolve(scope).await?.execution_authorized {
            Ok(())
        } else {
            Err(BrokerPortError {
                service: "CredentialResolver",
                method: "AuthorizeExecution",
                class: BrokerResultClass::Permission,
                message: "execution authorization disabled".to_owned(),
                retry_after: None,
            })
        }
    }
}

#[async_trait]
pub trait BrokerReadPort: Send + Sync {
    async fn accounts(&self, scope: &RuntimeScope) -> Result<Vec<BrokerAccount>, BrokerPortError>;
    async fn portfolio(&self, scope: &RuntimeScope) -> Result<PortfolioFact, BrokerPortError>;
    async fn positions(
        &self,
        scope: &RuntimeScope,
    ) -> Result<crate::model::PositionsFact, BrokerPortError>;
    async fn active_orders(&self, scope: &RuntimeScope) -> Result<Vec<OrderFact>, BrokerPortError>;
    async fn stop_orders(
        &self,
        scope: &RuntimeScope,
        include_terminal_since_unix_ms: i64,
    ) -> Result<Vec<StopFact>, BrokerPortError>;
    async fn order_state(
        &self,
        scope: &RuntimeScope,
        broker_order_id: Option<&str>,
        logical_request_id: Option<&str>,
    ) -> Result<Option<OrderFact>, BrokerPortError>;
    async fn operations_page(
        &self,
        scope: &RuntimeScope,
        cursor: Option<&str>,
        from_unix_ms: i64,
        limit: u16,
    ) -> Result<OperationsPage, BrokerPortError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionResult {
    Acknowledged {
        broker_evidence_ref: String,
        links: BrokerIdentityLinks,
    },
    Rejected {
        broker_evidence_ref: String,
    },
    UnknownAfterDispatch {
        broker_evidence_ref: Option<String>,
    },
}

#[async_trait]
pub trait ExecutionPort: Send + Sync {
    /// Exactly one transport attempt. Runtime has already durably fenced request as UNKNOWN.
    async fn dispatch_once(
        &self,
        scope: &RuntimeScope,
        purpose: RuntimeExecutionPurpose,
        command: &RuntimeExecutionCommand,
        mutation: &MutationRecord,
    ) -> Result<ExecutionResult, BrokerPortError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamSignal {
    Connected(StreamKind),
    Event(BrokerEvent),
    Gap { stream: StreamKind, reason: String },
    Disconnected { stream: StreamKind, reason: String },
}

#[async_trait]
pub trait ExecutionStreamPort: Send + Sync {
    /// Returns exact streams whose provider subscription ACKs were verified.
    /// Returning transport success without required ACKs cannot open runtime admission.
    async fn connect(
        &self,
        scope: &RuntimeScope,
        runtime_epoch: u64,
        output: mpsc::Sender<StreamSignal>,
    ) -> Result<BTreeSet<StreamKind>, BrokerPortError>;
    async fn disconnect(&self) -> Result<(), BrokerPortError>;
}

pub trait RuntimeStore: Clone + Send + Sync + 'static {
    fn acquire_ownership(
        &self,
        scope: &RuntimeScope,
        owner_id: &str,
        started_at_unix_ms: i64,
    ) -> Result<u64, StoreError>;
    fn verify_epoch(&self, scope_key: &str, runtime_epoch: u64) -> Result<(), StoreError>;
    fn release_ownership(
        &self,
        scope_key: &str,
        runtime_epoch: u64,
        clean_shutdown_at_unix_ms: i64,
    ) -> Result<(), StoreError>;
    fn record_transition(&self, transition: &StateTransition) -> Result<(), StoreError>;
    fn insert_mutation(&self, record: &MutationRecord) -> Result<(), StoreError>;
    fn mutation(
        &self,
        scope_key: &str,
        logical_request_id: &str,
    ) -> Result<Option<MutationRecord>, StoreError>;
    fn mutations(&self, scope_key: &str) -> Result<Vec<MutationRecord>, StoreError>;
    fn claim_dispatch_unknown(
        &self,
        scope_key: &str,
        logical_request_id: &str,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<MutationRecord, StoreError>;
    #[allow(clippy::too_many_arguments)]
    fn transition_mutation(
        &self,
        scope_key: &str,
        logical_request_id: &str,
        expected: &[JournalState],
        target: JournalState,
        broker_evidence_ref: Option<&str>,
        disposition: Option<&ReconciliationDisposition>,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<MutationRecord, StoreError>;
    fn unresolved_mutations(&self, scope_key: &str) -> Result<Vec<MutationRecord>, StoreError>;
    fn all_identity_links(&self, scope_key: &str) -> Result<Vec<BrokerIdentityLinks>, StoreError>;
    fn upsert_identity_links(
        &self,
        scope_key: &str,
        links: &BrokerIdentityLinks,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<(), StoreError>;
    fn load_checkpoint(
        &self,
        scope_key: &str,
    ) -> Result<Option<ReconciliationCheckpoint>, StoreError>;
    fn discard_checkpoint(&self, scope_key: &str, runtime_epoch: u64) -> Result<(), StoreError>;
    fn commit_reconciliation(
        &self,
        checkpoint: &ReconciliationCheckpoint,
        resolved: &[MutationRecord],
        links: &[BrokerIdentityLinks],
        readiness_state: crate::model::RuntimeState,
        reason_code: ReasonCode,
    ) -> Result<(), StoreError>;
    fn record_broker_event(
        &self,
        scope_key: &str,
        event: &BrokerEvent,
        first_seen_at_unix_ms: i64,
    ) -> Result<bool, StoreError>;
    fn expected_positions(
        &self,
        scope_key: &str,
    ) -> Result<Vec<DerivedPositionExpectation>, StoreError>;
    fn set_expected_position(
        &self,
        expectation: &DerivedPositionExpectation,
    ) -> Result<(), StoreError>;
    fn append_audit(&self, record: &RuntimeAuditRecord) -> Result<(), StoreError>;
    fn compact(
        &self,
        scope_key: &str,
        retain_after_unix_ms: i64,
        max_events: u32,
        max_audit: u32,
    ) -> Result<(), StoreError>;
    fn counts(&self, scope_key: &str) -> Result<StoreCounts, StoreError>;
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum StoreError {
    #[error("runtime ownership unavailable")]
    OwnershipUnavailable,
    #[error("stale runtime epoch")]
    StaleEpoch,
    #[error("duplicate logical mutation identity")]
    DuplicateMutation,
    #[error("invalid mutation transition")]
    InvalidMutationTransition,
    #[error("runtime store corruption: {0}")]
    Corrupt(String),
    #[error("unsupported runtime schema version {0}")]
    UnsupportedSchema(u32),
    #[error("runtime persistence failure: {0}")]
    Persistence(String),
    #[error("blocking store task failed: {0}")]
    BlockingTask(String),
}

#[async_trait]
pub trait HealthReadPort: Send + Sync {
    async fn health(&self) -> RuntimeHealth;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MetricName {
    RuntimeState,
    RuntimeReady,
    ReconciliationTotal,
    ReconciliationDurationSeconds,
    UnresolvedUnknown,
    BrokerRequestsTotal,
    BrokerRequestDurationSeconds,
    BrokerRateLimitedTotal,
    StreamConnections,
    StreamReconnectTotal,
    StreamQueueDepth,
    StreamDroppedTotal,
    RuntimeCommandQueueDepth,
    PersistenceOperationDurationSeconds,
    PersistenceErrorsTotal,
    EventDeduplicatedTotal,
    RuntimeEpochChanges,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MetricLabel {
    Result(BrokerResultClass),
    Stream(StreamKind),
    Reason(ReasonCode),
    Operation(StoreOperation),
    EventClass(crate::model::BrokerEventClass),
    Service(BrokerService),
    Method(BrokerMethod),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StoreOperation {
    Open,
    Acquire,
    Mutation,
    Reconcile,
    Event,
    Audit,
    Compact,
    Shutdown,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BrokerMethod {
    GetAccounts,
    GetPortfolio,
    GetPositions,
    GetOrders,
    GetOrderState,
    GetStopOrders,
    GetOperationsByCursor,
    StreamConnect,
    Mutation,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BrokerService {
    Users,
    Operations,
    Orders,
    StopOrders,
    OrdersStream,
    Execution,
}

impl BrokerMethod {
    #[must_use]
    pub const fn service(self) -> BrokerService {
        match self {
            Self::GetAccounts => BrokerService::Users,
            Self::GetPortfolio | Self::GetPositions | Self::GetOperationsByCursor => {
                BrokerService::Operations
            }
            Self::GetOrders | Self::GetOrderState => BrokerService::Orders,
            Self::GetStopOrders => BrokerService::StopOrders,
            Self::StreamConnect => BrokerService::OrdersStream,
            Self::Mutation => BrokerService::Execution,
        }
    }
}

pub trait MetricsPort: Send + Sync {
    fn set_gauge(&self, metric: MetricName, labels: &[MetricLabel], value: f64);
    fn increment(&self, metric: MetricName, labels: &[MetricLabel], amount: u64);
    fn observe_seconds(&self, metric: MetricName, labels: &[MetricLabel], value: f64);
}
