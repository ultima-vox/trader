//! Production T-Invest instrument catalogue and reference-data adapter.
//!
//! Wire JSON is decoded here into Vox-owned types. Provider records are kept
//! separate from Nautilus runtime instruments because several T-Invest
//! families have no faithful Nautilus representation.

use core::fmt;
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Number;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use vox_domain::{InstrumentIdentity, InstrumentIdentityError, MutationAuthorization, UnitsNano};

use crate::{ProviderResponse, RestError, TInvestRestClient};

const SERVICE: &str = "tinkoff.public.invest.api.contract.v1.InstrumentsService";

fn path(method: &str) -> String {
    format!("/{SERVICE}/{method}")
}

/// Exact protobuf `Quotation` value. REST encodes `units` as decimal string.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Quotation(UnitsNano);

impl Quotation {
    pub fn new(units: i64, nano: i32) -> Result<Self, vox_domain::FixedPointError> {
        UnitsNano::new(units, nano).map(Self)
    }

    #[must_use]
    pub const fn value(self) -> UnitsNano {
        self.0
    }
}

impl Serialize for Quotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire<'a> {
            units: &'a str,
            nano: i32,
        }
        let units = self.0.units().to_string();
        Wire {
            units: &units,
            nano: self.0.nano(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Quotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(deserialize_with = "deserialize_i64")]
            units: i64,
            nano: i32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.units, wire.nano).map_err(de::Error::custom)
    }
}

fn deserialize_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        Number(i64),
        Text(String),
    }
    match Wire::deserialize(deserializer)? {
        Wire::Number(value) => Ok(value),
        Wire::Text(value) => value.parse().map_err(de::Error::custom),
    }
}

/// Exact monetary amount with provider currency code kept separately.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MoneyValue {
    pub currency: String,
    #[serde(deserialize_with = "deserialize_i64", serialize_with = "serialize_i64")]
    pub units: i64,
    pub nano: i32,
}

impl MoneyValue {
    pub fn amount(&self) -> Result<UnitsNano, vox_domain::FixedPointError> {
        UnitsNano::new(self.units, self.nano)
    }
}

fn serialize_i64<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// Validated UTC RFC3339 timestamp used by protobuf REST transcoding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    pub fn parse(value: impl Into<String>) -> Result<Self, TimestampError> {
        let value = value.into();
        OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| TimestampError)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimestampError;

impl fmt::Display for TimestampError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("provider timestamp must be valid RFC3339 UTC time")
    }
}

impl std::error::Error for TimestampError {}

/// Forward-compatible provider enum. Known values can be matched without
/// converting an unknown future value into a wrong fallback.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProviderEnum(String);

impl ProviderEnum {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_known(&self, known: &[&str]) -> bool {
        known.contains(&self.0.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstrumentFamily {
    Share,
    Bond,
    Etf,
    Currency,
    Future,
    Option,
    StructuredNote,
    Dfa,
    Indicative,
}

/// Common superset retained for every current instrument family. Optional
/// fields express provider applicability, not fabricated defaults.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Instrument {
    #[serde(default)]
    pub figi: Option<String>,
    pub ticker: String,
    pub class_code: String,
    pub uid: String,
    #[serde(default)]
    pub position_uid: Option<String>,
    #[serde(default)]
    pub asset_uid: Option<String>,
    #[serde(default)]
    pub isin: Option<String>,
    pub name: String,
    #[serde(default)]
    pub instrument_kind: Option<ProviderEnum>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub exchange: Option<String>,
    #[serde(default)]
    pub lot: Option<i64>,
    #[serde(default)]
    pub min_price_increment: Option<Quotation>,
    #[serde(default)]
    pub nominal: Option<MoneyValue>,
    #[serde(default)]
    pub basic_asset: Option<String>,
    #[serde(default)]
    pub basic_asset_uid: Option<String>,
    #[serde(default)]
    pub basic_asset_position_uid: Option<String>,
    #[serde(default)]
    pub basic_asset_size: Option<Quotation>,
    #[serde(default)]
    pub strike_price: Option<MoneyValue>,
    #[serde(default)]
    pub expiration_date: Option<Timestamp>,
    #[serde(default)]
    pub first_trade_date: Option<Timestamp>,
    #[serde(default)]
    pub last_trade_date: Option<Timestamp>,
    #[serde(default)]
    pub api_trade_available_flag: Option<bool>,
    #[serde(default)]
    pub buy_available_flag: Option<bool>,
    #[serde(default)]
    pub sell_available_flag: Option<bool>,
    #[serde(default)]
    pub required_tests: Vec<String>,
    #[serde(default)]
    pub option_type: Option<ProviderEnum>,
    #[serde(default)]
    pub payment_type: Option<ProviderEnum>,
    #[serde(default)]
    pub style: Option<ProviderEnum>,
    #[serde(default)]
    pub settlement_type: Option<ProviderEnum>,
}

impl Instrument {
    pub fn try_identity(&self) -> Result<InstrumentIdentity, InstrumentIdentityError> {
        InstrumentIdentity::new(
            "tinvest",
            self.uid.clone(),
            self.figi.clone(),
            self.ticker.clone(),
            self.class_code.clone(),
        )
    }

