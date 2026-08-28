//! Exact money on the wire.
//!
//! A capital-affecting value never crosses this boundary as a JSON number. `FixedPoint`
//! carries an `i128` count of nanos; the public representation is the decimal string that
//! value denotes, so a JavaScript client can render and compare it without ever handing it
//! to `Number`. Construction is fallible: arbitrary strings never enter the application port.

use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use vox_domain::{FixedPoint, FixedPointError, NANO_SCALE};

/// Canonical public decimal grammar: optional minus, a canonical integer, a dot, exactly
/// nine fraction digits. Input may omit trailing fraction zeros (up to nine places) and is
/// stored in that canonical form.
const FRACTION_DIGITS: usize = 9;

/// Exact decimal value, serialized as a string (`"272.550000000"`, `"-3140.700000000"`).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, ToSchema)]
#[serde(transparent)]
#[schema(
    value_type = String,
    pattern = r"^-?(0|[1-9][0-9]*)\.[0-9]{9}$",
    example = "272.550000000"
)]
pub struct Decimal(String);

/// Why a public decimal cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecimalError {
    Empty,
    InvalidGrammar,
    TooPrecise,
    OutOfRange,
    NanoOutOfRange(i32),
}

impl fmt::Display for DecimalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("decimal cannot be empty or whitespace"),
            Self::InvalidGrammar => formatter.write_str(
                "decimal must be a canonical fixed-point string, not exponent, NaN, Infinity, or free-form text",
            ),
            Self::TooPrecise => formatter.write_str(
                "decimal has more precision than the canonical nano scale of nine fraction digits",
            ),
            Self::OutOfRange => formatter.write_str("decimal is outside the exact backend range"),
            Self::NanoOutOfRange(value) => write!(formatter, "nano out of range: {value}"),
        }
    }
}

impl std::error::Error for DecimalError {}

impl From<FixedPointError> for DecimalError {
    fn from(value: FixedPointError) -> Self {
        match value {
            FixedPointError::NanoOutOfRange(nano) => Self::NanoOutOfRange(nano),
        }
    }
}

impl Decimal {
    /// Builds the decimal from an exact fixed-point value without any float step.
    #[must_use]
    pub fn from_fixed_point(value: FixedPoint) -> Self {
        Self(render_nanos(value.total_nanos()))
    }

    /// Builds the decimal from a provider units/nano pair.
    pub fn from_units_nano(units: i64, nano: i32) -> Result<Self, DecimalError> {
        Ok(Self::from_fixed_point(FixedPoint::from_units_nano(
            units, nano,
        )?))
    }

    /// Accepts a value already produced as an exact decimal string, after validation.
    pub fn from_exact_string(value: impl AsRef<str>) -> Result<Self, DecimalError> {
        Self::parse(value.as_ref())
    }

    /// Builds decimal units from canonical total-nanos storage used by runtime facts.
    pub fn from_total_nanos_string(value: impl AsRef<str>) -> Result<Self, DecimalError> {
        let value = value.as_ref();
        if value.is_empty() || value.chars().any(char::is_whitespace) || value.starts_with('+') {
            return Err(DecimalError::InvalidGrammar);
        }
        let nanos: i128 = value.parse().map_err(|_| DecimalError::OutOfRange)?;
        if nanos.to_string() != value {
            return Err(DecimalError::InvalidGrammar);
        }
        Ok(Self(render_nanos(nanos)))
    }

