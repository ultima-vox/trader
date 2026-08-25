use crate::{BrokerOrderId, BrokerStopOrderId, ClientRequestId, ExchangeOrderId};
use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationOutcome {
    NotDispatched,
    Accepted,
    Rejected,
    Unknown,
}

/// Terminal classification backed by an authoritative broker or exchange observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoritativeMutationOutcome {
    Accepted,
    Rejected,
}

impl AuthoritativeMutationOutcome {
    const fn into_outcome(self) -> MutationOutcome {
        match self {
            Self::Accepted => MutationOutcome::Accepted,
            Self::Rejected => MutationOutcome::Rejected,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MutationEvidence {
    client_request_id: ClientRequestId,
    outcome: MutationOutcome,
    broker_order_id: Option<BrokerOrderId>,
    #[serde(default)]
    broker_stop_order_id: Option<BrokerStopOrderId>,
    exchange_order_id: Option<ExchangeOrderId>,
    #[serde(default)]
    provider_operation_id: Option<String>,
    correlation_id: Option<String>,
}

impl MutationEvidence {
    #[must_use]
    fn prepared(client_request_id: ClientRequestId) -> Self {
        Self {
            client_request_id,
            outcome: MutationOutcome::NotDispatched,
            broker_order_id: None,
            broker_stop_order_id: None,
            exchange_order_id: None,
            provider_operation_id: None,
            correlation_id: None,
        }
    }

    #[must_use]
    fn dispatched_unknown(mut self, correlation_id: Option<String>) -> Self {
        self.outcome = MutationOutcome::Unknown;
        self.correlation_id = correlation_id;
        self
    }

    #[must_use]
    pub const fn client_request_id(&self) -> &ClientRequestId {
        &self.client_request_id
    }

    #[must_use]
    pub const fn outcome(&self) -> MutationOutcome {
        self.outcome
    }

    #[must_use]
    pub const fn broker_order_id(&self) -> Option<&BrokerOrderId> {
        self.broker_order_id.as_ref()
    }

    #[must_use]
    pub const fn broker_stop_order_id(&self) -> Option<&BrokerStopOrderId> {
        self.broker_stop_order_id.as_ref()
    }

    #[must_use]
    pub const fn exchange_order_id(&self) -> Option<&ExchangeOrderId> {
        self.exchange_order_id.as_ref()
    }

    #[must_use]
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    #[must_use]
    pub fn provider_operation_id(&self) -> Option<&str> {
        self.provider_operation_id.as_deref()
    }

    #[must_use]
    pub fn with_broker_order_id(mut self, broker_order_id: BrokerOrderId) -> Self {
        self.broker_order_id = Some(broker_order_id);
        self
    }

    #[must_use]
    pub fn with_exchange_order_id(mut self, exchange_order_id: ExchangeOrderId) -> Self {
        self.exchange_order_id = Some(exchange_order_id);
        self
    }

    #[must_use]
    pub fn with_broker_stop_order_id(mut self, broker_stop_order_id: BrokerStopOrderId) -> Self {
        self.broker_stop_order_id = Some(broker_stop_order_id);
        self
    }

    #[must_use]
    pub fn with_provider_operation_id(mut self, provider_operation_id: impl Into<String>) -> Self {
        self.provider_operation_id = Some(provider_operation_id.into());
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

/// Durable implementation must commit evidence before transport dispatch returns control.
pub trait MutationEvidenceStore {
    fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError>;
    fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError>;

    /// Atomically claims an identity before dispatch and durably persists UNKNOWN.
    /// Implementations must use compare-and-set or a uniqueness constraint across processes.
    fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError>;

    /// Atomically replaces expected UNKNOWN evidence with authoritative terminal evidence.
    fn resolve_unknown(
        &mut self,
        expected: &MutationEvidence,
        resolved: &MutationEvidence,
    ) -> Result<bool, StoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MutationDecision {
    Submit,
    Reconcile,
    DoNotSubmit,
}

pub struct MutationRecovery<S> {
    store: S,
}

impl<S: MutationEvidenceStore> MutationRecovery<S> {
    #[must_use]
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    pub fn decision(&self, id: &ClientRequestId) -> Result<MutationDecision, StoreError> {
        Ok(match self.store.load(id)? {
            None
            | Some(MutationEvidence {
                outcome: MutationOutcome::NotDispatched,
                ..
            }) => MutationDecision::Submit,
            Some(MutationEvidence {
                outcome: MutationOutcome::Unknown,
                ..
            }) => MutationDecision::Reconcile,
            Some(_) => MutationDecision::DoNotSubmit,
        })
    }

    /// Persists UNKNOWN before dispatch, closing crash window between send and journal update.
    pub fn persist_before_dispatch(
        &mut self,
        id: ClientRequestId,
        correlation_id: Option<String>,
    ) -> Result<MutationEvidence, StoreError> {
        let evidence = MutationEvidence::prepared(id).dispatched_unknown(correlation_id);
        if self.store.claim_dispatch(&evidence)? {
            Ok(evidence)
        } else {
            Err(StoreError(
                "mutation identity already dispatched; reconcile instead of resubmitting".into(),
            ))
        }
    }

    /// Resolves UNKNOWN only from authoritative reconciliation or a broker response.
    pub fn persist_authoritative_outcome(
        &mut self,
        mut evidence: MutationEvidence,
        authoritative_outcome: AuthoritativeMutationOutcome,
    ) -> Result<MutationEvidence, StoreError> {
        let outcome = authoritative_outcome.into_outcome();
        let stored = self
            .store
            .load(evidence.client_request_id())?
            .ok_or_else(|| StoreError("mutation dispatch evidence is not persisted".into()))?;
        if matches!(
            stored.outcome,
            MutationOutcome::Accepted | MutationOutcome::Rejected
        ) {
            if stored == evidence && stored.outcome == outcome {
                return Ok(stored);
            }
            return Err(StoreError("terminal mutation evidence is immutable".into()));
        }
        if stored.outcome != MutationOutcome::Unknown {
            return Err(StoreError(format!(
                "authoritative outcome cannot resolve {:?} evidence",
                stored.outcome
            )));
        }
        evidence.outcome = outcome;
        if self.store.resolve_unknown(&stored, &evidence)? {
            Ok(evidence)
        } else {
            Err(StoreError(
                "mutation evidence changed during authoritative reconciliation".into(),
            ))
        }
    }

    #[must_use]
    pub fn into_store(self) -> S {
        self.store
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct PersistentHarness(Arc<Mutex<HashMap<ClientRequestId, MutationEvidence>>>);

    impl MutationEvidenceStore for PersistentHarness {
        fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
            let evidence = self
                .0
                .lock()
                .map_err(|error| StoreError(error.to_string()))?
                .get(id)
                .cloned();
            Ok(evidence)
        }

        fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
            self.0
                .lock()
                .map_err(|error| StoreError(error.to_string()))?
                .insert(evidence.client_request_id.clone(), evidence.clone());
            Ok(())
        }

        fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
            let mut records = self
                .0
                .lock()
                .map_err(|error| StoreError(error.to_string()))?;
            match records.get(&evidence.client_request_id) {
                None
                | Some(MutationEvidence {
                    outcome: MutationOutcome::NotDispatched,
                    ..
                }) => {
                    records.insert(evidence.client_request_id.clone(), evidence.clone());
                    Ok(true)
                }
                Some(_) => Ok(false),
            }
        }

        fn resolve_unknown(
            &mut self,
            expected: &MutationEvidence,
            resolved: &MutationEvidence,
        ) -> Result<bool, StoreError> {
            let mut records = self
                .0
                .lock()
                .map_err(|error| StoreError(error.to_string()))?;
            if records.get(expected.client_request_id()) != Some(expected) {
                return Ok(false);
            }
            records.insert(resolved.client_request_id().clone(), resolved.clone());
            Ok(true)
        }
    }

    #[test]
    fn restart_reconciles_unknown_dispatch_and_never_resubmits()
    -> Result<(), Box<dyn std::error::Error>> {
        let durable_store = PersistentHarness::default();
        let request_id = ClientRequestId::new("request-1")?;

        let mut first_process = MutationRecovery::new(durable_store.clone());
        assert_eq!(
            first_process.decision(&request_id)?,
            MutationDecision::Submit
        );
        first_process.persist_before_dispatch(request_id.clone(), Some("tracking-1".into()))?;
        drop(first_process);

        let restarted_process = MutationRecovery::new(durable_store);
        assert_eq!(
            restarted_process.decision(&request_id)?,
            MutationDecision::Reconcile
        );
        assert_ne!(
            restarted_process.decision(&request_id)?,
            MutationDecision::Submit
        );
        Ok(())
    }

    #[test]
    fn duplicate_dispatch_claim_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
        let request_id = ClientRequestId::new("request-duplicate")?;
        let mut recovery = MutationRecovery::new(PersistentHarness::default());
        recovery.persist_before_dispatch(request_id.clone(), None)?;

        let error = match recovery.persist_before_dispatch(request_id, None) {
            Ok(_) => {
                return Err(
                    StoreError("second dispatch claim unexpectedly succeeded".into()).into(),
                );
            }
            Err(error) => error,
        };
        assert_eq!(
            error.0,
            "mutation identity already dispatched; reconcile instead of resubmitting"
        );
        Ok(())
    }

    #[test]
    fn unknown_remains_reconciliation_only() -> Result<(), Box<dyn std::error::Error>> {
        let request_id = ClientRequestId::new("request-unknown")?;
        let mut recovery = MutationRecovery::new(PersistentHarness::default());
        let evidence = recovery.persist_before_dispatch(request_id.clone(), None)?;

        assert_eq!(evidence.outcome(), MutationOutcome::Unknown);
        assert_eq!(recovery.decision(&request_id)?, MutationDecision::Reconcile);
        Ok(())
    }

    #[test]
    fn terminal_outcome_is_immutable_and_never_resubmitted()
    -> Result<(), Box<dyn std::error::Error>> {
        let request_id = ClientRequestId::new("request-terminal")?;
        let mut recovery = MutationRecovery::new(PersistentHarness::default());
        let evidence = recovery
            .persist_before_dispatch(request_id.clone(), None)?
            .with_broker_order_id(BrokerOrderId::new("broker-terminal")?);
        let accepted = recovery
            .persist_authoritative_outcome(evidence, AuthoritativeMutationOutcome::Accepted)?;

        assert!(
            recovery
                .persist_authoritative_outcome(accepted, AuthoritativeMutationOutcome::Rejected)
                .is_err()
        );
        assert_eq!(
            recovery.decision(&request_id)?,
            MutationDecision::DoNotSubmit
        );
        Ok(())
    }

    #[test]
    fn unknown_has_stable_persisted_name() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_string(&MutationOutcome::Unknown)?,
            "\"UNKNOWN\""
        );
        Ok(())
    }
}