    pub fn require_option_economics(&self) -> Result<OptionEconomics<'_>, CriticalDataError> {
        Ok(OptionEconomics {
            basic_asset_uid: required_text("basic_asset_uid", self.basic_asset_uid.as_deref())?,
            strike_price: self
                .strike_price
                .as_ref()
                .ok_or(CriticalDataError::Missing("strike_price"))?,
            expiration_date: self
                .expiration_date
                .as_ref()
                .ok_or(CriticalDataError::Missing("expiration_date"))?,
            option_type: self
                .option_type
                .as_ref()
                .ok_or(CriticalDataError::Missing("option_type"))?,
        })
    }
}

fn required_text<'a>(
    field: &'static str,
    value: Option<&'a str>,
) -> Result<&'a str, CriticalDataError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(CriticalDataError::Missing(field))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OptionEconomics<'a> {
    pub basic_asset_uid: &'a str,
    pub strike_price: &'a MoneyValue,
    pub expiration_date: &'a Timestamp,
    pub option_type: &'a ProviderEnum,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CriticalDataError {
    Missing(&'static str),
}

impl fmt::Display for CriticalDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(field) => write!(formatter, "missing trading-critical field {field}"),
        }
    }
}

impl std::error::Error for CriticalDataError {}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstrumentIdType {
    InstrumentIdTypeFigi,
    InstrumentIdTypeTicker,
    InstrumentIdTypeUid,
    InstrumentIdTypePositionUid,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentRequest<'a> {
    pub id_type: InstrumentIdType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_code: Option<&'a str>,
    pub id: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstrumentStatus {
    InstrumentStatusUnspecified,
    InstrumentStatusBase,
    InstrumentStatusAll,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstrumentExchange {
    InstrumentExchangeUnspecified,
    InstrumentExchangeDealer,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentsRequest {
    pub instrument_status: InstrumentStatus,
    pub instrument_exchange: InstrumentExchange,
}

impl Default for InstrumentsRequest {
    fn default() -> Self {
        Self {
            instrument_status: InstrumentStatus::InstrumentStatusBase,
            instrument_exchange: InstrumentExchange::InstrumentExchangeUnspecified,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsByRequest<'a> {
    pub basic_asset_uid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basic_asset_position_uid: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basic_instrument_id: Option<&'a str>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct InstrumentsResponse {
    #[serde(default)]
    pub instruments: Vec<Instrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct InstrumentResponse {
    pub instrument: Instrument,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dfa {
    pub uid: String,
    pub ticker: String,
    pub name: String,
    pub position_uid: String,
    pub min_price_increment: Quotation,
    pub lot: i32,
    pub nominal: MoneyValue,
    pub currency: String,
    pub maturity_date: Timestamp,
    #[serde(default)]
    pub short_enabled_flag: bool,
    #[serde(default)]
    pub api_trade_available_flag: bool,
    #[serde(default)]
    pub buy_available_flag: bool,
    #[serde(default)]
    pub sell_available_flag: bool,
    #[serde(default)]
    pub limit_order_available_flag: bool,
    #[serde(default)]
    pub market_order_available_flag: bool,
    #[serde(default)]
    pub bestprice_order_available_flag: bool,
    #[serde(default)]
    pub for_iis_flag: bool,
    #[serde(default)]
    pub for_qual_investor_flag: bool,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub basic_assets: Vec<DfaBasicAsset>,
    #[serde(default)]
    pub forecast_yield: Option<DfaForecastYield>,
    #[serde(default)]
    pub yield_to_maturity: Option<Quotation>,
    #[serde(default)]
    pub coupon_value: Option<Quotation>,
    #[serde(default)]
    pub coupon_payment_frequency: Option<i32>,
    #[serde(default)]
    pub coupon_payment_date: Option<Timestamp>,
    #[serde(default)]
    pub aci_value: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct DfaBasicAsset {
    pub uid: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DfaForecastYield {
    pub min_value: Quotation,
    pub max_value: Quotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DfasResponse {
    #[serde(default)]
    pub instruments: Vec<Dfa>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct IndexInstrument {
    pub uid: String,
    pub weight: Quotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Indicative {
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub currency: String,
    pub instrument_kind: ProviderEnum,
    pub name: String,
    pub exchange: String,
    pub uid: String,
    pub buy_available_flag: bool,
    pub sell_available_flag: bool,
    #[serde(default)]
    pub index_composition: Vec<IndexInstrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct IndicativesResponse {
    #[serde(default)]
    pub instruments: Vec<Indicative>,
}

macro_rules! instrument_methods {
    ($(($list_fn:ident, $list_method:literal, $one_fn:ident, $one_method:literal)),+ $(,)?) => {
        impl TInvestRestClient {
            $(
                pub async fn $list_fn(&self, request: &InstrumentsRequest) -> Result<ProviderResponse<InstrumentsResponse>, RestError> {
                    self.post_read(&path($list_method), request).await
                }

                pub async fn $one_fn(&self, request: &InstrumentRequest<'_>) -> Result<ProviderResponse<InstrumentResponse>, RestError> {
                    self.post_read(&path($one_method), request).await
                }
            )+
        }
    };
}

instrument_methods!(
    (shares, "Shares", share_by, "ShareBy"),
    (bonds, "Bonds", bond_by, "BondBy"),
    (etfs, "Etfs", etf_by, "EtfBy"),
    (currencies, "Currencies", currency_by, "CurrencyBy"),
    (futures, "Futures", future_by, "FutureBy"),
    (
        structured_notes,
        "StructuredNotes",
        structured_note_by,
        "StructuredNoteBy"
    ),
);

impl TInvestRestClient {
    pub async fn options_by(
        &self,
        request: &OptionsByRequest<'_>,
    ) -> Result<ProviderResponse<InstrumentsResponse>, RestError> {
        self.post_read(&path("OptionsBy"), request).await
    }

    pub async fn option_by(
        &self,
        request: &InstrumentRequest<'_>,
    ) -> Result<ProviderResponse<InstrumentResponse>, RestError> {
        self.post_read(&path("OptionBy"), request).await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityState {
    Supported,
    UnsupportedByProvider,
    UnsupportedInEnvironment,
    PermissionDenied,
    TemporarilyUnavailable,
    Deprecated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MethodClass {
    SafeRead,
    Mutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingClass {
    TraderOnly,
    TraderAndNautilus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MethodCapability {
    pub method: &'static str,
    pub class: MethodClass,
    pub state: CapabilityState,
    pub routing: RoutingClass,
    pub nautilus_target: Option<&'static str>,
    pub requirements: &'static str,
    pub replacement: Option<&'static str>,
}

macro_rules! capability {
    ($method:literal, $class:ident, $routing:ident, $target:expr, $requirements:literal) => {
        MethodCapability {
            method: $method,
            class: MethodClass::$class,
            state: CapabilityState::Supported,
            routing: RoutingClass::$routing,
            nautilus_target: $target,
            requirements: $requirements,
            replacement: None,
        }
    };
}

/// Complete official InstrumentsService inventory checked 2026-08-24.
pub static CAPABILITIES: &[MethodCapability] = &[
    capability!("TradingSchedules", SafeRead, TraderOnly, None, "token"),
    capability!(
        "BondBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Bond"),
        "token"
    ),
    capability!(
        "Bonds",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Bond"),
        "token"
    ),
    capability!("GetBondCoupons", SafeRead, TraderOnly, None, "token"),
    capability!(
        "GetBondEvents",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout may vary"
    ),
    capability!(
        "CurrencyBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::CurrencyPair"),
        "token"
    ),
    capability!(
        "Currencies",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::CurrencyPair"),
        "token"
    ),
    capability!(
        "EtfBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Equity"),
        "token"
    ),
    capability!(
        "Etfs",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Equity"),
        "token"
    ),
    capability!(
        "FutureBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::FuturesContract"),
        "token"
    ),
    capability!(
        "Futures",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::FuturesContract"),
        "token"
    ),
    capability!(
        "OptionBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::OptionsContract"),
        "token"
    ),
    MethodCapability {
        method: "Options",
        class: MethodClass::SafeRead,
        state: CapabilityState::Deprecated,
        routing: RoutingClass::TraderOnly,
        nautilus_target: None,
        requirements: "do not call",
        replacement: Some("OptionsBy"),
    },
    capability!(
        "OptionsBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::OptionsContract"),
        "token; basic asset filter"
    ),
    capability!(
        "ShareBy",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Equity"),
        "token"
    ),
    capability!(
        "Shares",
        SafeRead,
        TraderAndNautilus,
        Some("nautilus_model::instruments::Equity"),
        "token"
    ),
    capability!("Indicatives", SafeRead, TraderOnly, None, "token"),
    capability!(
        "DfaBy",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout may vary"
    ),
    capability!(
        "Dfas",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout may vary"
    ),
    capability!("GetAccruedInterests", SafeRead, TraderOnly, None, "token"),
    capability!("GetFuturesMargin", SafeRead, TraderOnly, None, "token"),
    capability!("GetInstrumentBy", SafeRead, TraderOnly, None, "token"),
    capability!("GetDividends", SafeRead, TraderOnly, None, "token"),
    capability!("GetAssetBy", SafeRead, TraderOnly, None, "token"),
    capability!("GetAssets", SafeRead, TraderOnly, None, "token"),
    capability!("GetFavorites", SafeRead, TraderOnly, None, "token; account"),
    capability!(
        "EditFavorites",
        Mutation,
        TraderOnly,
        None,
        "account; explicit mutation authorization"
    ),
    capability!(
        "CreateFavoriteGroup",
        Mutation,
        TraderOnly,
        None,
        "account; explicit mutation authorization"
    ),
    capability!(
        "DeleteFavoriteGroup",
        Mutation,
        TraderOnly,
        None,
        "account; explicit mutation authorization"
    ),
    capability!(
        "GetFavoriteGroups",
        SafeRead,
        TraderOnly,
        None,
        "token; account"
    ),
    capability!("GetCountries", SafeRead, TraderOnly, None, "token"),
    capability!("FindInstrument", SafeRead, TraderOnly, None, "token"),
    capability!("GetBrands", SafeRead, TraderOnly, None, "token"),
    capability!("GetBrandBy", SafeRead, TraderOnly, None, "token"),
    capability!(
        "GetAssetFundamentals",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "GetAssetReports",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "GetConsensusForecasts",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "GetForecastBy",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "GetInsiderDeals",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "StructuredNoteBy",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout may vary"
    ),
    capability!(
        "StructuredNotes",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout may vary"
    ),
    capability!(
        "News",
        SafeRead,
        TraderOnly,
        None,
        "token; provider rollout/tariff may vary"
    ),
    capability!(
        "GetRiskRates",
        SafeRead,
        TraderOnly,
        None,
        "token; account risk profile"
    ),
];

#[must_use]
pub fn capability(method: &str) -> Option<&'static MethodCapability> {
    CAPABILITIES
        .iter()
        .find(|capability| capability.method == method)
}

#[derive(Clone, Debug)]
pub struct CapabilityRegistry {
    states: BTreeMap<&'static str, CapabilityState>,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self {
            states: CAPABILITIES
                .iter()
                .map(|entry| (entry.method, entry.state))
                .collect(),
        }
    }
}

impl CapabilityRegistry {
    #[must_use]
    pub fn state(&self, method: &str) -> Option<CapabilityState> {
        self.states.get(method).copied()
    }

    pub fn mark_unsupported_in_environment(&mut self, method: &str) -> bool {
        self.set(method, CapabilityState::UnsupportedInEnvironment)
    }

    pub fn record_provider_http(&mut self, method: &str, status: u16) -> bool {
        let Some(state) = capability_state_for_http_status(status) else {
            return false;
        };
        self.set(method, state)
    }

    fn set(&mut self, method: &str, state: CapabilityState) -> bool {
        let Some(value) = self.states.get_mut(method) else {
            return false;
        };
        if *value == CapabilityState::Deprecated {
            return false;
        }
        *value = state;
        true
    }
}

#[must_use]
pub const fn capability_state_for_http_status(status: u16) -> Option<CapabilityState> {
    match status {
        403 => Some(CapabilityState::PermissionDenied),
        501 => Some(CapabilityState::UnsupportedByProvider),
        408 | 429 | 500 | 502 | 503 | 504 => Some(CapabilityState::TemporarilyUnavailable),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_current_family_preserves_identity_and_exact_values()
    -> Result<(), Box<dyn std::error::Error>> {
        for kind in [
            "INSTRUMENT_TYPE_SHARE",
            "INSTRUMENT_TYPE_BOND",
            "INSTRUMENT_TYPE_ETF",
            "INSTRUMENT_TYPE_CURRENCY",
            "INSTRUMENT_TYPE_FUTURES",
            "INSTRUMENT_TYPE_OPTION",
            "INSTRUMENT_TYPE_SP",
            "INSTRUMENT_TYPE_DFA",
            "INSTRUMENT_TYPE_INDEX",
        ] {
            let instrument: Instrument = serde_json::from_value(json!({
                "figi": "FIGI", "ticker": "TICK", "classCode": "CLASS", "uid": "UID",
                "positionUid": "POSITION", "name": "Name", "instrumentKind": kind,
                "minPriceIncrement": {"units": "12", "nano": 345678901}
            }))?;
            let identity = instrument.try_identity()?;
            assert_eq!(identity.uid(), "UID");
            assert_eq!(identity.figi(), Some("FIGI"));
            assert_eq!(
                instrument.min_price_increment.map(Quotation::value),
                Some(UnitsNano::new(12, 345_678_901)?)
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_financial_data_and_missing_option_economics_fail_closed() {
        assert!(
            serde_json::from_value::<Quotation>(json!({"units": "0", "nano": 1_000_000_000}))
                .is_err()
        );
        let instrument: Instrument = serde_json::from_value(json!({
            "ticker": "OPT", "classCode": "SPBOPT", "uid": "UID", "name": "Option"
        }))
        .expect("minimal non-qualified provider record must decode");
        assert_eq!(
            instrument.require_option_economics(),
            Err(CriticalDataError::Missing("basic_asset_uid"))
        );
    }

    #[test]
    fn unknown_enums_are_preserved() -> Result<(), serde_json::Error> {
        let value: ProviderEnum =
            serde_json::from_str("\"INSTRUMENT_TYPE_FUTURE_PROVIDER_VALUE\"")?;
        assert_eq!(value.as_str(), "INSTRUMENT_TYPE_FUTURE_PROVIDER_VALUE");
        assert!(!value.is_known(&["INSTRUMENT_TYPE_SHARE"]));
        Ok(())
    }

    #[test]
    fn pagination_is_bounded_and_monotonic() -> Result<(), PaginationError> {
        let page = PageRequest::new(100, 0)?.next()?;
        assert_eq!(
            page,
            PageRequest {
                limit: 100,
                page_number: 1
            }
        );
        assert!(PageRequest::new(0, 0).is_err());
        assert!(PageRequest::new(1, -1).is_err());
        Ok(())
    }

    #[test]
    fn fundamentals_never_cross_binary_float_boundary() -> Result<(), serde_json::Error> {
        let response: FundamentalsResponse = serde_json::from_str(
            r#"{"fundamentals":[{"assetUid":"ASSET","currency":"rub","marketCapitalization":1234567890.123456789,"beta":"0.000000001"}]}"#,
        )?;
        assert_eq!(response.fundamentals[0].metrics.len(), 2);
        assert_eq!(
            response.fundamentals[0].metrics[0].value.as_str(),
            "0.000000001"
        );
        assert_eq!(
            response.fundamentals[0].metrics[1].value.as_str(),
            "1234567890.123456789"
        );
        Ok(())
    }

    #[test]
    fn current_request_shapes_match_provider_contract() -> Result<(), Box<dyn std::error::Error>> {
        let page = PageRequest::new(50, 0)?;
        assert_eq!(
            serde_json::to_value(PagedRequest { paging: page })?,
            json!({"paging":{"limit":50,"pageNumber":0}})
        );
        assert_eq!(
            serde_json::to_value(AssetFundamentalsRequest { assets: &["ASSET"] })?,
            json!({"assets":["ASSET"]})
        );
        assert_eq!(
            serde_json::to_value(CreateFavoriteGroupRequest {
                group_name: "Long term",
                group_color: "00AAFF",
                note: "core",
            })?,
            json!({"groupName":"Long term","groupColor":"00AAFF","note":"core"})
        );
        Ok(())
    }

    #[test]
    fn capability_inventory_has_no_duplicate_or_silent_method() {
        assert_eq!(CAPABILITIES.len(), 43);
        for (index, entry) in CAPABILITIES.iter().enumerate() {
            assert!(
                !CAPABILITIES[..index]
                    .iter()
                    .any(|other| other.method == entry.method)
            );
        }
        assert_eq!(
            capability("Options").map(|entry| entry.state),
            Some(CapabilityState::Deprecated)
        );
        assert_eq!(
            capability("Options").and_then(|entry| entry.replacement),
            Some("OptionsBy")
        );
    }

    #[test]
    fn capability_gates_degrade_only_affected_method() {
        let mut registry = CapabilityRegistry::default();
        assert!(registry.record_provider_http("GetAssetFundamentals", 403));
        assert_eq!(
            registry.state("GetAssetFundamentals"),
            Some(CapabilityState::PermissionDenied)
        );
        assert_eq!(registry.state("Shares"), Some(CapabilityState::Supported));
        assert!(registry.mark_unsupported_in_environment("News"));
        assert_eq!(
            registry.state("News"),
            Some(CapabilityState::UnsupportedInEnvironment)
        );
        assert!(!registry.record_provider_http("Options", 503));
        assert_eq!(registry.state("Options"), Some(CapabilityState::Deprecated));
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct EmptyRequest {}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdRequest<'a> {
    pub id: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentIdRequest<'a> {
    pub instrument_id: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodRequest<'a> {
    pub instrument_id: &'a str,
    pub from: &'a Timestamp,
    pub to: &'a Timestamp,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BondEventsRequest<'a> {
    pub instrument_id: &'a str,
    pub from: &'a Timestamp,
    pub to: &'a Timestamp,
    #[serde(rename = "type")]
    pub event_type: &'a str,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub limit: i32,
    pub page_number: i32,
}

impl PageRequest {
    pub fn new(limit: i32, page_number: i32) -> Result<Self, PaginationError> {
        if limit <= 0 || page_number < 0 {
            return Err(PaginationError);
        }
        Ok(Self { limit, page_number })
    }

    pub fn next(&self) -> Result<Self, PaginationError> {
        Self::new(
            self.limit,
            self.page_number.checked_add(1).ok_or(PaginationError)?,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PaginationError;

impl fmt::Display for PaginationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("page limit must be positive and page number non-negative")
    }
}

impl std::error::Error for PaginationError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageResponse {
    pub limit: i32,
    pub page_number: i32,
    pub total_count: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceRecord {
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub instrument_uid: Option<String>,
    #[serde(default)]
    pub asset_uid: Option<String>,
    #[serde(default)]
    pub figi: Option<String>,
    #[serde(default)]
    pub ticker: Option<String>,
    #[serde(default)]
    pub class_code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub exchange: Option<String>,
    #[serde(default)]
    pub value: Option<Quotation>,
    #[serde(default)]
    pub amount: Option<MoneyValue>,
    #[serde(default)]
    pub payment: Option<MoneyValue>,
    #[serde(default)]
    pub dividend_net: Option<MoneyValue>,
    #[serde(default)]
    pub coupon_number: Option<i64>,
    #[serde(default)]
    pub event_number: Option<i32>,
    #[serde(default)]
    pub date: Option<Timestamp>,
    #[serde(default)]
    pub event_date: Option<Timestamp>,
    #[serde(default)]
    pub payment_date: Option<Timestamp>,
    #[serde(default)]
    pub coupon_date: Option<Timestamp>,
    #[serde(default)]
    pub record_date: Option<Timestamp>,
    #[serde(default)]
    pub created_at: Option<Timestamp>,
    #[serde(default)]
    pub event_type: Option<ProviderEnum>,
    #[serde(default, rename = "type")]
    pub kind: Option<ProviderEnum>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetInstrument {
    pub uid: String,
    #[serde(default)]
    pub figi: Option<String>,
    pub instrument_type: String,
    pub ticker: String,
    pub class_code: String,
    #[serde(default)]
    pub position_uid: Option<String>,
    #[serde(default)]
    pub instrument_kind: Option<ProviderEnum>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub uid: String,
    #[serde(rename = "type")]
    pub kind: ProviderEnum,
    pub name: String,
    #[serde(default)]
    pub name_brief: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required_tests: Vec<String>,
    #[serde(default)]
    pub instruments: Vec<AssetInstrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssetsResponse {
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssetResponse {
    pub asset: Asset,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Brand {
    pub uid: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub sector: String,
    #[serde(default)]
    pub country_of_risk: String,
    #[serde(default)]
    pub country_of_risk_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BrandsResponse {
    #[serde(default)]
    pub brands: Vec<Brand>,
    pub paging: PageResponse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Country {
    pub alfa_two: String,
    pub alfa_three: String,
    pub name: String,
    pub name_brief: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CountriesResponse {
    #[serde(default)]
    pub countries: Vec<Country>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentShort {
    #[serde(default)]
    pub isin: Option<String>,
    #[serde(default)]
    pub figi: Option<String>,
    pub ticker: String,
    pub class_code: String,
    pub instrument_type: String,
    pub name: String,
    pub uid: String,
    #[serde(default)]
    pub position_uid: Option<String>,
    pub instrument_kind: ProviderEnum,
    #[serde(default)]
    pub api_trade_available_flag: bool,
    #[serde(default)]
    pub for_iis_flag: bool,
    #[serde(default)]
    pub for_qual_investor_flag: bool,
    #[serde(default)]
    pub weekend_flag: bool,
    #[serde(default)]
    pub blocked_tca_flag: bool,
    #[serde(default)]
    pub lot: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FindInstrumentResponse {
    #[serde(default)]
    pub instruments: Vec<InstrumentShort>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct RecordsResponse {
    #[serde(
        default,
        alias = "events",
        alias = "items",
        alias = "assets",
        alias = "brands",
        alias = "countries",
        alias = "instruments",
        alias = "fundamentals",
        alias = "reports",
        alias = "consensus",
        alias = "targets",
        alias = "insiderDeals",
        alias = "news"
    )]
    pub records: Vec<ReferenceRecord>,
    #[serde(default, alias = "page")]
    pub paging: Option<PageResponse>,
}

/// Exact lexical decimal for provider float/double reference fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExactDecimal(String);

impl ExactDecimal {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ExactDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Number(Number),
            Text(String),
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Number(value) => value.to_string(),
            Wire::Text(value) => value,
        };
        value
            .parse::<Number>()
            .map_err(de::Error::custom)
            .map(|_| Self(value))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FundamentalMetric {
    pub name: String,
    pub value: ExactDecimal,
}

/// Fundamentals use provider float/double fields. Adapter captures their exact
/// JSON decimal spelling and exposes no `f32`/`f64` conversion.
#[derive(Clone, Debug, PartialEq)]
pub struct Fundamental {
    pub asset_uid: String,
    pub currency: String,
    pub domicile_indicator_code: Option<String>,
    pub ex_dividend_date: Option<Timestamp>,
    pub fiscal_period_start_date: Option<Timestamp>,
    pub fiscal_period_end_date: Option<Timestamp>,
    pub metrics: Vec<FundamentalMetric>,
}

impl<'de> Deserialize<'de> for Fundamental {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut fields = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
        let asset_uid = take_required::<String, D::Error>(&mut fields, "assetUid")?;
        let currency = take_required::<String, D::Error>(&mut fields, "currency")?;
        let domicile_indicator_code = take_optional(&mut fields, "domicileIndicatorCode")?;
        let ex_dividend_date = take_optional(&mut fields, "exDividendDate")?;
        let fiscal_period_start_date = take_optional(&mut fields, "fiscalPeriodStartDate")?;
        let fiscal_period_end_date = take_optional(&mut fields, "fiscalPeriodEndDate")?;
        let metrics = fields
            .into_iter()
            .map(|(name, value)| {
                serde_json::from_value(value)
                    .map(|value| FundamentalMetric { name, value })
                    .map_err(de::Error::custom)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            asset_uid,
            currency,
            domicile_indicator_code,
            ex_dividend_date,
            fiscal_period_start_date,
            fiscal_period_end_date,
            metrics,
        })
    }
}

fn take_required<T, E>(
    fields: &mut BTreeMap<String, serde_json::Value>,
    name: &'static str,
) -> Result<T, E>
where
    T: serde::de::DeserializeOwned,
    E: de::Error,
{
    fields
        .remove(name)
        .ok_or_else(|| E::missing_field(name))
        .and_then(|value| serde_json::from_value(value).map_err(E::custom))
}

fn take_optional<T, E>(
    fields: &mut BTreeMap<String, serde_json::Value>,
    name: &'static str,
) -> Result<Option<T>, E>
where
    T: serde::de::DeserializeOwned,
    E: de::Error,
{
    fields
        .remove(name)
        .map(serde_json::from_value)
        .transpose()
        .map_err(E::custom)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FundamentalsResponse {
    #[serde(default)]
    pub fundamentals: Vec<Fundamental>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FuturesMargin {
    pub initial_margin_on_buy: MoneyValue,
    pub initial_margin_on_sell: MoneyValue,
    pub min_price_increment: Quotation,
    pub min_price_increment_amount: Quotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FuturesMarginResponse {
    pub initial_margin_on_buy: MoneyValue,
    pub initial_margin_on_sell: MoneyValue,
    pub min_price_increment: Quotation,
    pub min_price_increment_amount: Quotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiskRate {
    pub risk_level_code: String,
    pub value: Quotation,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiskRateResult {
    pub instrument_uid: String,
    #[serde(default)]
    pub short_risk_rate: Option<RiskRate>,
    #[serde(default)]
    pub long_risk_rate: Option<RiskRate>,
    #[serde(default)]
    pub short_risk_rates: Vec<RiskRate>,
    #[serde(default)]
    pub long_risk_rates: Vec<RiskRate>,
    #[serde(default)]
    pub error: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiskRatesResponse {
    #[serde(default)]
    pub instrument_risk_rates: Vec<RiskRateResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskRatesRequest<'a> {
    pub instrument_id: &'a [&'a str],
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradingInterval {
    #[serde(rename = "type")]
    pub kind: String,
    pub interval: TimeInterval,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimeInterval {
    pub start_ts: Timestamp,
    pub end_ts: Timestamp,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradingDay {
    pub date: Timestamp,
    pub is_trading_day: bool,
    #[serde(default)]
    pub start_time: Option<Timestamp>,
    #[serde(default)]
    pub end_time: Option<Timestamp>,
    #[serde(default)]
    pub intervals: Vec<TradingInterval>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TradingSchedule {
    pub exchange: String,
    #[serde(default)]
    pub days: Vec<TradingDay>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TradingSchedulesResponse {
    #[serde(default)]
    pub exchanges: Vec<TradingSchedule>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TradingSchedulesRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exchange: Option<&'a str>,
    pub from: &'a Timestamp,
    pub to: &'a Timestamp,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindInstrumentRequest<'a> {
    pub query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_kind: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_trade_available_flag: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i64>,
    pub limit: i32,
}

impl NewsRequest {
    pub fn first(limit: i32) -> Result<Self, PaginationError> {
        if limit <= 0 {
            return Err(PaginationError);
        }
        Ok(Self {
            cursor: None,
            limit,
        })
    }

    pub fn after(&self, cursor: i64) -> Self {
        Self {
            cursor: Some(cursor),
            limit: self.limit,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagedInstrumentRequest<'a> {
    pub instrument_id: &'a str,
    pub paging: PageRequest,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetsRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_status: Option<&'a str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PagedRequest {
    pub paging: PageRequest,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssetFundamentalsRequest<'a> {
    pub assets: &'a [&'a str],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsiderDealsRequest<'a> {
    pub instrument_id: &'a str,
    pub limit: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<&'a str>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InsiderDealsResponse {
    #[serde(default)]
    pub insider_deals: Vec<ReferenceRecord>,
    pub next_cursor: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewsResponse {
    pub has_next: bool,
    pub next_cursor: i64,
    #[serde(default)]
    pub items: Vec<ReferenceRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteInstrument {
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub instrument_uid: String,
    #[serde(default)]
    pub position_uid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteGroup {
    pub group_id: String,
    pub group_name: String,
    pub color: String,
    #[serde(default)]
    pub size: i32,
    #[serde(default)]
    pub contains_instrument: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoritesResponse {
    #[serde(default)]
    pub favorite_instruments: Vec<FavoriteInstrument>,
    #[serde(default)]
    pub group_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FavoriteGroupsResponse {
    #[serde(default)]
    pub groups: Vec<FavoriteGroup>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritesRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<&'a str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteGroupsRequest<'a> {
    #[serde(default)]
    pub instrument_id: &'a [&'a str],
    #[serde(default)]
    pub excluded_group_id: &'a [&'a str],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFavoritesRequest<'a> {
    pub instruments: &'a [EditFavoriteInstrument<'a>],
    pub action_type: EditFavoritesAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EditFavoritesAction {
    EditFavoritesActionTypeAdd,
    EditFavoritesActionTypeDel,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFavoriteInstrument<'a> {
    pub instrument_id: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFavoriteGroupRequest<'a> {
    pub group_name: &'a str,
    pub group_color: &'a str,
    pub note: &'a str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFavoriteGroupRequest<'a> {
    pub group_id: &'a str,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteGroupMutationResponse {
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub group_name: Option<String>,
}

macro_rules! read_methods {
    ($(($fn_name:ident, $method:literal, $request:ty, $response:ty)),+ $(,)?) => {
        impl TInvestRestClient {
            $(
                pub async fn $fn_name(&self, request: &$request) -> Result<ProviderResponse<$response>, RestError> {
                    self.post_read(&path($method), request).await
                }
            )+
        }
    };
}

read_methods!(
    (dfas, "Dfas", EmptyRequest, DfasResponse),
    (dfa_by, "DfaBy", InstrumentRequest<'_>, Dfa),
    (indicatives, "Indicatives", EmptyRequest, IndicativesResponse),
    (get_instrument_by, "GetInstrumentBy", InstrumentRequest<'_>, InstrumentResponse),
    (find_instrument, "FindInstrument", FindInstrumentRequest<'_>, FindInstrumentResponse),
    (get_assets, "GetAssets", AssetsRequest<'_>, AssetsResponse),
    (get_asset_by, "GetAssetBy", IdRequest<'_>, AssetResponse),
    (get_brands, "GetBrands", PagedRequest, BrandsResponse),
    (get_brand_by, "GetBrandBy", IdRequest<'_>, Brand),
    (get_countries, "GetCountries", EmptyRequest, CountriesResponse),
    (trading_schedules, "TradingSchedules", TradingSchedulesRequest<'_>, TradingSchedulesResponse),
    (get_dividends, "GetDividends", PeriodRequest<'_>, RecordsResponse),
    (get_bond_coupons, "GetBondCoupons", PeriodRequest<'_>, RecordsResponse),
    (get_accrued_interests, "GetAccruedInterests", PeriodRequest<'_>, RecordsResponse),
    (get_bond_events, "GetBondEvents", BondEventsRequest<'_>, RecordsResponse),
    (get_futures_margin, "GetFuturesMargin", InstrumentIdRequest<'_>, FuturesMarginResponse),
    (get_risk_rates, "GetRiskRates", RiskRatesRequest<'_>, RiskRatesResponse),
    (get_asset_fundamentals, "GetAssetFundamentals", AssetFundamentalsRequest<'_>, FundamentalsResponse),
    (get_asset_reports, "GetAssetReports", PeriodRequest<'_>, RecordsResponse),
    (get_consensus_forecasts, "GetConsensusForecasts", PagedRequest, RecordsResponse),
    (get_forecast_by, "GetForecastBy", InstrumentIdRequest<'_>, RecordsResponse),
    (get_insider_deals, "GetInsiderDeals", InsiderDealsRequest<'_>, InsiderDealsResponse),
    (news, "News", NewsRequest, NewsResponse),
    (get_favorites, "GetFavorites", FavoritesRequest<'_>, FavoritesResponse),
    (get_favorite_groups, "GetFavoriteGroups", FavoriteGroupsRequest<'_>, FavoriteGroupsResponse),
);

impl TInvestRestClient {
    pub async fn edit_favorites(
        &self,
        authorization: MutationAuthorization,
        request: &EditFavoritesRequest<'_>,
    ) -> Result<ProviderResponse<FavoritesResponse>, RestError> {
        self.post_mutation(authorization, &path("EditFavorites"), request)
            .await
    }

    pub async fn create_favorite_group(
        &self,
        authorization: MutationAuthorization,
        request: &CreateFavoriteGroupRequest<'_>,
    ) -> Result<ProviderResponse<FavoriteGroupMutationResponse>, RestError> {
        self.post_mutation(authorization, &path("CreateFavoriteGroup"), request)
            .await
    }

    pub async fn delete_favorite_group(
        &self,
        authorization: MutationAuthorization,
        request: &DeleteFavoriteGroupRequest<'_>,
    ) -> Result<ProviderResponse<FavoriteGroupMutationResponse>, RestError> {
        self.post_mutation(authorization, &path("DeleteFavoriteGroup"), request)
            .await
    }
}
