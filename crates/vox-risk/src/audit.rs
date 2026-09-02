//! Audit events for risk policy changes and policy-driven risk boundary mutations.
//!
//! Every policy revision, state transition, and limit change emits a structured audit event
//! so that downstream observability pipelines (tracing, metrics, audit logs) can reconstruct
//! the exact policy configuration that governed any given risk decision.

use crate::model::{RiskPolicySet, RiskState};

/// Represents a single risk policy change that is emitted as an audit event.
///
/// Callers should invoke [`PolicyAuditEvent::emit`] immediately after applying a new
/// [`RiskPolicySet`] so that the audit trail captures the exact revision and state
/// that subsequent risk decisions observe.
#[derive(Clone, Debug)]
pub struct PolicyAuditEvent {
    pub event_id: String,
    pub old_revision: Option<u64>,
    pub new_revision: u64,
    pub old_state: Option<RiskState>,
    pub new_state: RiskState,
    pub state_transition: StateTransition,
    pub limits_changed: LimitsDelta,
    pub emitted_at_unix_ms: i64,
}

/// Describes the type of state transition that occurred.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateTransition {
    /// No state change; revision bumped for limit/config updates only.
    ConfigOnly,
    /// Normal -> Warning: early signal of risk pressure.
    NormalToWarning,
    /// Warning -> Halted: critical risk containment.
    WarningToHalted,
    /// Normal -> Halted: immediate halt from normal.
    NormalToHalted,
    /// Any transition to ReduceOnly.
    ToReduceOnly,
    /// ReduceOnly -> Normal: risk pressure resolved.
    ReduceOnlyToNormal,
    /// Halted -> ReduceOnly: partial recovery.
    HaltedToReduceOnly,
    /// Halted -> Normal: full recovery.
    HaltedToNormal,
    /// Warning -> ReduceOnly.
    WarningToReduceOnly,
    /// Any transition to KillSwitch.
    ToKillSwitch,
    /// KillSwitch -> ReduceOnly: partial recovery from kill-switch.
    KillSwitchToReduceOnly,
    /// KillSwitch -> Normal: full recovery from kill-switch.
    KillSwitchToNormal,
    /// KillSwitch -> Halted.
    KillSwitchToHalted,
    /// Unknown or unmapped transition.
    Other,
}

/// Tracks which numeric limits changed during a policy revision.
#[derive(Clone, Debug, Default)]
pub struct LimitsDelta {
    pub max_single_order_lots_changed: bool,
    pub max_position_abs_lots_changed: bool,
    pub max_gross_exposure_nanos_changed: bool,
    pub max_net_exposure_abs_nanos_changed: bool,
    pub max_instrument_exposure_nanos_changed: bool,
    pub max_margin_utilization_ppm_changed: bool,
    pub max_daily_loss_nanos_changed: bool,
    pub max_market_data_age_ms_changed: bool,
    pub allow_margin_changed: bool,
    pub protection_required_changed: bool,
}

impl LimitsDelta {
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.max_single_order_lots_changed
            || self.max_position_abs_lots_changed
            || self.max_gross_exposure_nanos_changed
            || self.max_net_exposure_abs_nanos_changed
            || self.max_instrument_exposure_nanos_changed
            || self.max_margin_utilization_ppm_changed
            || self.max_daily_loss_nanos_changed
            || self.max_market_data_age_ms_changed
            || self.allow_margin_changed
            || self.protection_required_changed
    }
}

