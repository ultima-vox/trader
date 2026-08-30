use thiserror::Error;

use crate::model::{
    ReservationState, RiskActionKind, RiskDecision, RiskRequest, RiskReservation,
    RiskValidityContext,
};

/// Immutable #21 -> #10 hand-off.
///
/// A caller must not treat a bare `RiskDecision` as permission to mutate broker state.
/// Dispatch permission exists only after the decision and any required reservation were
/// durably persisted and bound to the exact logical request/account/connection/instrument
/// context. Exposure-bearing actions require a reservation; cleanup/protection maintenance
/// actions are explicitly reservation-free.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskApprovedExecutionPlan {
    pub decision_id: String,
    pub reservation_id: Option<String>,
    pub action: RiskActionKind,
    pub logical_request_id: String,
    pub account_id: String,
    pub broker_connection_id: String,
    pub instrument_id: String,
    pub approved_delta_lots: i64,
    pub validity: RiskValidityContext,
}

impl RiskApprovedExecutionPlan {
    pub fn from_persisted(
        decision: &RiskDecision,
        reservation: Option<&RiskReservation>,
        request: &RiskRequest,
    ) -> Result<Self, RiskApprovedExecutionPlanError> {
        if !decision.permits_dispatch() {
            return Err(RiskApprovedExecutionPlanError::DecisionDoesNotPermitDispatch);
        }
        if decision.request_id != request.request_id {
            return Err(RiskApprovedExecutionPlanError::RequestIdentityMismatch);
        }
        if decision.account_id != request.account_id {
            return Err(RiskApprovedExecutionPlanError::AccountMismatch);
        }
        if decision.action != request.action {
            return Err(RiskApprovedExecutionPlanError::ActionMismatch);
        }
        if decision.validity != request.snapshot.validity {
            return Err(RiskApprovedExecutionPlanError::ValidityMismatch);
        }

        let reservation_id = match request.action {
            RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder => {
                let reservation =
                    reservation.ok_or(RiskApprovedExecutionPlanError::ReservationRequired)?;
                if reservation.logical_request_id != request.request_id {
                    return Err(RiskApprovedExecutionPlanError::RequestIdentityMismatch);
                }
                if reservation.account_id != request.account_id {
                    return Err(RiskApprovedExecutionPlanError::AccountMismatch);
                }
                if reservation.instrument_id != request.instrument_id {
                    return Err(RiskApprovedExecutionPlanError::InstrumentMismatch);
                }
                if decision.reservation_id.as_deref() != Some(reservation.reservation_id.as_str()) {
                    return Err(RiskApprovedExecutionPlanError::ReservationMismatch);
                }
                if decision.approved_delta_lots != reservation.reserved_delta_lots
                    || reservation.remaining_delta_lots != decision.approved_delta_lots
                {
                    return Err(RiskApprovedExecutionPlanError::QuantityMismatch);
                }
                if !matches!(reservation.state, ReservationState::Active) {
                    return Err(RiskApprovedExecutionPlanError::ReservationNotActive);
                }
                Some(reservation.reservation_id.clone())
            }
            RiskActionKind::CancelOrder
            | RiskActionKind::ProtectionMaintenance
            | RiskActionKind::CancelProtection => {
                if reservation.is_some() || decision.reservation_id.is_some() {
                    return Err(RiskApprovedExecutionPlanError::UnexpectedReservation);
                }
                if decision.approved_delta_lots != 0 {
                    return Err(RiskApprovedExecutionPlanError::QuantityMismatch);
                }
                None
            }
        };

        Ok(Self {
            decision_id: decision.decision_id.clone(),
            reservation_id,
            action: decision.action,
            logical_request_id: request.request_id.clone(),
            account_id: request.account_id.clone(),
            broker_connection_id: request.broker_connection_id.clone(),
            instrument_id: request.instrument_id.clone(),
            approved_delta_lots: decision.approved_delta_lots,
            validity: decision.validity.clone(),
        })
    }

