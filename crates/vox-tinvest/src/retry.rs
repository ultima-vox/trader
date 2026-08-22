use std::fmt;
use std::time::Duration;

use thiserror::Error;
use uuid::Uuid;

use crate::RestOperation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
    jitter_basis_points: u16,
}

impl RetryPolicy {
    pub fn new(
        max_attempts: u32,
        initial_delay: Duration,
        max_delay: Duration,
        jitter_basis_points: u16,
    ) -> Result<Self, RetryPolicyError> {
        if max_attempts == 0 {
            return Err(RetryPolicyError::ZeroAttempts);
        }
        if initial_delay.is_zero() {
            return Err(RetryPolicyError::ZeroInitialDelay);
        }
        if max_delay < initial_delay {
            return Err(RetryPolicyError::MaxDelayBelowInitial);
        }
        if jitter_basis_points > 10_000 {
            return Err(RetryPolicyError::InvalidJitter);
        }
        Ok(Self {
            max_attempts,
            initial_delay,
            max_delay,
            jitter_basis_points,
        })
    }

    pub const fn max_attempts(self) -> u32 {
        self.max_attempts
    }

    pub const fn initial_delay(self) -> Duration {
        self.initial_delay
    }

    pub const fn max_delay(self) -> Duration {
        self.max_delay
    }

    pub const fn jitter_basis_points(self) -> u16 {
        self.jitter_basis_points
    }

    /// Calculates bounded exponential backoff. Jitter is deterministic per request,
    /// keeping tests reproducible while avoiding synchronized clients in production.
    pub fn delay_for(self, failed_attempt: u32, request_id: Uuid) -> Duration {
        let mut delay = self.initial_delay;
        for _ in 1..failed_attempt {
            if delay >= self.max_delay {
                break;
            }
            delay = delay.saturating_mul(2).min(self.max_delay);
        }

        if self.jitter_basis_points == 0 || delay >= self.max_delay {
            return delay;
        }

        let max_jitter_nanos = delay
            .as_nanos()
            .saturating_mul(u128::from(self.jitter_basis_points))
            / 10_000;
        if max_jitter_nanos == 0 {
            return delay;
        }

        let mixed = request_id.as_u128() ^ u128::from(failed_attempt).wrapping_mul(0x9e37_79b9);
        let jitter_nanos = mixed % (max_jitter_nanos + 1);
        let jitter = duration_from_nanos_saturating(jitter_nanos);
        delay.saturating_add(jitter).min(self.max_delay)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(5),
            jitter_basis_points: 2_000,
        }
    }
}

fn duration_from_nanos_saturating(nanos: u128) -> Duration {
    let seconds = nanos / 1_000_000_000;
    let subsecond_nanos = (nanos % 1_000_000_000) as u32;
    let seconds = u64::try_from(seconds).unwrap_or(u64::MAX);
    Duration::new(seconds, subsecond_nanos)
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RetryPolicyError {
    #[error("retry policy must allow at least one attempt")]
    ZeroAttempts,
    #[error("retry initial delay must be positive")]
    ZeroInitialDelay,
    #[error("retry maximum delay must be at least the initial delay")]
    MaxDelayBelowInitial,
    #[error("retry jitter must be between 0 and 10,000 basis points")]
    InvalidJitter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryReason {
    HttpStatus(u16),
    Connect,
    Timeout,
    ResponseBody,
    Transport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryEvent {
    pub operation: RestOperation,
    pub request_id: Uuid,
    pub failed_attempt: u32,
    pub next_attempt: u32,
    pub delay: Duration,
    pub server_retry_after: Option<Duration>,
    pub server_retry_after_raw: Option<String>,
    pub reason: RetryReason,
}

pub trait RetryObserver: Send + Sync + fmt::Debug {
    fn on_retry(&self, event: &RetryEvent);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopRetryObserver;

impl RetryObserver for NoopRetryObserver {
    fn on_retry(&self, _event: &RetryEvent) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_jitter_policy() -> RetryPolicy {
        match RetryPolicy::new(5, Duration::from_millis(100), Duration::from_millis(450), 0) {
            Ok(policy) => policy,
            Err(error) => panic!("unexpected policy error: {error}"),
        }
    }

    #[test]
    fn exponential_delay_is_bounded() {
        let policy = no_jitter_policy();
        let id = Uuid::nil();
        assert_eq!(policy.delay_for(1, id), Duration::from_millis(100));
        assert_eq!(policy.delay_for(2, id), Duration::from_millis(200));
        assert_eq!(policy.delay_for(3, id), Duration::from_millis(400));
        assert_eq!(policy.delay_for(4, id), Duration::from_millis(450));
        assert_eq!(policy.delay_for(u32::MAX, id), Duration::from_millis(450));
    }

    #[test]
    fn policy_validation_fails_closed() {
        assert_eq!(
            RetryPolicy::new(0, Duration::from_millis(1), Duration::from_secs(1), 0),
            Err(RetryPolicyError::ZeroAttempts)
        );
        assert_eq!(
            RetryPolicy::new(1, Duration::ZERO, Duration::from_secs(1), 0),
            Err(RetryPolicyError::ZeroInitialDelay)
        );
        assert_eq!(
            RetryPolicy::new(1, Duration::from_secs(2), Duration::from_secs(1), 0),
            Err(RetryPolicyError::MaxDelayBelowInitial)
        );
    }
}
