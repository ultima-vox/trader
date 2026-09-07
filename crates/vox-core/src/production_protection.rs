use vox_connections::ConnectionId;
use vox_domain::{PositionSide, ProtectionLeg, ProtectionPlanId, RuntimeExecutionCommand};
use vox_risk::{ProtectionPlanState, RiskProtectionLegRow, RiskStore};
use vox_runtime::{RiskAdmissionError, RiskDispatchOutcome, RuntimeScope, RuntimeStore};

use super::ProductionRiskAdapter;

impl ProductionRiskAdapter {
    pub(crate) fn canonical_protection_identity(
        &self,
        entry_reservation_id: Option<&str>,
        instrument_id: &str,
    ) -> Result<Option<ProtectionPlanId>, RiskAdmissionError> {
        let Some(entry_reservation_id) = entry_reservation_id else {
            return Ok(None);
        };
        let reservation = self
            .risk_store
            .reservation_by_id(&self.canonical_account_id, entry_reservation_id)
            .map_err(super::store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Denied {
                code: "ENTRY_RESERVATION_NOT_FOUND".into(),
                message: "protection entry reservation does not exist".into(),
            })?;
        if reservation.instrument_id != instrument_id {
            return Err(RiskAdmissionError::Denied {
                code: "PROTECTION_SCOPE_MISMATCH".into(),
                message: "protection instrument does not match entry reservation".into(),
            });
        }
        let plan = self
            .risk_store
            .protection_plan_by_entry_reservation(&self.canonical_account_id, entry_reservation_id)
            .map_err(super::store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Denied {
                code: "PROTECTION_PLAN_NOT_FOUND".into(),
                message: "entry reservation has no canonical protection plan".into(),
            })?;
        plan.canonical_plan_id
            .map(ProtectionPlanId::new)
            .transpose()
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))
    }

    pub(super) async fn protection_status(
        &self,
        report: &vox_runtime::ReconciliationReport,
        instrument_id: &str,
        lot_size: i64,
    ) -> Result<vox_risk::RiskProtectionStatus, RiskAdmissionError> {
        let position_lots = report
            .positions
            .iter()
            .find(|position| position.instrument_uid == instrument_id)
            .map(|position| normalize_position_lots(position.quantity_units, lot_size))
            .transpose()?
            .unwrap_or(0);
        let active_stop_lots = report
            .active_stops
            .iter()
            .filter(|stop| stop_covers_position(stop, instrument_id, position_lots))
            .filter_map(|stop| stop.quantity_lots)
            .sum();
        let plans = self
            .risk_store
            .protection_plans_for_instrument(&self.canonical_account_id, instrument_id)
            .map_err(super::store_unavailable)?;
        let latest = plans.iter().max_by_key(|plan| plan.created_at_unix_ms);
        Ok(vox_risk::RiskProtectionStatus {
            active_stop_lots,
            position_lots,
            position_entered_at_unix_ms: latest.map(|plan| plan.created_at_unix_ms),
            plan_id: latest.and_then(|plan| plan.canonical_plan_id.clone()),
            plan_state: latest.map(|plan| plan.state),
        })
    }

    pub(super) async fn prepare_protection(
        &self,
        scope: &RuntimeScope,
        command: &RuntimeExecutionCommand,
    ) -> Result<(), RiskAdmissionError> {
        let RuntimeExecutionCommand::ProtectionLeg(command) = command else {
            return Ok(());
        };
        let entry_reservation_id =
            command
                .entry_reservation_id
                .as_deref()
                .ok_or_else(|| RiskAdmissionError::Denied {
                    code: "ENTRY_RESERVATION_REQUIRED".into(),
                    message: "canonical protection leg requires entry reservation".into(),
                })?;
        let canonical_plan_id =
            command
                .canonical_plan_id
                .as_ref()
                .ok_or_else(|| RiskAdmissionError::Denied {
                    code: "CANONICAL_PROTECTION_PLAN_REQUIRED".into(),
                    message: "canonical protection leg requires plan identity".into(),
                })?;
        let plan = self
            .risk_store
            .protection_plan_by_entry_reservation(&self.canonical_account_id, entry_reservation_id)
            .map_err(super::store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Denied {
                code: "PROTECTION_PLAN_NOT_FOUND".into(),
                message: "entry reservation has no protection plan".into(),
            })?;
        if plan.canonical_plan_id.as_deref() != Some(canonical_plan_id.as_str())
            || plan.instrument_id != command.instrument_id
            || command.quantity_lots <= 0
            || command.quantity_lots.unsigned_abs() > plan.protected_delta_lots.unsigned_abs()
        {
            return Err(RiskAdmissionError::Denied {
                code: "PROTECTION_CORRELATION_MISMATCH".into(),
                message: "protection leg does not match approved entry plan".into(),
            });
        }
        let expected_side = if plan.protected_delta_lots > 0 {
            PositionSide::Long
        } else {
            PositionSide::Short
        };
        if command.position_side != expected_side {
            return Err(RiskAdmissionError::Denied {
                code: "PROTECTION_DIRECTION_MISMATCH".into(),
                message: "protection direction does not oppose entry position".into(),
            });
        }
        let reservation = self
            .risk_store
            .reservation_by_id(&self.canonical_account_id, entry_reservation_id)
            .map_err(super::store_unavailable)?
            .ok_or_else(|| {
                RiskAdmissionError::Unavailable("entry reservation disappeared".into())
            })?;
        let decision = self
            .risk_store
            .decision_for_request(&self.canonical_account_id, &reservation.logical_request_id)
            .map_err(super::store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Unavailable("entry decision disappeared".into()))?;
        let connection_id = ConnectionId::parse(scope.connection_ref.as_str().to_owned())
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let reads = self
            .factory
            .read_session(&connection_id, &scope.broker_account_id)
            .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
        let lot_size = reads
            .risk_reads
            .instrument_constraints(&command.instrument_id)
            .await
            .map_err(|error| RiskAdmissionError::Denied {
                code: "INSTRUMENT_UNAVAILABLE".into(),
                message: error.to_string(),
            })?
            .lot_size;
        self.risk_store
            .put_protection_leg(&RiskProtectionLegRow {
                canonical_plan_id: canonical_plan_id.as_str().to_owned(),
                entry_decision_id: decision.decision_id,
                entry_reservation_id: entry_reservation_id.to_owned(),
                account_id: self.canonical_account_id.clone(),
                instrument_id: command.instrument_id.clone(),
                command_id: command.client_request_id.clone(),
                broker_stop_order_id: None,
                is_stop_loss: matches!(command.leg, ProtectionLeg::StopLoss(_)),
                position_lots: if expected_side == PositionSide::Long {
                    command.quantity_lots
                } else {
                    -command.quantity_lots
                },
                lot_size,
                state: ProtectionPlanState::Planned,
                created_at_unix_ms: super::now_unix_ms()?,
                updated_at_unix_ms: super::now_unix_ms()?,
            })
            .map_err(super::store_unavailable)?;
        Ok(())
    }

    pub(super) fn protection_dispatch_outcome(
        &self,
        scope: &RuntimeScope,
        logical_request_id: &str,
        outcome: RiskDispatchOutcome,
        now_unix_ms: i64,
    ) -> Result<(), RiskAdmissionError> {
        let Some(mut leg) = self
            .risk_store
            .protection_leg_for_command(&self.canonical_account_id, logical_request_id)
            .map_err(super::store_unavailable)?
        else {
            return Ok(());
        };
        if outcome == RiskDispatchOutcome::Acknowledged {
            leg.broker_stop_order_id = self
                .runtime_store
                .all_identity_links(&scope.key())
                .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?
                .into_iter()
                .find(|links| links.logical_request_id == logical_request_id)
                .and_then(|links| links.broker_stop_order_id);
        }
        leg.state = match outcome {
            RiskDispatchOutcome::Acknowledged | RiskDispatchOutcome::UnknownAfterDispatch => {
                ProtectionPlanState::Submitted
            }
            RiskDispatchOutcome::Rejected => ProtectionPlanState::Failed,
        };
        leg.updated_at_unix_ms = now_unix_ms;
        self.risk_store
            .put_protection_leg(&leg)
            .map_err(super::store_unavailable)?;
        let plan = self
            .risk_store
            .protection_plan_by_entry_reservation(
                &self.canonical_account_id,
                &leg.entry_reservation_id,
            )
            .map_err(super::store_unavailable)?
            .ok_or_else(|| RiskAdmissionError::Unavailable("protection plan disappeared".into()))?;
        self.risk_store
            .transition_protection_plan(
                &plan.plan_id,
                &[ProtectionPlanState::Planned, ProtectionPlanState::Submitted],
                leg.state,
                now_unix_ms,
            )
            .or_else(|error| {
                if plan.state == leg.state {
                    Ok(plan)
                } else {
                    Err(error)
                }
            })
            .map_err(super::store_unavailable)?;
        Ok(())
    }

    pub(super) fn reconcile_protection(
        &self,
        scope: &RuntimeScope,
        report: &vox_runtime::ReconciliationReport,
    ) -> Result<(), RiskAdmissionError> {
        for mut plan in self
            .risk_store
            .protection_plans(&self.canonical_account_id)
            .map_err(super::store_unavailable)?
        {
            let Some(canonical_plan_id) = plan.canonical_plan_id.as_deref() else {
                return Err(RiskAdmissionError::Unavailable(
                    "persisted protection plan lacks canonical identity".into(),
                ));
            };
            let position = report
                .positions
                .iter()
                .find(|position| position.instrument_uid == plan.instrument_id);
            let mut legs = self
                .risk_store
                .protection_legs(canonical_plan_id)
                .map_err(super::store_unavailable)?;
            let identity_links = self
                .runtime_store
                .all_identity_links(&scope.key())
                .map_err(|error| RiskAdmissionError::Unavailable(error.to_string()))?;
            for leg in &mut legs {
                if leg.broker_stop_order_id.is_none()
                    && let Some(broker_stop_order_id) = identity_links
                        .iter()
                        .find(|links| links.logical_request_id == leg.command_id)
                        .and_then(|links| links.broker_stop_order_id.clone())
                {
                    leg.broker_stop_order_id = Some(broker_stop_order_id);
                    leg.updated_at_unix_ms = report.completed_at_unix_ms;
                    self.risk_store
                        .put_protection_leg(leg)
                        .map_err(super::store_unavailable)?;
                }
            }
            let lot_size = legs.first().map_or(1, |leg| leg.lot_size);
            let position_lots = position
                .map(|position| normalize_position_lots(position.quantity_units, lot_size))
                .transpose()?
                .unwrap_or(0);
            let covered_lots: i64 = legs
                .iter()
                .filter(|leg| {
                    leg.is_stop_loss && leg.position_lots.signum() == position_lots.signum()
                })
                .filter_map(|leg| {
                    let broker_id = leg.broker_stop_order_id.as_deref()?;
                    report.active_stops.iter().find(|stop| {
                        stop.broker_stop_order_id == broker_id
                            && stop_covers_position(stop, &plan.instrument_id, position_lots)
                    })
                })
                .filter_map(|stop| stop.quantity_lots)
                .sum();
            let target = if position_lots == 0 {
                ProtectionPlanState::Cancelled
            } else if covered_lots >= position_lots.unsigned_abs() as i64 {
                ProtectionPlanState::FullCoverage
            } else if covered_lots > 0 {
                ProtectionPlanState::PartialCoverage
            } else if plan.state == ProtectionPlanState::Planned && legs.is_empty() {
                ProtectionPlanState::Planned
            } else {
                ProtectionPlanState::Stale
            };
            if target != plan.state {
                plan.state = target;
                plan.updated_at_unix_ms = report.completed_at_unix_ms;
                self.risk_store
                    .put_protection_plan(&plan)
                    .map_err(super::store_unavailable)?;
            }
        }
        Ok(())
    }
}

