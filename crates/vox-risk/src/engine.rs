use thiserror::Error;

use crate::model::{
    BrokerLotLimits, LotLimit, RiskDecision, RiskOutcome, RiskPolicySet, RiskReason,
    RiskReasonCode, RiskRequest, RiskState,
};

pub struct RiskEngine;

impl RiskEngine {
    pub fn evaluate(
        policy: &RiskPolicySet,
        request: &RiskRequest,
    ) -> Result<RiskDecision, RiskEngineError> {
        if request.requested_delta_lots == 0 {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::InvalidQuantity,
                "requested quantity must be non-zero",
            ));
        }

        if request.snapshot.validity.policy_revision != policy.revision {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::PolicyRevisionChanged,
                "risk policy revision changed before evaluation",
            ));
        }

        if !request.snapshot.runtime_ready {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Halt,
                RiskReasonCode::RuntimeNotReady,
                "runtime is not READY",
            ));
        }
        if !request.snapshot.execution_authorized {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::ExecutionUnauthorized,
                "Vox execution authorization is disabled",
            ));
        }
        if request.snapshot.unresolved_unknown_conflict {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Halt,
                RiskReasonCode::UnknownMutationConflict,
                "conflicting UNKNOWN_AFTER_DISPATCH mutation exists",
            ));
        }

        let base_position = request
            .snapshot
            .current_position_lots
            .checked_add(request.snapshot.open_order_delta_lots)
            .and_then(|value| value.checked_add(request.snapshot.unresolved_unknown_delta_lots))
            .and_then(|value| value.checked_add(request.snapshot.active_reservation_delta_lots))
            .ok_or(RiskEngineError::ArithmeticOverflow)?;
        let projected_position = base_position
            .checked_add(request.requested_delta_lots)
            .ok_or(RiskEngineError::ArithmeticOverflow)?;
        let increasing_lots = increasing_portion(base_position, request.requested_delta_lots)?;

        match policy.state {
            RiskState::Halted if !(request.emergency_reduction && increasing_lots == 0) => {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Halt,
                    RiskReasonCode::Halted,
                    "risk state is HALTED",
                ));
            }
            RiskState::ReduceOnly if increasing_lots > 0 => {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::ReduceOnly,
                    RiskReasonCode::ReduceOnly,
                    "risk state permits reductions only",
                ));
            }
            RiskState::Normal | RiskState::Warning | RiskState::ReduceOnly | RiskState::Halted => {}
        }

        if increasing_lots > 0 {
            if let Some(decision) = validate_market_data(policy, request)? {
                return Ok(decision);
            }
            if policy.protection_required_for_new_exposure
                && !request.protection_established_or_planned
            {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::ProtectionRequired,
                    "new exposure requires an explicit protection plan",
                ));
            }
        }

        if request.confirm_margin_trade && !policy.allow_margin {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::MarginNotAllowed,
                "margin execution is disabled by Vox risk policy",
            ));
        }

        if let Some(max_loss) = policy.max_daily_loss_nanos
            && let Some(day_pnl) = request.snapshot.broker_daily_pnl_nanos
            && day_pnl < -max_loss.abs()
            && increasing_lots > 0
        {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::ReduceOnly,
                RiskReasonCode::DailyLossExceeded,
                "broker day P&L breached the configured loss limit",
            ));
        }

        if let Some(max_position) = policy.max_position_abs_lots
            && projected_position.unsigned_abs() > max_position.unsigned_abs()
            && increasing_lots > 0
        {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::MaxPositionExceeded,
                "projected position exceeds the configured position limit",
            ));
        }

        if let Some(max_gross) = policy.max_gross_exposure_nanos {
            let projected = request
                .snapshot
                .gross_exposure_nanos
                .checked_add(request.requested_notional_nanos.abs())
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if projected > max_gross.abs() && increasing_lots > 0 {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MaxGrossExposureExceeded,
                    "projected gross exposure exceeds policy",
                ));
            }
        }

        if let Some(max_net) = policy.max_net_exposure_abs_nanos {
            let projected = request
                .snapshot
                .net_exposure_nanos
                .checked_add(request.requested_notional_nanos)
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if projected.abs() > max_net.abs() && increasing_lots > 0 {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MaxNetExposureExceeded,
                    "projected net exposure exceeds policy",
                ));
            }
        }

        if let Some(max_instrument) = policy.max_instrument_exposure_nanos {
            let projected = request
                .snapshot
                .instrument_exposure_nanos
                .checked_add(request.requested_notional_nanos)
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if projected.abs() > max_instrument.abs() && increasing_lots > 0 {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MaxInstrumentExposureExceeded,
                    "projected instrument exposure exceeds policy",
                ));
            }
        }

        let requested_abs = i64::try_from(request.requested_delta_lots.unsigned_abs())
            .map_err(|_| RiskEngineError::ArithmeticOverflow)?;
        let mut approved_abs = requested_abs;
        let mut resize_reason = None;

        if let Some(max_order) = policy.max_single_order_lots {
            if max_order <= 0 {
                return Err(RiskEngineError::InvalidPolicy(
                    "max_single_order_lots must be positive",
                ));
            }
            if approved_abs > max_order {
                approved_abs = max_order;
                resize_reason = Some((
                    RiskReasonCode::ResizedToPolicyLimit,
                    "quantity resized to configured max single-order limit",
                ));
            }
        }

        match provider_limit(policy, request)? {
            Some(limit) if approved_abs > limit => {
                approved_abs = limit;
                resize_reason = Some((
                    RiskReasonCode::ResizedToProviderLimit,
                    "quantity resized to the applicable broker max-lots constraint",
                ));
            }
            None if policy.require_provider_lot_limit && increasing_lots > 0 => {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::ProviderLimitUnavailable,
                    "applicable broker max-lots constraint is unavailable",
                ));
            }
            _ => {}
        }

        if approved_abs == 0 {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::ProviderLimitExceeded,
                "broker/policy constraints leave zero executable quantity",
            ));
        }

        let sign = request.requested_delta_lots.signum();
        let approved_delta_lots = approved_abs
            .checked_mul(sign)
            .ok_or(RiskEngineError::ArithmeticOverflow)?;
        let (outcome, reasons) = if approved_delta_lots == request.requested_delta_lots {
            (
                RiskOutcome::Approve,
                vec![RiskReason::new(
                    RiskReasonCode::Approved,
                    "request satisfies current risk policy and provider constraints",
                )],
            )
        } else {
            let (code, message) = resize_reason.ok_or(RiskEngineError::ArithmeticOverflow)?;
            (RiskOutcome::Resize, vec![RiskReason::new(code, message)])
        };

        Ok(RiskDecision {
            decision_id: RiskDecision::new_id(),
            request_id: request.request_id.clone(),
            policy_revision: policy.revision,
            account_id: request.account_id.clone(),
            requested_delta_lots: request.requested_delta_lots,
            approved_delta_lots,
            outcome,
            reasons,
            reservation_id: None,
            expires_at_unix_ms: None,
            validity: request.snapshot.validity.clone(),
        })
    }

    #[must_use]
    pub fn still_valid(
        decision: &RiskDecision,
        request: &RiskRequest,
        current_policy_revision: u64,
    ) -> bool {
        decision.policy_revision == current_policy_revision
            && decision.request_id == request.request_id
            && decision.validity == request.snapshot.validity
            && decision.approved_delta_lots != 0
            && decision.permits_dispatch()
    }
}

