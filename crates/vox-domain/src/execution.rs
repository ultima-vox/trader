use crate::{BrokerOrderId, BrokerStopOrderId, FixedPoint};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionMutationState {
    NotDispatched,
    Dispatching,
    Acknowledged,
    Rejected,
    UnknownAfterDispatch,
    ReconciledNonTerminal,
    ReconciledTerminal,
}

impl ExecutionMutationState {
    #[must_use]
    pub const fn permits_dispatch(self) -> bool {
        matches!(self, Self::NotDispatched)
    }

    #[must_use]
    pub const fn requires_authoritative_reconciliation(self) -> bool {
        matches!(self, Self::UnknownAfterDispatch)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PositionSide {
    Long,
    Short,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegularOrderType {
    Limit,
    Market,
    BestPrice,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimeInForce {
    Day,
    FillAndKill,
    FillOrKill,
}

/// Broker-neutral convention for interpreting an exact execution price.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionPriceConvention {
    SettlementCurrency,
    Points,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegularOrderCommand {
    pub account_id: String,
    pub instrument_id: String,
    pub client_request_id: String,
    pub quantity_lots: i64,
    pub price: Option<FixedPoint>,
    pub price_convention: ExecutionPriceConvention,
    pub side: OrderSide,
    pub order_type: RegularOrderType,
    pub time_in_force: Option<TimeInForce>,
    pub confirm_margin_trade: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderOrderIdentityKind {
    BrokerOrder,
    ClientRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplaceOrderCommand {
    pub account_id: String,
    pub existing_order_id: String,
    pub existing_order_id_kind: Option<ProviderOrderIdentityKind>,
    pub replacement_request_id: String,
    pub quantity_lots: i64,
    pub price: FixedPoint,
    pub price_convention: ExecutionPriceConvention,
    pub confirm_margin_trade: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelOrderCommand {
    pub account_id: String,
    pub order_id: String,
    pub order_id_kind: Option<ProviderOrderIdentityKind>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelStopOrderCommand {
    pub account_id: String,
    pub broker_stop_order_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TrailingDistanceMode {
    AbsolutePrice,
    RelativePercent,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrailingDistance {
    pub value: FixedPoint,
    pub mode: TrailingDistanceMode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StopLossProtection {
    Fixed {
        trigger_price: FixedPoint,
        limit_price: Option<FixedPoint>,
    },
    Trailing {
        distance: TrailingDistance,
        activation_price: Option<FixedPoint>,
        protective_spread: Option<TrailingDistance>,
        instant_execution: Option<bool>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TakeProfitProtection {
    pub trigger_price: Option<FixedPoint>,
    pub limit_price: Option<FixedPoint>,
    pub trailing: Option<TrailingDistance>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtectionPlan {
    pub stop_loss: Option<StopLossProtection>,
    pub take_profit: Option<TakeProfitProtection>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProtectionCapability {
    pub fixed_stop: bool,
    pub native_trailing_relative: bool,
    pub native_trailing_absolute: bool,
    pub take_profit: bool,
    pub stop_limit: bool,
}

impl ProtectionCapability {
    pub fn validate(self, plan: &ProtectionPlan) -> Result<(), ProtectionCapabilityError> {
        if let Some(stop) = &plan.stop_loss {
            match stop {
                StopLossProtection::Fixed { limit_price, .. } => {
                    if !self.fixed_stop {
                        return Err(ProtectionCapabilityError::FixedStopUnsupported);
                    }
                    if limit_price.is_some() && !self.stop_limit {
                        return Err(ProtectionCapabilityError::StopLimitUnsupported);
                    }
                }
                StopLossProtection::Trailing { distance, .. } => {
                    self.validate_trailing(distance.mode)?;
                }
            }
        }
        if let Some(take_profit) = &plan.take_profit {
            if !self.take_profit {
                return Err(ProtectionCapabilityError::TakeProfitUnsupported);
            }
            if take_profit.limit_price.is_some() && !self.stop_limit {
                return Err(ProtectionCapabilityError::StopLimitUnsupported);
            }
            if let Some(trailing) = take_profit.trailing {
                self.validate_trailing(trailing.mode)?;
            }
        }
        Ok(())
    }

    const fn validate_trailing(
        self,
        mode: TrailingDistanceMode,
    ) -> Result<(), ProtectionCapabilityError> {
        match mode {
            TrailingDistanceMode::RelativePercent if !self.native_trailing_relative => {
                Err(ProtectionCapabilityError::NativeRelativeTrailingUnsupported)
            }
            TrailingDistanceMode::AbsolutePrice if !self.native_trailing_absolute => {
                Err(ProtectionCapabilityError::NativeAbsoluteTrailingUnsupported)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectionCapabilityError {
    FixedStopUnsupported,
    StopLimitUnsupported,
    TakeProfitUnsupported,
    NativeRelativeTrailingUnsupported,
    NativeAbsoluteTrailingUnsupported,
}

impl core::fmt::Display for ProtectionCapabilityError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "requested protection is capability-gated: {self:?}"
        )
    }
}

impl std::error::Error for ProtectionCapabilityError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProtectionEstablishmentState {
    AwaitingEntry,
    EntryPartiallyFilled {
        filled_lots: i64,
        protected_lots: i64,
    },
    Establishing,
    Active,
    FailedAfterEntry {
        reason: String,
    },
    UnknownAfterDispatch,
    ReconciliationRequired,
    ClosingPosition,
    Orphaned,
    Terminal,
}

impl ProtectionEstablishmentState {
    #[must_use]
    pub const fn permits_additional_exposure(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectionLifecycle {
    pub broker_stop_order_id: BrokerStopOrderId,
    pub broker_child_order_id: Option<BrokerOrderId>,
    pub provider_status: i32,
    pub provider_trailing_status: Option<i32>,
    pub broker_reported_extreme: Option<FixedPoint>,
    pub broker_reported_execution_price: Option<FixedPoint>,
}

/// Offline semantic reference only. Broker-native trailing remains execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrailingSemanticReference {
    side: PositionSide,
    distance: TrailingDistance,
    favorable_extreme: FixedPoint,
}

impl TrailingSemanticReference {
    #[must_use]
    pub const fn new(
        side: PositionSide,
        reference: FixedPoint,
        distance: TrailingDistance,
    ) -> Self {
        Self {
            side,
            distance,
            favorable_extreme: reference,
        }
    }

    pub fn observe(&mut self, price: FixedPoint) {
        let favorable = match self.side {
            PositionSide::Long => price > self.favorable_extreme,
            PositionSide::Short => price < self.favorable_extreme,
        };
        if favorable {
            self.favorable_extreme = price;
        }
    }

    #[must_use]
    pub fn threshold(self) -> Option<FixedPoint> {
        let extreme = self.favorable_extreme.total_nanos();
        let distance = self.distance.value.total_nanos();
        let delta = match self.distance.mode {
            TrailingDistanceMode::AbsolutePrice => distance,
            TrailingDistanceMode::RelativePercent => extreme
                .checked_mul(distance)?
                .checked_div(100_000_000_000)?,
        };
        let threshold = match self.side {
            PositionSide::Long => extreme.checked_sub(delta)?,
            PositionSide::Short => extreme.checked_add(delta)?,
        };
        Some(FixedPoint::from_total_nanos(threshold))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(units: i64, nano: i32) -> FixedPoint {
        FixedPoint::from_units_nano(units, nano).expect("valid fixed point")
    }

    #[test]
    fn protection_legs_are_independently_optional() {
        let none = ProtectionPlan::default();
        assert_eq!(none.stop_loss, None);
        assert_eq!(none.take_profit, None);
        let stop_only = ProtectionPlan {
            stop_loss: Some(StopLossProtection::Fixed {
                trigger_price: fp(95, 0),
                limit_price: None,
            }),
            take_profit: None,
        };
        assert!(stop_only.stop_loss.is_some());
        assert!(stop_only.take_profit.is_none());
    }

    #[test]
    fn long_relative_trailing_never_widens_after_fall() {
        let five_percent = TrailingDistance {
            value: fp(5, 0),
            mode: TrailingDistanceMode::RelativePercent,
        };
        let mut tracker =
            TrailingSemanticReference::new(PositionSide::Long, fp(100, 0), five_percent);
        tracker.observe(fp(110, 0));
        assert_eq!(tracker.threshold(), Some(fp(104, 500_000_000)));
        tracker.observe(fp(120, 0));
        assert_eq!(tracker.threshold(), Some(fp(114, 0)));
        tracker.observe(fp(116, 0));
        assert_eq!(tracker.threshold(), Some(fp(114, 0)));
    }

    #[test]
    fn short_absolute_trailing_tracks_only_favorable_low() {
        let mut tracker = TrailingSemanticReference::new(
            PositionSide::Short,
            fp(100, 0),
            TrailingDistance {
                value: fp(5, 0),
                mode: TrailingDistanceMode::AbsolutePrice,
            },
        );
        tracker.observe(fp(90, 0));
        assert_eq!(tracker.threshold(), Some(fp(95, 0)));
        tracker.observe(fp(94, 0));
        assert_eq!(tracker.threshold(), Some(fp(95, 0)));
    }

    #[test]
    fn unknown_after_dispatch_never_permits_resubmit() {
        assert!(!ExecutionMutationState::UnknownAfterDispatch.permits_dispatch());
        assert!(
            ExecutionMutationState::UnknownAfterDispatch.requires_authoritative_reconciliation()
        );
    }

    #[test]
    fn unsupported_native_trailing_is_explicitly_gated() {
        let capability = ProtectionCapability {
            fixed_stop: true,
            native_trailing_relative: false,
            native_trailing_absolute: false,
            take_profit: true,
            stop_limit: true,
        };
        let plan = ProtectionPlan {
            stop_loss: Some(StopLossProtection::Trailing {
                distance: TrailingDistance {
                    value: fp(5, 0),
                    mode: TrailingDistanceMode::RelativePercent,
                },
                activation_price: None,
                protective_spread: None,
                instant_execution: None,
            }),
            take_profit: None,
        };
        assert_eq!(
            capability.validate(&plan),
            Err(ProtectionCapabilityError::NativeRelativeTrailingUnsupported)
        );
    }

    #[test]
    fn partial_or_unknown_protection_blocks_more_exposure() {
        assert!(
            !ProtectionEstablishmentState::EntryPartiallyFilled {
                filled_lots: 2,
                protected_lots: 1,
            }
            .permits_additional_exposure()
        );
        assert!(!ProtectionEstablishmentState::UnknownAfterDispatch.permits_additional_exposure());
        assert!(ProtectionEstablishmentState::Active.permits_additional_exposure());
    }
}
