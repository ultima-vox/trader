//! Vox-owned reference values mapped from generated T-Invest provider messages.

use thiserror::Error;
use vox_domain::UnitsNano;

use crate::generated::v1;
use crate::reference::{InstrumentUid, RequestValidationError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderInstrumentKind {
    Known(v1::InstrumentType),
    Unknown(i32),
}

impl From<i32> for ProviderInstrumentKind {
    fn from(value: i32) -> Self {
        v1::InstrumentType::try_from(value).map_or(Self::Unknown(value), Self::Known)
    }
}

/// Canonical provider catalogue identity. Empty proto3 strings remain absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalInstrument {
    pub uid: Option<InstrumentUid>,
    pub position_uid: Option<String>,
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub isin: Option<String>,
    pub name: Option<String>,
    pub kind: ProviderInstrumentKind,
    pub lot: i32,
    pub api_trade_available: bool,
}

impl From<v1::InstrumentShort> for CanonicalInstrument {
    fn from(value: v1::InstrumentShort) -> Self {
        Self {
            uid: instrument_uid(value.uid),
            position_uid: optional_text(value.position_uid),
            figi: optional_text(value.figi),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
            isin: optional_text(value.isin),
            name: optional_text(value.name),
            kind: value.instrument_kind.into(),
            lot: value.lot,
            api_trade_available: value.api_trade_available_flag,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalAssetInstrument {
    pub uid: Option<InstrumentUid>,
    pub position_uid: Option<String>,
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub kind: ProviderInstrumentKind,
}

impl From<v1::AssetInstrument> for CanonicalAssetInstrument {
    fn from(value: v1::AssetInstrument) -> Self {
        Self {
            uid: instrument_uid(value.uid),
            position_uid: optional_text(value.position_uid),
            figi: optional_text(value.figi),
            ticker: optional_text(value.ticker),
            class_code: optional_text(value.class_code),
            kind: value.instrument_kind.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalMoney {
    pub currency: Option<String>,
    pub amount: UnitsNano,
}

impl TryFrom<v1::MoneyValue> for CanonicalMoney {
    type Error = EconomicsError;

    fn try_from(value: v1::MoneyValue) -> Result<Self, Self::Error> {
        Ok(Self {
            currency: optional_text(value.currency),
            amount: UnitsNano::new(value.units, value.nano)
                .map_err(|_| EconomicsError::InvalidUnitsNano)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalFuturesMargin {
    pub initial_margin_on_buy: Option<CanonicalMoney>,
    pub initial_margin_on_sell: Option<CanonicalMoney>,
    pub min_price_increment: Option<UnitsNano>,
    pub min_price_increment_amount: Option<UnitsNano>,
}

impl TryFrom<v1::GetFuturesMarginResponse> for CanonicalFuturesMargin {
    type Error = EconomicsError;

    fn try_from(value: v1::GetFuturesMarginResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            initial_margin_on_buy: value
                .initial_margin_on_buy
                .map(TryInto::try_into)
                .transpose()?,
            initial_margin_on_sell: value
                .initial_margin_on_sell
                .map(TryInto::try_into)
                .transpose()?,
            min_price_increment: quotation(value.min_price_increment)?,
            min_price_increment_amount: quotation(value.min_price_increment_amount)?,
        })
    }
}

impl CanonicalFuturesMargin {
    /// Trading consumer must call this before using margin economics.
    pub fn require_complete(&self) -> Result<(), EconomicsError> {
        if self.initial_margin_on_buy.is_none() {
            return Err(EconomicsError::Missing("initial_margin_on_buy"));
        }
        if self.initial_margin_on_sell.is_none() {
            return Err(EconomicsError::Missing("initial_margin_on_sell"));
        }
        if self.min_price_increment.is_none() {
            return Err(EconomicsError::Missing("min_price_increment"));
        }
        if self.min_price_increment_amount.is_none() {
            return Err(EconomicsError::Missing("min_price_increment_amount"));
        }
        Ok(())
    }
}

fn quotation(value: Option<v1::Quotation>) -> Result<Option<UnitsNano>, EconomicsError> {
    value
        .map(|value| {
            UnitsNano::new(value.units, value.nano).map_err(|_| EconomicsError::InvalidUnitsNano)
        })
        .transpose()
}

fn instrument_uid(value: String) -> Option<InstrumentUid> {
    InstrumentUid::new(value).ok()
}

fn optional_text(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum EconomicsError {
    #[error("missing trading-critical provider economics: {0}")]
    Missing(&'static str),
    #[error("provider units/nano value is outside canonical range")]
    InvalidUnitsNano,
}

impl From<RequestValidationError> for EconomicsError {
    fn from(_: RequestValidationError) -> Self {
        Self::InvalidUnitsNano
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn observed_margin_omission_survives_generated_decode_and_fails_closed() {
        let observed_shape = v1::GetFuturesMarginResponse {
            initial_margin_on_buy: None,
            initial_margin_on_sell: Some(v1::MoneyValue {
                currency: "rub".to_owned(),
                units: 123,
                nano: 0,
            }),
            min_price_increment: Some(v1::Quotation { units: 0, nano: 10 }),
            min_price_increment_amount: Some(v1::Quotation { units: 1, nano: 0 }),
        };
        let decoded =
            v1::GetFuturesMarginResponse::decode(observed_shape.encode_to_vec().as_slice())
                .expect("official generated contract must decode fixture");
        assert!(decoded.initial_margin_on_buy.is_none());
        let canonical =
            CanonicalFuturesMargin::try_from(decoded).expect("mapping must preserve omission");
        assert_eq!(
            canonical.require_complete(),
            Err(EconomicsError::Missing("initial_margin_on_buy"))
        );
    }

    #[test]
    fn provider_identity_kinds_are_not_interchangeable_or_fabricated() {
        let mapped = CanonicalAssetInstrument::from(v1::AssetInstrument {
            uid: "instrument-uid".to_owned(),
            figi: "figi".to_owned(),
            instrument_type: "share".to_owned(),
            ticker: "SBER".to_owned(),
            class_code: "TQBR".to_owned(),
            links: Vec::new(),
            instrument_kind: 99_999,
            position_uid: String::new(),
        });
        assert_eq!(
            mapped.uid.as_ref().map(InstrumentUid::as_str),
            Some("instrument-uid")
        );
        assert_eq!(mapped.position_uid, None);
        assert_eq!(mapped.kind, ProviderInstrumentKind::Unknown(99_999));
    }
}
