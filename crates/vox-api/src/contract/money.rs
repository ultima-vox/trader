//! Exact money on the wire.
//!
//! A capital-affecting value never crosses this boundary as a JSON number. `FixedPoint`
//! carries an `i128` count of nanos; the public representation is the decimal string that
//! value denotes, so a JavaScript client can render and compare it without ever handing it
//! to `Number`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use vox_domain::{FixedPoint, NANO_SCALE};

/// Exact decimal value, serialized as a string (`"272.55"`, `"-3140.70"`).
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String, example = "272.550000000")]
pub struct Decimal(String);

impl Decimal {
    /// Builds the decimal from an exact fixed-point value without any float step.
    #[must_use]
    pub fn from_fixed_point(value: FixedPoint) -> Self {
        Self(render_nanos(value.total_nanos()))
    }

    /// Builds the decimal from a provider units/nano pair.
    #[must_use]
    pub fn from_units_nano(units: i64, nano: i32) -> Self {
        Self(render_nanos(i128::from(units) * NANO_SCALE + i128::from(nano)))
    }

    /// Accepts a value the backend already produced as an exact decimal string.
    #[must_use]
    pub fn from_exact_string(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Renders total nanos as a fixed nine-decimal string. Nine places always, so two values
/// of the same instrument are directly comparable as text.
fn render_nanos(total: i128) -> String {
    let negative = total < 0;
    let magnitude = total.unsigned_abs();
    let scale = NANO_SCALE.unsigned_abs();
    let units = magnitude / scale;
    let fraction = magnitude % scale;
    let sign = if negative { "-" } else { "" };
    format!("{sign}{units}.{fraction:09}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_exactly_and_keeps_the_sign() {
        assert_eq!(Decimal::from_units_nano(272, 550_000_000).as_str(), "272.550000000");
        assert_eq!(Decimal::from_units_nano(-3140, -700_000_000).as_str(), "-3140.700000000");
        assert_eq!(Decimal::from_units_nano(0, 0).as_str(), "0.000000000");
    }

    #[test]
    fn keeps_precision_a_double_would_lose() {
        // 12450.247381830 is not representable exactly as f64; the string is.
        let value = Decimal::from_units_nano(12_450, 247_381_830);
        assert_eq!(value.as_str(), "12450.247381830");
    }

    #[test]
    fn round_trips_through_json_as_a_string() -> Result<(), serde_json::Error> {
        let value = Decimal::from_fixed_point(FixedPoint::from_total_nanos(1_284_200_000_000));
        let json = serde_json::to_string(&value)?;
        assert_eq!(json, "\"1284.200000000\"");
        assert!(!json.contains(':'), "money must not serialize as an object");
        let back: Decimal = serde_json::from_str(&json)?;
        assert_eq!(back, value);
        Ok(())
    }
}
