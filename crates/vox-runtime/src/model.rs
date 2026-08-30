use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
pub use vox_domain::RuntimeExecutionCommand;
use vox_domain::{OrderSide, ProtectionLeg};

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
#[serde(try_from = "RuntimeScopeUnchecked")]
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
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

#[derive(Clone, Debug, Deserialize)]
struct RuntimeScopeUnchecked {
    provider: Provider,
    environment: RuntimeEnvironment,
    broker_account_id: String,
    connection_ref: OpaqueRef,
    credential_ref: OpaqueRef,
}

impl TryFrom<RuntimeScopeUnchecked> for RuntimeScope {
    type Error = ModelError;

    fn try_from(value: RuntimeScopeUnchecked) -> Result<Self, Self::Error> {
        Self::new(
            value.provider,
            value.environment,
            value.broker_account_id,
            value.connection_ref,
            value.credential_ref,
        )
    }
}

impl<'de> Deserialize<'de> for OpaqueRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
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

pub(crate) trait RuntimeExecutionCommandExt {
    fn kind(&self) -> MutationKind;
    fn account_id(&self) -> &str;
    fn request_identity(&self) -> Option<&str>;
    fn evidence(&self) -> MutationEvidence;
}

impl RuntimeExecutionCommandExt for RuntimeExecutionCommand {
    fn kind(&self) -> MutationKind {
        match self {
            Self::RegularOrder(_) => MutationKind::PostOrder,
            Self::PostOrderAsync(_) => MutationKind::PostOrderAsync,
            Self::ReplaceOrder(_) => MutationKind::ReplaceOrder,
            Self::CancelOrder(_) => MutationKind::CancelOrder,
            Self::PostStopOrder(_) => MutationKind::PostStopOrder,
            Self::CancelStopOrder(_) => MutationKind::CancelStopOrder,
            Self::ProtectionLeg(_) => MutationKind::ProtectionLeg,
        }
    }

    fn account_id(&self) -> &str {
        match self {
            Self::RegularOrder(command) | Self::PostOrderAsync(command) => &command.account_id,
            Self::ReplaceOrder(command) => &command.account_id,
            Self::CancelOrder(command) => &command.account_id,
            Self::PostStopOrder(command) | Self::ProtectionLeg(command) => &command.account_id,
            Self::CancelStopOrder(command) => &command.account_id,
        }
    }

    fn request_identity(&self) -> Option<&str> {
        match self {
            Self::RegularOrder(command) | Self::PostOrderAsync(command) => {
                Some(&command.client_request_id)
            }
            Self::ReplaceOrder(command) => Some(&command.replacement_request_id),
            Self::PostStopOrder(command) | Self::ProtectionLeg(command) => {
                Some(&command.client_request_id)
            }
            Self::CancelOrder(_) | Self::CancelStopOrder(_) => None,
        }
    }