impl PolicyAuditEvent {
    /// Creates a new audit event comparing the previous and current policy configurations.
    ///
    /// Pass `old_policy` when a prior policy revision exists so that limit changes can be
    /// accurately detected. The `old_state` parameter is used for the state transition
    /// classification; when `old_policy` is provided its state is used as the default
    /// for `old_state`.
    #[must_use]
    pub fn new(
        old_policy: Option<&RiskPolicySet>,
        new_policy: &RiskPolicySet,
        old_state: Option<RiskState>,
        emitted_at_unix_ms: i64,
    ) -> Self {
        let old_revision = old_policy.map(|p| p.revision);
        let old_state = old_state.or(old_policy.map(|p| p.state));
        let limits_changed = Self::compute_limits_delta(old_policy, new_policy);

        PolicyAuditEvent {
            event_id: format!("policy-audit:{}", uuid::Uuid::new_v4()),
            old_revision,
            new_revision: new_policy.revision,
            old_state,
            new_state: new_policy.state,
            state_transition: Self::compute_transition(old_state, new_policy.state),
            limits_changed,
            emitted_at_unix_ms,
        }
    }

    /// Emits the audit event as a structured tracing record.
    ///
    /// This is the primary observability hook for policy changes. The event is logged at
    /// `info` level so it surfaces in production traces and audit log streams.
    pub fn emit(&self) {
        tracing::info!(
            audit.event = "policy_change",
            audit.event_id = %self.event_id,
            policy.old_revision = ?self.old_revision,
            policy.new_revision = self.new_revision,
            policy.old_state = ?self.old_state,
            policy.new_state = ?self.new_state,
            policy.state_transition = ?self.state_transition,
            limits.changed = self.limits_changed.has_changes(),
            limits.single_order = self.limits_changed.max_single_order_lots_changed,
            limits.position = self.limits_changed.max_position_abs_lots_changed,
            limits.gross = self.limits_changed.max_gross_exposure_nanos_changed,
            limits.net = self.limits_changed.max_net_exposure_abs_nanos_changed,
            limits.instrument = self.limits_changed.max_instrument_exposure_nanos_changed,
            limits.margin_utilization = self.limits_changed.max_margin_utilization_ppm_changed,
            limits.daily_loss = self.limits_changed.max_daily_loss_nanos_changed,
            limits.market_data_age = self.limits_changed.max_market_data_age_ms_changed,
            limits.margin = self.limits_changed.allow_margin_changed,
            limits.protection = self.limits_changed.protection_required_changed,
            emitted_at_unix_ms = self.emitted_at_unix_ms,
            "risk policy revision {} applied (state: {:?} -> {:?}, transition: {:?})",
            self.new_revision,
            self.old_state,
            self.new_state,
            self.state_transition,
        );
    }

    fn compute_transition(old: Option<RiskState>, new: RiskState) -> StateTransition {
        let Some(old_state) = old else {
            return StateTransition::ConfigOnly;
        };
        if old_state == new {
            return StateTransition::ConfigOnly;
        };
        match (old_state, new) {
            (RiskState::Normal, RiskState::Warning) => StateTransition::NormalToWarning,
            (RiskState::Warning, RiskState::Halted) => StateTransition::WarningToHalted,
            (RiskState::Normal, RiskState::Halted) => StateTransition::NormalToHalted,
            (RiskState::Warning, RiskState::ReduceOnly)
            | (RiskState::Normal, RiskState::ReduceOnly) => StateTransition::ToReduceOnly,
            (RiskState::ReduceOnly, RiskState::Normal) => StateTransition::ReduceOnlyToNormal,
            (RiskState::Halted, RiskState::ReduceOnly) => StateTransition::HaltedToReduceOnly,
            (RiskState::Halted, RiskState::Normal) => StateTransition::HaltedToNormal,
            (_, RiskState::KillSwitch) => StateTransition::ToKillSwitch,
            (RiskState::KillSwitch, RiskState::ReduceOnly) => {
                StateTransition::KillSwitchToReduceOnly
            }
            (RiskState::KillSwitch, RiskState::Normal) => StateTransition::KillSwitchToNormal,
            (RiskState::KillSwitch, RiskState::Halted) => StateTransition::KillSwitchToHalted,
            _ => StateTransition::Other,
        }
    }

