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
#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema,
)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttachedBackends {
    pub runtime: bool,
    pub accounts: bool,
    pub execution: bool,
    pub market_data: bool,
    pub connections: bool,
    pub risk: bool,
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
        attached: AttachedBackends,
    ) -> Self {
        let mut supported = Vec::new();
        let mut unavailable = Vec::new();
        if attached.runtime {
            supported.push(Capability::RuntimeHealth);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::RuntimeHealth,
                reason: "no runtime health port is attached to this process".to_owned(),
                owner: "#11".to_owned(),
            });
        }
        if attached.accounts {
            supported.push(Capability::AccountReadSide);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::AccountReadSide,
                reason: "no broker read port is attached to this process".to_owned(),
                owner: "#17".to_owned(),
            });
        }
        if attached.execution {
            supported.push(Capability::OrderExecution);
            supported.push(Capability::ProtectionExecution);
        } else {
            for capability in [Capability::OrderExecution, Capability::ProtectionExecution] {
                unavailable.push(UnavailableCapability {
                    capability,
                    reason: "no execution port is attached to this process".to_owned(),
                    owner: "#10".to_owned(),
                });
            }
        }
        if attached.market_data {
            supported.push(Capability::MarketData);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::MarketData,
                reason: "no market-data projection is attached to this process".to_owned(),
                owner: "#38 projection over the accepted #8 layer".to_owned(),
            });
        }
        if attached.connections {
            supported.push(Capability::ConnectionManagement);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::ConnectionManagement,
                reason: "no connection administration port is attached to this process".to_owned(),
                owner: "#17".to_owned(),
            });
        }
        if attached.risk {
            supported.push(Capability::RiskVerdict);
        } else {
            unavailable.push(UnavailableCapability {
                capability: Capability::RiskVerdict,
                reason: "no risk engine contract exists; new_exposure_allowed is the only safety fact".to_owned(),
                owner: "#21".to_owned(),
            });
        }
        for (capability, reason, owner) in [
            (
                Capability::ProtectionDefaults,
                "account-scoped default protection policy has no contract",
                "#10",
            ),
            (
                Capability::BulkProtectionMigration,
                "no bulk mutation contract exists; single mutations only",
                "#10",
            ),
            (
                Capability::Rbac,
                "role and permission read model has no contract",
                "#17",
            ),
            (
                Capability::PortfolioValuation,
                "valuation, P&L, exposure and margin have no contract",
                "#22",
            ),
            (
                Capability::Strategy,
                "strategy runtime has no contract",
                "#23",
            ),
            (
                Capability::Decision,
                "decision aggregation has no contract",
                "#27",
            ),
            (
                Capability::MachineLearning,
                "model registry and training have no contract",
                "#26",
            ),
            (
                Capability::Research,
                "backtest and research runs have no contract",
                "#29",
            ),
            (
                Capability::AggregateAccounts,
                "no aggregate read model exists; aggregate execution is not defined",
                "#22",
            ),
            (
                Capability::MultiProvider,
                "only the T-Invest adapter is registered",
                "#17",
            ),
            (
                Capability::NonLiveTradingMode,
                "PAPER and BACKTEST trading modes have no runtime",
                "#23/#29",
            ),
        ] {
            unavailable.push(UnavailableCapability {
                capability,
                reason: reason.to_owned(),
                owner: owner.to_owned(),
            });
        }
        Self {
            provider,
            environment,
            account_id,
            supported,
            unavailable,
        }
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
            AttachedBackends::default(),
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
            assert!(
                !set.supported.contains(&capability),
                "{capability:?} must not be supported"
            );
            assert!(
                set.unavailable
                    .iter()
                    .any(|u| u.capability == capability && !u.owner.is_empty()),
                "{capability:?} must be listed as unavailable with an owner"
            );
        }
    }

    #[test]
    fn an_attached_runtime_unlocks_only_the_read_and_execution_side() {
        let set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            BrokerEnvironment::Production,
            Some("account:primary".to_owned()),
            AttachedBackends {
                runtime: true,
                accounts: true,
                execution: true,
                ..AttachedBackends::default()
            },
        );
        assert!(set.supported.contains(&Capability::RuntimeHealth));
        assert!(set.supported.contains(&Capability::AccountReadSide));
        assert!(set.supported.contains(&Capability::OrderExecution));
        assert!(!set.supported.contains(&Capability::RiskVerdict));
    }

    #[test]
    fn attached_connection_port_advertises_connection_management() {
        let set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            None,
            AttachedBackends {
                connections: true,
                ..AttachedBackends::default()
            },
        );
        assert!(set.supported.contains(&Capability::ConnectionManagement));
        assert!(
            !set.unavailable
                .iter()
                .any(|item| item.capability == Capability::ConnectionManagement)
        );
    }

    #[test]
    fn attached_risk_port_advertises_risk_verdict() {
        let set = CapabilitySet::without_backend_owners(
            ProviderDto::TInvest,
            BrokerEnvironment::Sandbox,
            None,
            AttachedBackends {
                risk: true,
                ..AttachedBackends::default()
            },
        );
        assert!(set.supported.contains(&Capability::RiskVerdict));
        assert!(
            !set.unavailable
                .iter()
                .any(|item| item.capability == Capability::RiskVerdict)
        );
    }
}
