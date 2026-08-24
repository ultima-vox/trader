#![forbid(unsafe_code)]

use std::env;
use std::fmt;
use thiserror::Error;
use vox_domain::{
    Environment, LiveMutationError, MutationAuthorization, MutationGuard, Readiness,
    ReadinessError, ReadinessState,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreConfig {
    environment: Environment,
    live_mutations_enabled: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            environment: Environment::Sandbox,
            live_mutations_enabled: false,
        }
    }
}

impl CoreConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let environment = match env::var("VOX_ENV") {
            Ok(value) => parse_environment(&value)?,
            Err(env::VarError::NotPresent) => Environment::Sandbox,
            Err(env::VarError::NotUnicode(_)) => return Err(ConfigError::NonUnicode("VOX_ENV")),
        };
        let live_mutations_enabled = match env::var("VOX_LIVE_MUTATIONS_ENABLED") {
            Ok(value) => parse_bool(&value)?,
            Err(env::VarError::NotPresent) => false,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::NonUnicode("VOX_LIVE_MUTATIONS_ENABLED"));
            }
        };
        Ok(Self {
            environment,
            live_mutations_enabled,
        })
    }

    #[must_use]
    pub const fn environment(self) -> Environment {
        self.environment
    }

    #[must_use]
    pub const fn mutation_guard(self) -> MutationGuard {
        if self.live_mutations_enabled {
            MutationGuard::with_live_mutations_enabled(self.environment)
        } else {
            MutationGuard::new(self.environment)
        }
    }
}

#[derive(Clone, Debug)]
pub struct CoreRuntime {
    config: CoreConfig,
    readiness: Readiness,
    reconciliation: Option<ReconciliationEvidence>,
}

impl CoreRuntime {
    #[must_use]
    pub fn new(config: CoreConfig) -> Self {
        Self {
            config,
            readiness: Readiness::default(),
            reconciliation: None,
        }
    }

    #[must_use]
    pub const fn readiness(&self) -> &Readiness {
        &self.readiness
    }

    pub fn begin_connecting(&mut self) -> Result<(), ReadinessError> {
        self.reconciliation = None;
        self.readiness.transition(ReadinessState::Connecting, None)
    }

    pub fn begin_reconciliation(&mut self) -> Result<(), ReadinessError> {
        self.reconciliation = None;
        self.readiness.transition(ReadinessState::Reconciling, None)
    }

    pub fn complete_reconciliation(
        &mut self,
        evidence: ReconciliationEvidence,
    ) -> Result<(), ReadinessError> {
        self.readiness.transition(ReadinessState::Ready, None)?;
        self.reconciliation = Some(evidence);
        Ok(())
    }

    pub fn mark_degraded(&mut self, reason: impl Into<String>) -> Result<(), ReadinessError> {
        self.reconciliation = None;
        self.readiness
            .transition(ReadinessState::Degraded, Some(reason.into()))
    }

    pub fn halt(&mut self, reason: impl Into<String>) -> Result<(), ReadinessError> {
        self.reconciliation = None;
        self.readiness
            .transition(ReadinessState::Halted, Some(reason.into()))
    }

    /// New exposure requires both reconciled readiness and explicit environment authorization.
    pub fn authorize_new_exposure(&self) -> Result<NewExposureAuthorization, CoreSafetyError> {
        if !self.readiness.can_open_new_exposure() {
            return Err(CoreSafetyError::NotReady(self.readiness.state()));
        }
        let generation = self
            .reconciliation
            .as_ref()
            .ok_or(CoreSafetyError::MissingReconciliation)?
            .connection_generation;
        let mutation = self
            .config
            .mutation_guard()
            .authorize_mutation()
            .map_err(CoreSafetyError::Mutation)?;
        Ok(NewExposureAuthorization {
            mutation,
            connection_generation: generation,
        })
    }

    pub fn consume_new_exposure_authorization(
        &self,
        authorization: NewExposureAuthorization,
    ) -> Result<MutationAuthorization, CoreSafetyError> {
        let current_generation = self
            .reconciliation
            .as_ref()
            .filter(|_| self.readiness.can_open_new_exposure())
            .ok_or(CoreSafetyError::NotReady(self.readiness.state()))?
            .connection_generation;
        if current_generation != authorization.connection_generation {
            return Err(CoreSafetyError::StaleAuthorization);
        }
        Ok(authorization.mutation)
    }
}

