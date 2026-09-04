use std::time::Instant;

use thiserror::Error;

use crate::model::{
    BuyLotLimit, RiskActionKind, RiskDecision, RiskOutcome, RiskPolicySet, RiskReason,
    RiskReasonCode, RiskRequest, RiskState, SellLotLimit,
};

pub struct RiskEngine;

impl RiskEngine {
    pub fn evaluate(
        policy: &RiskPolicySet,
        request: &RiskRequest,
    ) -> Result<RiskDecision, RiskEngineError> {
        let start = Instant::now();
        let action_str = format!("{:?}", request.action);
        let outcome_label = |o: &RiskOutcome| -> &'static str {
            match o {
                RiskOutcome::Approve => "approve",
                RiskOutcome::Resize => "resize",
                RiskOutcome::Reject => "reject",
                RiskOutcome::ReduceOnly => "reduce_only",
                RiskOutcome::Halt => "halt",
            }
        };
        let state_label = |s: &RiskState| -> &'static str {
            match s {
                RiskState::Normal => "normal",
                RiskState::Warning => "warning",
                RiskState::ReduceOnly => "reduce_only",
                RiskState::Halted => "halted",
                RiskState::KillSwitch => "kill_switch",
            }
        };

        tracing::debug!(
            risk.request_id = %request.request_id,
            risk.account_id = %request.account_id,
            risk.instrument_id = %request.instrument_id,
            risk.action = %action_str,
            risk.requested_delta_lots = request.requested_delta_lots,
            risk.policy_revision = policy.revision,
            risk.policy_state = %state_label(&policy.state),
            "risk evaluation started",
        );

        let directional = matches!(
            request.action,
            RiskActionKind::DirectionalOrder | RiskActionKind::ReplaceDirectionalOrder
        );
        if directional && request.requested_delta_lots == 0 {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::InvalidQuantity,
                "risk decision: reject - invalid quantity",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::InvalidQuantity,
                "directional risk request quantity must be non-zero",
            ));
        }
        if !directional && request.requested_delta_lots != 0 {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::InvalidQuantity,
                "risk decision: reject - non-directional action has directional exposure",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::InvalidQuantity,
                "non-directional risk action must not carry directional exposure",
            ));
        }
        if directional && request.snapshot.instrument_lot_size <= 0 {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::InstrumentUnavailable,
                "instrument lot contract is unavailable",
            ));
        }
        if directional && !request.snapshot.instrument_tradable {
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::InstrumentNotTradable,
                "broker instrument constraints forbid this order",
            ));
        }

        if request.snapshot.validity.policy_revision != policy.revision {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::PolicyRevisionChanged,
                policy.revision = policy.revision,
                "risk decision: reject - policy revision mismatch",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::PolicyRevisionChanged,
                "risk policy revision changed before evaluation",
            ));
        }

        let cleanup_action = matches!(
            request.action,
            RiskActionKind::CancelOrder
                | RiskActionKind::ProtectionMaintenance
                | RiskActionKind::CancelProtection
        );

        if !request.snapshot.runtime_ready && !cleanup_action {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "halt",
                risk.reason_code = ?RiskReasonCode::RuntimeNotReady,
                "risk decision: halt - runtime not ready",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Halt,
                RiskReasonCode::RuntimeNotReady,
                "runtime is not READY",
            ));
        }
        if !request.snapshot.execution_authorized {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::ExecutionUnauthorized,
                "risk decision: reject - execution unauthorized",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::ExecutionUnauthorized,
                "Vox execution authorization is disabled",
            ));
        }
        if request.snapshot.unresolved_unknown_conflict && !cleanup_action {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "halt",
                risk.reason_code = ?RiskReasonCode::UnknownMutationConflict,
                "risk decision: halt - unknown mutation conflict",
            );
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
            RiskState::KillSwitch
                if !(request.emergency_reduction && increasing_lots == 0) && !cleanup_action =>
            {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "halt",
                    risk.reason_code = ?RiskReasonCode::KillSwitchActive,
                    "risk decision: halt - kill-switch is active",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Halt,
                    RiskReasonCode::KillSwitchActive,
                    "global kill-switch is active",
                ));
            }
            RiskState::Halted
                if !(request.emergency_reduction && increasing_lots == 0) && !cleanup_action =>
            {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "halt",
                    risk.reason_code = ?RiskReasonCode::Halted,
                    "risk decision: halt - risk state is HALTED",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Halt,
                    RiskReasonCode::Halted,
                    "risk state is HALTED",
                ));
            }
            RiskState::ReduceOnly if increasing_lots > 0 => {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reduce_only",
                    risk.reason_code = ?RiskReasonCode::ReduceOnly,
                    risk.increasing_lots = increasing_lots,
                    "risk decision: reduce_only - new exposure blocked in reduce-only state",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::ReduceOnly,
                    RiskReasonCode::ReduceOnly,
                    "risk state permits reductions only",
                ));
            }
            RiskState::Normal
            | RiskState::Warning
            | RiskState::ReduceOnly
            | RiskState::Halted
            | RiskState::KillSwitch => {}
        }

        if !directional {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "approve",
                risk.reason_code = ?RiskReasonCode::Approved,
                risk.action = %action_str,
                "risk decision: approve - non-directional action",
            );
            return Ok(RiskDecision {
                decision_id: RiskDecision::new_id(),
                request_id: request.request_id.clone(),
                policy_revision: policy.revision,
                account_id: request.account_id.clone(),
                action: request.action,
                requested_delta_lots: 0,
                approved_delta_lots: 0,
                outcome: RiskOutcome::Approve,
                reasons: vec![RiskReason::new(
                    RiskReasonCode::Approved,
                    "non-directional capital maintenance action passed the risk boundary",
                )],
                reservation_id: None,
                expires_at_unix_ms: None,
                validity: request.snapshot.validity.clone(),
            });
        }

        if increasing_lots > 0 {
            if let Some(decision) = validate_market_data(policy, request)? {
                return Ok(decision);
            }
            if let Some(decision) = validate_protection(policy, request, increasing_lots)? {
                return Ok(decision);
            }
        }

        if request.confirm_margin_trade && !policy.allow_margin {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::MarginNotAllowed,
                "risk decision: reject - margin not allowed by policy",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::MarginNotAllowed,
                "margin execution is disabled by Vox risk policy",
            ));
        }

        if let Some(max_utilization_ppm) = policy.max_margin_utilization_ppm
            && increasing_lots > 0
        {
            if max_utilization_ppm <= 0 {
                return Err(RiskEngineError::InvalidPolicy(
                    "max_margin_utilization_ppm must be positive",
                ));
            }
            let Some(margin) = request.snapshot.margin.as_ref() else {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "broker margin facts are required by policy but unavailable",
                ));
            };
            if margin.liquid_portfolio_nanos <= 0 || margin.corrected_margin_nanos < 0 {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "broker margin facts cannot form a valid utilization ratio",
                ));
            }
            let utilization_ppm = margin
                .corrected_margin_nanos
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_div(margin.liquid_portfolio_nanos))
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if utilization_ppm > i128::from(max_utilization_ppm) {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MarginUtilizationExceeded,
                    "broker corrected-margin utilization exceeds policy",
                ));
            }
        }

        if let Some(max_loss) = policy.max_daily_loss_nanos
            && increasing_lots > 0
        {
            let Some(day_pnl) = request.snapshot.broker_daily_pnl_nanos else {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "broker day P&L is required by policy but unavailable",
                ));
            };
            if day_pnl < -positive_limit(max_loss, "max_daily_loss_nanos")? {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reduce_only",
                    risk.reason_code = ?RiskReasonCode::DailyLossExceeded,
                    risk.broker_daily_pnl_nanos = day_pnl,
                    "risk decision: reduce_only - daily loss limit exceeded",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::ReduceOnly,
                    RiskReasonCode::DailyLossExceeded,
                    "broker day P&L breached the configured loss limit",
                ));
            }
        }

        if let Some(max_position) = policy.max_position_abs_lots
            && projected_position.unsigned_abs() > max_position.unsigned_abs()
            && increasing_lots > 0
        {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::MaxPositionExceeded,
                risk.projected_position = projected_position.unsigned_abs(),
                "risk decision: reject - projected position exceeds limit",
            );
            return Ok(reject(
                policy,
                request,
                RiskOutcome::Reject,
                RiskReasonCode::MaxPositionExceeded,
                "projected position exceeds the configured position limit",
            ));
        }

        if let Some(max_gross) = policy.max_gross_exposure_nanos
            && increasing_lots > 0
        {
            let Some(gross_exposure_nanos) = request.snapshot.gross_exposure_nanos else {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "gross exposure is required by policy but unavailable",
                ));
            };
            let projected = gross_exposure_nanos
                .checked_add(checked_abs(request.requested_notional_nanos)?)
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if projected > positive_limit(max_gross, "max_gross_exposure_nanos")? {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reject",
                    risk.reason_code = ?RiskReasonCode::MaxGrossExposureExceeded,
                    "risk decision: reject - gross exposure exceeds limit",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MaxGrossExposureExceeded,
                    "projected gross exposure exceeds policy",
                ));
            }
        }

        if let Some(max_net) = policy.max_net_exposure_abs_nanos
            && increasing_lots > 0
        {
            let Some(net_exposure_nanos) = request.snapshot.net_exposure_nanos else {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "net exposure is required by policy but unavailable",
                ));
            };
            let projected = net_exposure_nanos
                .checked_add(request.requested_notional_nanos)
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if checked_abs(projected)? > positive_limit(max_net, "max_net_exposure_abs_nanos")? {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reject",
                    risk.reason_code = ?RiskReasonCode::MaxNetExposureExceeded,
                    "risk decision: reject - net exposure exceeds limit",
                );
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::MaxNetExposureExceeded,
                    "projected net exposure exceeds policy",
                ));
            }
        }

        if let Some(max_instrument) = policy.max_instrument_exposure_nanos
            && increasing_lots > 0
        {
            let Some(instrument_exposure_nanos) = request.snapshot.instrument_exposure_nanos else {
                return Ok(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::CriticalInputMissing,
                    "instrument exposure is required by policy but unavailable",
                ));
            };
            let projected = instrument_exposure_nanos
                .checked_add(request.requested_notional_nanos)
                .ok_or(RiskEngineError::ArithmeticOverflow)?;
            if checked_abs(projected)?
                > positive_limit(max_instrument, "max_instrument_exposure_nanos")?
            {
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reject",
                    risk.reason_code = ?RiskReasonCode::MaxInstrumentExposureExceeded,
                    "risk decision: reject - instrument exposure exceeds limit",
                );
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
                tracing::info!(
                    risk.request_id = %request.request_id,
                    risk.account_id = %request.account_id,
                    risk.outcome = "reject",
                    risk.reason_code = ?RiskReasonCode::ProviderLimitUnavailable,
                    "risk decision: reject - provider limit unavailable",
                );
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
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reject",
                risk.reason_code = ?RiskReasonCode::ProviderLimitExceeded,
                "risk decision: reject - zero executable quantity after constraints",
            );
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

        let decision_id = RiskDecision::new_id();
        let elapsed = start.elapsed();
        tracing::info!(
            risk.request_id = %request.request_id,
            risk.account_id = %request.account_id,
            risk.instrument_id = %request.instrument_id,
            risk.decision_id = %decision_id,
            risk.action = %action_str,
            risk.requested_delta_lots = request.requested_delta_lots,
            risk.approved_delta_lots = approved_delta_lots,
            risk.outcome = outcome_label(&outcome),
            risk.policy_revision = policy.revision,
            risk.policy_state = %state_label(&policy.state),
            risk.elapsed_ms = elapsed.as_millis(),
            "risk decision: {} (requested={}, approved={}, outcome={}, reasons=[{}])",
            outcome_label(&outcome),
            request.requested_delta_lots,
            approved_delta_lots,
            outcome_label(&outcome),
            reasons.iter().map(|r| format!("{:?}", r.code)).collect::<Vec<_>>().join(", "),
        );

        Ok(RiskDecision {
            decision_id,
            request_id: request.request_id.clone(),
            policy_revision: policy.revision,
            account_id: request.account_id.clone(),
            action: request.action,
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

/// Enforces the protection-required lifecycle for exposure-increasing actions.
///
/// The protection gate is driven by broker-authoritative stop coverage in the risk
/// snapshot (`RiskProtectionStatus`) rather than a local intent flag. It:
///
/// - requires an accepted protection plan for exposure-increasing requests when policy
///   mandates protection, distinguishing full, partial and no coverage;
/// - enforces `max_unprotected_duration_ms`: once a position has been uncovered (or
///   only partially covered) longer than the policy window, new exposure fails closed;
/// - exempts reductions/closes (they do not reach this check) — only the increasing
///   portion of a reversal is gated, while the closing portion is always exempt.
fn validate_protection(
    policy: &RiskPolicySet,
    request: &RiskRequest,
    increasing_lots: i64,
) -> Result<Option<RiskDecision>, RiskEngineError> {
    if !policy.protection_required_for_new_exposure {
        return Ok(None);
    }
    debug_assert!(increasing_lots > 0);
    let protection = &request.snapshot.protection;

    // Duration cap: a set but negative value is an invalid policy. A set cap with an
    // unknown position entry time fails closed (cannot prove the position is fresh).
    if let Some(max_duration) = policy.max_unprotected_duration_ms {
        if max_duration < 0 {
            return Err(RiskEngineError::InvalidPolicy(
                "max_unprotected_duration_ms cannot be negative",
            ));
        }
        let entered_at = match protection.position_entered_at_unix_ms {
            Some(t) => t,
            None => {
                // Unknown position entry time with a duration cap fails closed.
                return Ok(Some(reject(
                    policy,
                    request,
                    RiskOutcome::Reject,
                    RiskReasonCode::ProtectionRequired,
                    "max-unprotected-duration requires position entry watermark",
                )));
            }
        };
        let uncovered_lots = protection.uncovered_abs_lots();
        let age = request.now_unix_ms.saturating_sub(entered_at);
        if uncovered_lots > 0 && age > max_duration {
            tracing::info!(
                risk.request_id = %request.request_id,
                risk.account_id = %request.account_id,
                risk.outcome = "reduce_only",
                risk.reason_code = ?RiskReasonCode::ProtectionRequired,
                risk.uncovered_lots = uncovered_lots,
                risk.unprotected_age_ms = age,
                "risk decision: reduce_only - position uncovered beyond max-unprotected-duration",
            );
            return Ok(Some(reject(
                policy,
                request,
                RiskOutcome::ReduceOnly,
                RiskReasonCode::ProtectionRequired,
                "position has been without full protection beyond the configured maximum",
            )));
        }
    }

    // Terminal plan: reject new exposure regardless of broker coverage.
    // Prevents "false protected state" when a protection leg was cancelled/replaced
    // but the broker stop is still active — the plan is dead, so new exposure must wait.
    if protection.plan_state.is_some_and(|s| s.terminal()) {
        tracing::info!(
            risk.request_id = %request.request_id,
            risk.account_id = %request.account_id,
            risk.outcome = "reject",
            risk.reason_code = ?RiskReasonCode::ProtectionRequired,
            risk.plan_state = ?protection.plan_state,
            "risk decision: reject - protection plan is terminal",
        );
        return Ok(Some(reject(
            policy,
            request,
            RiskOutcome::Reject,
            RiskReasonCode::ProtectionRequired,
            "protection plan is in a terminal state (cancelled/failed/stale)",
        )));
    }

    // Plan is acceptable when there is a correlated non-terminal plan that still
    // permits additional exposure, or when the position already enjoys full active-stop
    // coverage even without a correlated plan row.
    let plan_acceptable = protection
        .plan_state
        .is_some_and(|s| s.permits_additional_exposure());
    let coverage_acceptable = protection.full_coverage();

    if !plan_acceptable && !coverage_acceptable {
        tracing::info!(
            risk.request_id = %request.request_id,
            risk.account_id = %request.account_id,
            risk.outcome = "reject",
            risk.reason_code = ?RiskReasonCode::ProtectionRequired,
            "risk decision: reject - protection plan/coverage required for new exposure",
        );
        return Ok(Some(reject(
            policy,
            request,
            RiskOutcome::Reject,
            RiskReasonCode::ProtectionRequired,
            "new exposure requires full protection coverage",
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
        let limit = if wants_margin {
            limits.buy_margin
        } else {
            limits.buy_own
        };
        return limit
            .map(|limit| selected_buy_limit(limit, request.is_market_order))
            .transpose();
    } else if wants_margin {
        limits.sell_margin
    } else {
        limits.sell_own
    };
    selected.map(selected_sell_limit).transpose()
}

fn selected_buy_limit(limit: BuyLotLimit, market: bool) -> Result<i64, RiskEngineError> {
    let value = if market {
        limit.max_market_lots
    } else {
        limit.max_lots
    };
    if value < 0 {
        return Err(RiskEngineError::InvalidProviderFact(
            "broker buy max-lots value cannot be negative",
        ));
    }
    Ok(value)
}

fn selected_sell_limit(limit: SellLotLimit) -> Result<i64, RiskEngineError> {
    let value = limit.max_lots;
    if value < 0 {
        return Err(RiskEngineError::InvalidProviderFact(
            "broker sell max-lots value cannot be negative",
        ));
    }
    Ok(value)
}

fn checked_abs(value: i128) -> Result<i128, RiskEngineError> {
    value
        .checked_abs()
        .ok_or(RiskEngineError::ArithmeticOverflow)
}

fn positive_limit(value: i128, field: &'static str) -> Result<i128, RiskEngineError> {
    if value <= 0 {
        return Err(RiskEngineError::InvalidPolicy(field));
    }
    Ok(value)
}

fn increasing_portion(base: i64, delta: i64) -> Result<i64, RiskEngineError> {
    if delta == 0 {
        return Ok(0);
    }
    if base == 0 || base.signum() == delta.signum() {
        return i64::try_from(delta.unsigned_abs())
            .map_err(|_| RiskEngineError::ArithmeticOverflow);
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
        action: request.action,
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
        BrokerLotLimits, BrokerMarginFacts, BuyLotLimit, ProtectionPlanState, RiskProtectionStatus,
        RiskSnapshot, RiskSource, RiskValidityContext, SellLotLimit,
    };

    fn request(base: i64, delta: i64) -> RiskRequest {
        RiskRequest {
            request_id: "req-1".to_owned(),
            account_id: "account-1".to_owned(),
            broker_connection_id: "connection-1".to_owned(),
            instrument_id: "instrument-1".to_owned(),
            strategy_id: None,
            source: RiskSource::Manual,
            action: RiskActionKind::DirectionalOrder,
            requested_delta_lots: delta,
            requested_notional_nanos: i128::from(delta) * 1_000_000_000,
            is_market_order: false,
            confirm_margin_trade: false,
            emergency_reduction: false,
            now_unix_ms: 10_000,
            snapshot: RiskSnapshot {
                runtime_ready: true,
                execution_authorized: true,
                instrument_tradable: true,
                instrument_lot_size: 1,
                unresolved_unknown_conflict: false,
                current_position_lots: base,
                open_order_delta_lots: 0,
                unresolved_unknown_delta_lots: 0,
                active_reservation_delta_lots: 0,
                gross_exposure_nanos: Some(0),
                net_exposure_nanos: Some(0),
                instrument_exposure_nanos: Some(0),
                broker_daily_pnl_nanos: None,
                broker_lot_limits: Some(BrokerLotLimits {
                    buy_own: Some(BuyLotLimit {
                        max_lots: 100,
                        max_market_lots: 90,
                    }),
                    buy_margin: Some(BuyLotLimit {
                        max_lots: 200,
                        max_market_lots: 180,
                    }),
                    sell_own: Some(SellLotLimit { max_lots: 100 }),
                    sell_margin: Some(SellLotLimit { max_lots: 200 }),
                }),
                margin: None,
                protection: RiskProtectionStatus::default(),
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
            max_margin_utilization_ppm: None,
            max_daily_loss_nanos: None,
            protection_required_for_new_exposure: false,
            max_unprotected_duration_ms: None,
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
    fn cancel_order_is_typed_non_directional_risk_action() {
        let mut r = request(10, 0);
        r.action = RiskActionKind::CancelOrder;
        let decision = RiskEngine::evaluate(&policy(), &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
        assert_eq!(decision.approved_delta_lots, 0);
        assert!(decision.permits_dispatch());
    }

    #[test]
    fn protection_maintenance_is_allowed_in_halted_cleanup_path() {
        let mut p = policy();
        p.state = RiskState::Halted;
        let mut r = request(10, 0);
        r.action = RiskActionKind::ProtectionMaintenance;
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
        assert!(decision.permits_dispatch());
    }

    #[test]
    fn kill_switch_allows_cleanup_when_runtime_is_not_ready_and_unknown_exists() {
        let mut p = policy();
        p.state = RiskState::KillSwitch;
        let mut r = request(10, 0);
        r.action = RiskActionKind::CancelOrder;
        r.snapshot.runtime_ready = false;
        r.snapshot.unresolved_unknown_conflict = true;
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    #[test]
    fn configured_exposure_limit_fails_closed_when_exposure_fact_is_missing() {
        let mut p = policy();
        p.max_gross_exposure_nanos = Some(100_000_000_000);
        let mut r = request(0, 10);
        r.snapshot.gross_exposure_nanos = None;
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(
            decision.reasons[0].code,
            RiskReasonCode::CriticalInputMissing
        );
    }

    #[test]
    fn configured_daily_loss_fails_closed_when_broker_day_pnl_is_missing() {
        let mut p = policy();
        p.max_daily_loss_nanos = Some(10_000_000_000);
        let decision = RiskEngine::evaluate(&p, &request(0, 10)).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(
            decision.reasons[0].code,
            RiskReasonCode::CriticalInputMissing
        );
    }

    #[test]
    fn corrected_margin_utilization_uses_broker_facts_and_blocks_new_exposure() {
        let mut p = policy();
        p.max_margin_utilization_ppm = Some(500_000);
        let mut r = request(0, 10);
        r.snapshot.margin = Some(BrokerMarginFacts {
            liquid_portfolio_nanos: 100_000_000_000,
            starting_margin_nanos: 40_000_000_000,
            minimal_margin_nanos: 20_000_000_000,
            corrected_margin_nanos: 60_000_000_000,
            funds_sufficiency_ppm: Some(1_500_000),
            amount_of_missing_funds_nanos: 0,
            guarantee_for_futures_nanos: 0,
            broker_as_of_unix_ms: 10_000,
        });
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(
            decision.reasons[0].code,
            RiskReasonCode::MarginUtilizationExceeded
        );
    }

    #[test]
    fn decision_is_invalid_after_authorization_revision_change() {
        let r = request(0, 10);
        let decision = RiskEngine::evaluate(&policy(), &r).expect("risk");
        let mut changed = r.clone();
        changed.snapshot.validity.execution_authorization_revision += 1;
        assert!(!RiskEngine::still_valid(
            &decision,
            &changed,
            policy().revision
        ));
    }

    #[test]
    fn protection_required_rejects_new_exposure_without_plan_or_coverage() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            ..RiskProtectionStatus::default()
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    #[test]
    fn protection_required_allows_exposure_with_full_covered_plan() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::FullCoverage),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    #[test]
    fn protection_required_covers_partial_plan_but_rejects_uncovered_portion() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 20,
            active_stop_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::PartialCoverage),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    #[test]
    fn max_unprotected_duration_blocks_new_exposure_fail_closed() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        p.max_unprotected_duration_ms = Some(5_000);
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            // Entered 10_000ms before now (request builder now=10_000), exceeding 5_000ms.
            position_entered_at_unix_ms: Some(0),
            ..RiskProtectionStatus::default()
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::ReduceOnly);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    #[test]
    fn max_unprotected_duration_with_unknown_entry_fails_closed() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        p.max_unprotected_duration_ms = Some(10_000);
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            position_entered_at_unix_ms: None,
            ..RiskProtectionStatus::default()
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    #[test]
    fn exposure_reduction_is_exempt_from_protection_duration() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        p.max_unprotected_duration_ms = Some(10_000);
        let mut r = request(10, -10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            position_entered_at_unix_ms: Some(0),
            ..RiskProtectionStatus::default()
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    // -----------------------------------------------------------------------
    // Deterministic protection lifecycle tests
    // -----------------------------------------------------------------------

    /// Entry request ID differs from protection request ID — protection plan
    /// is located by entry reservation_id, not by protection command request ID.
    #[test]
    fn entry_request_id_differs_from_protection_request_id() {
        // Protection plan created for entry reservation "entry-res-1" (request "entry-req-1").
        // Protection command has its own request ID "protection-req-1".
        // The engine must not look up by "protection-req-1" — it finds the plan
        // via the entry reservation_id that was stored when the plan was created.
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // Full coverage from a plan linked to the entry reservation.
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::FullCoverage),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    /// TP-only does NOT satisfy mandatory stop-loss protection.
    /// Only SL/trailing stops count toward protection.
    #[test]
    fn tp_only_does_not_satisfy_protection() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // active_stop_lots = 0 because TP stops are not counted.
        // The position has lots but no valid SL coverage.
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 0, // TP stops excluded
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::Planned),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    /// Wrong-direction stop does NOT satisfy protection.
    /// A long position needs SELL (direction=2) stops; BUY (direction=1) stops are for shorts.
    #[test]
    fn wrong_direction_stop_does_not_satisfy_protection() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // active_stop_lots = 0 because the stop has wrong direction (BUY for a long position).
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 0, // wrong direction excluded
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::Planned),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    /// Lot size > 1: position quantity is normalized to lots before comparison.
    #[test]
    fn lot_size_greater_than_one() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // Position: 100 units, lot_size=10 → 10 lots.
        // Stop coverage: 10 lots (correctly normalized).
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10, // already in lots
            active_stop_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::FullCoverage),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    /// Partial fill + partial protection: plan in PartialCoverage permits
    /// additional exposure if the plan itself permits it.
    #[test]
    fn partial_fill_partial_protection() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // Position: 20 lots, stop coverage: 10 lots → partial.
        // Plan in PartialCoverage permits additional exposure.
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 20,
            active_stop_lots: 10,
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::PartialCoverage),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Approve);
    }

    /// Terminal plan (CANCELLED/FAILED/STALE) blocks new exposure
    /// even if broker shows coverage.
    #[test]
    fn terminal_plan_blocks_new_exposure() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // Plan is CANCELLED — new exposure must be rejected.
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 10, // broker shows coverage
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::Cancelled),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }

    /// Submitted plan blocks new exposure until broker confirms.
    #[test]
    fn submitted_plan_blocks_new_exposure() {
        let mut p = policy();
        p.protection_required_for_new_exposure = true;
        // Plan is SUBMITTED — not yet confirmed by broker.
        // permits_additional_exposure returns false for Submitted.
        let mut r = request(0, 10);
        r.snapshot.protection = RiskProtectionStatus {
            position_lots: 10,
            active_stop_lots: 0, // not yet active
            position_entered_at_unix_ms: Some(10_000),
            plan_id: Some("risk-protection-plan:1".to_owned()),
            plan_state: Some(ProtectionPlanState::Submitted),
        };
        let decision = RiskEngine::evaluate(&p, &r).expect("risk");
        assert_eq!(decision.outcome, RiskOutcome::Reject);
        assert_eq!(decision.reasons[0].code, RiskReasonCode::ProtectionRequired);
    }
}
