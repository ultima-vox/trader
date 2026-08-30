use std::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

/// T-Invest bearer credential. Formatting never reveals its contents.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(Zeroizing<String>);

impl SecretToken {
    pub fn new(value: impl Into<String>) -> Result<Self, SecretTokenError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SecretTokenError::Empty);
        }
        if value.chars().any(char::is_whitespace) || value.chars().any(char::is_control) {
            return Err(SecretTokenError::InvalidBearerValue);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
    }
}

/// Bearer credential bound to one provider environment before transport construction.
#[derive(Clone, PartialEq, Eq)]
pub enum GrpcCredential {
    Production(SecretToken),
    Sandbox(SecretToken),
}

impl GrpcCredential {
    #[must_use]
    pub const fn environment(&self) -> vox_domain::Environment {
        match self {
            Self::Production(_) => vox_domain::Environment::Live,
            Self::Sandbox(_) => vox_domain::Environment::Sandbox,
        }
    }

    pub(crate) fn token(&self) -> &SecretToken {
        match self {
            Self::Production(token) | Self::Sandbox(token) => token,
        }
    }
}

impl fmt::Debug for GrpcCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GrpcCredential")
            .field("environment", &self.environment())
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretToken([REDACTED])")
    }
}

impl fmt::Display for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl TryFrom<String> for SecretToken {
    type Error = SecretTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for SecretToken {
    type Error = SecretTokenError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SecretTokenError {
    #[error("T-Invest token must not be empty")]
    Empty,
    #[error("T-Invest token is not a valid Bearer credential")]
    InvalidBearerValue,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_redacted_from_debug_and_display() {
        let result = SecretToken::new("super-secret-token");
        assert!(result.is_ok());
        let token = match result {
            Ok(token) => token,
            Err(error) => panic!("unexpected token error: {error}"),
        };

        assert_eq!(format!("{token:?}"), "SecretToken([REDACTED])");
        assert_eq!(format!("{token}"), "[REDACTED]");
        assert!(!format!("{token:?}").contains("super-secret-token"));
    }

    #[test]
    fn token_rejects_empty_and_header_injection() {
        assert_eq!(SecretToken::new("   "), Err(SecretTokenError::Empty));
        assert_eq!(
            SecretToken::new("secret\r\nX-Evil: yes"),
            Err(SecretTokenError::InvalidBearerValue)
        );
    }

    #[test]
    fn credential_debug_exposes_environment_but_never_secret() {
        let credential = GrpcCredential::Sandbox(
            SecretToken::new("sandbox-secret").unwrap_or_else(|error| panic!("{error}")),
        );
        let debug = format!("{credential:?}");
        assert!(debug.contains("Sandbox"));
        assert!(!debug.contains("sandbox-secret"));
    }
}
