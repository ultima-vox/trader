//! The execution target that travels with every account-scoped read and command.
//!
//! Broker environment and trading mode are two different axes and this boundary keeps them
//! apart. `SANDBOX`/`PRODUCTION` is what a broker connection actually has; `PAPER` and
//! `BACKTEST` are application trading modes owned by #23/#29 and are absent here until
//! those contracts exist. Nothing in this file may grow a value to satisfy a design badge.

use serde::{Deserialize, Serialize};
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

/// The immutable target of a read or a capital-affecting command.
///
/// `connection_ref` is an opaque reference, never a credential: the runtime rejects any
/// value that resembles secret material before it can reach this boundary.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
pub struct ExecutionScope {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    /// Broker account identifier. Human labels live in the UI; identity stays explicit here.
    #[schema(example = "2000000001")]
    pub broker_account_id: String,
    /// Opaque connection reference from the runtime. Never a token.
    #[schema(example = "connection:primary")]
    pub connection_ref: String,
    /// How Vox executes for this scope.
    pub trading_mode: TradingMode,
}

impl ExecutionScope {
    /// Stable key of the scope, matching the runtime's own scope key semantics.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{:?}:{:?}:{}",
            self.provider, self.environment, self.broker_account_id
        )
    }
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
}
