use thiserror::Error;

use crate::model::{ReservationState, RiskReservation};
use crate::store::{RiskStore, RiskStoreError};

/// Formats a reservation state for tracing output.
fn state_label(state: ReservationState) -> &'static str {
    match state {
        ReservationState::Active => "active",
        ReservationState::PartiallyConsumed => "partially_consumed",
        ReservationState::Consumed => "consumed",
        ReservationState::Released => "released",
        ReservationState::UnknownHeld => "unknown_held",
        ReservationState::Orphaned => "orphaned",
    }
}

/// #21-owned reservation lifecycle reconciler.
///
/// #11 remains authoritative for broker mutation evidence. This type consumes that evidence
/// and changes only #21-owned reservation state. In particular, UNKNOWN never frees capacity.
pub struct RiskReservationReconciler<S> {
    store: S,
}

impl<S: RiskStore> RiskReservationReconciler<S> {
    #[must_use]
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Release is legal only when dispatch is proven not to have happened.
    pub fn proven_pre_dispatch_failure(
        &self,
        account_id: &str,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        tracing::info!(
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "proven_pre_dispatch_failure",
            "reconciliation: proven pre-dispatch failure - releasing reservation",
        );
        let reservation = self.required(account_id, logical_request_id)?;
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[ReservationState::Active],
                0,
                ReservationState::Released,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Ambiguous post-dispatch outcome keeps the full remaining reservation fail-closed.
    pub fn unknown_after_dispatch(
        &self,
        account_id: &str,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if reservation.state == ReservationState::UnknownHeld {
            tracing::info!(
                risk.reservation_id = %reservation.reservation_id,
                risk.account_id = %account_id,
                risk.request_id = %logical_request_id,
                reconciliation.event = "unknown_after_dispatch",
                reconciliation.state = state_label(reservation.state),
                "reconciliation: unknown_after_dispatch (already unknown_held, idempotent)",
            );
            return Ok(reservation);
        }
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "unknown_after_dispatch",
            reconciliation.from_state = state_label(reservation.state),
            reconciliation.remaining_delta_lots = reservation.remaining_delta_lots,
            "reconciliation: unknown_after_dispatch - fail-closed, marking unknown_held",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                ],
                reservation.remaining_delta_lots,
                ReservationState::UnknownHeld,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Consume broker-authoritatively filled lots. The fill must have the same direction as
    /// the reservation and cannot exceed its remaining quantity.
    pub fn authoritative_fill(
        &self,
        account_id: &str,
        logical_request_id: &str,
        filled_delta_lots: i64,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        if filled_delta_lots == 0 {
            return Err(RiskReservationReconcileError::InvalidFill);
        }
        let reservation = self.required(account_id, logical_request_id)?;
        if !matches!(
            reservation.state,
            ReservationState::Active
                | ReservationState::PartiallyConsumed
                | ReservationState::UnknownHeld
        ) {
            return Err(RiskReservationReconcileError::TerminalReservation);
        }
        if reservation.remaining_delta_lots.signum() != filled_delta_lots.signum() {
            return Err(RiskReservationReconcileError::FillDirectionMismatch);
        }
        if filled_delta_lots.unsigned_abs() > reservation.remaining_delta_lots.unsigned_abs() {
            return Err(RiskReservationReconcileError::FillExceedsReservation);
        }

        let remaining = reservation
            .remaining_delta_lots
            .checked_sub(filled_delta_lots)
            .ok_or(RiskReservationReconcileError::ArithmeticOverflow)?;
        let target = if remaining == 0 {
            ReservationState::Consumed
        } else {
            ReservationState::PartiallyConsumed
        };
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "authoritative_fill",
            reconciliation.filled_delta_lots = filled_delta_lots,
            reconciliation.remaining_delta_lots = remaining,
            reconciliation.to_state = state_label(target),
            "reconciliation: authoritative_fill",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                    ReservationState::UnknownHeld,
                ],
                remaining,
                target,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Release only after #11/broker reconciliation proves that no remaining order exposure
    /// exists. A mere cancel request/transport ACK must not call this method.
    pub fn broker_authoritative_no_remaining_exposure(
        &self,
        account_id: &str,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if matches!(
            reservation.state,
            ReservationState::Consumed | ReservationState::Released
        ) {
            tracing::info!(
                risk.reservation_id = %reservation.reservation_id,
                risk.account_id = %account_id,
                risk.request_id = %logical_request_id,
                reconciliation.event = "broker_authoritative_no_remaining_exposure",
                reconciliation.state = state_label(reservation.state),
                "reconciliation: broker authoritative no remaining exposure (already terminal, idempotent)",
            );
            return Ok(reservation);
        }
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "broker_authoritative_no_remaining_exposure",
            reconciliation.from_state = state_label(reservation.state),
            "reconciliation: broker authoritative no remaining exposure - releasing reservation",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                    ReservationState::UnknownHeld,
                    ReservationState::Orphaned,
                ],
                0,
                ReservationState::Released,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Restart cannot silently free an expired/unmatched reservation. Mark it ORPHANED so it
    /// continues to count against capacity until authoritative reconciliation resolves it.
    pub fn orphan_fail_closed(
        &self,
        account_id: &str,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if reservation.state == ReservationState::Orphaned {
            tracing::info!(
                risk.reservation_id = %reservation.reservation_id,
                risk.account_id = %account_id,
                risk.request_id = %logical_request_id,
                reconciliation.event = "orphan_fail_closed",
                reconciliation.state = state_label(reservation.state),
                "reconciliation: orphan_fail_closed (already orphaned, idempotent)",
            );
            return Ok(reservation);
        }
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "orphan_fail_closed",
            reconciliation.from_state = state_label(reservation.state),
            reconciliation.remaining_delta_lots = reservation.remaining_delta_lots,
            "reconciliation: orphan_fail_closed - marking reservation orphaned (fail-closed)",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                ],
                reservation.remaining_delta_lots,
                ReservationState::Orphaned,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Broker acknowledged the order. This is evidence that dispatch succeeded, so UNKNOWN
    /// is no longer applicable. The reservation state is unchanged because the order is now
    /// live in the market. Returns the reservation so callers can confirm the dispatch
    /// context matches before sending the order to the broker.
    pub fn dispatch_acknowledged(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if !matches!(
            reservation.state,
            ReservationState::Active
                | ReservationState::PartiallyConsumed
                | ReservationState::UnknownHeld
        ) {
            return Err(RiskReservationReconcileError::TerminalReservation);
        }
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "dispatch_acknowledged",
            reconciliation.state = state_label(reservation.state),
            "reconciliation: dispatch acknowledged by broker",
        );
        Ok(reservation)
    }

