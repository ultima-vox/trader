#![forbid(unsafe_code)]

//! Explicit, fail-closed mappings from broker-neutral Vox projections into Nautilus.
//!
//! Provider payloads must be normalized before entering this crate. All broker economics use
//! [`vox_domain::FixedPoint`]; no binary floating-point conversion is available here.

mod exact;
mod execution_mapping;
mod mapping;
mod market_mapping;
mod market_spec;
mod spec;

pub use exact::{ExactDecimal, future_money_per_point, to_nautilus_price};
pub use execution_mapping::{NautilusRegularOrderCommand, to_nautilus_regular_order};
pub use mapping::{MappedEquity, MappedFuture, to_nautilus_equity, to_nautilus_future};
pub use market_mapping::{to_nautilus_bar, to_nautilus_order_book_snapshot, to_nautilus_trade};
pub use market_spec::{
    BarSpec, BookLevelSpec, OrderBookSnapshotSpec, TimeBarInterval, TradeAggressor, TradeTickSpec,
};
pub use spec::{EquitySpec, FutureAssetClass, FutureSpec, InstrumentSpec};

use thiserror::Error;

/// Failure to preserve a Vox value or invariant in Nautilus exactly.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MappingError {
    #[error("{field} must be positive, got {total_nanos} nanos")]
    NonPositive {
        field: &'static str,
        total_nanos: i128,
    },
    #[error(
        "future tick metadata mismatch: instrument={instrument_tick_nanos} nanos, economics={economics_tick_nanos} nanos"
    )]
    TickMismatch {
        instrument_tick_nanos: i128,
        economics_tick_nanos: i128,
    },
    #[error("exact arithmetic overflow while mapping {field}")]
    ArithmeticOverflow { field: &'static str },
    #[error(
        "invalid futures lifecycle: activation_ns={activation_ns}, expiration_ns={expiration_ns}"
    )]
    InvalidLifecycle {
        activation_ns: u64,
        expiration_ns: u64,
    },
    #[error("invalid Nautilus {field}: {reason}")]
    InvalidNautilusValue { field: &'static str, reason: String },
    #[error("incomplete provider candle cannot become an authoritative Nautilus bar")]
    IncompleteBar,
    #[error("inconsistent provider order book cannot become a Nautilus snapshot")]
    InconsistentOrderBook,
    #[error("execution semantic has no faithful Nautilus representation: {semantic}")]
    UnsupportedExecutionSemantic { semantic: &'static str },
}
