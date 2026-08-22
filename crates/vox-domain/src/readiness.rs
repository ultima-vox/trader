use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessState {
    Starting,
    Connecting,
    Reconciling,
    Ready,
    Degraded,
    Halted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Readiness {
    state: ReadinessState,
    reason: Option<String>,
}

impl Default for Readiness {
    fn default() -> Self {
        Self {
            state: ReadinessState::Starting,
            reason: None,
        }
    }
}

impl Readiness {
    #[must_use]
    pub const fn state(&self) -> ReadinessState {
        self.state
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    #[must_use]
    pub const fn can_open_new_exposure(&self) -> bool {
        matches!(self.state, ReadinessState::Ready)
    }

    pub fn transition(
        &mut self,
        target: ReadinessState,
        reason: Option<String>,
    ) -> Result<(), ReadinessError> {
        if self.state != target && !allowed(self.state, target) {
            return Err(ReadinessError {
                from: self.state,
                to: target,
            });
        }
        self.state = target;
        self.reason = reason;
        Ok(())
    }
}

const fn allowed(from: ReadinessState, to: ReadinessState) -> bool {
    use ReadinessState::{Connecting, Degraded, Halted, Ready, Reconciling, Starting};
    matches!(
        (from, to),
        (Starting, Connecting | Halted)
            | (Connecting, Reconciling | Degraded | Halted)
            | (Reconciling, Ready | Degraded | Halted)
            | (Ready, Reconciling | Degraded | Halted)
            | (Degraded, Connecting | Reconciling | Halted)
            | (Halted, Connecting)
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadinessError {
    pub from: ReadinessState,
    pub to: ReadinessState,
}

impl fmt::Display for ReadinessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid readiness transition {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl std::error::Error for ReadinessError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposure_stays_closed_until_reconciliation_reaches_ready() -> Result<(), ReadinessError> {
        let mut readiness = Readiness::default();
        assert!(!readiness.can_open_new_exposure());
        readiness.transition(ReadinessState::Connecting, None)?;
        readiness.transition(ReadinessState::Reconciling, None)?;
        assert!(!readiness.can_open_new_exposure());
        readiness.transition(ReadinessState::Ready, None)?;
        assert!(readiness.can_open_new_exposure());
        readiness.transition(ReadinessState::Degraded, Some("broker stream lost".into()))?;
        assert!(!readiness.can_open_new_exposure());
        Ok(())
    }

    #[test]
    fn ready_cannot_be_skipped() {
        let mut readiness = Readiness::default();
        assert!(readiness.transition(ReadinessState::Ready, None).is_err());
        assert!(!readiness.can_open_new_exposure());
    }

    #[test]
    fn every_non_ready_state_blocks_new_exposure() {
        for state in [
            ReadinessState::Starting,
            ReadinessState::Connecting,
            ReadinessState::Reconciling,
            ReadinessState::Degraded,
            ReadinessState::Halted,
        ] {
            let readiness = Readiness {
                state,
                reason: None,
            };
            assert!(!readiness.can_open_new_exposure(), "state={state:?}");
        }
    }
}
