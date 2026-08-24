use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Sandbox,
    Paper,
    Live,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutationGuard {
    environment: Environment,
    live_mutations_enabled: bool,
}

/// Unforgeable proof that environment mutation policy was checked explicitly.
#[derive(Debug, Eq, PartialEq)]
pub struct MutationAuthorization {
    environment: Environment,
}

impl MutationAuthorization {
    #[must_use]
    pub const fn environment(&self) -> Environment {
        self.environment
    }
}

impl MutationGuard {
    #[must_use]
    pub const fn new(environment: Environment) -> Self {
        Self {
            environment,
            live_mutations_enabled: false,
        }
    }

    /// Creates explicit live authorization. Call site must opt in; default guard cannot mutate live.
    #[must_use]
    pub const fn with_live_mutations_enabled(environment: Environment) -> Self {
        Self {
            environment,
            live_mutations_enabled: true,
        }
    }

    /// Read-only broker access remains available in every environment, including live.
    pub const fn authorize_read(self) -> Result<(), LiveMutationError> {
        Ok(())
    }

    pub const fn authorize_mutation(self) -> Result<MutationAuthorization, LiveMutationError> {
        match (self.environment, self.live_mutations_enabled) {
            (Environment::Live, false) => Err(LiveMutationError::ExplicitEnablementRequired),
            _ => Ok(MutationAuthorization {
                environment: self.environment,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveMutationError {
    ExplicitEnablementRequired,
}

impl fmt::Display for LiveMutationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("live mutations require explicit enablement")
    }
}

impl std::error::Error for LiveMutationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_is_readable_but_mutations_fail_closed() {
        assert!(
            MutationGuard::new(Environment::Live)
                .authorize_read()
                .is_ok()
        );
        assert_eq!(
            MutationGuard::new(Environment::Live).authorize_mutation(),
            Err(LiveMutationError::ExplicitEnablementRequired)
        );
        assert!(
            MutationGuard::with_live_mutations_enabled(Environment::Live)
                .authorize_mutation()
                .is_ok()
        );
    }

    #[test]
    fn environment_defaults_to_sandbox_and_has_stable_wire_names() -> Result<(), serde_json::Error>
    {
        assert_eq!(Environment::default(), Environment::Sandbox);
        assert_eq!(serde_json::to_string(&Environment::Sandbox)?, "\"sandbox\"");
        assert_eq!(serde_json::to_string(&Environment::Paper)?, "\"paper\"");
        assert_eq!(serde_json::to_string(&Environment::Live)?, "\"live\"");
        Ok(())
    }

    #[test]
    fn non_live_environments_allow_mutations() {
        for environment in [Environment::Sandbox, Environment::Paper] {
            assert!(MutationGuard::new(environment).authorize_mutation().is_ok());
        }
    }
}