fn normalize_position_lots(quantity_units: i64, lot_size: i64) -> Result<i64, RiskAdmissionError> {
    if lot_size <= 0 || quantity_units % lot_size != 0 {
        return Err(RiskAdmissionError::Unavailable(
            "broker position is not exactly divisible by instrument lot".into(),
        ));
    }
    Ok(quantity_units / lot_size)
}

fn stop_covers_position(
    stop: &vox_runtime::StopFact,
    instrument_id: &str,
    position_lots: i64,
) -> bool {
    stop.instrument_uid == instrument_id
        && stop.status.active()
        && stop.stop_order_type == Some(3)
        && stop.quantity_lots.is_some_and(|lots| lots > 0)
        && ((position_lots > 0 && stop.direction == Some(2))
            || (position_lots < 0 && stop.direction == Some(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_runtime::{StopExecutionStatus, StopFact};

    fn stop(stop_order_type: i32, direction: i32) -> StopFact {
        StopFact {
            account_id: "account-1".into(),
            broker_stop_order_id: "stop-1".into(),
            instrument_uid: "instrument-1".into(),
            status: StopExecutionStatus::Active,
            status_cause: None,
            quantity_lots: Some(3),
            direction: Some(direction),
            stop_order_type: Some(stop_order_type),
        }
    }

    #[test]
    fn lot_size_above_one_normalizes_units_without_truncation() {
        assert_eq!(normalize_position_lots(30, 10), Ok(3));
        assert!(normalize_position_lots(31, 10).is_err());
    }

    #[test]
    fn only_opposing_stop_loss_covers_position() {
        assert!(stop_covers_position(&stop(3, 2), "instrument-1", 3));
        assert!(!stop_covers_position(&stop(2, 2), "instrument-1", 3));
        assert!(!stop_covers_position(&stop(3, 1), "instrument-1", 3));
        assert!(stop_covers_position(&stop(3, 1), "instrument-1", -3));
    }
}
