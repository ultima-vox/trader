//! Production T-Invest instrument catalogue and reference-data adapter.
//!
//! Wire JSON is decoded here into Vox-owned types. Provider records are kept
//! separate from Nautilus runtime instruments because several T-Invest
//! families have no faithful Nautilus representation.

use core::fmt;
use std::collections::BTreeMap;
use std::future::Future;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Number;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use vox_domain::{InstrumentIdentity, MutationAuthorization, UnitsNano};

use crate::{ProviderResponse, ResponseMetadata, RestError, TInvestRestClient};

const SERVICE: &str = "tinkoff.public.invest.api.contract.v1.InstrumentsService";

fn path(method: &str) -> String {
    format!("/{SERVICE}/{method}")
}

/// Exact protobuf `Quotation` wire value.
///
/// Proto3 JSON may omit either scalar when it has its default value. Absence
/// stays distinct from an explicit zero until a consumer validates economics.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Quotation {
    pub units: Option<i64>,
    pub nano: Option<i32>,
}

impl Quotation {
    pub fn new(units: i64, nano: i32) -> Result<Self, vox_domain::FixedPointError> {
        UnitsNano::new(units, nano)?;
        Ok(Self {
            units: Some(units),
            nano: Some(nano),
        })
    }

    pub fn value(self) -> Result<UnitsNano, CriticalDataError> {
        let units = self
            .units
            .ok_or(CriticalDataError::Missing("quotation.units"))?;
        let nano = self
            .nano
            .ok_or(CriticalDataError::Missing("quotation.nano"))?;
        UnitsNano::new(units, nano).map_err(|_| CriticalDataError::Invalid("quotation units/nano"))
    }
}

impl Serialize for Quotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            #[serde(skip_serializing_if = "Option::is_none")]
            units: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            nano: Option<i32>,
        }
        Wire {
            units: self.units.map(|value| value.to_string()),
            nano: self.nano,
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
            #[serde(default, deserialize_with = "deserialize_optional_i64")]
            units: Option<i64>,
            #[serde(default)]
            nano: Option<i32>,
        }
        let wire = Wire::deserialize(deserializer)?;
        if let (Some(units), Some(nano)) = (wire.units, wire.nano) {
            Self::new(units, nano).map_err(de::Error::custom)
        } else {
            Ok(Self {
                units: wire.units,
                nano: wire.nano,
            })
        }
    }
}

fn deserialize_optional_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<serde_json::Value>::deserialize(deserializer)?
        .map(|value| serde_json::from_value(value).and_then(parse_i64_value))
        .transpose()
        .map_err(de::Error::custom)
}

fn parse_i64_value(value: serde_json::Value) -> Result<i64, serde_json::Error> {
    match value {
        serde_json::Value::Number(value) => value
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("protobuf int64 is out of range")),
        serde_json::Value::String(value) => value.parse().map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom(
            "protobuf int64 must be a number or decimal string",
        )),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_i64",
        skip_serializing_if = "Option::is_none"
    )]
    pub units: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nano: Option<i32>,
}

impl MoneyValue {
    pub fn amount(&self) -> Result<UnitsNano, CriticalDataError> {
        let units = self
            .units
            .ok_or(CriticalDataError::Missing("money_value.units"))?;
        let nano = self
            .nano
            .ok_or(CriticalDataError::Missing("money_value.nano"))?;
        UnitsNano::new(units, nano)
            .map_err(|_| CriticalDataError::Invalid("money_value units/nano"))
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

    fn datetime(&self) -> OffsetDateTime {
        match OffsetDateTime::parse(&self.0, &Rfc3339) {
            Ok(value) => value,
            Err(_) => unreachable!("Timestamp is validated at construction"),
        }
    }

    fn from_datetime(value: OffsetDateTime) -> Self {
        match value.format(&Rfc3339) {
            Ok(value) => Self(value),
            Err(error) => unreachable!("OffsetDateTime always formats as RFC3339: {error}"),
        }
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

/// Vox-owned forward-compatible value for provider fields added after this
/// contract snapshot. Numbers retain exact JSON spelling; raw JSON never
/// crosses the adapter boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderValue {
    Null,
    Bool(bool),
    Decimal(ExactDecimal),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl<'de> Deserialize<'de> for ProviderValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        fn convert(value: serde_json::Value) -> ProviderValue {
            match value {
                serde_json::Value::Null => ProviderValue::Null,
                serde_json::Value::Bool(value) => ProviderValue::Bool(value),
                serde_json::Value::Number(value) => {
                    ProviderValue::Decimal(ExactDecimal(value.to_string()))
                }
                serde_json::Value::String(value) => ProviderValue::String(value),
                serde_json::Value::Array(values) => {
                    ProviderValue::Array(values.into_iter().map(convert).collect())
                }
                serde_json::Value::Object(values) => ProviderValue::Object(
                    values
                        .into_iter()
                        .map(|(name, value)| (name, convert(value)))
                        .collect(),
                ),
            }
        }

        serde_json::Value::deserialize(deserializer).map(convert)
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
    ClearingCertificate,
    Index,
    Commodity,
    Dfa,
    Indicative,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ProviderInstrumentType {
    Unspecified,
    Bond,
    Share,
    Currency,
    Etf,
    Futures,
    StructuredNote,
    Option,
    ClearingCertificate,
    Index,
    Commodity,
    Dfa,
    Unknown(String),
}

impl ProviderInstrumentType {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unspecified => "INSTRUMENT_TYPE_UNSPECIFIED",
            Self::Bond => "INSTRUMENT_TYPE_BOND",
            Self::Share => "INSTRUMENT_TYPE_SHARE",
            Self::Currency => "INSTRUMENT_TYPE_CURRENCY",
            Self::Etf => "INSTRUMENT_TYPE_ETF",
            Self::Futures => "INSTRUMENT_TYPE_FUTURES",
            Self::StructuredNote => "INSTRUMENT_TYPE_SP",
            Self::Option => "INSTRUMENT_TYPE_OPTION",
            Self::ClearingCertificate => "INSTRUMENT_TYPE_CLEARING_CERTIFICATE",
            Self::Index => "INSTRUMENT_TYPE_INDEX",
            Self::Commodity => "INSTRUMENT_TYPE_COMMODITY",
            Self::Dfa => "INSTRUMENT_TYPE_DFA",
            Self::Unknown(value) => value,
        }
    }

    #[must_use]
    pub const fn routing(&self) -> RoutingClass {
        match self {
            Self::Bond
            | Self::Share
            | Self::Currency
            | Self::Etf
            | Self::Futures
            | Self::Option => RoutingClass::TraderAndNautilus,
            Self::Unspecified
            | Self::StructuredNote
            | Self::ClearingCertificate
            | Self::Index
            | Self::Commodity
            | Self::Dfa
            | Self::Unknown(_) => RoutingClass::TraderOnly,
        }
    }
}

impl Serialize for ProviderInstrumentType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderInstrumentType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "INSTRUMENT_TYPE_UNSPECIFIED" => Self::Unspecified,
            "INSTRUMENT_TYPE_BOND" => Self::Bond,
            "INSTRUMENT_TYPE_SHARE" => Self::Share,
            "INSTRUMENT_TYPE_CURRENCY" => Self::Currency,
            "INSTRUMENT_TYPE_ETF" => Self::Etf,
            "INSTRUMENT_TYPE_FUTURES" => Self::Futures,
            "INSTRUMENT_TYPE_SP" => Self::StructuredNote,
            "INSTRUMENT_TYPE_OPTION" => Self::Option,
            "INSTRUMENT_TYPE_CLEARING_CERTIFICATE" => Self::ClearingCertificate,
            "INSTRUMENT_TYPE_INDEX" => Self::Index,
            "INSTRUMENT_TYPE_COMMODITY" => Self::Commodity,
            "INSTRUMENT_TYPE_DFA" => Self::Dfa,
            _ => Self::Unknown(value),
        })
    }
}

