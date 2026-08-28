#![forbid(unsafe_code)]

pub mod environment;
pub mod execution;
pub mod identity;
pub mod instrument;
pub mod money;
pub mod mutation;
pub mod readiness;

pub use environment::{Environment, LiveMutationError, MutationAuthorization, MutationGuard};
pub use execution::{
    CancelOrderCommand, CancelStopOrderCommand, ExecutionMutationState, ExecutionPriceConvention,
    OrderSide, PositionSide, ProtectionCapability, ProtectionCapabilityError,
    ProtectionEstablishmentState, ProtectionLeg, ProtectionLegCommand, ProtectionLifecycle,
    ProtectionPlan, ProviderOrderIdentityKind, RegularOrderCommand, RegularOrderType,
    ReplaceOrderCommand, RuntimeExecutionCommand, StopLossProtection, TakeProfitProtection,
    TimeInForce, TrailingDistance, TrailingDistanceMode, TrailingSemanticReference,
};
pub use identity::{
    BrokerFillId, BrokerOrderId, BrokerStopOrderId, ClientOrderId, ClientRequestId,
    ExchangeOrderId, IdentityError,
};
pub use instrument::{InstrumentIdentity, InstrumentIdentityError};
pub use money::{
    FixedPoint, FixedPointError, FuturesEconomics, FuturesEconomicsError, NANO_SCALE, UnitsNano,
};
pub use mutation::{
    AuthoritativeMutationOutcome, MutationDecision, MutationEvidence, MutationEvidenceStore,
    MutationOutcome, MutationRecovery, StoreError,
};
pub use readiness::{Readiness, ReadinessError, ReadinessState};
