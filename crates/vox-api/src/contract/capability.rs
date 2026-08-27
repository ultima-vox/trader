//! Capability discovery.
//!
//! The UI gates features on this, never on a hard-coded guess and never on the existence of
//! a screen. A capability is `supported` only when a backend owner has landed it; everything
//! else is listed as unavailable with the issue that owns it, so a deferred region in the
//! design maps to a fact here.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::scope::{BrokerEnvironment, ProviderDto};

/// A capability the frontend may gate on.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Capability {
    /// Runtime health and readiness reads.
    RuntimeHealth,
    /// Account, portfolio, position, order, stop and operation reads.
    AccountReadSide,
    /// Order submit, replace and cancel.
    OrderExecution,
    /// Stop order and protection commands.
    ProtectionExecution,
    /// Account-scoped default protection policy.
    ProtectionDefaults,
    /// Bulk re-application of protection to existing positions.
    BulkProtectionMigration,
    /// Broker connection and credential lifecycle.
    ConnectionManagement,
    /// Role and permission model.
    Rbac,
    /// Pre-trade risk verdicts and guardrails.
    RiskVerdict,
    /// Valuation, P&L, exposure and margin.
    PortfolioValuation,
    /// Quotes, order book, tape and candles.
    MarketData,
    /// Strategy registry and lifecycle.
    Strategy,
    /// Decision candidates and approval.
    Decision,
    /// Model registry, training and promotion.
    MachineLearning,
    /// Backtest and research runs.
    Research,
    /// Read-only aggregate across accounts.
    AggregateAccounts,
    /// A second provider adapter.
    MultiProvider,
    /// Trading modes other than LIVE.
    NonLiveTradingMode,
}

/// Why a capability is not available, and who owns making it so.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct UnavailableCapability {
    pub capability: Capability,
    /// Human sentence naming what is missing.
    pub reason: String,
    /// The issue that owns the missing contract, for example `#21`.
    #[schema(example = "#21")]
    pub owner: String,
}

/// What this deployment can actually do, for one scope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct CapabilitySet {
    pub provider: ProviderDto,
    pub environment: BrokerEnvironment,
    /// Present when the capability set is account-scoped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub supported: Vec<Capability>,
    pub unavailable: Vec<UnavailableCapability>,
}

impl CapabilitySet {
    /// The capability set of a deployment where only the runtime read side exists.
    ///
    /// Every entry mirrors a tracked dependency in `docs/design/BACKEND_CONTRACTS.md`, so a
    /// deferred region in the design has exactly one reason here.
    #[must_use]
    pub fn without_backend_owners(
        provider: ProviderDto,
        environment: BrokerEnvironment,
        account_id: Option<String>,
        runtime_attached: bool,
        market_data_attached: bool,
    ) -> Self {
        let mut supported = vec![Capability::RuntimeHealth];
        let mut unavailable = Vec::new();
        if runtime_attached {
            supported.push(Capability::AccountReadSide);
            supported.push(Capability::OrderExecution);
            supported.push(Capability::ProtectionExecution);
        } else {
            for (capability, reason) in [
                (Capability::AccountReadSide, "no broker runtime is attached to this process"),
                (Capability::OrderExecution, "no broker runtime is attached to this process"),
                (Capability::ProtectionExecution, "no broker runtime is attached to this process"),
            ] {
                unavailable.push(UnavailableCapability {
                    capability,
                    reason: reason.to_owned(),
                    owner: "#11".to_owned(),
                });
            }
        }
        if market_data_attached {
            supported.push(Capability::MarketData);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::MarketData,
                reason: "no market-data projection is attached to this process".to_owned(),
                owner: "#38 projection over the accepted #8 layer".to_owned(),
            });
        }
        for (capability, reason, owner) in [
            (Capability::ProtectionDefaults, "account-scoped default protection policy has no contract", "#10"),
            (Capability::BulkProtectionMigration, "no bulk mutation contract exists; single mutations only", "#10"),
            (Capability::ConnectionManagement, "connection and credential lifecycle has no contract", "#17"),
            (Capability::Rbac, "role and permission read model has no contract", "#17"),
            (Capability::RiskVerdict, "no risk engine contract exists; new_exposure_allowed is the only safety fact", "#21"),
            (Capability::PortfolioValuation, "valuation, P&L, exposure and margin have no contract", "#22"),
            (Capability::Strategy, "strategy runtime has no contract", "#23"),
            (Capability::Decision, "decision aggregation has no contract", "#27"),
            (Capability::MachineLearning, "model registry and training have no contract", "#26"),
            (Capability::Research, "backtest and research runs have no contract", "#29"),
            (Capability::AggregateAccounts, "no aggregate read model exists; aggregate execution is not defined", "#22"),
            (Capability::MultiProvider, "only the T-Invest adapter is registered", "#17"),
            (Capability::NonLiveTradingMode, "PAPER and BACKTEST trading modes have no runtime", "#23/#29"),
        ] {
            unavailable.push(UnavailableCapability {
                capability,
                reason: reason.to_owned(),
                owner: owner.to_owned(),
            });
        }
        Self { provider, environment, account_id, supported, unavailable }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_unowned_is_reported_as_supported() {
        let set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            None,
            false,
            false,
        );
        for capability in [
            Capability::RiskVerdict,
            Capability::PortfolioValuation,
            Capability::MarketData,
            Capability::Strategy,
            Capability::Decision,
            Capability::MachineLearning,
            Capability::Research,
            Capability::AggregateAccounts,
            Capability::MultiProvider,
            Capability::NonLiveTradingMode,
            Capability::BulkProtectionMigration,
        ] {
            assert!(!set.supported.contains(&capability), "{capability:?} must not be supported");
            assert!(
                set.unavailable.iter().any(|u| u.capability == capability && !u.owner.is_empty()),
                "{capability:?} must be listed as unavailable with an owner"
            );
        }
    }

    #[test]
    fn an_attached_runtime_unlocks_only_the_read_and_execution_side() {
        let set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            BrokerEnvironment::Production,
            Some("2000000001".to_owned()),
            true,
            false,
        );
        assert!(set.supported.contains(&Capability::AccountReadSide));
        assert!(set.supported.contains(&Capability::OrderExecution));
        assert!(!set.supported.contains(&Capability::RiskVerdict));
    }
}