    fn compute_limits_delta(old: Option<&RiskPolicySet>, new: &RiskPolicySet) -> LimitsDelta {
        let Some(old_policy) = old else {
            return LimitsDelta::default();
        };
        LimitsDelta {
            max_single_order_lots_changed: old_policy.max_single_order_lots
                != new.max_single_order_lots,
            max_position_abs_lots_changed: old_policy.max_position_abs_lots
                != new.max_position_abs_lots,
            max_gross_exposure_nanos_changed: old_policy.max_gross_exposure_nanos
                != new.max_gross_exposure_nanos,
            max_net_exposure_abs_nanos_changed: old_policy.max_net_exposure_abs_nanos
                != new.max_net_exposure_abs_nanos,
            max_instrument_exposure_nanos_changed: old_policy.max_instrument_exposure_nanos
                != new.max_instrument_exposure_nanos,
            max_margin_utilization_ppm_changed: old_policy.max_margin_utilization_ppm
                != new.max_margin_utilization_ppm,
            max_daily_loss_nanos_changed: old_policy.max_daily_loss_nanos
                != new.max_daily_loss_nanos,
            max_market_data_age_ms_changed: old_policy.max_market_data_age_ms
                != new.max_market_data_age_ms,
            allow_margin_changed: old_policy.allow_margin != new.allow_margin,
            protection_required_changed: old_policy.protection_required_for_new_exposure
                != new.protection_required_for_new_exposure,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_policy(revision: u64, state: RiskState) -> RiskPolicySet {
        RiskPolicySet {
            revision,
            state,
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
        }
    }

    #[test]
    fn first_revision_has_no_old_revision() {
        let policy = base_policy(1, RiskState::Normal);
        let event = PolicyAuditEvent::new(None, &policy, None, 100);
        assert_eq!(event.old_revision, None);
        assert_eq!(event.new_revision, 1);
        assert_eq!(event.state_transition, StateTransition::ConfigOnly);
    }

    #[test]
    fn state_change_to_warning_detected() {
        let old = base_policy(1, RiskState::Normal);
        let new = base_policy(2, RiskState::Warning);
        let event = PolicyAuditEvent::new(Some(&old), &new, Some(old.state), 200);
        assert_eq!(event.state_transition, StateTransition::NormalToWarning);
        assert_eq!(event.old_state, Some(RiskState::Normal));
        assert_eq!(event.new_state, RiskState::Warning);
    }

    #[test]
    fn config_only_revision_has_no_state_transition() {
        let old = base_policy(1, RiskState::Normal);
        let mut new = base_policy(2, RiskState::Normal);
        new.max_single_order_lots = Some(40);
        let event = PolicyAuditEvent::new(Some(&old), &new, Some(old.state), 300);
        assert_eq!(event.state_transition, StateTransition::ConfigOnly);
        assert!(event.limits_changed.max_single_order_lots_changed);
    }

    #[test]
    fn halt_from_normal_detected() {
        let old = base_policy(1, RiskState::Normal);
        let new = base_policy(2, RiskState::Halted);
        let event = PolicyAuditEvent::new(Some(&old), &new, Some(old.state), 400);
        assert_eq!(event.state_transition, StateTransition::NormalToHalted);
    }

    #[test]
    fn limits_delta_reports_correct_changes() {
        let old = base_policy(1, RiskState::Normal);
        let mut new = base_policy(2, RiskState::Normal);
        new.max_single_order_lots = Some(30);
        new.max_position_abs_lots = Some(80);
        new.allow_margin = true;
        let event = PolicyAuditEvent::new(Some(&old), &new, Some(old.state), 500);
        assert!(event.limits_changed.max_single_order_lots_changed);
        assert!(event.limits_changed.max_position_abs_lots_changed);
        assert!(event.limits_changed.allow_margin_changed);
        assert!(!event.limits_changed.max_gross_exposure_nanos_changed);
        assert!(event.limits_changed.has_changes());
    }

    #[test]
    fn empty_delta_has_no_changes() {
        let delta = LimitsDelta::default();
        assert!(!delta.has_changes());
    }
}
