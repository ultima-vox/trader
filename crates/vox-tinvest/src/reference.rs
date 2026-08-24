//! Vox reference policy above generated T-Invest protobuf types.
//!
//! Provider request/response definitions live only in [`crate::generated`].

use std::collections::BTreeSet;
use std::fmt;

use prost_types::Timestamp;
use thiserror::Error;

use crate::generated::v1;

pub const INSTRUMENTS_SERVICE_METHODS: &[&str] = &[
    "TradingSchedules",
    "BondBy",
    "Bonds",
    "GetBondCoupons",
    "GetBondEvents",
    "CurrencyBy",
    "Currencies",
    "EtfBy",
    "Etfs",
    "FutureBy",
    "Futures",
    "OptionBy",
    "Options",
    "OptionsBy",
    "ShareBy",
    "Shares",
    "DfaBy",
    "Dfas",
    "Indicatives",
    "GetAccruedInterests",
    "GetFuturesMargin",
    "GetInstrumentBy",
    "GetDividends",
    "GetAssetBy",
    "GetAssets",
    "GetFavorites",
    "EditFavorites",
    "CreateFavoriteGroup",
    "DeleteFavoriteGroup",
    "GetFavoriteGroups",
    "GetCountries",
    "FindInstrument",
    "GetBrands",
    "GetBrandBy",
    "GetAssetFundamentals",
    "GetAssetReports",
    "GetConsensusForecasts",
    "GetForecastBy",
    "GetRiskRates",
    "GetInsiderDeals",
    "StructuredNoteBy",
    "StructuredNotes",
    "News",
];

pub const MUTATION_METHODS: &[&str] = &[
    "EditFavorites",
    "CreateFavoriteGroup",
    "DeleteFavoriteGroup",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityState {
    Supported,
    ProviderDataUnavailable,
    ProviderInconsistency,
    UnsupportedInEnvironment,
    PermissionDenied,
    TemporarilyUnavailable,
    Deprecated,
}

#[derive(Clone, Debug)]
pub struct CapabilityRegistry {
    states: std::collections::BTreeMap<&'static str, CapabilityState>,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        let states = INSTRUMENTS_SERVICE_METHODS
            .iter()
            .copied()
            .map(|method| {
                let state = if method == "Options" {
                    CapabilityState::Deprecated
                } else {
                    CapabilityState::Supported
                };
                (method, state)
            })
            .collect();
        Self { states }
    }
}

impl CapabilityRegistry {
    #[must_use]
    pub fn state(&self, method: &str) -> Option<CapabilityState> {
        self.states.get(method).copied()
    }

    pub fn record_provider_code(&mut self, method: &'static str, code: tonic::Code) -> bool {
        let state = match code {
            tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
                CapabilityState::PermissionDenied
            }
            tonic::Code::Unimplemented => CapabilityState::UnsupportedInEnvironment,
            tonic::Code::Unavailable | tonic::Code::ResourceExhausted => {
                CapabilityState::TemporarilyUnavailable
            }
            _ => return false,
        };
        self.states.insert(method, state);
        true
    }

    pub fn mark_provider_data_unavailable(&mut self, method: &'static str) {
        self.states
            .insert(method, CapabilityState::ProviderDataUnavailable);
    }

    pub fn mark_provider_inconsistency(&mut self, method: &'static str) {
        self.states
            .insert(method, CapabilityState::ProviderInconsistency);
    }
}

macro_rules! provider_identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, RequestValidationError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(RequestValidationError::MissingIdentifier);
                }
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

provider_identifier!(InstrumentUid);
provider_identifier!(AssetUid);
provider_identifier!(PositionUid);
provider_identifier!(BrandUid);
provider_identifier!(ForecastRecordUid);

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum RequestValidationError {
    #[error("required provider identifier is empty")]
    MissingIdentifier,
    #[error("ticker lookup requires non-empty class_code")]
    MissingClassCode,
    #[error("request period starts after it ends")]
    InvalidRange,
    #[error("limit is outside documented provider bounds")]
    InvalidLimit,
    #[error("fundamentals request requires 1..=100 asset UIDs")]
    InvalidAssetCount,
}

#[must_use]
pub fn catalogue_request() -> v1::InstrumentsRequest {
    v1::InstrumentsRequest {
        instrument_status: Some(v1::InstrumentStatus::Base as i32),
        instrument_exchange: Some(v1::InstrumentExchangeType::InstrumentExchangeUnspecified as i32),
    }
}

#[must_use]
pub fn instrument_by_uid(uid: &InstrumentUid) -> v1::InstrumentRequest {
    v1::InstrumentRequest {
        id_type: v1::InstrumentIdType::Uid as i32,
        class_code: None,
        id: uid.as_str().to_owned(),
    }
}