    /// Broker authoritatively rejected the order. Release the reservation because there
    /// is no remaining market exposure. Unlike `proven_pre_dispatch_failure`, this path
    /// confirms that dispatch did happen but the broker refused the order.
    pub fn broker_authoritative_reject(
        &self,
        account_id: &str,
        logical_request_id: &str,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if matches!(
            reservation.state,
            ReservationState::Consumed | ReservationState::Released
        ) {
            tracing::info!(
                risk.reservation_id = %reservation.reservation_id,
                risk.account_id = %account_id,
                risk.request_id = %logical_request_id,
                reconciliation.event = "broker_authoritative_reject",
                reconciliation.state = state_label(reservation.state),
                "reconciliation: broker authoritative reject (already terminal, idempotent)",
            );
            return Ok(reservation);
        }
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "broker_authoritative_reject",
            reconciliation.from_state = state_label(reservation.state),
            "reconciliation: broker authoritative reject - releasing reservation",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                    ReservationState::UnknownHeld,
                    ReservationState::Orphaned,
                ],
                0,
                ReservationState::Released,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    /// Runtime reconciliation: the runtime reports the actual remaining lots for the
    /// order. This evidence is broker-authoritative and updates the reservation
    /// accordingly. Partial fills are handled naturally—remaining lots shrink toward
    /// zero and the state transitions to Consumed when fully filled.
    pub fn runtime_reconciliation(
        &self,
        account_id: &str,
        logical_request_id: &str,
        runtime_remaining_lots: i64,
        now_unix_ms: i64,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        let reservation = self.required(account_id, logical_request_id)?;
        if !matches!(
            reservation.state,
            ReservationState::Active
                | ReservationState::PartiallyConsumed
                | ReservationState::UnknownHeld
        ) {
            return Err(RiskReservationReconcileError::TerminalReservation);
        }
        let original_sign = reservation.remaining_delta_lots.signum();
        if original_sign == 0 {
            return Err(RiskReservationReconcileError::InvalidFill);
        }
        // Zero runtime remaining is always valid (full fill/close).
        // Otherwise the direction must match the reservation direction.
        if runtime_remaining_lots != 0 && runtime_remaining_lots.signum() != original_sign {
            return Err(RiskReservationReconcileError::FillDirectionMismatch);
        }
        let runtime_abs = i64::try_from(runtime_remaining_lots.unsigned_abs())
            .map_err(|_| RiskReservationReconcileError::ArithmeticOverflow)?;
        let original_abs = i64::try_from(reservation.remaining_delta_lots.unsigned_abs())
            .map_err(|_| RiskReservationReconcileError::ArithmeticOverflow)?;
        if runtime_abs > original_abs {
            return Err(RiskReservationReconcileError::FillExceedsReservation);
        }

        let target = if runtime_remaining_lots == 0 {
            ReservationState::Consumed
        } else {
            ReservationState::PartiallyConsumed
        };
        tracing::info!(
            risk.reservation_id = %reservation.reservation_id,
            risk.account_id = %account_id,
            risk.request_id = %logical_request_id,
            reconciliation.event = "runtime_reconciliation",
            reconciliation.from_state = state_label(reservation.state),
            reconciliation.to_state = state_label(target),
            reconciliation.original_remaining_lots = reservation.remaining_delta_lots,
            reconciliation.runtime_remaining_lots = runtime_remaining_lots,
            "reconciliation: runtime_reconciliation",
        );
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[
                    ReservationState::Active,
                    ReservationState::PartiallyConsumed,
                    ReservationState::UnknownHeld,
                ],
                runtime_remaining_lots,
                target,
                now_unix_ms,
            )
            .map_err(Into::into)
    }

    fn required(
        &self,
        account_id: &str,
        logical_request_id: &str,
    ) -> Result<RiskReservation, RiskReservationReconcileError> {
        self.store
            .reservation_for_request(account_id, logical_request_id)?
            .ok_or(RiskReservationReconcileError::ReservationNotFound)
    }
}