/// Common superset retained for every current instrument family. Optional
/// fields express provider applicability, not fabricated defaults.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Instrument {
    #[serde(default)]
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub uid: Option<String>,
    #[serde(default)]
    pub position_uid: Option<String>,
    #[serde(default)]
    pub asset_uid: Option<String>,
    #[serde(default)]
    pub isin: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub instrument_kind: Option<ProviderInstrumentType>,
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
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, ProviderValue>,
}

impl Instrument {
    pub fn try_identity(&self) -> Result<InstrumentIdentity, CriticalDataError> {
        let uid = required_text("uid", self.uid.as_deref())?;
        let ticker = required_text("ticker", self.ticker.as_deref())?;
        let class_code = required_text("class_code", self.class_code.as_deref())?;
        InstrumentIdentity::new(
            "tinvest",
            uid.to_owned(),
            self.figi.clone(),
            ticker.to_owned(),
            class_code.to_owned(),
        )
        .map_err(|_| CriticalDataError::Invalid("instrument identity"))
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
    Invalid(&'static str),
}

impl fmt::Display for CriticalDataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(field) => write!(formatter, "missing trading-critical field {field}"),
            Self::Invalid(field) => write!(formatter, "invalid trading-critical field {field}"),
        }
    }
}

impl std::error::Error for CriticalDataError {}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstrumentIdType {
    InstrumentIdUnspecified,
    InstrumentIdTypeFigi,
    InstrumentIdTypeTicker,
    InstrumentIdTypeUid,
    InstrumentIdTypePositionUid,
    InstrumentIdTypeId,
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
    pub instrument: Option<Instrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dfa {
    pub uid: Option<String>,
    pub ticker: Option<String>,
    pub name: Option<String>,
    pub position_uid: Option<String>,
    pub min_price_increment: Option<Quotation>,
    pub lot: Option<i32>,
    pub nominal: Option<MoneyValue>,
    pub currency: Option<String>,
    pub maturity_date: Option<Timestamp>,
    pub short_enabled_flag: Option<bool>,
    pub api_trade_available_flag: Option<bool>,
    pub buy_available_flag: Option<bool>,
    pub sell_available_flag: Option<bool>,
    pub limit_order_available_flag: Option<bool>,
    pub market_order_available_flag: Option<bool>,
    pub bestprice_order_available_flag: Option<bool>,
    pub for_iis_flag: Option<bool>,
    pub for_qual_investor_flag: Option<bool>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
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
    pub uid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DfaForecastYield {
    pub min_value: Option<Quotation>,
    pub max_value: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DfasResponse {
    #[serde(default)]
    pub instruments: Vec<Dfa>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct IndexInstrument {
    pub uid: Option<String>,
    pub weight: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Indicative {
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub currency: Option<String>,
    pub instrument_kind: Option<ProviderInstrumentType>,
    pub name: Option<String>,
    pub exchange: Option<String>,
    pub uid: Option<String>,
    pub buy_available_flag: Option<bool>,
    pub sell_available_flag: Option<bool>,
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
                instrument
                    .min_price_increment
                    .map(Quotation::value)
                    .transpose()?,
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
    fn every_current_instrument_type_has_explicit_routing() -> Result<(), serde_json::Error> {
        let expected = [
            ("INSTRUMENT_TYPE_UNSPECIFIED", RoutingClass::TraderOnly),
            ("INSTRUMENT_TYPE_BOND", RoutingClass::TraderAndNautilus),
            ("INSTRUMENT_TYPE_SHARE", RoutingClass::TraderAndNautilus),
            ("INSTRUMENT_TYPE_CURRENCY", RoutingClass::TraderAndNautilus),
            ("INSTRUMENT_TYPE_ETF", RoutingClass::TraderAndNautilus),
            ("INSTRUMENT_TYPE_FUTURES", RoutingClass::TraderAndNautilus),
            ("INSTRUMENT_TYPE_SP", RoutingClass::TraderOnly),
            ("INSTRUMENT_TYPE_OPTION", RoutingClass::TraderAndNautilus),
            (
                "INSTRUMENT_TYPE_CLEARING_CERTIFICATE",
                RoutingClass::TraderOnly,
            ),
            ("INSTRUMENT_TYPE_INDEX", RoutingClass::TraderOnly),
            ("INSTRUMENT_TYPE_COMMODITY", RoutingClass::TraderOnly),
            ("INSTRUMENT_TYPE_DFA", RoutingClass::TraderOnly),
        ];
        for (wire, routing) in expected {
            let kind: ProviderInstrumentType = serde_json::from_value(json!(wire))?;
            assert_eq!(kind.as_str(), wire);
            assert_eq!(kind.routing(), routing);
        }
        let unknown: ProviderInstrumentType = serde_json::from_value(json!("INSTRUMENT_TYPE_NEW"))?;
        assert_eq!(unknown.as_str(), "INSTRUMENT_TYPE_NEW");
        assert_eq!(unknown.routing(), RoutingClass::TraderOnly);
        Ok(())
    }

    #[test]
    fn every_legal_instrument_id_type_has_exact_request_shape() -> Result<(), serde_json::Error> {
        let cases = [
            (
                InstrumentIdType::InstrumentIdUnspecified,
                "INSTRUMENT_ID_UNSPECIFIED",
            ),
            (
                InstrumentIdType::InstrumentIdTypeFigi,
                "INSTRUMENT_ID_TYPE_FIGI",
            ),
            (
                InstrumentIdType::InstrumentIdTypeTicker,
                "INSTRUMENT_ID_TYPE_TICKER",
            ),
            (
                InstrumentIdType::InstrumentIdTypeUid,
                "INSTRUMENT_ID_TYPE_UID",
            ),
            (
                InstrumentIdType::InstrumentIdTypePositionUid,
                "INSTRUMENT_ID_TYPE_POSITION_UID",
            ),
            (
                InstrumentIdType::InstrumentIdTypeId,
                "INSTRUMENT_ID_TYPE_ID",
            ),
        ];
        for (id_type, wire) in cases {
            assert_eq!(
                serde_json::to_value(InstrumentRequest {
                    id_type,
                    class_code: None,
                    id: "IDENTIFIER",
                })?,
                json!({"idType":wire,"id":"IDENTIFIER"})
            );
        }
        Ok(())
    }

    #[test]
    fn bond_event_contract_retains_all_audit_and_economic_fields() -> Result<(), serde_json::Error>
    {
        let response: BondEventsResponse = serde_json::from_value(json!({"events":[{
            "instrumentId":"BOND","eventNumber":7,"eventDate":"2026-01-01T00:00:00Z",
            "eventType":"EVENT_TYPE_CPN","eventTotalVol":{"units":"100","nano":1},
            "fixDate":"2025-12-20T00:00:00Z","rateDate":"2025-12-21T00:00:00Z",
            "defaultDate":"2025-12-22T00:00:00Z","realPayDate":"2026-01-02T00:00:00Z",
            "payDate":"2026-01-01T00:00:00Z",
            "payOneBond":{"currency":"rub","units":"12","nano":3},
            "moneyFlowVal":{"currency":"rub","units":"1200","nano":4},
            "execution":"E","operationType":"FIX","value":{"units":"5","nano":6},
            "note":"paid","convertToFinToolId":"NEXT",
            "couponStartDate":"2025-07-01T00:00:00Z","couponEndDate":"2026-01-01T00:00:00Z",
            "couponPeriod":184,"couponInterestRate":{"units":"8","nano":9}
        }]}))?;
        let event = &response.events[0];
        assert_eq!(event.instrument_id.as_deref(), Some("BOND"));
        assert_eq!(
            event
                .pay_one_bond
                .as_ref()
                .map(MoneyValue::amount)
                .transpose(),
            Ok(Some(UnitsNano::new(12, 3).expect("valid fixture")))
        );
        assert_eq!(event.convert_to_fin_tool_id.as_deref(), Some("NEXT"));
        assert_eq!(
            event.coupon_interest_rate.expect("coupon rate").value(),
            Ok(UnitsNano::new(8, 9).expect("valid fixture"))
        );
        Ok(())
    }

    #[test]
    fn distinct_reference_responses_decode_without_field_loss() -> Result<(), serde_json::Error> {
        let dividends: DividendsResponse = serde_json::from_value(json!({"dividends":[{
            "dividendNet":{"currency":"rub","units":"1","nano":2},
            "paymentDate":"2026-02-01T00:00:00Z","declaredDate":"2026-01-01T00:00:00Z",
            "lastBuyDate":"2026-01-20T00:00:00Z","dividendType":"Regular Cash",
            "recordDate":"2026-01-22T00:00:00Z","regularity":"Annual",
            "closePrice":{"currency":"rub","units":"300","nano":4},
            "yieldValue":{"units":"5","nano":6},"createdAt":"2026-01-01T01:00:00Z"
        }]}))?;
        assert_eq!(dividends.dividends[0].regularity.as_deref(), Some("Annual"));

        let coupons: BondCouponsResponse = serde_json::from_value(json!({"events":[{
            "figi":"FIGI","couponDate":"2026-02-01T00:00:00Z","couponNumber":"17",
            "fixDate":"2026-01-20T00:00:00Z","payOneBond":{"currency":"rub","units":"9","nano":8},
            "couponType":"COUPON_TYPE_FLOATING","couponStartDate":"2025-08-01T00:00:00Z",
            "couponEndDate":"2026-02-01T00:00:00Z","couponPeriod":184
        }]}))?;
        assert_eq!(
            coupons.events[0].coupon_number.map(ProtoInt64::get),
            Some(17)
        );

        let accrued: AccruedInterestsResponse =
            serde_json::from_value(json!({"accruedInterests":[{
                "date":"2026-01-01T00:00:00Z","value":{"units":"2","nano":3},
                "valuePercent":{"units":"4","nano":5},"nominal":{"units":"1000","nano":0}
            }]}))?;
        assert_eq!(
            accrued.accrued_interests[0]
                .nominal
                .expect("nominal")
                .value()
                .expect("complete quotation")
                .units(),
            1000
        );

        let reports: AssetReportsResponse = serde_json::from_value(json!({"events":[{
            "instrumentId":"UID","reportDate":"2026-04-01T00:00:00Z","periodYear":2025,
            "periodNum":4,"periodType":"PERIOD_TYPE_ANNUAL","createdAt":"2026-01-01T00:00:00Z"
        }]}))?;
        assert_eq!(reports.events[0].period_year, Some(2025));

        let forecasts: ConsensusForecastsResponse = serde_json::from_value(json!({
            "items":[{"uid":"UID","assetUid":"ASSET","createdAt":"2026-01-01T00:00:00Z",
                "bestTargetPrice":{"units":"10","nano":1},"bestTargetLow":{"units":"8","nano":2},
                "bestTargetHigh":{"units":"12","nano":3},"totalBuyRecommend":4,
                "totalHoldRecommend":5,"totalSellRecommend":6,"currency":"rub",
                "consensus":"RECOMMENDATION_BUY","prognosisDate":"2026-06-01T00:00:00Z"}],
            "page":{"limit":20,"pageNumber":0,"totalCount":1}
        }))?;
        assert_eq!(forecasts.items[0].total_hold_recommend, Some(5));

        let forecast: ForecastResponse = serde_json::from_value(json!({
            "targets":[{"uid":"UID","ticker":"TICK","company":"House",
                "recommendation":"RECOMMENDATION_HOLD","recommendationDate":"2026-01-01T00:00:00Z",
                "currency":"rub","currentPrice":{"units":"1","nano":1},
                "targetPrice":{"units":"2","nano":2},"priceChange":{"units":"1","nano":1},
                "priceChangeRel":{"units":"100","nano":0},"showName":"Name"}],
            "consensus":{"uid":"UID","ticker":"TICK","recommendation":"RECOMMENDATION_HOLD",
                "currency":"rub","currentPrice":{"units":"1","nano":1},
                "consensus":{"units":"2","nano":2},"minTarget":{"units":"1","nano":0},
                "maxTarget":{"units":"3","nano":0},"priceChange":{"units":"1","nano":1},
                "priceChangeRel":{"units":"100","nano":0}}
        }))?;
        assert_eq!(forecast.targets[0].company.as_deref(), Some("House"));

        let insider: InsiderDealsResponse = serde_json::from_value(json!({
            "insiderDeals":[{"tradeId":"9","direction":"TRADE_DIRECTION_BUY","currency":"rub",
                "date":"2026-01-01T00:00:00Z","quantity":"10","price":{"units":"11","nano":12},
                "instrumentUid":"UID","ticker":"TICK","investorName":"Person",
                "investorPosition":"Director","percentage":0.123456789,"isOptionExecution":false,
                "disclosureDate":"2026-01-02T00:00:00Z"}],"nextCursor":"CURSOR"
        }))?;
        assert_eq!(
            insider.insider_deals[0]
                .percentage
                .as_ref()
                .map(ExactDecimal::as_str),
            Some("0.123456789")
        );

        let news: NewsResponse = serde_json::from_value(json!({"hasNext":true,"nextCursor":"42",
            "items":[{"id":"41","source":"T","title":"Title","content":"Body","summary":"Summary",
                "tables":[{"table":"a|b"}],"instrumentId":[{"instrument":{"instrumentUid":"UID",
                "ticker":"TICK","classCode":"TQBR"}}],"priority":true,"ts":"2026-01-01T00:00:00Z"}]
        }))?;
        assert_eq!(
            news.items[0].instrument_id[0]
                .instrument
                .as_ref()
                .and_then(|instrument| instrument.class_code.as_deref()),
            Some("TQBR")
        );
        Ok(())
    }

    #[test]
    fn futures_margin_omission_stays_none_and_economics_fail_closed()
    -> Result<(), serde_json::Error> {
        let response: FuturesMarginResponse = serde_json::from_value(json!({
            "initialMarginOnSell":{"currency":"rub","units":"15000","nano":0},
            "minPriceIncrement":{"units":"1","nano":0},
            "minPriceIncrementAmount":{"units":"10","nano":0}
        }))?;

        assert_eq!(response.initial_margin_on_buy, None);
        assert_eq!(
            response.require_economics(),
            Err(CriticalDataError::Missing("initial_margin_on_buy"))
        );
        Ok(())
    }

    #[test]
    fn proto3_json_omission_decodes_across_complete_reference_wire_surface()
    -> Result<(), serde_json::Error> {
        fn empty<T: serde::de::DeserializeOwned>() -> Result<T, serde_json::Error> {
            serde_json::from_value(json!({}))
        }

        let quotation: Quotation = empty()?;
        assert_eq!(quotation.units, None);
        assert_eq!(quotation.nano, None);
        assert_eq!(
            quotation.value(),
            Err(CriticalDataError::Missing("quotation.units"))
        );

        let money: MoneyValue = empty()?;
        assert_eq!(money.currency, None);
        assert_eq!(
            money.amount(),
            Err(CriticalDataError::Missing("money_value.units"))
        );

        let _: Instrument = empty()?;
        let _: InstrumentResponse = empty()?;
        let _: Dfa = empty()?;
        let _: DfaBasicAsset = empty()?;
        let _: DfaForecastYield = empty()?;
        let _: IndexInstrument = empty()?;
        let _: Indicative = empty()?;
        let _: PageResponse = empty()?;
        let _: Dividend = empty()?;
        let _: Coupon = empty()?;
        let _: AccruedInterest = empty()?;
        let _: BondEvent = empty()?;
        let _: AssetReport = empty()?;
        let _: ConsensusForecast = empty()?;
        let _: ForecastTarget = empty()?;
        let _: ForecastConsensus = empty()?;
        let _: AssetInstrument = empty()?;
        let _: Asset = empty()?;
        let _: AssetResponse = empty()?;
        let _: Brand = empty()?;
        let _: Country = empty()?;
        let _: InstrumentShort = empty()?;
        let _: Fundamental = empty()?;
        let _: FuturesMarginResponse = empty()?;
        let _: RiskRate = empty()?;
        let _: RiskRateResult = empty()?;
        let _: TradingInterval = empty()?;
        let _: TimeInterval = empty()?;
        let _: TradingDay = empty()?;
        let _: TradingSchedule = empty()?;
        let _: InsiderDealsResponse = empty()?;
        let _: InsiderDeal = empty()?;
        let _: NewsResponse = empty()?;
        let _: NewsItem = empty()?;
        let _: NewsTable = empty()?;
        let _: NewsInstrument = empty()?;
        let _: NewsInstrumentInfo = empty()?;
        let _: FavoriteInstrument = empty()?;
        let _: FavoriteGroup = empty()?;
        let _: FavoriteGroupMutationResponse = empty()?;
        Ok(())
    }

    #[test]
    fn trading_schedule_windows_honor_legal_and_exact_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        let start = Timestamp::parse("2026-01-01T00:00:00Z")?;
        let seven_days = Timestamp::parse("2026-01-08T00:00:00Z")?;
        let exact_limit = Timestamp::parse("2026-01-15T00:00:00Z")?;
        let over_limit = Timestamp::parse("2026-01-15T00:00:01Z")?;

        let legal = trading_schedule_windows(&TradingSchedulesRequest {
            exchange: None,
            from: &start,
            to: &seven_days,
        })?;
        assert_eq!(legal.len(), 1);
        assert_eq!(legal[0].from, start);
        assert_eq!(legal[0].to, seven_days);

        let exact = trading_schedule_windows(&TradingSchedulesRequest {
            exchange: Some("MOEX"),
            from: &start,
            to: &exact_limit,
        })?;
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].to, exact_limit);

        let split = trading_schedule_windows(&TradingSchedulesRequest {
            exchange: None,
            from: &start,
            to: &over_limit,
        })?;
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].to, exact_limit);
        assert_eq!(split[1].from, exact_limit);
        assert_eq!(split[1].to, over_limit);
        Ok(())
    }