fn validate_market_data(
    policy: &RiskPolicySet,
    request: &RiskRequest,
) -> Result<Option<RiskDecision>, RiskEngineError> {
    let Some(max_age) = policy.max_market_data_age_ms else {
        return Ok(None);
    };
    if max_age < 0 {
        return Err(RiskEngineError::InvalidPolicy(
            "max_market_data_age_ms cannot be negative",
        ));
    }
    let Some(as_of) = request.snapshot.validity.market_data_as_of_unix_ms else {
        return Ok(Some(reject(
            policy,
            request,
            RiskOutcome::Reject,
            RiskReasonCode::MarketDataMissing,
            "price-dependent new exposure has no market-data watermark",
        )));
    };
    let age = request.now_unix_ms.saturating_sub(as_of);
    if age > max_age {
        return Ok(Some(reject(
            policy,
            request,
            RiskOutcome::Reject,
            RiskReasonCode::MarketDataStale,
            "price-dependent new exposure uses stale market data",
        )));
    }
    Ok(None)
}

fn provider_limit(
    policy: &RiskPolicySet,
    request: &RiskRequest,
) -> Result<Option<i64>, RiskEngineError> {
    let Some(limits) = request.snapshot.broker_lot_limits else {
        return Ok(None);
    };
    let wants_margin = request.confirm_margin_trade;
    if wants_margin && !policy.allow_margin {
        return Ok(None);
    }
    let selected = if request.requested_delta_lots > 0 {
        if wants_margin { limits.buy_margin } else { limits.buy_own }
    } else if wants_margin {
        limits.sell_margin
    } else {
        limits.sell_own
    };
    selected
        .map(|limit| selected_lot_limit(limit, request.is_market_order))
        .transpose()
}

