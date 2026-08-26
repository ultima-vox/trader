use core::fmt;
use serde::{Deserialize, Deserializer, Serialize};

pub const NANO_SCALE: i128 = 1_000_000_000;

/// Exact provider units/nano pair. Components remain available without a float conversion.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct UnitsNano {
    units: i64,
    nano: i32,
}

impl<'de> Deserialize<'de> for UnitsNano {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawUnitsNano {
            units: i64,
            nano: i32,
        }

        let raw = RawUnitsNano::deserialize(deserializer)?;
        Self::new(raw.units, raw.nano).map_err(serde::de::Error::custom)
    }
}

impl UnitsNano {
    pub fn new(units: i64, nano: i32) -> Result<Self, FixedPointError> {
        validate_nano(nano)?;
        Ok(Self { units, nano })
    }

    #[must_use]
    pub const fn units(self) -> i64 {
        self.units
    }

    #[must_use]
    pub const fn nano(self) -> i32 {
        self.nano
    }

    #[must_use]
    pub const fn fixed_point(self) -> FixedPoint {
        FixedPoint(self.units as i128 * NANO_SCALE + self.nano as i128)
    }
}

/// Exact fixed-point value at provider nano precision. No floating-point conversion exists.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct FixedPoint(i128);

impl FixedPoint {
    #[must_use]
    pub const fn from_total_nanos(total_nanos: i128) -> Self {
        Self(total_nanos)
    }
    pub fn from_units_nano(units: i64, nano: i32) -> Result<Self, FixedPointError> {
        Ok(UnitsNano::new(units, nano)?.fixed_point())
    }

    #[must_use]
    pub const fn total_nanos(self) -> i128 {
        self.0
    }

    /// Returns a canonical units/nano pair with no loss of numeric value.
    #[must_use]
    pub fn units_nano(self) -> (i128, i32) {
        let units = self.0 / NANO_SCALE;
        let nano = (self.0 % NANO_SCALE) as i32;
        (units, nano)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedPointError {
    NanoOutOfRange(i32),
}

impl fmt::Display for FixedPointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NanoOutOfRange(value) => write!(formatter, "nano out of range: {value}"),
        }
    }
}

impl std::error::Error for FixedPointError {}

fn validate_nano(nano: i32) -> Result<(), FixedPointError> {
    if (-999_999_999..=999_999_999).contains(&nano) {
        Ok(())
    } else {
        Err(FixedPointError::NanoOutOfRange(nano))
    }
}

/// Validated futures tick economics from catalogue and authoritative margin metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FuturesEconomics {
    min_price_increment: FixedPoint,
    min_price_increment_amount: FixedPoint,
    money_per_point: FixedPoint,
}

impl FuturesEconomics {
    /// Derives exact money per quoted point and rejects stale or inconsistent metadata.
    pub fn new(
        catalogue_min_price_increment: FixedPoint,
        margin_min_price_increment: FixedPoint,
        min_price_increment_amount: FixedPoint,
    ) -> Result<Self, FuturesEconomicsError> {
        require_positive(
            "catalogue_min_price_increment",
            catalogue_min_price_increment,
        )?;
        require_positive("margin_min_price_increment", margin_min_price_increment)?;
        require_positive("min_price_increment_amount", min_price_increment_amount)?;

        if catalogue_min_price_increment != margin_min_price_increment {
            return Err(FuturesEconomicsError::TickMismatch);
        }

        let numerator = min_price_increment_amount
            .0
            .checked_mul(NANO_SCALE)
            .ok_or(FuturesEconomicsError::ArithmeticOverflow)?;
        if numerator % margin_min_price_increment.0 != 0 {
            return Err(FuturesEconomicsError::InexactMoneyPerPoint);
        }
        let money_per_point = FixedPoint(numerator / margin_min_price_increment.0);

        Ok(Self {
            min_price_increment: margin_min_price_increment,
            min_price_increment_amount,
            money_per_point,
        })
    }

    #[must_use]
    pub const fn min_price_increment(self) -> FixedPoint {
        self.min_price_increment
    }

    #[must_use]
    pub const fn min_price_increment_amount(self) -> FixedPoint {
        self.min_price_increment_amount
    }

    #[must_use]
    pub const fn money_per_point(self) -> FixedPoint {
        self.money_per_point
    }

