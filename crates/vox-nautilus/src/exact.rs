use core::fmt;

use nautilus_model::types::{Price, Quantity, fixed::FIXED_PRECISION};
use vox_domain::FixedPoint;

use crate::{FutureSpec, MappingError};

const _: () = assert!(FIXED_PRECISION == 16);
const NANO_PRECISION: u8 = 9;

/// Positive finite decimal stored as an integer coefficient and decimal scale.
///
/// Values are normalized, so `833.55` is represented by coefficient `83355`, scale `2`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExactDecimal {
    coefficient: u128,
    scale: u8,
}

impl ExactDecimal {
    #[must_use]
    pub const fn coefficient(self) -> u128 {
        self.coefficient
    }

    #[must_use]
    pub const fn scale(self) -> u8 {
        self.scale
    }

    /// Converts into Nautilus high-precision quantity without string or float conversion.
    pub fn to_nautilus_quantity(self) -> Result<Quantity, MappingError> {
        let exponent = u32::from(FIXED_PRECISION - self.scale);
        let scalar = 10_u128
            .checked_pow(exponent)
            .ok_or(MappingError::ArithmeticOverflow {
                field: "Nautilus quantity scale",
            })?;
        let raw = self
            .coefficient
            .checked_mul(scalar)
            .ok_or(MappingError::ArithmeticOverflow {
                field: "Nautilus quantity",
            })?;

        Quantity::from_raw_checked(raw, self.scale).map_err(|error| {
            MappingError::InvalidNautilusValue {
                field: "quantity",
                reason: error.to_string(),
            }
        })
    }

    fn normalized(mut coefficient: u128, mut scale: u8) -> Self {
        while scale > 0 && coefficient.is_multiple_of(10) {
            coefficient /= 10;
            scale -= 1;
        }
        Self { coefficient, scale }
    }
}

impl fmt::Display for ExactDecimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            return write!(formatter, "{}", self.coefficient);
        }

        let digits = self.coefficient.to_string();
        let scale = usize::from(self.scale);
        if digits.len() > scale {
            let split = digits.len() - scale;
            write!(formatter, "{}.{}", &digits[..split], &digits[split..])
        } else {
            write!(formatter, "0.{:0>width$}", digits, width = scale)
        }
    }
}

/// Derives settlement-currency money per quoted point from authoritative future economics.
///
/// [`vox_domain::FuturesEconomics`] has already enforced the exact ratio
/// `price_increment_amount / economics_price_increment`. This boundary additionally rejects a
/// tick which disagrees with the instrument projection.
pub fn future_money_per_point(spec: &FutureSpec) -> Result<ExactDecimal, MappingError> {
    let instrument_tick = require_positive(
        spec.instrument.price_increment,
        "instrument price increment",
    )?;
    let economics_tick = require_positive(
        spec.economics.min_price_increment(),
        "economics price increment",
    )?;
    if instrument_tick != economics_tick {
        return Err(MappingError::TickMismatch {
            instrument_tick_nanos: instrument_tick,
            economics_tick_nanos: economics_tick,
        });
    }

    exact_decimal_from_positive(spec.economics.money_per_point(), "money per point")
}

/// Converts Vox nano fixed-point into Nautilus high-precision `Price` exactly.
pub fn to_nautilus_price(value: FixedPoint) -> Result<Price, MappingError> {
    to_nautilus_price_named(value, "price")
}

pub(crate) fn to_nautilus_positive_price(
    value: FixedPoint,
    field: &'static str,
) -> Result<Price, MappingError> {
    require_positive(value, field)?;
    to_nautilus_price_named(value, field)
}

pub(crate) fn quantity_from_whole(
    value: u64,
    field: &'static str,
) -> Result<Quantity, MappingError> {
    if value == 0 {
        return Err(MappingError::NonPositive {
            field,
            total_nanos: 0,
        });
    }
    ExactDecimal {
        coefficient: u128::from(value),
        scale: 0,
    }
    .to_nautilus_quantity()
    .map_err(|error| remap_invalid_field(error, field))
}

pub(crate) fn quantity_from_nonnegative_whole(
    value: u64,
    field: &'static str,
) -> Result<Quantity, MappingError> {
    ExactDecimal {
        coefficient: u128::from(value),
        scale: 0,
    }
    .to_nautilus_quantity()
    .map_err(|error| remap_invalid_field(error, field))
}

fn to_nautilus_price_named(value: FixedPoint, field: &'static str) -> Result<Price, MappingError> {
    let total_nanos = value.total_nanos();
    let precision = fixed_point_precision(total_nanos);
    let exponent = u32::from(FIXED_PRECISION - NANO_PRECISION);
    let scalar = 10_i128
        .checked_pow(exponent)
        .ok_or(MappingError::ArithmeticOverflow { field })?;
    let raw = total_nanos
        .checked_mul(scalar)
        .ok_or(MappingError::ArithmeticOverflow { field })?;

    Price::from_raw_checked(raw, precision).map_err(|error| MappingError::InvalidNautilusValue {
        field,
        reason: error.to_string(),
    })
}

fn exact_decimal_from_positive(
    value: FixedPoint,
    field: &'static str,
) -> Result<ExactDecimal, MappingError> {
    let total_nanos = require_positive(value, field)?;
    let coefficient =
        u128::try_from(total_nanos).map_err(|_| MappingError::ArithmeticOverflow { field })?;
    Ok(ExactDecimal::normalized(coefficient, NANO_PRECISION))
}

fn fixed_point_precision(total_nanos: i128) -> u8 {
    let mut magnitude = total_nanos.unsigned_abs();
    let mut precision = NANO_PRECISION;
    while precision > 0 && magnitude.is_multiple_of(10) {
        magnitude /= 10;
        precision -= 1;
    }
    precision
}

fn require_positive(value: FixedPoint, field: &'static str) -> Result<i128, MappingError> {
    let total_nanos = value.total_nanos();
    if total_nanos <= 0 {
        Err(MappingError::NonPositive { field, total_nanos })
    } else {
        Ok(total_nanos)
    }
}

fn remap_invalid_field(error: MappingError, field: &'static str) -> MappingError {
    match error {
        MappingError::InvalidNautilusValue { reason, .. } => {
            MappingError::InvalidNautilusValue { field, reason }
        }
        MappingError::ArithmeticOverflow { .. } => MappingError::ArithmeticOverflow { field },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_decimal_display_places_leading_zeroes() {
        let value = ExactDecimal {
            coefficient: 125,
            scale: 5,
        };
        assert_eq!(value.to_string(), "0.00125");
    }
}
