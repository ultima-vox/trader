use std::fmt;

use thiserror::Error;

/// T-Invest bearer credential. Formatting never reveals its contents.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretToken(Box<str>);

impl SecretToken {
    pub fn new(value: impl Into<String>) -> Result<Self, SecretTokenError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SecretTokenError::Empty);
        }
        if value.chars().any(char::is_whitespace) || value.chars().any(char::is_control) {
            return Err(SecretTokenError::InvalidBearerValue);
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub(crate) fn expose_secret(&self) -> &str {
        &self.0
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
}