    /// T-Invest formula: quoted price * money per point * contract count.
    pub fn value_for_price(
        self,
        quoted_price: FixedPoint,
        contracts: u64,
    ) -> Result<FixedPoint, FuturesEconomicsError> {
        require_positive("quoted_price", quoted_price)?;
        if contracts == 0 {
            return Err(FuturesEconomicsError::NonPositive("contracts"));
        }

        let product = quoted_price
            .0
            .checked_mul(self.money_per_point.0)
            .ok_or(FuturesEconomicsError::ArithmeticOverflow)?;
        if product % NANO_SCALE != 0 {
            return Err(FuturesEconomicsError::InexactMonetaryValue);
        }
        let per_contract = product / NANO_SCALE;
        per_contract
            .checked_mul(i128::from(contracts))
            .map(FixedPoint)
            .ok_or(FuturesEconomicsError::ArithmeticOverflow)
    }
}

fn require_positive(field: &'static str, value: FixedPoint) -> Result<(), FuturesEconomicsError> {
    if value.0 > 0 {
        Ok(())
    } else {
        Err(FuturesEconomicsError::NonPositive(field))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuturesEconomicsError {
    NonPositive(&'static str),
    TickMismatch,
    InexactMoneyPerPoint,
    InexactMonetaryValue,
    ArithmeticOverflow,
}

impl fmt::Display for FuturesEconomicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonPositive(field) => write!(formatter, "{field} must be positive"),
            Self::TickMismatch => {
                formatter.write_str("catalogue and margin minimum price increments differ")
            }
            Self::InexactMoneyPerPoint => formatter
                .write_str("money per point is not exactly representable at nano precision"),
            Self::InexactMonetaryValue => formatter
                .write_str("futures monetary value is not exactly representable at nano precision"),
            Self::ArithmeticOverflow => formatter.write_str("exact futures arithmetic overflow"),
        }
    }
}

impl std::error::Error for FuturesEconomicsError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(units: i64, nano: i32) -> Result<FixedPoint, FixedPointError> {
        FixedPoint::from_units_nano(units, nano)
    }

    #[test]
    fn unit_nano_conversion_is_exact_for_positive_and_negative_values()
    -> Result<(), FixedPointError> {
        let positive = UnitsNano::new(12, 345_678_901)?;
        assert_eq!(positive.units(), 12);
        assert_eq!(positive.nano(), 345_678_901);
        assert_eq!(positive.fixed_point().total_nanos(), 12_345_678_901);
        assert_eq!(positive.fixed_point().units_nano(), (12, 345_678_901));

        let negative = UnitsNano::new(-12, -345_678_901)?;
        assert_eq!(negative.units(), -12);
        assert_eq!(negative.nano(), -345_678_901);
        assert_eq!(negative.fixed_point().total_nanos(), -12_345_678_901);
        assert_eq!(negative.fixed_point().units_nano(), (-12, -345_678_901));
        Ok(())
    }

    #[test]
    fn invalid_nano_fails_instead_of_approximating() {
        assert_eq!(
            FixedPoint::from_units_nano(0, 1_000_000_000),
            Err(FixedPointError::NanoOutOfRange(1_000_000_000))
        );
        assert!(serde_json::from_str::<UnitsNano>(r#"{"units":0,"nano":1000000000}"#).is_err());
    }

    #[test]
    fn live_q1_futures_vector_is_exact() -> Result<(), Box<dyn std::error::Error>> {
        let tick = fixed(0, 1_000_000)?;
        let tick_amount = fixed(0, 833_550_000)?;
        let economics = FuturesEconomics::new(tick, tick, tick_amount)?;

        assert_eq!(economics.min_price_increment(), tick);
        assert_eq!(economics.min_price_increment_amount(), tick_amount);
        assert_eq!(economics.money_per_point(), fixed(833, 550_000_000)?);
        assert_eq!(
            economics.value_for_price(fixed(100_000, 0)?, 1)?,
            fixed(83_355_000, 0)?
        );
        Ok(())
    }

    #[test]
    fn futures_economics_rejects_inconsistent_or_inexact_metadata() -> Result<(), FixedPointError> {
        let one = fixed(1, 0)?;
        let two = fixed(2, 0)?;
        assert_eq!(
            FuturesEconomics::new(one, two, one),
            Err(FuturesEconomicsError::TickMismatch)
        );
        assert_eq!(
            FuturesEconomics::new(fixed(0, 3)?, fixed(0, 3)?, fixed(0, 1)?),
            Err(FuturesEconomicsError::InexactMoneyPerPoint)
        );
        assert_eq!(
            FuturesEconomics::new(fixed(0, 0)?, fixed(0, 0)?, one),
            Err(FuturesEconomicsError::NonPositive(
                "catalogue_min_price_increment"
            ))
        );
        Ok(())
    }
}
