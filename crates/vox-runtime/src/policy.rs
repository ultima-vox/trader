use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{ReasonCode, RuntimeState};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SafetyCondition {
    StartupBeforeReconciliation,
    UnresolvedUnknownMutation,
    PositionConflict,
    OrderIdentityConflict,
    StopIdentityConflict,
    RequiredReadUnavailable,
    AccountUnavailable,
    CredentialInvalid,
    ExecutionAuthorizationDisabled,
    ProductionStreamDisconnectedAfterUnaryRecovery,
    OptionalAnalyticsUnavailable,
    CheckpointCorruptSnapshotAvailable,
    PersistenceFailure,
    OwnershipFailure,
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    pub state: RuntimeState,
    pub reason_code: ReasonCode,
    pub new_exposure_allowed: bool,
    pub requires_reconciliation: bool,
    pub operator_intervention_required: bool,
}

#[must_use]
pub const fn readiness_policy(condition: SafetyCondition) -> PolicyDecision {
    use ReasonCode as Reason;
    use RuntimeState as State;
    use SafetyCondition as Condition;
    match condition {
        Condition::StartupBeforeReconciliation => decision(State::Reconciling, Reason::Startup),
        Condition::UnresolvedUnknownMutation => halt(Reason::UnknownMutation),
        Condition::PositionConflict => halt(Reason::BrokerPositionConflict),
        Condition::OrderIdentityConflict => halt(Reason::BrokerOrderConflict),
        Condition::StopIdentityConflict => halt(Reason::BrokerStopConflict),
        Condition::RequiredReadUnavailable => halt(Reason::RequiredReadUnavailable),
        Condition::AccountUnavailable => halt(Reason::AccountUnavailable),
        Condition::CredentialInvalid => halt(Reason::CredentialRejected),
        Condition::ExecutionAuthorizationDisabled => PolicyDecision {
            state: State::Ready,
            reason_code: Reason::ExecutionUnauthorized,
            new_exposure_allowed: false,
            requires_reconciliation: false,
            operator_intervention_required: false,
        },
        Condition::ProductionStreamDisconnectedAfterUnaryRecovery => PolicyDecision {
            state: State::Degraded,
            reason_code: Reason::StreamDisconnected,
            new_exposure_allowed: false,
            requires_reconciliation: false,
            operator_intervention_required: false,
        },
        Condition::OptionalAnalyticsUnavailable => PolicyDecision {
            state: State::Degraded,
            reason_code: Reason::OptionalCapabilityUnavailable,
            new_exposure_allowed: false,
            requires_reconciliation: false,
            operator_intervention_required: false,
        },
        Condition::CheckpointCorruptSnapshotAvailable => PolicyDecision {
            state: State::Reconciling,
            reason_code: Reason::CheckpointRebuild,
            new_exposure_allowed: false,
            requires_reconciliation: true,
            operator_intervention_required: false,
        },
        Condition::PersistenceFailure => halt(Reason::PersistenceFailure),
        Condition::OwnershipFailure => halt(Reason::OwnershipFailure),
        Condition::Clear => PolicyDecision {
            state: State::Ready,
            reason_code: Reason::ReconciliationComplete,
            new_exposure_allowed: true,
            requires_reconciliation: false,
            operator_intervention_required: false,
        },
    }
}

const fn decision(state: RuntimeState, reason_code: ReasonCode) -> PolicyDecision {
    PolicyDecision {
        state,
        reason_code,
        new_exposure_allowed: false,
        requires_reconciliation: true,
        operator_intervention_required: false,
    }
}

const fn halt(reason_code: ReasonCode) -> PolicyDecision {
    PolicyDecision {
        state: RuntimeState::Halted,
        reason_code,
        new_exposure_allowed: false,
        requires_reconciliation: true,
        operator_intervention_required: true,
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeStateMachine {
    state: RuntimeState,
}

impl Default for RuntimeStateMachine {
    fn default() -> Self {
        Self {
            state: RuntimeState::Starting,
        }
    }
}

impl RuntimeStateMachine {
    #[must_use]
    pub const fn state(&self) -> RuntimeState {
        self.state
    }

    pub fn transition(&mut self, target: RuntimeState) -> Result<RuntimeState, TransitionError> {
        if self.state != target && !allowed(self.state, target) {
            return Err(TransitionError {
                from: self.state,
                to: target,
            });
        }
        let previous = self.state;
        self.state = target;
        Ok(previous)
    }
}

const fn allowed(from: RuntimeState, to: RuntimeState) -> bool {
    use RuntimeState::{
        Connecting, Degraded, Halted, Ready, Reconciling, Starting, Stopped, Stopping,
    };
    matches!(
        (from, to),
        (Starting, Connecting | Halted | Stopping)
            | (Connecting, Reconciling | Degraded | Halted | Stopping)
            | (Reconciling, Ready | Degraded | Halted | Stopping)
            | (Ready, Reconciling | Degraded | Halted | Stopping)
            | (Degraded, Connecting | Reconciling | Halted | Stopping)
            | (Halted, Connecting | Reconciling | Stopping)
            | (Stopping, Stopped)
    )
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("invalid runtime transition {from:?} -> {to:?}")]
pub struct TransitionError {
    pub from: RuntimeState,
    pub to: RuntimeState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_runtime_transition_is_explicit_and_ready_cannot_be_skipped() {
        let mut machine = RuntimeStateMachine::default();
        assert_eq!(
            machine.transition(RuntimeState::Ready),
            Err(TransitionError {
                from: RuntimeState::Starting,
                to: RuntimeState::Ready
            })
        );
        for target in [
            RuntimeState::Connecting,
            RuntimeState::Reconciling,
            RuntimeState::Ready,
            RuntimeState::Degraded,
            RuntimeState::Reconciling,
            RuntimeState::Halted,
            RuntimeState::Stopping,
            RuntimeState::Stopped,
        ] {
            machine.transition(target).expect("legal transition");
        }
    }

    #[test]
    fn readiness_matrix_fails_closed() {
        let conditions = [
            SafetyCondition::StartupBeforeReconciliation,
            SafetyCondition::UnresolvedUnknownMutation,
            SafetyCondition::PositionConflict,
            SafetyCondition::OrderIdentityConflict,
            SafetyCondition::StopIdentityConflict,
            SafetyCondition::RequiredReadUnavailable,
            SafetyCondition::AccountUnavailable,
            SafetyCondition::CredentialInvalid,
            SafetyCondition::ExecutionAuthorizationDisabled,
            SafetyCondition::ProductionStreamDisconnectedAfterUnaryRecovery,
            SafetyCondition::OptionalAnalyticsUnavailable,
            SafetyCondition::CheckpointCorruptSnapshotAvailable,
            SafetyCondition::PersistenceFailure,
            SafetyCondition::OwnershipFailure,
        ];
        assert!(conditions.into_iter().all(|condition| {
            let decision = readiness_policy(condition);
            !decision.new_exposure_allowed && decision.state != RuntimeState::Ready
                || condition == SafetyCondition::ExecutionAuthorizationDisabled
        }));
        assert!(readiness_policy(SafetyCondition::Clear).new_exposure_allowed);
    }
}
