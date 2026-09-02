use std::sync::{Arc, Barrier};

use vox_risk::{
    ReservationCapacity, ReservationState, RiskActionKind, RiskDecision, RiskOutcome,
    RiskReservation, RiskSource, RiskStore, RiskStoreError, RiskValidityContext, SqliteRiskStore,
};

fn validity() -> RiskValidityContext {
    RiskValidityContext {
        runtime_epoch: 1,
        reconciliation_revision: 1,
        position_revision: 1,
        order_revision: 1,
        market_data_as_of_unix_ms: Some(1),
        instrument_constraints_revision: 1,
        policy_revision: 1,
        execution_authorization_revision: 1,
    }
}

fn approval(request_id: &str, delta: i64) -> (RiskDecision, RiskReservation) {
    let reservation = RiskReservation {
        reservation_id: RiskReservation::new_id(),
        account_id: "account-1".to_owned(),
        instrument_id: "instrument-1".to_owned(),
        strategy_id: None,
        source: RiskSource::Strategy,
        logical_request_id: request_id.to_owned(),
        reserved_delta_lots: delta,
        remaining_delta_lots: delta,
        reserved_notional_nanos: i128::from(delta) * 1_000_000_000,
        state: ReservationState::Active,
        created_at_unix_ms: 1,
        updated_at_unix_ms: 1,
        expires_at_unix_ms: None,
    };
    let decision = RiskDecision {
        decision_id: RiskDecision::new_id(),
        request_id: request_id.to_owned(),
        policy_revision: 1,
        account_id: "account-1".to_owned(),
        action: RiskActionKind::DirectionalOrder,
        requested_delta_lots: delta,
        approved_delta_lots: delta,
        outcome: RiskOutcome::Approve,
        reasons: Vec::new(),
        reservation_id: Some(reservation.reservation_id.clone()),
        expires_at_unix_ms: None,
        validity: validity(),
    };
    (decision, reservation)
}

#[test]
fn simultaneous_approvals_cannot_oversubscribe_remaining_capacity() {
    let path = std::env::temp_dir().join(format!(
        "vox-risk-concurrency-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let store = SqliteRiskStore::open(&path).expect("open risk store");
    let barrier = Arc::new(Barrier::new(3));
    let capacity = ReservationCapacity {
        max_account_reserved_notional_nanos: Some(10_000_000_000),
        max_instrument_reserved_abs_lots: Some(10),
    };

    let spawn = |request_id: &'static str| {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let (decision, reservation) = approval(request_id, 6);
            barrier.wait();
            store.persist_approval_atomic(&decision, &reservation, capacity)
        })
    };

    let first = spawn("req-concurrent-1");
    let second = spawn("req-concurrent-2");
    barrier.wait();

    let results = [
        first.join().expect("first thread"),
        second.join().expect("second thread"),
    ];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RiskStoreError::CapacityExceeded)))
            .count(),
        1
    );
    assert_eq!(
        store
            .active_reserved_delta("account-1", "instrument-1")
            .expect("reserved delta")
            .unsigned_abs(),
        6
    );

    drop(store);
    let _ = std::fs::remove_file(path);
}