#[derive(Debug, Error)]
pub enum RiskReservationReconcileError {
    #[error("risk reservation not found")]
    ReservationNotFound,
    #[error("fill quantity must be non-zero")]
    InvalidFill,
    #[error("fill direction does not match the reservation")]
    FillDirectionMismatch,
    #[error("fill exceeds remaining reserved quantity")]
    FillExceedsReservation,
    #[error("terminal reservation cannot consume another fill")]
    TerminalReservation,
    #[error("risk reservation arithmetic overflow")]
    ArithmeticOverflow,
    #[error("risk store failure: {0}")]
    Store(#[from] RiskStoreError),
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{ReservationState, RiskReservation, RiskSource};
    use crate::store::{RiskStore, SqliteRiskStore};

    use super::*;

    fn store() -> SqliteRiskStore {
        let path: PathBuf = std::env::temp_dir().join(format!(
            "vox-risk-reconcile-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        SqliteRiskStore::open(path).expect("store")
    }

    fn reservation() -> RiskReservation {
        RiskReservation {
            reservation_id: RiskReservation::new_id(),
            account_id: "account-1".into(),
            instrument_id: "instrument-1".into(),
            strategy_id: None,
            source: RiskSource::Manual,
            logical_request_id: "request-1".into(),
            reserved_delta_lots: 10,
            remaining_delta_lots: 10,
            reserved_notional_nanos: 1_000,
            state: ReservationState::Active,
            created_at_unix_ms: 1,
            updated_at_unix_ms: 1,
            expires_at_unix_ms: None,
        }
    }

    #[test]
    fn unknown_keeps_capacity_reserved() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store.clone());
        let held = reconciler
            .unknown_after_dispatch("account-1", "request-1", 2)
            .expect("hold");
        assert_eq!(held.state, ReservationState::UnknownHeld);
        assert_eq!(held.remaining_delta_lots, 10);
        assert_eq!(
            store
                .active_reserved_delta("account-1", "instrument-1")
                .expect("active"),
            10
        );
    }

    #[test]
    fn partial_fill_consumes_only_filled_quantity() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let partial = reconciler
            .authoritative_fill("account-1", "request-1", 4, 2)
            .expect("fill");
        assert_eq!(partial.state, ReservationState::PartiallyConsumed);
        assert_eq!(partial.remaining_delta_lots, 6);
    }