fn selected_lot_limit(limit: LotLimit, market: bool) -> Result<i64, RiskEngineError> {
    let value = if market {
        limit.max_market_lots
    } else {
        limit.max_lots
    };
    if value < 0 {
        return Err(RiskEngineError::InvalidProviderFact(
            "broker max-lots value cannot be negative",
        ));
    }
    Ok(value)
}

fn increasing_portion(base: i64, delta: i64) -> Result<i64, RiskEngineError> {
    if delta == 0 {
        return Ok(0);
    }
    if base == 0 || base.signum() == delta.signum() {
        return i64::try_from(delta.unsigned_abs()).map_err(|_| RiskEngineError::ArithmeticOverflow);
    }
    let base_abs =
        i64::try_from(base.unsigned_abs()).map_err(|_| RiskEngineError::ArithmeticOverflow)?;
    let delta_abs =
        i64::try_from(delta.unsigned_abs()).map_err(|_| RiskEngineError::ArithmeticOverflow)?;
    Ok(delta_abs.saturating_sub(base_abs))
}

fn reject(
    policy: &RiskPolicySet,
    request: &RiskRequest,
    outcome: RiskOutcome,
    code: RiskReasonCode,
    message: &'static str,
) -> RiskDecision {
    RiskDecision {
        decision_id: RiskDecision::new_id(),
        request_id: request.request_id.clone(),
        policy_revision: policy.revision,
        account_id: request.account_id.clone(),
        requested_delta_lots: request.requested_delta_lots,
        approved_delta_lots: 0,
        outcome,
        reasons: vec![RiskReason::new(code, message)],
        reservation_id: None,
        expires_at_unix_ms: None,
        validity: request.snapshot.validity.clone(),
    }
}

