use core::fmt;
use serde::{Deserialize, Serialize};

/// Provider-normalized instrument identity retained beside runtime projections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstrumentIdentity {
    provider: String,
    uid: String,
    figi: Option<String>,
    ticker: String,
    class_code: String,
}

impl InstrumentIdentity {
    pub fn new(
        provider: impl Into<String>,
        uid: impl Into<String>,
        figi: Option<String>,
        ticker: impl Into<String>,
        class_code: impl Into<String>,
    ) -> Result<Self, InstrumentIdentityError> {
        let identity = Self {
            provider: provider.into(),
            uid: uid.into(),
            figi,
            ticker: ticker.into(),
            class_code: class_code.into(),
        };
        if identity.provider.trim().is_empty()
            || identity.uid.trim().is_empty()
            || identity.ticker.trim().is_empty()
            || identity.class_code.trim().is_empty()
            || identity
                .figi
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err(InstrumentIdentityError::EmptyField);
        }
        Ok(identity)
    }

    #[must_use]
    pub fn provider(&self) -> &str {
        &self.provider
    }

    #[must_use]
    pub fn uid(&self) -> &str {
        &self.uid
    }

    #[must_use]
    pub fn figi(&self) -> Option<&str> {
        self.figi.as_deref()
    }

    #[must_use]
    pub fn ticker(&self) -> &str {
        &self.ticker
    }

    #[must_use]
    pub fn class_code(&self) -> &str {
        &self.class_code
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstrumentIdentityError {
    EmptyField,
}

impl fmt::Display for InstrumentIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("provider, UID, ticker, class code, and present FIGI must be non-empty")
    }
}

impl std::error::Error for InstrumentIdentityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_aliases_remain_distinct_and_required() -> Result<(), InstrumentIdentityError> {
        let identity = InstrumentIdentity::new(
            "tinvest",
            "e6123145-9665-43e0-8413-cd61b8aa9b13",
            Some("BBG004730N88".into()),
            "SBER",
            "TQBR",
        )?;
        assert_eq!(identity.provider(), "tinvest");
        assert_eq!(identity.uid(), "e6123145-9665-43e0-8413-cd61b8aa9b13");
        assert_eq!(identity.figi(), Some("BBG004730N88"));
        assert_eq!(identity.ticker(), "SBER");
        assert_eq!(identity.class_code(), "TQBR");
        assert!(InstrumentIdentity::new("tinvest", "", None, "SBER", "TQBR").is_err());
        Ok(())
    }
}
