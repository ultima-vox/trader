#![forbid(unsafe_code)]

pub mod environment;
pub mod identity;
pub mod instrument;
pub mod money;
pub mod mutation;
pub mod readiness;

pub use environment::{Environment, LiveMutationError, MutationAuthorization, MutationGuard};
pub use identity::{BrokerOrderId, ClientOrderId, ClientRequestId, ExchangeOrderId, IdentityError};
pub use instrument::{InstrumentIdentity, InstrumentIdentityError};
pub use money::{
    FixedPoint, FixedPointError, FuturesEconomics, FuturesEconomicsError, NANO_SCALE, UnitsNano,
};
pub use mutation::{
    AuthoritativeMutationOutcome, MutationDecision, MutationEvidence, MutationEvidenceStore,
    MutationOutcome, MutationRecovery, StoreError,
};
pub use readiness::{Readiness, ReadinessError, ReadinessState};