    /// Re-validates correctness-critical state immediately before dispatch.
    #[must_use]
    pub fn matches_dispatch_context(
        &self,
        logical_request_id: &str,
        account_id: &str,
        broker_connection_id: &str,
        instrument_id: &str,
        action: RiskActionKind,
        delta_lots: i64,
        current_validity: &RiskValidityContext,
    ) -> bool {
        self.logical_request_id == logical_request_id
            && self.account_id == account_id
            && self.broker_connection_id == broker_connection_id
            && self.instrument_id == instrument_id
            && self.action == action
            && self.approved_delta_lots == delta_lots
            && &self.validity == current_validity
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum RiskApprovedExecutionPlanError {
    #[error("risk decision does not permit dispatch")]
    DecisionDoesNotPermitDispatch,
    #[error("risk decision/reservation request identity mismatch")]
    RequestIdentityMismatch,
    #[error("risk decision/reservation account mismatch")]
    AccountMismatch,
    #[error("risk decision/request action mismatch")]
    ActionMismatch,
    #[error("risk reservation instrument mismatch")]
    InstrumentMismatch,
    #[error("risk decision does not reference the persisted reservation")]
    ReservationMismatch,
    #[error("directional risk approval requires a persisted reservation")]
    ReservationRequired,
    #[error("non-exposure risk approval must not carry a reservation")]
    UnexpectedReservation,
    #[error("approved and reserved quantities differ")]
    QuantityMismatch,
    #[error("risk reservation is not ACTIVE")]
    ReservationNotActive,
    #[error("risk approval validity watermark changed")]
    ValidityMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BrokerLotLimits, RiskActionKind, RiskOutcome, RiskReason, RiskReasonCode, RiskSnapshot,
        RiskSource,
    };

    fn validity() -> RiskValidityContext {
        RiskValidityContext {
            runtime_epoch: 1,
            reconciliation_revision: 2,
            position_revision: 3,
            order_revision: 4,
            market_data_as_of_unix_ms: Some(5),
            instrument_constraints_revision: 6,
            policy_revision: 7,
            execution_authorization_revision: 8,
        }
    }

    fn request() -> RiskRequest {
        RiskRequest {
            request_id: "req-1".into(),
            account_id: "account-1".into(),
            broker_connection_id: "connection-1".into(),
            instrument_id: "instrument-1".into(),
            strategy_id: None,
            source: RiskSource::Manual,
            action: RiskActionKind::DirectionalOrder,
            requested_delta_lots: 10,
            requested_notional_nanos: 10,
            is_market_order: false,
            confirm_margin_trade: false,
            protection_established_or_planned: false,
            emergency_reduction: false,
            now_unix_ms: 10,
            snapshot: RiskSnapshot {
                runtime_ready: true,
                execution_authorized: true,
                unresolved_unknown_conflict: false,
                current_position_lots: 0,
                open_order_delta_lots: 0,
                unresolved_unknown_delta_lots: 0,
                active_reservation_delta_lots: 0,
                gross_exposure_nanos: 0,
                net_exposure_nanos: 0,
                instrument_exposure_nanos: 0,
                broker_daily_pnl_nanos: None,
                broker_lot_limits: Some(BrokerLotLimits::default()),
                margin: None,
                validity: validity(),
            },
        }
    }

    fn decision() -> RiskDecision {
        RiskDecision {
            decision_id: "decision-1".into(),
            request_id: "req-1".into(),
            policy_revision: 7,
            account_id: "account-1".into(),
            action: RiskActionKind::DirectionalOrder,
            requested_delta_lots: 10,
            approved_delta_lots: 10,
            outcome: RiskOutcome::Approve,
            reasons: vec![RiskReason::new(RiskReasonCode::Approved, "approved")],
            reservation_id: Some("reservation-1".into()),
            expires_at_unix_ms: None,
            validity: validity(),
        }
    }

    fn reservation() -> RiskReservation {
        RiskReservation {
            reservation_id: "reservation-1".into(),
            account_id: "account-1".into(),
            instrument_id: "instrument-1".into(),
            strategy_id: None,
            source: RiskSource::Manual,
            logical_request_id: "req-1".into(),
            reserved_delta_lots: 10,
            remaining_delta_lots: 10,
            reserved_notional_nanos: 10,
            state: ReservationState::Active,
            created_at_unix_ms: 10,
            updated_at_unix_ms: 10,
            expires_at_unix_ms: None,
        }
    }

    #[test]
    fn persisted_decision_and_reservation_create_dispatch_plan() {
        let request = request();
        let plan =
            RiskApprovedExecutionPlan::from_persisted(&decision(), Some(&reservation()), &request)
                .expect("plan");
        assert!(plan.matches_dispatch_context(
            "req-1",
            "account-1",
            "connection-1",
            "instrument-1",
            RiskActionKind::DirectionalOrder,
            10,
            &validity(),
        ));
    }

    #[test]
    fn changed_authorization_revision_invalidates_dispatch_context() {
        let request = request();
        let plan =
            RiskApprovedExecutionPlan::from_persisted(&decision(), Some(&reservation()), &request)
                .expect("plan");
        let mut changed = validity();
        changed.execution_authorization_revision += 1;
        assert!(!plan.matches_dispatch_context(
            "req-1",
            "account-1",
            "connection-1",
            "instrument-1",
            RiskActionKind::DirectionalOrder,
            10,
            &changed,
        ));
    }

    #[test]
    fn maintenance_plan_requires_no_reservation() {
        let mut request = request();
        request.action = RiskActionKind::CancelOrder;
        request.requested_delta_lots = 0;
        let mut decision = decision();
        decision.action = RiskActionKind::CancelOrder;
        decision.requested_delta_lots = 0;
        decision.approved_delta_lots = 0;
        decision.reservation_id = None;

        let plan = RiskApprovedExecutionPlan::from_persisted(&decision, None, &request)
            .expect("maintenance plan");
        assert!(plan.reservation_id.is_none());
        assert!(plan.matches_dispatch_context(
            "req-1",
            "account-1",
            "connection-1",
            "instrument-1",
            RiskActionKind::CancelOrder,
            0,
            &validity(),
        ));
    }

    #[test]
    fn unknown_held_reservation_cannot_create_fresh_dispatch_plan() {
        let request = request();
        let mut reservation = reservation();
        reservation.state = ReservationState::UnknownHeld;
        assert_eq!(
            RiskApprovedExecutionPlan::from_persisted(&decision(), Some(&reservation), &request),
            Err(RiskApprovedExecutionPlanError::ReservationNotActive)
        );
    }
}