    fn evidence(&self) -> MutationEvidence {
        match self {
            Self::RegularOrder(command) | Self::PostOrderAsync(command) => MutationEvidence {
                command_kind: self.kind(),
                instrument_ref: Some(command.instrument_id.clone()),
                quantity_lots: Some(command.quantity_lots),
                price_present: command.price.is_some(),
                protection_kind: None,
            },
            Self::ReplaceOrder(command) => MutationEvidence {
                command_kind: self.kind(),
                instrument_ref: None,
                quantity_lots: Some(command.quantity_lots),
                price_present: true,
                protection_kind: None,
            },
            Self::CancelOrder(_) | Self::CancelStopOrder(_) => MutationEvidence {
                command_kind: self.kind(),
                instrument_ref: None,
                quantity_lots: None,
                price_present: false,
                protection_kind: None,
            },
            Self::PostStopOrder(command) | Self::ProtectionLeg(command) => MutationEvidence {
                command_kind: self.kind(),
                instrument_ref: Some(command.instrument_id.clone()),
                quantity_lots: Some(command.quantity_lots),
                price_present: match &command.leg {
                    ProtectionLeg::StopLoss(stop) => match stop {
                        vox_domain::StopLossProtection::Fixed { .. } => true,
                        vox_domain::StopLossProtection::Trailing {
                            activation_price, ..
                        } => activation_price.is_some(),
                    },
                    ProtectionLeg::TakeProfit(take_profit) => {
                        take_profit.trigger_price.is_some() || take_profit.limit_price.is_some()
                    }
                },
                protection_kind: Some(match command.leg {
                    ProtectionLeg::StopLoss(_) => ProtectionKind::StopLoss,
                    ProtectionLeg::TakeProfit(_) => ProtectionKind::TakeProfit,
                }),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectionKind {
    StopLoss,
    TakeProfit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "MutationEvidenceUnchecked")]
pub struct MutationEvidence {
    pub command_kind: MutationKind,
    pub instrument_ref: Option<String>,
    pub quantity_lots: Option<i64>,
    pub price_present: bool,
    pub protection_kind: Option<ProtectionKind>,
}

#[derive(Clone, Debug, Deserialize)]
struct MutationEvidenceUnchecked {
    command_kind: MutationKind,
    instrument_ref: Option<String>,
    quantity_lots: Option<i64>,
    price_present: bool,
    protection_kind: Option<ProtectionKind>,
}

impl TryFrom<MutationEvidenceUnchecked> for MutationEvidence {
    type Error = ModelError;

    fn try_from(value: MutationEvidenceUnchecked) -> Result<Self, Self::Error> {
        Self {
            command_kind: value.command_kind,
            instrument_ref: value.instrument_ref,
            quantity_lots: value.quantity_lots,
            price_present: value.price_present,
            protection_kind: value.protection_kind,
        }
        .validate()
    }
}

impl MutationEvidence {
    fn validate(self) -> Result<Self, ModelError> {
        if let Some(value) = &self.instrument_ref {
            validate_safe_text(value, "instrument_ref", 256)?;
        }
        if self.quantity_lots.is_some_and(|value| value <= 0) {
            return Err(ModelError::InvalidField("quantity_lots"));
        }
        let encoded = serde_json::to_string(&self)
            .map_err(|_| ModelError::InvalidField("mutation_evidence"))?;
        validate_redacted(encoded)?;
        Ok(self)
    }
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
#[serde(try_from = "MutationRecordUnchecked")]
pub struct MutationRecord {
    pub scope_key: String,
    pub logical_request_id: String,
    pub kind: MutationKind,
    pub state: JournalState,
    pub request_evidence: MutationEvidence,
    pub broker_evidence_ref: Option<String>,
    pub correlation_id: String,
    pub reconciliation_disposition: Option<ReconciliationDisposition>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub runtime_epoch: u64,
}

impl MutationRecord {
    pub fn prepared(
        scope: &RuntimeScope,
        logical_request_id: impl Into<String>,
        kind: MutationKind,
        request_evidence: MutationEvidence,
        correlation_id: impl Into<String>,
        runtime_epoch: u64,
        now_unix_ms: i64,
    ) -> Result<Self, ModelError> {
        let request_evidence = request_evidence.validate()?;
        if request_evidence.command_kind != kind {
            return Err(ModelError::InvalidField("request_evidence.command_kind"));
        }
        Ok(Self {
            scope_key: required(scope.key(), "scope_key")?,
            logical_request_id: validate_safe_text_owned(
                logical_request_id.into(),
                "logical_request_id",
                256,
            )?,
            kind,
            state: JournalState::NotDispatched,
            request_evidence,
            broker_evidence_ref: None,
            correlation_id: validate_safe_text_owned(correlation_id.into(), "correlation_id", 256)?,
            reconciliation_disposition: None,
            created_at_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            runtime_epoch,
        })
    }

    pub(crate) fn validated(self) -> Result<Self, ModelError> {
        MutationRecordUnchecked {
            scope_key: self.scope_key,
            logical_request_id: self.logical_request_id,
            kind: self.kind,
            state: self.state,
            request_evidence: self.request_evidence,
            broker_evidence_ref: self.broker_evidence_ref,
            correlation_id: self.correlation_id,
            reconciliation_disposition: self.reconciliation_disposition,
            created_at_unix_ms: self.created_at_unix_ms,
            updated_at_unix_ms: self.updated_at_unix_ms,
            runtime_epoch: self.runtime_epoch,
        }
        .try_into()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct MutationRecordUnchecked {
    scope_key: String,
    logical_request_id: String,
    kind: MutationKind,
    state: JournalState,
    request_evidence: MutationEvidence,
    broker_evidence_ref: Option<String>,
    correlation_id: String,
    reconciliation_disposition: Option<ReconciliationDisposition>,
    created_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    runtime_epoch: u64,
}

impl TryFrom<MutationRecordUnchecked> for MutationRecord {
    type Error = ModelError;

    fn try_from(value: MutationRecordUnchecked) -> Result<Self, Self::Error> {
        let scope_key = validate_safe_text_owned(value.scope_key, "scope_key", 1_024)?;
        let logical_request_id =
            validate_safe_text_owned(value.logical_request_id, "logical_request_id", 256)?;
        let correlation_id = validate_safe_text_owned(value.correlation_id, "correlation_id", 256)?;
        let request_evidence = value.request_evidence.validate()?;
        if request_evidence.command_kind != value.kind {
            return Err(ModelError::InvalidField("request_evidence.command_kind"));
        }
        Ok(Self {
            scope_key,
            logical_request_id,
            kind: value.kind,
            state: value.state,
            request_evidence,
            broker_evidence_ref: value
                .broker_evidence_ref
                .map(|evidence| validate_safe_text_owned(evidence, "broker_evidence_ref", 2_048))
                .transpose()?,
            correlation_id,
            reconciliation_disposition: value.reconciliation_disposition,
            created_at_unix_ms: value.created_at_unix_ms,
            updated_at_unix_ms: value.updated_at_unix_ms,
            runtime_epoch: value.runtime_epoch,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReconciliationDisposition {
    OrderAccepted,
    OrderActiveNew,
    OrderActivePartial,
    OrderFilled,
    OrderCancelled,
    OrderRejected,
    StopActive,
    StopExecuted,
    StopCanceled,
    StopExpired,
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
    pub total_portfolio_valuation: Option<MoneyFact>,
    pub total_currency_valuation: Option<MoneyFact>,
    pub cash_balances: BTreeMap<String, String>,
    pub broker_observed_at_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PositionsFact {
    pub instruments: Vec<PositionFact>,
    pub cash_balances: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MoneyFact {
    pub currency: String,
    pub amount_nanos: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "wire_value",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum OrderExecutionStatus {
    New,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    UnknownProviderStatus(i32),
}

impl OrderExecutionStatus {
    #[must_use]
    pub const fn active(self) -> bool {
        matches!(self, Self::New | Self::PartiallyFilled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    content = "wire_value",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum StopExecutionStatus {
    Active,
    Executed,
    Canceled,
    Expired,
    UnknownProviderStatus(i32),
}

impl StopExecutionStatus {
    #[must_use]
    pub const fn active(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderStatusCause {
    pub code: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrderFact {
    pub account_id: String,
    pub broker_order_id: String,
    pub logical_request_id: Option<String>,
    pub instrument_uid: String,
    /// Provider-authoritative direction. None means the provider value was unknown and
    /// directional exposure must fail closed rather than being guessed.
    pub side: Option<OrderSide>,
    pub lots_requested: i64,
    pub lots_executed: i64,
    pub status: OrderExecutionStatus,
    pub status_cause: Option<ProviderStatusCause>,
}

impl OrderFact {
    pub fn remaining_lots(&self) -> Result<i64, ModelError> {
        if self.lots_requested < 0
            || self.lots_executed < 0
            || self.lots_executed > self.lots_requested
        {
            return Err(ModelError::InvalidField("order lot progress"));
        }
        Ok(self.lots_requested - self.lots_executed)
    }

    pub fn signed_remaining_lots(&self) -> Result<i64, ModelError> {
        let remaining = self.remaining_lots()?;
        match self.side {
            Some(OrderSide::Buy) => Ok(remaining),
            Some(OrderSide::Sell) => remaining
                .checked_neg()
                .ok_or(ModelError::InvalidField("order lot progress")),
            None => Err(ModelError::InvalidField("order direction")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StopFact {
    pub account_id: String,
    pub broker_stop_order_id: String,
    pub instrument_uid: String,
    pub status: StopExecutionStatus,
    pub status_cause: Option<ProviderStatusCause>,
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
    pub execution_state: Option<BrokerExecutionState>,
    pub runtime_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerExecutionState {
    Order {
        status: OrderExecutionStatus,
        status_cause: Option<ProviderStatusCause>,
    },
    Stop {
        status: StopExecutionStatus,
        status_cause: Option<ProviderStatusCause>,
    },
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
    pub required_for_ready: bool,
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
    #[error("invalid {0}")]
    InvalidField(&'static str),
}

fn validate_safe_text(
    value: &str,
    field: &'static str,
    maximum_length: usize,
) -> Result<(), ModelError> {
    if value.trim().is_empty() || value.len() > maximum_length {
        return Err(ModelError::InvalidField(field));
    }
    validate_redacted(value.to_owned()).map(|_| ())
}

fn validate_safe_text_owned(
    value: String,
    field: &'static str,
    maximum_length: usize,
) -> Result<String, ModelError> {
    validate_safe_text(&value, field, maximum_length)?;
    Ok(value)
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
    fn order_fact_preserves_signed_remaining_exposure() -> Result<(), ModelError> {
        let buy = OrderFact {
            account_id: "account".into(),
            broker_order_id: "order".into(),
            logical_request_id: None,
            instrument_uid: "instrument".into(),
            side: Some(OrderSide::Buy),
            lots_requested: 10,
            lots_executed: 4,
            status: OrderExecutionStatus::PartiallyFilled,
            status_cause: None,
        };
        assert_eq!(buy.signed_remaining_lots()?, 6);

        let mut sell = buy.clone();
        sell.side = Some(OrderSide::Sell);
        assert_eq!(sell.signed_remaining_lots()?, -6);

        let mut invalid = buy;
        invalid.lots_executed = 11;
        assert!(invalid.signed_remaining_lots().is_err());
        Ok(())
    }

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
        let serialized = r#"{"command_kind":"POST_ORDER","instrument_ref":"Bearer abc","quantity_lots":1,"price_present":true,"protection_kind":null}"#;
        assert!(serde_json::from_str::<MutationEvidence>(serialized).is_err());
        assert!(
            MutationEvidence {
                command_kind: MutationKind::PostOrder,
                instrument_ref: Some("Bearer abc".into()),
                quantity_lots: Some(1),
                price_present: true,
                protection_kind: None,
            }
            .validate()
            .is_err()
        );
        assert!(serde_json::from_str::<OpaqueRef>(r#""token=abc""#).is_err());
        let _ = scope;
        Ok(())
    }

    #[test]
    fn serde_cannot_bypass_constructor_invariants() -> Result<(), Box<dyn std::error::Error>> {
        assert!(serde_json::from_str::<OpaqueRef>(r#""""#).is_err());
        assert!(serde_json::from_str::<OpaqueRef>(r#""Bearer abc""#).is_err());
        assert!(serde_json::from_str::<OpaqueRef>(r#""secret=value""#).is_err());
        let scope_json = r#"{
            "provider":"T_INVEST",
            "environment":"SANDBOX",
            "broker_account_id":"",
            "connection_ref":"connection:primary",
            "credential_ref":"credential:primary"
        }"#;
        assert!(serde_json::from_str::<RuntimeScope>(scope_json).is_err());

        let scope = RuntimeScope::new(
            Provider::TInvest,
            RuntimeEnvironment::Sandbox,
            "account-1",
            OpaqueRef::new("connection:primary")?,
            OpaqueRef::new("credential:primary")?,
        )?;
        let record = MutationRecord::prepared(
            &scope,
            "request-1",
            MutationKind::PostOrder,
            MutationEvidence {
                command_kind: MutationKind::PostOrder,
                instrument_ref: Some("instrument-1".into()),
                quantity_lots: Some(1),
                price_present: true,
                protection_kind: None,
            },
            "correlation-1",
            1,
            1,
        )?;
        let mut value = serde_json::to_value(record)?;
        value["request_evidence"]["instrument_ref"] = serde_json::json!("token=secret");
        assert!(serde_json::from_value::<MutationRecord>(value.clone()).is_err());
        value["request_evidence"]["instrument_ref"] = serde_json::json!("x".repeat(300));
        assert!(serde_json::from_value::<MutationRecord>(value).is_err());
        Ok(())
    }

    #[test]
    fn portfolio_aggregates_cash_and_unknown_statuses_remain_distinct()
    -> Result<(), serde_json::Error> {
        let portfolio = PortfolioFact {
            account_id: "account-1".into(),
            total_portfolio_valuation: Some(MoneyFact {
                currency: "RUB".into(),
                amount_nanos: "100000000000".into(),
            }),
            total_currency_valuation: Some(MoneyFact {
                currency: "RUB".into(),
                amount_nanos: "25000000000".into(),
            }),
            cash_balances: [("RUB".into(), "20000000000".into())].into_iter().collect(),
            broker_observed_at_unix_ms: Some(1),
        };
        assert_ne!(
            portfolio.total_portfolio_valuation,
            portfolio.total_currency_valuation
        );
        assert_eq!(portfolio.cash_balances["RUB"], "20000000000");
        let unknown = OrderExecutionStatus::UnknownProviderStatus(77_777);
        let decoded: OrderExecutionStatus =
            serde_json::from_str(&serde_json::to_string(&unknown)?)?;
        assert_eq!(decoded, unknown);
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
            contracts["stream_policy"]["documented_ping_interval_ms"]["TradesStream"]["minimum"],
            5_000
        );
        assert_eq!(
            contracts["stream_policy"]["documented_ping_interval_ms"]["OrderStateStream"]["minimum"],
            1_000
        );

        let live: serde_json::Value = serde_json::from_str(include_str!(
            "../../../qualification/tinvest_runtime_qualification_rows.json"
        ))?;
        assert_eq!(live["rows"].as_array().map(Vec::len), Some(16));
        let chaos: serde_json::Value = serde_json::from_str(include_str!(
            "../../../qualification/tinvest_runtime_chaos_matrix.json"
        ))?;
        assert_eq!(chaos["scenarios"].as_array().map(Vec::len), Some(31));
        Ok(())
    }
}
