use thiserror::Error;

use crate::model::{ReservationState, RiskReservation};
use crate::store::{RiskStore, RiskStoreError};

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
            return Ok(reservation);
        }
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[ReservationState::Active, ReservationState::PartiallyConsumed],
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
            return Ok(reservation);
        }
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
            return Ok(reservation);
        }
        self.store
            .update_reservation(
                &reservation.reservation_id,
                &[ReservationState::Active, ReservationState::PartiallyConsumed],
                reservation.remaining_delta_lots,
                ReservationState::Orphaned,
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

#[derive(Clone, Debug, Error, Eq, PartialEq)]
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
}