#[derive(Debug, Error)]
pub enum RiskEngineError {
    #[error("risk arithmetic overflow")]
    ArithmeticOverflow,
    #[error("invalid risk policy: {0}")]
    InvalidPolicy(&'static str),
    #[error("invalid provider risk fact: {0}")]
    InvalidProviderFact(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BrokerLotLimits, LotLimit, RiskSnapshot, RiskSource, RiskValidityContext,
    };

    fn request(base: i64, delta: i64) -> RiskRequest {
        RiskRequest {
            request_id: "req-1".to_owned(),
            account_id: "account-1".to_owned(),
            broker_connection_id: "connection-1".to_owned(),
            instrument_id: "instrument-1".to_owned(),
            strategy_id: None,
            source: RiskSource::Manual,
            requested_delta_lots: delta,
            requested_notional_nanos: i128::from(delta) * 1_000_000_000,
            is_market_order: false,
            confirm_margin_trade: false,
            protection_established_or_planned: false,
            emergency_reduction: false,
            now_unix_ms: 10_000,
            snapshot: RiskSnapshot {
                runtime_ready: true,
                execution_authorized: true,
                unresolved_unknown_conflict: false,
                current_position_lots: base,
                open_order_delta_lots: 0,
                unresolved_unknown_delta_lots: 0,
                active_reservation_delta_lots: 0,
                gross_exposure_nanos: 0,
                net_exposure_nanos: 0,
                instrument_exposure_nanos: 0,
                broker_daily_pnl_nanos: None,
                broker_lot_limits: Some(BrokerLotLimits {
                    buy_own: Some(LotLimit {
                        max_lots: 100,
                        max_market_lots: 90,
                    }),
                    buy_margin: Some(LotLimit {
                        max_lots: 200,
                        max_market_lots: 180,
                    }),
                    sell_own: Some(LotLimit {
                        max_lots: 100,
                        max_market_lots: 90,
                    }),
                    sell_margin: Some(LotLimit {
                        max_lots: 200,
                        max_market_lots: 180,
                    }),
                }),
                margin: None,
                validity: RiskValidityContext {
                    runtime_epoch: 1,
                    reconciliation_revision: 2,
                    position_revision: 3,
                    order_revision: 4,
                    market_data_as_of_unix_ms: Some(9_900),
                    instrument_constraints_revision: 5,
                    policy_revision: 6,
                    execution_authorization_revision: 7,
                },
            },
        }
    }

    fn policy() -> RiskPolicySet {
        RiskPolicySet {
            revision: 6,
            state: RiskState::Normal,
            allow_margin: false,
            require_provider_lot_limit: true,
            max_market_data_age_ms: Some(1_000),
            max_single_order_lots: Some(50),
            max_position_abs_lots: Some(100),
            max_gross_exposure_nanos: None,
            max_net_exposure_abs_nanos: None,
            max_instrument_exposure_nanos: None,
            max_daily_loss_nanos: None,
            protection_required_for_new_exposure: false,
        }
    }

    #[test]
    fn ordinary_order_is_approved() {
        let decision = RiskEngine::evaluate(&policy(), &request(0, 10)).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
        assert_eq!(decision.approved_delta_lots, 10);
    }

    #[test]
    fn oversized_order_is_resized() {
        let decision = RiskEngine::evaluate(&policy(), &request(0, 75)).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Resize);
        assert_eq!(decision.approved_delta_lots, 50);
    }

    #[test]
    fn sell_close_is_not_new_exposure() {
        let mut p = policy();
        p.state = RiskState::ReduceOnly;
        let decision = RiskEngine::evaluate(&p, &request(10, -10)).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    #[test]
    fn reversal_has_new_exposure_and_is_blocked_in_reduce_only() {
        let mut p = policy();
        p.state = RiskState::ReduceOnly;
        let decision = RiskEngine::evaluate(&p, &request(10, -15)).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::ReduceOnly);
    }

    #[test]
    fn margin_provider_limit_cannot_bypass_vox_policy() {
        let mut r = request(0, 150);
        r.confirm_margin_trade = true;
        let decision = RiskEngine::evaluate(&policy(), &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::MarginNotAllowed);
    }

    #[test]
    fn stale_market_data_fails_closed_for_new_exposure() {
        let mut r = request(0, 10);
        r.snapshot.validity.market_data_as_of_unix_ms = Some(1);
        let decision = RiskEngine::evaluate(&policy(), &r).expect("risk decision");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::MarketDataStale);
    }

    #[test]
    fn decision_is_invalid_after_authorization_revision_change() {
        let r = request(0, 10);
        let decision = RiskEngine::evaluate(&policy(), &r).expect("risk");
        let mut changed = r.clone();
        changed.snapshot.validity.execution_authorization_revision += 1;
        assert!(!RiskEngine::still_valid(&decision, &changed, policy().revision));
    }
}
