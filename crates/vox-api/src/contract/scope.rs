//! The execution target that travels with every account-scoped read and command.
//!
//! Broker environment and trading mode are two different axes and this boundary keeps them
//! apart. `SANDBOX`/`PRODUCTION` is what a broker connection actually has; `PAPER` and
//! `BACKTEST` are application trading modes owned by #23/#29 and are absent here until
//! those contracts exist. Nothing in this file may grow a value to satisfy a design badge.
//!
//! Public identity is canonical Vox identity, not provider wire identity:
//! `broker_connection_id` and `account_id`. Provider broker-account identifiers remain
//! read-side metadata. The scope key includes the connection so two connections that expose
//! the same broker account cannot collide.

use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

/// Providers with a registered adapter. A provider appears here only once it is real.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderDto {
    TInvest,
}

/// The broker-side environment of a connection. Exactly the runtime contract's two values.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BrokerEnvironment {
    Sandbox,
    Production,
}

/// How Vox executes, which is not where the broker lives.
///
/// Only `LIVE` exists: orders go to the broker connection named by the scope. `PAPER` and
/// `BACKTEST` are owned by #23 and #29 and will be added here when those runtimes exist,
/// not before.
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradingMode {
    Live,
}

/// Why a public execution scope cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeError {
    EmptyAccount,
    EmptyConnection,
    SecretLikeConnection,
}

impl core::fmt::Display for ScopeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyAccount => formatter.write_str("account_id cannot be empty"),
            Self::EmptyConnection => formatter.write_str("broker_connection_id cannot be empty"),
            Self::SecretLikeConnection => {
                formatter.write_str("broker_connection_id resembles secret material")
            }
        }
    }
}

impl std::error::Error for ScopeError {}

/// The immutable target of a read or a capital-affecting command.
///
/// `broker_connection_id` is the application connection identity, never a credential.
/// `account_id` is the canonical Vox account/binding identity. Provider broker-account
/// identifiers are read-side metadata, not this key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
pub struct ExecutionScope {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    /// Application connection identity. Opaque; never a token.
    #[schema(example = "connection:primary")]
    pub broker_connection_id: String,
    /// Canonical Vox account/binding identity.
    #[schema(example = "account:primary")]
    pub account_id: String,
    /// How Vox executes for this scope.
    pub trading_mode: TradingMode,
}

impl ExecutionScope {
    pub fn new(
        provider: ProviderDto,
        environment: BrokerEnvironment,
        broker_connection_id: impl Into<String>,
        account_id: impl Into<String>,
        trading_mode: TradingMode,
    ) -> Result<Self, ScopeError> {
        let broker_connection_id = require_connection_id(broker_connection_id.into())?;
        let account_id = require_account_id(account_id.into())?;
        Ok(Self {
            provider,
            environment,
            broker_connection_id,
            account_id,
            trading_mode,
        })
    }

    /// Stable key of the scope. Connection identity is part of the key so two connections
    /// that expose the same account cannot collide for idempotency or reconciliation.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{:?}:{:?}:{}:{}",
            self.provider, self.environment, self.broker_connection_id, self.account_id
        )
    }
}

impl<'de> Deserialize<'de> for ExecutionScope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            provider: ProviderDto,
            environment: BrokerEnvironment,
            broker_connection_id: String,
            account_id: String,
            trading_mode: TradingMode,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.provider,
            raw.environment,
            raw.broker_connection_id,
            raw.account_id,
            raw.trading_mode,
        )
        .map_err(serde::de::Error::custom)
    }
}

fn require_account_id(value: String) -> Result<String, ScopeError> {
    if value.trim().is_empty() {
        Err(ScopeError::EmptyAccount)
    } else {
        Ok(value)
    }
}

fn require_connection_id(value: String) -> Result<String, ScopeError> {
    if value.trim().is_empty() {
        return Err(ScopeError::EmptyConnection);
    }
    if value.len() > 256
        || value.chars().any(char::is_whitespace)
        || ["Bearer", "token=", "secret=", "t."].iter().any(|needle| {
            value
                .to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        })
    {
        return Err(ScopeError::SecretLikeConnection);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_spells_exactly_what_the_contract_spells() -> Result<(), serde_json::Error> {
        assert_eq!(
            serde_json::to_string(&BrokerEnvironment::Production)?,
            "\"PRODUCTION\""
        );
        assert_eq!(
            serde_json::to_string(&BrokerEnvironment::Sandbox)?,
            "\"SANDBOX\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderDto::TInvest)?,
            "\"T_INVEST\""
        );
        Ok(())
    }

    #[test]
    fn trading_mode_is_a_separate_axis_with_only_live_implemented() -> Result<(), serde_json::Error>
    {
        assert_eq!(serde_json::to_string(&TradingMode::Live)?, "\"LIVE\"");
        // PAPER and BACKTEST must not be constructible until #23/#29 land.
        assert!(serde_json::from_str::<TradingMode>("\"PAPER\"").is_err());
        assert!(serde_json::from_str::<TradingMode>("\"BACKTEST\"").is_err());
        Ok(())
    }

    #[test]
    fn broker_environment_rejects_a_trading_mode_value() {
        assert!(serde_json::from_str::<BrokerEnvironment>("\"PAPER\"").is_err());
        assert!(serde_json::from_str::<BrokerEnvironment>("\"LIVE\"").is_err());
    }

    #[test]
    fn public_scope_uses_canonical_application_identities() -> Result<(), ScopeError> {
        let scope = ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:primary",
            "account:primary",
            TradingMode::Live,
        )?;
        assert_eq!(scope.broker_connection_id, "connection:primary");
        assert_eq!(scope.account_id, "account:primary");
        Ok(())
    }

    #[test]
    fn scope_key_includes_connection_identity() -> Result<(), ScopeError> {
        let left = ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:one",
            "account:shared",
            TradingMode::Live,
        )?;
        let right = ExecutionScope::new(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            "connection:two",
            "account:shared",
            TradingMode::Live,
        )?;
        assert_ne!(
            left.key(),
            right.key(),
            "two connections exposing the same account must not share a scope key"
        );
        assert!(left.key().contains("connection:one"));
        assert!(right.key().contains("connection:two"));
        Ok(())
    }

    #[test]
    fn empty_or_secret_like_identities_fail_closed() {
        assert_eq!(
            ExecutionScope::new(
                ProviderDto::TInvest,
                BrokerEnvironment::Sandbox,
                "connection:primary",
                "  ",
                TradingMode::Live,
            ),
            Err(ScopeError::EmptyAccount)
        );
        assert_eq!(
            ExecutionScope::new(
                ProviderDto::TInvest,
                BrokerEnvironment::Sandbox,
                "Bearer abc",
                "account:primary",
                TradingMode::Live,
            ),
            Err(ScopeError::SecretLikeConnection)
        );
        assert!(
            serde_json::from_str::<ExecutionScope>(
                r#"{"provider":"T_INVEST","environment":"SANDBOX","broker_connection_id":"token=abc","account_id":"account:primary","trading_mode":"LIVE"}"#
            )
            .is_err()
        );
    }
}