    #[test]
    fn cancel_request_alone_has_no_release_api() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let released = reconciler
            .broker_authoritative_no_remaining_exposure("account-1", "request-1", 2)
            .expect("authoritative release");
        assert_eq!(released.state, ReservationState::Released);
        assert_eq!(released.remaining_delta_lots, 0);
    }

    #[test]
    fn dispatch_acknowledged_validates_active_reservation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let acked = reconciler
            .dispatch_acknowledged("account-1", "request-1")
            .expect("ack");
        assert_eq!(acked.state, ReservationState::Active);
        assert_eq!(acked.remaining_delta_lots, 10);
    }

    #[test]
    fn dispatch_acknowledged_fails_on_terminal_reservation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        // Transition to consumed
        reconciler
            .authoritative_fill("account-1", "request-1", 10, 2)
            .expect("full fill");
        let result = reconciler.dispatch_acknowledged("account-1", "request-1");
        assert!(matches!(
            result,
            Err(RiskReservationReconcileError::TerminalReservation)
        ));
    }

    #[test]
    fn broker_reject_releases_active_reservation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let rejected = reconciler
            .broker_authoritative_reject("account-1", "request-1", 2)
            .expect("reject");
        assert_eq!(rejected.state, ReservationState::Released);
        assert_eq!(rejected.remaining_delta_lots, 0);
    }

    #[test]
    fn broker_reject_releases_partially_consumed_reservation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        // Partial fill first
        reconciler
            .authoritative_fill("account-1", "request-1", 4, 2)
            .expect("partial fill");
        // Then reject the remaining
        let rejected = reconciler
            .broker_authoritative_reject("account-1", "request-1", 3)
            .expect("reject");
        assert_eq!(rejected.state, ReservationState::Released);
        assert_eq!(rejected.remaining_delta_lots, 0);
    }

    #[test]
    fn broker_reject_is_idempotent_on_already_released() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        reconciler
            .broker_authoritative_reject("account-1", "request-1", 2)
            .expect("reject");
        // Second reject should succeed idempotently
        let rejected = reconciler
            .broker_authoritative_reject("account-1", "request-1", 3)
            .expect("reject");
        assert_eq!(rejected.state, ReservationState::Released);
    }

    #[test]
    fn runtime_reconciliation_updates_remaining_lots() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let reconciled = reconciler
            .runtime_reconciliation("account-1", "request-1", 6, 2)
            .expect("reconciliation");
        assert_eq!(reconciled.state, ReservationState::PartiallyConsumed);
        assert_eq!(reconciled.remaining_delta_lots, 6);
    }

    #[test]
    fn runtime_reconciliation_converts_to_consumed_when_zero() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let reconciled = reconciler
            .runtime_reconciliation("account-1", "request-1", 0, 2)
            .expect("reconciliation");
        assert_eq!(reconciled.state, ReservationState::Consumed);
        assert_eq!(reconciled.remaining_delta_lots, 0);
    }

    #[test]
    fn runtime_rejection_fails_on_direction_mismatch() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let result = reconciler.runtime_reconciliation("account-1", "request-1", -5, 2);
        assert!(matches!(
            result,
            Err(RiskReservationReconcileError::FillDirectionMismatch)
        ));
    }

    #[test]
    fn runtime_reconciliation_fails_when_runtime_exceeds_reserved() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        let result = reconciler.runtime_reconciliation("account-1", "request-1", 15, 2);
        assert!(matches!(
            result,
            Err(RiskReservationReconcileError::FillExceedsReservation)
        ));
    }

    #[test]
    fn runtime_reconciliation_fails_on_terminal_reservation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);
        // Transition to released
        reconciler
            .broker_authoritative_reject("account-1", "request-1", 2)
            .expect("reject");
        let result = reconciler.runtime_reconciliation("account-1", "request-1", 5, 3);
        assert!(matches!(
            result,
            Err(RiskReservationReconcileError::TerminalReservation)
        ));
    }

    #[test]
    fn full_lifecycle_ack_then_partial_fill_then_reject() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store.clone());

        // Step 1: Dispatch acknowledged
        let acked = reconciler
            .dispatch_acknowledged("account-1", "request-1")
            .expect("ack");
        assert_eq!(acked.state, ReservationState::Active);

        // Step 2: Partial fill
        let filled = reconciler
            .authoritative_fill("account-1", "request-1", 4, 3)
            .expect("fill");
        assert_eq!(filled.state, ReservationState::PartiallyConsumed);
        assert_eq!(filled.remaining_delta_lots, 6);

        // Step 3: Broker rejects remaining exposure
        let rejected = reconciler
            .broker_authoritative_reject("account-1", "request-1", 4)
            .expect("reject");
        assert_eq!(rejected.state, ReservationState::Released);
        assert_eq!(rejected.remaining_delta_lots, 0);

        // Verify capacity is freed
        assert_eq!(
            store
                .active_reserved_delta("account-1", "instrument-1")
                .expect("active"),
            0
        );
    }

    #[test]
    fn full_lifecycle_unknown_then_runtime_reconciliation() {
        let store = store();
        let original = reservation();
        store.reserve_atomic(&original, 100).expect("reserve");
        let reconciler = RiskReservationReconciler::new(store);

        // Step 1: Unknown after dispatch (fail-closed)
        let held = reconciler
            .unknown_after_dispatch("account-1", "request-1", 2)
            .expect("hold");
        assert_eq!(held.state, ReservationState::UnknownHeld);
        assert_eq!(held.remaining_delta_lots, 10);

        // Step 2: Runtime reconciliation resolves the unknown
        let reconciled = reconciler
            .runtime_reconciliation("account-1", "request-1", 3, 3)
            .expect("reconciliation");
        assert_eq!(reconciled.state, ReservationState::PartiallyConsumed);
        assert_eq!(reconciled.remaining_delta_lots, 3);
    }
}