#[derive(Debug)]
pub struct NewExposureAuthorization {
    mutation: MutationAuthorization,
    connection_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconciliationEvidence {
    broker_snapshot_id: String,
    connection_generation: u64,
}

impl ReconciliationEvidence {
    pub fn new(
        broker_snapshot_id: impl Into<String>,
        connection_generation: u64,
        checks: ReconciliationChecks,
        unresolved_unknowns: usize,
    ) -> Result<Self, ReconciliationEvidenceError> {
        let broker_snapshot_id = broker_snapshot_id.into();
        if broker_snapshot_id.trim().is_empty() {
            return Err(ReconciliationEvidenceError::MissingBrokerSnapshot);
        }
        if connection_generation == 0 {
            return Err(ReconciliationEvidenceError::MissingConnection);
        }
        if !checks.is_complete() {
            return Err(ReconciliationEvidenceError::IncompleteChecks);
        }
        if unresolved_unknowns != 0 {
            return Err(ReconciliationEvidenceError::UnresolvedUnknowns(
                unresolved_unknowns,
            ));
        }
        Ok(Self {
            broker_snapshot_id,
            connection_generation,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReconciliationChecks {
    reference_data: bool,
    account_snapshot: bool,
    orders: bool,
    positions: bool,
    risk_config: bool,
}

impl ReconciliationChecks {
    #[must_use]
    pub const fn new(
        reference_data: bool,
        account_snapshot: bool,
        orders: bool,
        positions: bool,
        risk_config: bool,
    ) -> Self {
        Self {
            reference_data,
            account_snapshot,
            orders,
            positions,
            risk_config,
        }
    }

    #[must_use]
    pub const fn complete() -> Self {
        Self::new(true, true, true, true, true)
    }

    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.reference_data
            && self.account_snapshot
            && self.orders
            && self.positions
            && self.risk_config
    }
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum ReconciliationEvidenceError {
    #[error("broker snapshot identity is required")]
    MissingBrokerSnapshot,
    #[error("connected broker generation is required")]
    MissingConnection,
    #[error("reference data, account, orders, positions, and risk checks must all complete")]
    IncompleteChecks,
    #[error("{0} UNKNOWN mutations remain unresolved")]
    UnresolvedUnknowns(usize),
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum CoreSafetyError {
    #[error("new exposure blocked while readiness is {0:?}")]
    NotReady(ReadinessState),
    #[error("mutation authorization failed: {0}")]
    Mutation(#[source] LiveMutationError),
    #[error("READY state has no reconciliation evidence")]
    MissingReconciliation,
    #[error("new-exposure authorization belongs to stale connection generation")]
    StaleAuthorization,
}

fn parse_environment(value: &str) -> Result<Environment, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "sandbox" => Ok(Environment::Sandbox),
        "paper" => Ok(Environment::Paper),
        "live" => Ok(Environment::Live),
        _ => Err(ConfigError::InvalidEnvironment),
    }
}

fn parse_bool(value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ConfigError::InvalidBoolean("VOX_LIVE_MUTATIONS_ENABLED")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidEnvironment,
    InvalidBoolean(&'static str),
    NonUnicode(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvironment => {
                formatter.write_str("VOX_ENV must be sandbox, paper, or live")
            }
            Self::InvalidBoolean(name) => write!(formatter, "{name} must be true, false, 1, or 0"),
            Self::NonUnicode(name) => write!(formatter, "{name} must be valid Unicode"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_sandbox_and_mutation_safe() {
        let config = CoreConfig::default();
        assert_eq!(config.environment(), Environment::Sandbox);
        assert!(config.mutation_guard().authorize_mutation().is_ok());
    }

    #[test]
    fn core_blocks_exposure_until_ready() -> Result<(), Box<dyn std::error::Error>> {
        let mut runtime = CoreRuntime::new(CoreConfig::default());
        assert!(matches!(
            runtime.authorize_new_exposure(),
            Err(CoreSafetyError::NotReady(ReadinessState::Starting))
        ));
        runtime.begin_connecting()?;
        runtime.begin_reconciliation()?;
        runtime.complete_reconciliation(ReconciliationEvidence::new(
            "snapshot-1",
            1,
            ReconciliationChecks::complete(),
            0,
        )?)?;
        assert!(runtime.authorize_new_exposure().is_ok());
        let stale = runtime.authorize_new_exposure()?;
        runtime.mark_degraded("stream disconnected")?;
        assert!(matches!(
            runtime.consume_new_exposure_authorization(stale),
            Err(CoreSafetyError::NotReady(ReadinessState::Degraded))
        ));
        Ok(())
    }

    #[test]
    fn parsers_fail_closed() {
        assert_eq!(parse_environment("LIVE"), Ok(Environment::Live));
        assert_eq!(
            parse_environment("production"),
            Err(ConfigError::InvalidEnvironment)
        );
        assert_eq!(
            parse_bool("yes"),
            Err(ConfigError::InvalidBoolean("VOX_LIVE_MUTATIONS_ENABLED"))
        );
    }
}