    #[test]
    fn trading_schedule_merge_preserves_order_and_deduplicates_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        fn day(value: &str) -> Result<TradingDay, TimestampError> {
            Ok(TradingDay {
                date: Some(Timestamp::parse(value)?),
                is_trading_day: Some(true),
                start_time: None,
                end_time: None,
                intervals: Vec::new(),
            })
        }

        let first = day("2026-01-01T00:00:00Z")?;
        let boundary = day("2026-01-15T00:00:00Z")?;
        let last = day("2026-01-20T00:00:00Z")?;
        let mut merged = TradingSchedulesResponse {
            exchanges: vec![TradingSchedule {
                exchange: Some("MOEX".to_owned()),
                days: vec![first.clone(), boundary.clone()],
            }],
        };
        merge_trading_schedules(
            &mut merged,
            TradingSchedulesResponse {
                exchanges: vec![
                    TradingSchedule {
                        exchange: Some("MOEX".to_owned()),
                        days: vec![boundary, last.clone()],
                    },
                    TradingSchedule {
                        exchange: Some("SPB".to_owned()),
                        days: vec![first.clone()],
                    },
                ],
            },
        )?;

        assert_eq!(merged.exchanges[0].exchange.as_deref(), Some("MOEX"));
        assert_eq!(
            merged.exchanges[0].days,
            vec![first.clone(), day("2026-01-15T00:00:00Z")?, last]
        );
        assert_eq!(merged.exchanges[1].exchange.as_deref(), Some("SPB"));
        assert_eq!(merged.exchanges[1].days, vec![first]);
        Ok(())
    }

    #[tokio::test]
    async fn trading_schedule_chunk_failure_never_returns_partial_body()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let start = Timestamp::parse("2026-01-01T00:00:00Z")?;
        let finish = Timestamp::parse("2026-02-01T00:00:00Z")?;
        let windows = trading_schedule_windows(&TradingSchedulesRequest {
            exchange: None,
            from: &start,
            to: &finish,
        })?;
        assert_eq!(windows.len(), 3);

        let calls = Arc::new(AtomicUsize::new(0));
        let result = execute_trading_schedule_windows(windows, {
            let calls = Arc::clone(&calls);
            move |_| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if call == 1 {
                        return Err("observed chunk failure");
                    }
                    Ok(ProviderResponse {
                        body: TradingSchedulesResponse {
                            exchanges: Vec::new(),
                        },
                        metadata: ResponseMetadata {
                            request: crate::RequestMetadata {
                                request_id: uuid::Uuid::nil(),
                                operation: crate::RestOperation::SafeRead,
                                method_path: path("TradingSchedules"),
                                attempt: 1,
                            },
                            http_status: 200,
                            provider_tracking_id: None,
                        },
                    })
                }
            }
        })
        .await;

        match result {
            Err(ChunkExecutionError::Fetch {
                completed_windows,
                window_number,
                total_windows,
                source,
                ..
            }) => {
                assert_eq!(completed_windows, 1);
                assert_eq!(window_number, 2);
                assert_eq!(total_windows, 3);
                assert_eq!(source, "observed chunk failure");
            }
            Err(ChunkExecutionError::Merge(_)) => panic!("unexpected merge failure"),
            Ok(_) => panic!("partial schedules must never be returned as complete"),
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figi: Option<&'a str>,
    pub instrument_id: &'a str,
    pub from: &'a Timestamp,
    pub to: &'a Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProtoInt64(i64);

impl ProtoInt64 {
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

impl Serialize for ProtoInt64 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_i64(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for ProtoInt64 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_i64(deserializer).map(Self)
    }
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
    pub limit: Option<i32>,
    pub page_number: Option<i32>,
    pub total_count: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dividend {
    pub dividend_net: Option<MoneyValue>,
    pub payment_date: Option<Timestamp>,
    pub declared_date: Option<Timestamp>,
    pub last_buy_date: Option<Timestamp>,
    pub dividend_type: Option<String>,
    pub record_date: Option<Timestamp>,
    pub regularity: Option<String>,
    pub close_price: Option<MoneyValue>,
    pub yield_value: Option<Quotation>,
    pub created_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DividendsResponse {
    #[serde(default)]
    pub dividends: Vec<Dividend>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Coupon {
    pub figi: Option<String>,
    pub coupon_date: Option<Timestamp>,
    pub coupon_number: Option<ProtoInt64>,
    #[serde(default)]
    pub fix_date: Option<Timestamp>,
    pub pay_one_bond: Option<MoneyValue>,
    pub coupon_type: Option<ProviderEnum>,
    pub coupon_start_date: Option<Timestamp>,
    pub coupon_end_date: Option<Timestamp>,
    pub coupon_period: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BondCouponsResponse {
    #[serde(default)]
    pub events: Vec<Coupon>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccruedInterest {
    pub date: Option<Timestamp>,
    pub value: Option<Quotation>,
    pub value_percent: Option<Quotation>,
    pub nominal: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccruedInterestsResponse {
    #[serde(default)]
    pub accrued_interests: Vec<AccruedInterest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BondEvent {
    pub instrument_id: Option<String>,
    pub event_number: Option<i32>,
    pub event_date: Option<Timestamp>,
    pub event_type: Option<ProviderEnum>,
    pub event_total_vol: Option<Quotation>,
    #[serde(default)]
    pub fix_date: Option<Timestamp>,
    #[serde(default)]
    pub rate_date: Option<Timestamp>,
    #[serde(default)]
    pub default_date: Option<Timestamp>,
    #[serde(default)]
    pub real_pay_date: Option<Timestamp>,
    #[serde(default)]
    pub pay_date: Option<Timestamp>,
    #[serde(default)]
    pub pay_one_bond: Option<MoneyValue>,
    #[serde(default)]
    pub money_flow_val: Option<MoneyValue>,
    pub execution: Option<String>,
    pub operation_type: Option<String>,
    pub value: Option<Quotation>,
    pub note: Option<String>,
    pub convert_to_fin_tool_id: Option<String>,
    #[serde(default)]
    pub coupon_start_date: Option<Timestamp>,
    #[serde(default)]
    pub coupon_end_date: Option<Timestamp>,
    pub coupon_period: Option<i32>,
    pub coupon_interest_rate: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BondEventsResponse {
    #[serde(default)]
    pub events: Vec<BondEvent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetReport {
    pub instrument_id: Option<String>,
    pub report_date: Option<Timestamp>,
    pub period_year: Option<i32>,
    pub period_num: Option<i32>,
    pub period_type: Option<ProviderEnum>,
    pub created_at: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssetReportsResponse {
    #[serde(default)]
    pub events: Vec<AssetReport>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusForecast {
    pub uid: Option<String>,
    pub asset_uid: Option<String>,
    pub created_at: Option<Timestamp>,
    pub best_target_price: Option<Quotation>,
    pub best_target_low: Option<Quotation>,
    pub best_target_high: Option<Quotation>,
    pub total_buy_recommend: Option<i32>,
    pub total_hold_recommend: Option<i32>,
    pub total_sell_recommend: Option<i32>,
    pub currency: Option<String>,
    pub consensus: Option<ProviderEnum>,
    pub prognosis_date: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ConsensusForecastsResponse {
    #[serde(default)]
    pub items: Vec<ConsensusForecast>,
    pub page: Option<PageResponse>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForecastTarget {
    pub uid: Option<String>,
    pub ticker: Option<String>,
    pub company: Option<String>,
    pub recommendation: Option<ProviderEnum>,
    pub recommendation_date: Option<Timestamp>,
    pub currency: Option<String>,
    pub current_price: Option<Quotation>,
    pub target_price: Option<Quotation>,
    pub price_change: Option<Quotation>,
    pub price_change_rel: Option<Quotation>,
    pub show_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForecastConsensus {
    pub uid: Option<String>,
    pub ticker: Option<String>,
    pub recommendation: Option<ProviderEnum>,
    pub currency: Option<String>,
    pub current_price: Option<Quotation>,
    pub consensus: Option<Quotation>,
    pub min_target: Option<Quotation>,
    pub max_target: Option<Quotation>,
    pub price_change: Option<Quotation>,
    pub price_change_rel: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ForecastResponse {
    #[serde(default)]
    pub targets: Vec<ForecastTarget>,
    #[serde(default)]
    pub consensus: Option<ForecastConsensus>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssetInstrument {
    pub uid: Option<String>,
    #[serde(default)]
    pub figi: Option<String>,
    pub instrument_type: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    #[serde(default)]
    pub position_uid: Option<String>,
    #[serde(default)]
    pub instrument_kind: Option<ProviderInstrumentType>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, ProviderValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub uid: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<ProviderEnum>,
    pub name: Option<String>,
    #[serde(default)]
    pub name_brief: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required_tests: Vec<String>,
    #[serde(default)]
    pub instruments: Vec<AssetInstrument>,
    #[serde(flatten)]
    pub additional_fields: BTreeMap<String, ProviderValue>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssetsResponse {
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AssetResponse {
    pub asset: Option<Asset>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Brand {
    pub uid: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub info: Option<String>,
    pub company: Option<String>,
    pub sector: Option<String>,
    pub country_of_risk: Option<String>,
    pub country_of_risk_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BrandsResponse {
    #[serde(default)]
    pub brands: Vec<Brand>,
    pub paging: Option<PageResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Country {
    pub alfa_two: Option<String>,
    pub alfa_three: Option<String>,
    pub name: Option<String>,
    pub name_brief: Option<String>,
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
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub instrument_type: Option<String>,
    pub name: Option<String>,
    pub uid: Option<String>,
    #[serde(default)]
    pub position_uid: Option<String>,
    pub instrument_kind: Option<ProviderInstrumentType>,
    pub api_trade_available_flag: Option<bool>,
    pub for_iis_flag: Option<bool>,
    pub for_qual_investor_flag: Option<bool>,
    pub weekend_flag: Option<bool>,
    pub blocked_tca_flag: Option<bool>,
    pub lot: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FindInstrumentResponse {
    #[serde(default)]
    pub instruments: Vec<InstrumentShort>,
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
    pub asset_uid: Option<String>,
    pub currency: Option<String>,
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
        let asset_uid = take_optional(&mut fields, "assetUid")?;
        let currency = take_optional(&mut fields, "currency")?;
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
    pub initial_margin_on_buy: Option<MoneyValue>,
    pub initial_margin_on_sell: Option<MoneyValue>,
    pub min_price_increment: Option<Quotation>,
    pub min_price_increment_amount: Option<Quotation>,
}

pub type FuturesMarginResponse = FuturesMargin;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequiredMoneyValue<'a> {
    pub currency: &'a str,
    pub amount: UnitsNano,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FuturesMarginEconomics<'a> {
    pub initial_margin_on_buy: RequiredMoneyValue<'a>,
    pub initial_margin_on_sell: RequiredMoneyValue<'a>,
    pub min_price_increment: UnitsNano,
    pub min_price_increment_amount: UnitsNano,
}

impl FuturesMargin {
    pub fn require_economics(&self) -> Result<FuturesMarginEconomics<'_>, CriticalDataError> {
        fn require_money<'a>(
            field: &'static str,
            value: Option<&'a MoneyValue>,
        ) -> Result<RequiredMoneyValue<'a>, CriticalDataError> {
            let value = value.ok_or(CriticalDataError::Missing(field))?;
            Ok(RequiredMoneyValue {
                currency: required_text("money_value.currency", value.currency.as_deref())?,
                amount: value.amount()?,
            })
        }

        Ok(FuturesMarginEconomics {
            initial_margin_on_buy: require_money(
                "initial_margin_on_buy",
                self.initial_margin_on_buy.as_ref(),
            )?,
            initial_margin_on_sell: require_money(
                "initial_margin_on_sell",
                self.initial_margin_on_sell.as_ref(),
            )?,
            min_price_increment: self
                .min_price_increment
                .ok_or(CriticalDataError::Missing("min_price_increment"))?
                .value()?,
            min_price_increment_amount: self
                .min_price_increment_amount
                .ok_or(CriticalDataError::Missing("min_price_increment_amount"))?
                .value()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiskRate {
    pub risk_level_code: Option<String>,
    pub value: Option<Quotation>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RiskRateResult {
    pub instrument_uid: Option<String>,
    #[serde(default)]
    pub short_risk_rate: Option<RiskRate>,
    #[serde(default)]
    pub long_risk_rate: Option<RiskRate>,
    #[serde(default)]
    pub short_risk_rates: Vec<RiskRate>,
    #[serde(default)]
    pub long_risk_rates: Vec<RiskRate>,
    pub error: Option<String>,
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradingInterval {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub interval: Option<TimeInterval>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimeInterval {
    pub start_ts: Option<Timestamp>,
    pub end_ts: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TradingDay {
    pub date: Option<Timestamp>,
    pub is_trading_day: Option<bool>,
    #[serde(default)]
    pub start_time: Option<Timestamp>,
    #[serde(default)]
    pub end_time: Option<Timestamp>,
    #[serde(default)]
    pub intervals: Vec<TradingInterval>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct TradingSchedule {
    pub exchange: Option<String>,
    #[serde(default)]
    pub days: Vec<TradingDay>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
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

const MAX_TRADING_SCHEDULE_PERIOD: Duration = Duration::days(14);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingSchedulesWindow {
    pub from: Timestamp,
    pub to: Timestamp,
}

impl TradingSchedulesWindow {
    fn request<'a>(&'a self, exchange: Option<&'a str>) -> TradingSchedulesRequest<'a> {
        TradingSchedulesRequest {
            exchange,
            from: &self.from,
            to: &self.to,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TradingSchedulesResult {
    pub body: TradingSchedulesResponse,
    pub window_metadata: Vec<ResponseMetadata>,
}

impl TradingSchedulesResult {
    pub fn into_body(self) -> TradingSchedulesResponse {
        self.body
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TradingSchedulesError {
    #[error("trading-schedule range starts after it ends")]
    InvalidRange,
    #[error(
        "trading-schedule chunk {window_number}/{total_windows} failed after {completed_windows} complete windows"
    )]
    Partial {
        completed_windows: usize,
        window_number: usize,
        total_windows: usize,
        from: Timestamp,
        to: Timestamp,
        #[source]
        source: Box<RestError>,
    },
    #[error("provider returned conflicting trading days for exchange/date")]
    ConflictingDay {
        exchange: Option<String>,
        date: Timestamp,
    },
}

impl TradingSchedulesError {
    pub fn rest_error(&self) -> Option<&RestError> {
        match self {
            Self::Partial { source, .. } => Some(source.as_ref()),
            Self::InvalidRange | Self::ConflictingDay { .. } => None,
        }
    }
}

pub fn trading_schedule_windows(
    request: &TradingSchedulesRequest<'_>,
) -> Result<Vec<TradingSchedulesWindow>, TradingSchedulesError> {
    let start = request.from.datetime();
    let finish = request.to.datetime();
    if start > finish {
        return Err(TradingSchedulesError::InvalidRange);
    }

    let mut windows = Vec::new();
    let mut cursor = start;
    loop {
        let window_finish = cursor
            .checked_add(MAX_TRADING_SCHEDULE_PERIOD)
            .map_or(finish, |limit| limit.min(finish));
        windows.push(TradingSchedulesWindow {
            from: if cursor == start {
                request.from.clone()
            } else {
                Timestamp::from_datetime(cursor)
            },
            to: if window_finish == finish {
                request.to.clone()
            } else {
                Timestamp::from_datetime(window_finish)
            },
        });
        if window_finish == finish {
            return Ok(windows);
        }
        cursor = window_finish;
    }
}

fn merge_trading_schedules(
    merged: &mut TradingSchedulesResponse,
    next: TradingSchedulesResponse,
) -> Result<(), TradingSchedulesError> {
    for mut schedule in next.exchanges {
        let Some(existing) = merged
            .exchanges
            .iter_mut()
            .find(|candidate| candidate.exchange == schedule.exchange)
        else {
            merged.exchanges.push(schedule);
            continue;
        };

        for day in schedule.days.drain(..) {
            if existing.days.contains(&day) {
                continue;
            }
            if let Some(date) = day.date.as_ref()
                && existing
                    .days
                    .iter()
                    .any(|candidate| candidate.date.as_ref() == Some(date))
            {
                return Err(TradingSchedulesError::ConflictingDay {
                    exchange: existing.exchange.clone(),
                    date: date.clone(),
                });
            }
            existing.days.push(day);
        }
    }
    Ok(())
}

async fn execute_trading_schedule_windows<E, Fetch, FetchFuture>(
    windows: Vec<TradingSchedulesWindow>,
    mut fetch: Fetch,
) -> Result<TradingSchedulesResult, ChunkExecutionError<E>>
where
    Fetch: FnMut(TradingSchedulesWindow) -> FetchFuture,
    FetchFuture: Future<Output = Result<ProviderResponse<TradingSchedulesResponse>, E>>,
{
    let total_windows = windows.len();
    let mut body = TradingSchedulesResponse {
        exchanges: Vec::new(),
    };
    let mut window_metadata = Vec::with_capacity(total_windows);
    for (index, window) in windows.into_iter().enumerate() {
        let response =
            fetch(window.clone())
                .await
                .map_err(|source| ChunkExecutionError::Fetch {
                    completed_windows: index,
                    window_number: index + 1,
                    total_windows,
                    window,
                    source,
                })?;
        merge_trading_schedules(&mut body, response.body).map_err(ChunkExecutionError::Merge)?;
        window_metadata.push(response.metadata);
    }
    Ok(TradingSchedulesResult {
        body,
        window_metadata,
    })
}

enum ChunkExecutionError<E> {
    Fetch {
        completed_windows: usize,
        window_number: usize,
        total_windows: usize,
        window: TradingSchedulesWindow,
        source: E,
    },
    Merge(TradingSchedulesError),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindInstrumentRequest<'a> {
    pub query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instrument_kind: Option<&'a ProviderInstrumentType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_trade_available_flag: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<ProtoInt64>,
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

    pub fn after(&self, cursor: ProtoInt64) -> Self {
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
    pub instrument_type: Option<&'a ProviderInstrumentType>,
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
    pub insider_deals: Vec<InsiderDeal>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InsiderDeal {
    pub trade_id: Option<ProtoInt64>,
    pub direction: Option<ProviderEnum>,
    pub currency: Option<String>,
    pub date: Option<Timestamp>,
    pub quantity: Option<ProtoInt64>,
    pub price: Option<Quotation>,
    pub instrument_uid: Option<String>,
    pub ticker: Option<String>,
    pub investor_name: Option<String>,
    pub investor_position: Option<String>,
    pub percentage: Option<ExactDecimal>,
    pub is_option_execution: Option<bool>,
    pub disclosure_date: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewsResponse {
    pub has_next: Option<bool>,
    pub next_cursor: Option<ProtoInt64>,
    #[serde(default)]
    pub items: Vec<NewsItem>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub id: Option<ProtoInt64>,
    pub source: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub tables: Vec<NewsTable>,
    #[serde(default)]
    pub instrument_id: Vec<NewsInstrument>,
    pub priority: Option<bool>,
    pub ts: Option<Timestamp>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct NewsTable {
    pub table: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct NewsInstrument {
    pub instrument: Option<NewsInstrumentInfo>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewsInstrumentInfo {
    pub instrument_uid: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteInstrument {
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub instrument_uid: Option<String>,
    #[serde(default)]
    pub position_uid: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteGroup {
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    pub color: Option<String>,
    pub size: Option<i32>,
    pub contains_instrument: Option<bool>,
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
    (get_dividends, "GetDividends", PeriodRequest<'_>, DividendsResponse),
    (get_bond_coupons, "GetBondCoupons", PeriodRequest<'_>, BondCouponsResponse),
    (get_accrued_interests, "GetAccruedInterests", PeriodRequest<'_>, AccruedInterestsResponse),
    (get_bond_events, "GetBondEvents", BondEventsRequest<'_>, BondEventsResponse),
    (get_futures_margin, "GetFuturesMargin", InstrumentIdRequest<'_>, FuturesMarginResponse),
    (get_risk_rates, "GetRiskRates", RiskRatesRequest<'_>, RiskRatesResponse),
    (get_asset_fundamentals, "GetAssetFundamentals", AssetFundamentalsRequest<'_>, FundamentalsResponse),
    (get_asset_reports, "GetAssetReports", PeriodRequest<'_>, AssetReportsResponse),
    (get_consensus_forecasts, "GetConsensusForecasts", PagedRequest, ConsensusForecastsResponse),
    (get_forecast_by, "GetForecastBy", InstrumentIdRequest<'_>, ForecastResponse),
    (get_insider_deals, "GetInsiderDeals", InsiderDealsRequest<'_>, InsiderDealsResponse),
    (news, "News", NewsRequest, NewsResponse),
    (get_favorites, "GetFavorites", FavoritesRequest<'_>, FavoritesResponse),
    (get_favorite_groups, "GetFavoriteGroups", FavoriteGroupsRequest<'_>, FavoriteGroupsResponse),
);

impl TInvestRestClient {
    pub async fn trading_schedules(
        &self,
        request: &TradingSchedulesRequest<'_>,
    ) -> Result<TradingSchedulesResult, TradingSchedulesError> {
        let windows = trading_schedule_windows(request)?;
        let exchange = request.exchange.map(str::to_owned);
        execute_trading_schedule_windows(windows, |window| {
            let exchange = exchange.clone();
            async move {
                self.post_read(
                    &path("TradingSchedules"),
                    &window.request(exchange.as_deref()),
                )
                .await
            }
        })
        .await
        .map_err(|error| match error {
            ChunkExecutionError::Fetch {
                completed_windows,
                window_number,
                total_windows,
                window,
                source,
            } => TradingSchedulesError::Partial {
                completed_windows,
                window_number,
                total_windows,
                from: window.from,
                to: window.to,
                source: Box::new(source),
            },
            ChunkExecutionError::Merge(error) => error,
        })
    }
}

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
