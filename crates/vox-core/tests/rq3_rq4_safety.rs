use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use vox_core::{CoreConfig, CoreRuntime, ReconciliationChecks, ReconciliationEvidence};
use vox_domain::{
    AuthoritativeMutationOutcome, BrokerOrderId, ClientRequestId, ExchangeOrderId,
    MutationDecision, MutationEvidence, MutationEvidenceStore, MutationRecovery, StoreError,
};
use vox_tinvest::DispatchCertainty;

#[derive(Clone, Default)]
struct DurableHarness(Arc<Mutex<HashMap<ClientRequestId, MutationEvidence>>>);

impl MutationEvidenceStore for DurableHarness {
    fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
        Ok(self.0.lock().map_err(store_error)?.get(id).cloned())
    }

    fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
        self.0
            .lock()
            .map_err(store_error)?
            .insert(evidence.client_request_id().clone(), evidence.clone());
        Ok(())
    }

    fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
        let mut records = self.0.lock().map_err(store_error)?;
        if records.contains_key(evidence.client_request_id()) {
            return Ok(false);
        }
        records.insert(evidence.client_request_id().clone(), evidence.clone());
        Ok(true)
    }

    fn resolve_unknown(
        &mut self,
        expected: &MutationEvidence,
        resolved: &MutationEvidence,
    ) -> Result<bool, StoreError> {
        let mut records = self.0.lock().map_err(store_error)?;
        if records.get(expected.client_request_id()) != Some(expected) {
            return Ok(false);
        }
        records.insert(resolved.client_request_id().clone(), resolved.clone());
        Ok(true)
    }
}

fn store_error(error: impl std::fmt::Display) -> StoreError {
    StoreError(error.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BrokerOrderState {
    Active,
    Cancelled,
}

#[derive(Default)]
struct SandboxHarness {
    orders: Mutex<HashMap<ClientRequestId, (BrokerOrderId, ExchangeOrderId, BrokerOrderState)>>,
    submit_dispatches: AtomicUsize,
    cancel_dispatches: AtomicUsize,
}

impl SandboxHarness {
    fn submit_with_response_loss(
        &self,
        _authorization: vox_domain::MutationAuthorization,
        request_id: ClientRequestId,
    ) -> Result<DispatchCertainty, StoreError> {
        self.submit_dispatches.fetch_add(1, Ordering::SeqCst);
        self.orders.lock().map_err(store_error)?.insert(
            request_id,
            (
                BrokerOrderId::new("broker-order-1").map_err(store_error)?,
                ExchangeOrderId::new("exchange-order-1").map_err(store_error)?,
                BrokerOrderState::Active,
            ),
        );
        Ok(DispatchCertainty::PossiblyDispatched)
    }

    fn readback(
        &self,
        request_id: &ClientRequestId,
    ) -> Result<(BrokerOrderId, ExchangeOrderId, BrokerOrderState), StoreError> {
        self.orders
            .lock()
            .map_err(store_error)?
            .get(request_id)
            .cloned()
            .ok_or_else(|| StoreError("authoritative order not found".into()))
    }

    fn cancel(
        &self,
        _authorization: vox_domain::MutationAuthorization,
        broker_order_id: &BrokerOrderId,
    ) -> Result<DispatchCertainty, StoreError> {
        self.cancel_dispatches.fetch_add(1, Ordering::SeqCst);
        let mut orders = self.orders.lock().map_err(store_error)?;
        let order = orders
            .values_mut()
            .find(|order| &order.0 == broker_order_id)
            .ok_or_else(|| StoreError("authoritative order not found".into()))?;
        order.2 = BrokerOrderState::Cancelled;
        Ok(DispatchCertainty::ProviderResponded)
    }
}

#[test]
fn sandbox_harness_proves_unknown_readback_cancel_and_no_blind_retry()
-> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = CoreRuntime::new(CoreConfig::default());
    runtime.begin_connecting()?;
    runtime.begin_reconciliation()?;
    runtime.complete_reconciliation(ReconciliationEvidence::new(
        "snapshot-1",
        1,
        ReconciliationChecks::complete(),
        0,
    )?)?;
    let submit_authorization =
        runtime.consume_new_exposure_authorization(runtime.authorize_new_exposure()?)?;

    let store = DurableHarness::default();
    let broker = SandboxHarness::default();
    let submit_id = ClientRequestId::new("sandbox-submit-request")?;
    let mut first_process = MutationRecovery::new(store.clone());
    let evidence =
        first_process.persist_before_dispatch(submit_id.clone(), Some("corr-1".into()))?;
    assert_eq!(
        broker.submit_with_response_loss(submit_authorization, submit_id.clone())?,
        DispatchCertainty::PossiblyDispatched
    );
    drop(first_process);

    let mut restarted_process = MutationRecovery::new(store.clone());
    assert_eq!(
        restarted_process.decision(&submit_id)?,
        MutationDecision::Reconcile
    );
    assert_eq!(broker.submit_dispatches.load(Ordering::SeqCst), 1);
    let (broker_id, exchange_id, state) = broker.readback(&submit_id)?;
    assert_eq!(state, BrokerOrderState::Active);
    let accepted = restarted_process.persist_authoritative_outcome(
        evidence
            .with_broker_order_id(broker_id.clone())
            .with_exchange_order_id(exchange_id),
        AuthoritativeMutationOutcome::Accepted,
    )?;
    assert_eq!(accepted.broker_order_id(), Some(&broker_id));
    assert_eq!(
        restarted_process.decision(&submit_id)?,
        MutationDecision::DoNotSubmit
    );

    let cancel_id = ClientRequestId::new("sandbox-cancel-request")?;
    let cancel_evidence =
        restarted_process.persist_before_dispatch(cancel_id.clone(), Some("corr-2".into()))?;
    let cancel_authorization =
        runtime.consume_new_exposure_authorization(runtime.authorize_new_exposure()?)?;
    assert_eq!(
        broker.cancel(cancel_authorization, &broker_id)?,
        DispatchCertainty::ProviderResponded
    );
    assert_eq!(broker.readback(&submit_id)?.2, BrokerOrderState::Cancelled);
    restarted_process.persist_authoritative_outcome(
        cancel_evidence.with_broker_order_id(broker_id),
        AuthoritativeMutationOutcome::Accepted,
    )?;
    assert_eq!(broker.cancel_dispatches.load(Ordering::SeqCst), 1);
    assert_eq!(
        restarted_process.decision(&cancel_id)?,
        MutationDecision::DoNotSubmit
    );
    Ok(())
}