    pub fn parse(value: &str) -> Result<Self, DecimalError> {
        Ok(Self(render_nanos(parse_nanos(value)?)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
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

fn parse_nanos(input: &str) -> Result<i128, DecimalError> {
    if input.is_empty() || input.chars().any(char::is_whitespace) {
        return Err(DecimalError::Empty);
    }
    let lower = input.to_ascii_lowercase();
    if lower.contains('e')
        || lower.contains("nan")
        || lower.contains("inf")
        || input.contains('+')
        || !input
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.' || byte == b'-')
    {
        return Err(DecimalError::InvalidGrammar);
    }

    let (negative, rest) = match input.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, input),
    };
    if rest.is_empty() || rest.starts_with('.') || rest.ends_with('.') || rest.contains('-') {
        return Err(DecimalError::InvalidGrammar);
    }

    let (whole, frac) = match rest.split_once('.') {
        Some((whole, frac)) => (whole, frac),
        None => (rest, ""),
    };
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !frac.bytes().all(|byte| byte.is_ascii_digit())
        || (whole.len() > 1 && whole.starts_with('0'))
    {
        return Err(DecimalError::InvalidGrammar);
    }
    if frac.len() > FRACTION_DIGITS {
        return Err(DecimalError::TooPrecise);
    }

    let units: i128 = whole.parse().map_err(|_| DecimalError::OutOfRange)?;
    if units > i128::from(i64::MAX) {
        return Err(DecimalError::OutOfRange);
    }

    let mut padded = [b'0'; FRACTION_DIGITS];
    padded[..frac.len()].copy_from_slice(frac.as_bytes());
    let nano: i32 = std::str::from_utf8(&padded)
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or(DecimalError::InvalidGrammar)?;
    let magnitude = units
        .checked_mul(NANO_SCALE)
        .and_then(|value| value.checked_add(i128::from(nano)))
        .ok_or(DecimalError::OutOfRange)?;
    Ok(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_exactly_and_keeps_the_sign() -> Result<(), DecimalError> {
        assert_eq!(
            Decimal::from_units_nano(272, 550_000_000)?.as_str(),
            "272.550000000"
        );
        assert_eq!(
            Decimal::from_units_nano(-3140, -700_000_000)?.as_str(),
            "-3140.700000000"
        );
        assert_eq!(Decimal::from_units_nano(0, 0)?.as_str(), "0.000000000");
        Ok(())
    }

    #[test]
    fn keeps_precision_a_double_would_lose() -> Result<(), DecimalError> {
        // 12450.247381830 is not representable exactly as f64; the string is.
        let value = Decimal::from_units_nano(12_450, 247_381_830)?;
        assert_eq!(value.as_str(), "12450.247381830");
        Ok(())
    }

    #[test]
    fn round_trips_through_json_as_a_string() -> Result<(), Box<dyn std::error::Error>> {
        let value = Decimal::from_fixed_point(FixedPoint::from_total_nanos(1_284_200_000_000));
        let json = serde_json::to_string(&value)?;
        assert_eq!(json, "\"1284.200000000\"");
        assert!(!json.contains(':'), "money must not serialize as an object");
        let back: Decimal = serde_json::from_str(&json)?;
        assert_eq!(back, value);
        Ok(())
    }

    #[test]
    fn total_nanos_storage_converts_without_float_or_scale_loss() -> Result<(), DecimalError> {
        assert_eq!(
            Decimal::from_total_nanos_string("100000000001")?.as_str(),
            "100.000000001"
        );
        assert!(Decimal::from_total_nanos_string("01").is_err());
        assert!(Decimal::from_total_nanos_string("1.0").is_err());
        Ok(())
    }

    #[test]
    fn parse_accepts_shorter_fractions_and_canonicalizes() -> Result<(), DecimalError> {
        assert_eq!(Decimal::parse("272.55")?.as_str(), "272.550000000");
        assert_eq!(Decimal::parse("0")?.as_str(), "0.000000000");
        assert_eq!(Decimal::parse("-0.0")?.as_str(), "0.000000000");
        Ok(())
    }

    #[test]
    fn parse_rejects_free_form_and_non_canonical_input() -> Result<(), serde_json::Error> {
        for invalid in [
            "",
            " ",
            " 1",
            "abc",
            "1e20",
            "1E-3",
            "NaN",
            "Infinity",
            "+1.0",
            "01.0",
            "1.",
            ".5",
            "1.0000000000",
            "1.1234567891",
        ] {
            assert!(
                Decimal::from_exact_string(invalid).is_err(),
                "{invalid} must not cross the public money boundary"
            );
            let json = serde_json::to_string(invalid)?;
            assert!(
                serde_json::from_str::<Decimal>(&json).is_err(),
                "{invalid} must fail at deserialization"
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_nano_fails_instead_of_approximating() {
        assert_eq!(
            Decimal::from_units_nano(0, 1_000_000_000),
            Err(DecimalError::NanoOutOfRange(1_000_000_000))
        );
    }
}
