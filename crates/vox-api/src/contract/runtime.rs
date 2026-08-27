//! Runtime, readiness and stream health as the frontend sees them.
//!
//! Every enum here mirrors a canonical runtime enum. The conversions are exhaustive
//! `match`es, so adding a variant upstream fails this crate to compile rather than silently
//! dropping a state, and the tests assert that both sides serialize to the same spelling.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_domain::ReadinessState;

use super::scope::{BrokerEnvironment, ProviderDto};

/// Runtime lifecycle. New exposure is permitted in `READY` only.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RuntimeStateDto {
    Starting,
    Connecting,
    Reconciling,
    Ready,
    Degraded,
    Halted,
    Stopping,
    Stopped,
}

impl From<ReadinessState> for RuntimeStateDto {
    /// The six states the domain readiness model defines today. `STOPPING` and `STOPPED`
    /// exist in the runtime contract owned by #11 and arrive with its adapter.
    fn from(value: ReadinessState) -> Self {
        match value {
            ReadinessState::Starting => Self::Starting,
            ReadinessState::Connecting => Self::Connecting,
            ReadinessState::Reconciling => Self::Reconciling,
            ReadinessState::Ready => Self::Ready,
            ReadinessState::Degraded => Self::Degraded,
            ReadinessState::Halted => Self::Halted,
        }
    }
}

/// Why the runtime is in its current state. The only reason vocabulary that exists.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReasonCodeDto {
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

/// Why new exposure is blocked, when it is. Authoritative safety facts until #21 lands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SafetyConditionDto {
    StartupBeforeReconciliation,
    UnresolvedUnknownMutation,
    PositionConflict,
    OrderIdentityConflict,
    StopIdentityConflict,
    RequiredReadUnavailable,
    AccountUnavailable,
    CredentialInvalid,
    ExecutionAuthorizationDisabled,
    ProductionStreamDisconnectedAfterUnaryRecovery,
    OptionalAnalyticsUnavailable,
    CheckpointCorruptSnapshotAvailable,
    PersistenceFailure,
    OwnershipFailure,
    Clear,
}

/// Which broker stream a health record describes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamKindDto {
    OrderState,
    Trades,
    Positions,
    Portfolio,
    Operations,
}

/// Stream connectivity. `STALE` here is a stream fact, never a protection lifecycle state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StreamStateDto {
    Disconnected,
    Connecting,
    Active,
    Stale,
    Failed,
}

/// Health of one broker stream, including how old its last event is.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct StreamHealthDto {
    pub stream: StreamKindDto,
    pub state: StreamStateDto,
    pub queue_depth: u32,
    /// Event time of the last message, in milliseconds since the Unix epoch, UTC.
    pub last_event_at_unix_ms: Option<i64>,
}

/// Everything the shell needs to answer "can I trade right now, and if not why".
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct RuntimeHealthDto {
    pub state: RuntimeStateDto,
    pub reason_code: ReasonCodeDto,
    /// Human sentence for the operator. The code is the diagnostic, this is the explanation.
    pub reason: String,
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    /// Human account label. Raw identifiers stay in diagnostics.
    pub account_display: String,
    /// Monotonic ownership epoch. A response from a previous epoch must be discarded.
    pub runtime_epoch: u64,
    pub connected: bool,
    pub last_successful_reconciliation_at_unix_ms: Option<i64>,
    pub reconciliation_age_ms: Option<u64>,
    pub unresolved_unknown_count: u64,
    pub open_order_count: u64,
    pub active_stop_count: u64,
    pub stream_states: Vec<StreamHealthDto>,
    pub persistence_healthy: bool,
    /// Whether Vox execution is authorized for this scope. Not the same as broker permission.
    pub execution_authorized: bool,
    /// Whether new exposure may be created right now.
    pub new_exposure_allowed: bool,
}

/// Liveness of the API process itself, independent of any broker connection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct SystemHealthDto {
    /// Always `"ok"` when the process can serve requests.
    #[schema(example = "ok")]
    pub status: String,
    /// Public API version this process serves.
    #[schema(example = "v1")]
    pub api_version: String,
    /// Server time in milliseconds since the Unix epoch, UTC.
    pub server_time_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary recorded in the contract map is the vocabulary this crate serves.
    ///
    /// `vox-runtime` is not on `main` yet — it lands with #11 — so the guard against drift
    /// is the committed contract map, which was written from those Rust enums. When #11
    /// merges, its adapter converts with exhaustive matches and drift becomes a compile
    /// error instead.
    fn contract_map() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/design/BACKEND_CONTRACTS.md");
        std::fs::read_to_string(path).expect("the contract map is committed")
    }

    fn serialized<T: Serialize>(value: &T) -> String {
        serde_json::to_string(value).expect("an enum serializes").trim_matches('"').to_owned()
    }

    #[test]
    fn every_runtime_state_is_in_the_contract_map() {
        let map = contract_map();
        for state in [
            RuntimeStateDto::Starting,
            RuntimeStateDto::Connecting,
            RuntimeStateDto::Reconciling,
            RuntimeStateDto::Ready,
            RuntimeStateDto::Degraded,
            RuntimeStateDto::Halted,
            RuntimeStateDto::Stopping,
            RuntimeStateDto::Stopped,
        ] {
            let spelling = serialized(&state);
            assert!(map.contains(&spelling), "{spelling} is not in the recorded contract map");
        }
    }

    #[test]
    fn every_reason_code_is_in_the_contract_map() {
        let map = contract_map();
        for code in [
            ReasonCodeDto::Startup,
            ReasonCodeDto::ReconciliationComplete,
            ReasonCodeDto::ReconciliationIncomplete,
            ReasonCodeDto::UnknownMutation,
            ReasonCodeDto::RequiredReadUnavailable,
            ReasonCodeDto::AccountUnavailable,
            ReasonCodeDto::CredentialRejected,
            ReasonCodeDto::ExecutionUnauthorized,
            ReasonCodeDto::StreamGap,
            ReasonCodeDto::StaleEpoch,
            ReasonCodeDto::ShutdownComplete,
        ] {
            let spelling = serialized(&code);
            assert!(map.contains(&spelling), "{spelling} is not in the recorded contract map");
        }
    }

    #[test]
    fn domain_readiness_maps_into_the_runtime_vocabulary() {
        assert_eq!(RuntimeStateDto::from(ReadinessState::Ready), RuntimeStateDto::Ready);
        assert_eq!(RuntimeStateDto::from(ReadinessState::Halted), RuntimeStateDto::Halted);
    }

    #[test]
    fn stream_state_stale_is_a_stream_fact_not_a_protection_state() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::to_string(&StreamStateDto::Stale)?, "\"STALE\"");
        Ok(())
    }
}