pub fn instrument_by_ticker(
    ticker: impl Into<String>,
    class_code: impl Into<String>,
) -> Result<v1::InstrumentRequest, RequestValidationError> {
    let ticker = ticker.into();
    let class_code = class_code.into();
    if ticker.trim().is_empty() {
        return Err(RequestValidationError::MissingIdentifier);
    }
    if class_code.trim().is_empty() {
        return Err(RequestValidationError::MissingClassCode);
    }
    Ok(v1::InstrumentRequest {
        id_type: v1::InstrumentIdType::Ticker as i32,
        class_code: Some(class_code),
        id: ticker,
    })
}

#[must_use]
pub fn consensus_asset_uids(response: &v1::GetConsensusForecastsResponse) -> Vec<AssetUid> {
    let mut seen = BTreeSet::new();
    response
        .items
        .iter()
        .filter_map(|item| AssetUid::new(item.asset_uid.clone()).ok())
        .filter(|uid| seen.insert(uid.clone()))
        .take(8)
        .collect()
}

/// Exact provider-issued candidates only; deterministic and bounded.
#[must_use]
pub fn forecast_instrument_candidates(asset: &v1::AssetFull) -> Vec<InstrumentUid> {
    let mut candidates = asset
        .instruments
        .iter()
        .filter_map(|instrument| InstrumentUid::new(instrument.uid.clone()).ok())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates.truncate(8);
    candidates
}

pub fn validate_period(
    instrument_id: &str,
    from: Option<&Timestamp>,
    to: Option<&Timestamp>,
) -> Result<(), RequestValidationError> {
    if instrument_id.trim().is_empty() {
        return Err(RequestValidationError::MissingIdentifier);
    }
    if from
        .zip(to)
        .is_some_and(|(from, to)| (from.seconds, from.nanos) > (to.seconds, to.nanos))
    {
        return Err(RequestValidationError::InvalidRange);
    }
    Ok(())
}

pub fn fundamentals_request(
    assets: &[AssetUid],
) -> Result<v1::GetAssetFundamentalsRequest, RequestValidationError> {
    if assets.is_empty() || assets.len() > 100 {
        return Err(RequestValidationError::InvalidAssetCount);
    }
    Ok(v1::GetAssetFundamentalsRequest {
        assets: assets.iter().map(|uid| uid.as_str().to_owned()).collect(),
    })
}

pub fn insider_deals_request(
    instrument_uid: &InstrumentUid,
    limit: i32,
    next_cursor: Option<String>,
) -> Result<v1::GetInsiderDealsRequest, RequestValidationError> {
    if !(1..=100).contains(&limit) {
        return Err(RequestValidationError::InvalidLimit);
    }
    Ok(v1::GetInsiderDealsRequest {
        instrument_id: instrument_uid.as_str().to_owned(),
        limit,
        next_cursor,
    })
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error(
    "consensus assets yielded no forecast-capable instrument: assets={asset_uids:?}, candidates={candidate_count}"
)]
pub struct ForecastProviderInconsistency {
    pub asset_uids: Vec<AssetUid>,
    pub candidate_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_has_43_unique_methods_and_three_mutations() {
        assert_eq!(INSTRUMENTS_SERVICE_METHODS.len(), 43);
        assert_eq!(
            INSTRUMENTS_SERVICE_METHODS
                .iter()
                .collect::<BTreeSet<_>>()
                .len(),
            43
        );
        assert_eq!(MUTATION_METHODS.len(), 3);
    }

    #[test]
    fn consensus_record_uid_never_becomes_instrument_uid() {
        let response = v1::GetConsensusForecastsResponse {
            items: vec![
                v1::get_consensus_forecasts_response::ConsensusForecastsItem {
                    uid: "forecast-record".to_owned(),
                    asset_uid: "asset-authoritative".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            consensus_asset_uids(&response)[0].as_str(),
            "asset-authoritative"
        );
    }

    #[test]
    fn asset_candidates_are_exact_sorted_and_bounded() {
        let asset = v1::AssetFull {
            instruments: (0..40)
                .rev()
                .map(|index| v1::AssetInstrument {
                    uid: format!("instrument-{index:02}"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let candidates = forecast_instrument_candidates(&asset);
        assert_eq!(candidates.len(), 8);
        assert_eq!(candidates[0].as_str(), "instrument-00");
        assert_eq!(candidates[7].as_str(), "instrument-07");
    }

    #[test]
    fn provider_limits_validate_before_dispatch() {
        let uid = InstrumentUid::new("uid").expect("valid uid");
        assert_eq!(
            insider_deals_request(&uid, 101, None),
            Err(RequestValidationError::InvalidLimit)
        );
        assert_eq!(
            fundamentals_request(&[]),
            Err(RequestValidationError::InvalidAssetCount)
        );
    }
}
