//! Narrow live qualification surface for issue #12. This is deliberately not
//! the reference-data catalogue owned by issue #7.

use std::{borrow::Cow, fmt};

use serde::de::{self, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use vox_domain::{FixedPoint, FuturesEconomics, FuturesEconomicsError, UnitsNano};

use crate::{
    AckKey, DesiredSubscription, ProviderMessage, ProviderResponse, RestError, SubscriptionId,
    SubscriptionRegistryError, TInvestRestClient, WebSocketError,
};

pub const SHARES_METHOD: &str = "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Shares";
pub const FUTURES_METHOD: &str =
    "/tinkoff.public.invest.api.contract.v1.InstrumentsService/Futures";
pub const FUTURES_MARGIN_METHOD: &str =
    "/tinkoff.public.invest.api.contract.v1.InstrumentsService/GetFuturesMargin";
pub const ORDER_BOOK_METHOD: &str =
    "/tinkoff.public.invest.api.contract.v1.MarketDataService/GetOrderBook";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quotation(UnitsNano);

impl Quotation {
    pub const fn exact(self) -> UnitsNano {
        self.0
    }

    pub const fn fixed_point(self) -> FixedPoint {
        self.0.fixed_point()
    }
}

impl Serialize for Quotation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut value = serializer.serialize_struct("Quotation", 2)?;
        value.serialize_field("units", &self.0.units().to_string())?;
        value.serialize_field("nano", &self.0.nano())?;
        value.end()
    }
}

impl<'de> Deserialize<'de> for Quotation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireQuotation {
            #[serde(deserialize_with = "deserialize_i64_string_or_number")]
            units: i64,
            nano: i32,
        }

        let wire = WireQuotation::deserialize(deserializer)?;
        UnitsNano::new(wire.units, wire.nano)
            .map(Self)
            .map_err(de::Error::custom)
    }
}

fn deserialize_i64_string_or_number<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    struct I64StringOrNumber;

    impl<'de> Visitor<'de> for I64StringOrNumber {
        type Value = i64;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an i64 or its exact decimal string")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            i64::try_from(value).map_err(E::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value.parse::<i64>().map_err(E::custom)
        }
    }

    deserializer.deserialize_any(I64StringOrNumber)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentsRequest {
    pub instrument_status: &'static str,
    pub instrument_exchange: &'static str,
}

impl InstrumentsRequest {
    pub const fn base() -> Self {
        Self {
            instrument_status: "INSTRUMENT_STATUS_BASE",
            instrument_exchange: "INSTRUMENT_EXCHANGE_UNSPECIFIED",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SharesResponse {
    #[serde(default)]
    pub instruments: Vec<ShareInstrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShareInstrument {
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub uid: Option<String>,
    pub currency: Option<String>,
    pub lot: Option<i64>,
    pub min_price_increment: Option<Quotation>,
    pub api_trade_available_flag: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualifiedShare<'a> {
    pub figi: &'a str,
    pub ticker: &'a str,
    pub class_code: &'a str,
    pub uid: &'a str,
    pub currency: &'a str,
    pub lot: i64,
    pub min_price_increment: Quotation,
}

impl<'a> TryFrom<&'a ShareInstrument> for QualifiedShare<'a> {
    type Error = QualificationDataError;

    fn try_from(instrument: &'a ShareInstrument) -> Result<Self, Self::Error> {
        let figi = required_share_text(instrument.figi.as_deref())?;
        let ticker = required_share_text(instrument.ticker.as_deref())?;
        let class_code = required_share_text(instrument.class_code.as_deref())?;
        let uid = required_share_text(instrument.uid.as_deref())?;
        let currency = required_share_text(instrument.currency.as_deref())?;
        if ticker != "SBER" || class_code != "TQBR" {
            return Err(QualificationDataError::InvalidShare(
                "selected share is not SBER/TQBR",
            ));
        }
        let lot = instrument
            .lot
            .ok_or(QualificationDataError::InvalidShare("lot is missing"))?;
        if lot <= 0 {
            return Err(QualificationDataError::InvalidShare("lot must be positive"));
        }
        let min_price_increment =
            instrument
                .min_price_increment
                .ok_or(QualificationDataError::InvalidShare(
                    "minimum price increment is missing",
                ))?;
        if min_price_increment.fixed_point().total_nanos() <= 0 {
            return Err(QualificationDataError::InvalidShare(
                "minimum price increment must be positive",
            ));
        }
        if instrument.api_trade_available_flag != Some(true) {
            return Err(QualificationDataError::InvalidShare(
                "API trading flag is missing or false",
            ));
        }
        Ok(Self {
            figi,
            ticker,
            class_code,
            uid,
            currency,
            lot,
            min_price_increment,
        })
    }
}

fn required_share_text(value: Option<&str>) -> Result<&str, QualificationDataError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(QualificationDataError::InvalidShare(
            "required identity field is missing or empty",
        ))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct FuturesResponse {
    #[serde(default)]
    pub instruments: Vec<FutureInstrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FutureInstrument {
    pub figi: Option<String>,
    pub ticker: Option<String>,
    pub class_code: Option<String>,
    pub uid: Option<String>,
    pub currency: Option<String>,
    pub lot: Option<i64>,
    pub min_price_increment: Option<Quotation>,
    pub api_trade_available_flag: Option<bool>,
    pub basic_asset: Option<String>,
    pub basic_asset_position_uid: Option<String>,
    pub basic_asset_uid: Option<String>,
    pub asset_type: Option<String>,
    pub first_trade_date: Option<String>,
    pub expiration_date: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualifiedFuture<'a> {
    pub figi: &'a str,
    pub ticker: &'a str,
    pub class_code: &'a str,
    pub uid: &'a str,
    pub currency: &'a str,
    pub lot: i64,
    pub min_price_increment: Quotation,
    pub basic_asset: Option<&'a str>,
    /// ASCII Nautilus adapter token derived from provider underlying metadata.
    pub underlying_id: Cow<'a, str>,
    pub asset_type: &'a str,
    pub first_trade_date: &'a str,
    pub expiration_date: &'a str,
}

impl<'a> TryFrom<&'a FutureInstrument> for QualifiedFuture<'a> {
    type Error = QualificationDataError;

    fn try_from(instrument: &'a FutureInstrument) -> Result<Self, Self::Error> {
        let figi = required_future_text(instrument.figi.as_deref())?;
        let ticker = required_future_text(instrument.ticker.as_deref())?;
        let class_code = required_future_text(instrument.class_code.as_deref())?;
        let uid = required_future_text(instrument.uid.as_deref())?;
        let currency = required_future_text(instrument.currency.as_deref())?;
        let basic_asset = instrument
            .basic_asset
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        let underlying_id = nautilus_underlying_token(instrument)?;
        let asset_type = required_future_text(instrument.asset_type.as_deref())?;
        let first_trade_date = required_future_text(instrument.first_trade_date.as_deref())?;
        let expiration_date = required_future_text(instrument.expiration_date.as_deref())?;
        if class_code != "SPBFUT" {
            return Err(QualificationDataError::InvalidFuture(
                "selected future class is not SPBFUT",
            ));
        }
        let activation = OffsetDateTime::parse(first_trade_date, &Rfc3339).map_err(|_| {
            QualificationDataError::InvalidFuture("first trade date is not RFC3339")
        })?;
        let expiration = OffsetDateTime::parse(expiration_date, &Rfc3339)
            .map_err(|_| QualificationDataError::InvalidFuture("expiration date is not RFC3339"))?;
        if activation >= expiration {
            return Err(QualificationDataError::InvalidFuture(
                "future lifecycle is empty or reversed",
            ));
        }
        let lot = instrument
            .lot
            .ok_or(QualificationDataError::InvalidFuture("lot is missing"))?;
        if lot <= 0 || instrument.api_trade_available_flag != Some(true) {
            return Err(QualificationDataError::InvalidFuture(
                "lot must be positive and API trading flag present and true",
            ));
        }
        let min_price_increment =
            instrument
                .min_price_increment
                .ok_or(QualificationDataError::InvalidFuture(
                    "minimum price increment is missing",
                ))?;
        if min_price_increment.fixed_point().total_nanos() <= 0 {
            return Err(QualificationDataError::InvalidFuture(
                "minimum price increment must be positive",
            ));
        }
        Ok(Self {
            figi,
            ticker,
            class_code,
            uid,
            currency,
            lot,
            min_price_increment,
            basic_asset,
            underlying_id,
            asset_type,
            first_trade_date,
            expiration_date,
        })
    }
}

fn nautilus_underlying_token(
    instrument: &FutureInstrument,
) -> Result<Cow<'_, str>, QualificationDataError> {
    for provider_uid in [
        instrument.basic_asset_position_uid.as_deref(),
        instrument.basic_asset_uid.as_deref(),
    ] {
        if let Some(provider_uid) =
            provider_uid.filter(|value| !value.trim().is_empty() && value.is_ascii())
        {
            return Ok(Cow::Borrowed(provider_uid));
        }
    }

    let basic_asset = instrument
        .basic_asset
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or(QualificationDataError::InvalidFuture(
            "provider underlying UID and basic asset are all missing",
        ))?;
    Ok(Cow::Owned(encode_basic_asset(basic_asset)))
}

fn encode_basic_asset(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    const PREFIX: &str = "TINVEST_BA_HEX_";

    let mut token = String::with_capacity(PREFIX.len() + value.len() * 2);
    token.push_str(PREFIX);
    for byte in value.as_bytes() {
        token.push(HEX[usize::from(byte >> 4)] as char);
        token.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    token
}

impl QualifiedFuture<'_> {
    pub fn exact_economics(
        &self,
        margin: &FuturesMarginResponse,
    ) -> Result<FuturesEconomics, QualificationDataError> {
        FuturesEconomics::new(
            self.min_price_increment.fixed_point(),
            margin.min_price_increment.fixed_point(),
            margin.min_price_increment_amount.fixed_point(),
        )
        .map_err(QualificationDataError::FuturesEconomics)
    }
}

fn required_future_text(value: Option<&str>) -> Result<&str, QualificationDataError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(QualificationDataError::InvalidFuture(
            "required identity/economics field is missing or empty",
        ))
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesMarginRequest<'a> {
    pub instrument_id: &'a str,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuturesMarginResponse {
    pub initial_margin_on_buy: Quotation,
    pub initial_margin_on_sell: Quotation,
    pub min_price_increment: Quotation,
    pub min_price_increment_amount: Quotation,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookRequest<'a> {
    pub instrument_id: &'a str,
    pub depth: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OrderBookResponse {
    pub figi: String,
    pub instrument_uid: String,
    pub depth: u32,
    pub orderbook_ts: String,
    #[serde(default)]
    pub bids: Vec<OrderBookLevel>,
    #[serde(default)]
    pub asks: Vec<OrderBookLevel>,
}

impl OrderBookResponse {
    pub fn validate_rq2(&self, expected_uid: &str) -> Result<(), QualificationDataError> {
        if self.instrument_uid != expected_uid || self.orderbook_ts.is_empty() || self.depth == 0 {
            return Err(QualificationDataError::InvalidOrderBook(
                "identity, timestamp, and positive depth are required",
            ));
        }
        if self.bids.is_empty() && self.asks.is_empty() {
            return Err(QualificationDataError::InvalidOrderBook(
                "snapshot has no levels",
            ));
        }
        for level in self.bids.iter().chain(&self.asks) {
            if level.quantity <= 0 || level.price.fixed_point().total_nanos() <= 0 {
                return Err(QualificationDataError::InvalidOrderBook(
                    "level price and quantity must be positive",
                ));
            }
        }
        Ok(())
    }
}

impl TInvestRestClient {
    pub async fn qualification_shares(
        &self,
    ) -> Result<ProviderResponse<SharesResponse>, RestError> {
        self.post_read(SHARES_METHOD, &InstrumentsRequest::base())
            .await
    }

    pub async fn qualification_futures(
        &self,
    ) -> Result<ProviderResponse<FuturesResponse>, RestError> {
        self.post_read(FUTURES_METHOD, &InstrumentsRequest::base())
            .await
    }

    pub async fn qualification_futures_margin(
        &self,
        instrument_uid: &str,
    ) -> Result<ProviderResponse<FuturesMarginResponse>, RestError> {
        self.post_read(
            FUTURES_MARGIN_METHOD,
            &FuturesMarginRequest {
                instrument_id: instrument_uid,
            },
        )
        .await
    }

    pub async fn qualification_order_book(
        &self,
        instrument_uid: &str,
        depth: u32,
    ) -> Result<ProviderResponse<OrderBookResponse>, RestError> {
        self.post_read(
            ORDER_BOOK_METHOD,
            &OrderBookRequest {
                instrument_id: instrument_uid,
                depth,
            },
        )
        .await
    }
}

pub fn select_sber(
    response: &SharesResponse,
) -> Result<QualifiedShare<'_>, QualificationDataError> {
    let mut candidates = response.instruments.iter().filter(|instrument| {
        instrument.ticker.as_deref() == Some("SBER")
            && instrument.class_code.as_deref() == Some("TQBR")
    });
    let candidate = candidates
        .next()
        .ok_or(QualificationDataError::SberNotFound)?;
    if candidates.next().is_some() {
        return Err(QualificationDataError::AmbiguousSber);
    }
    QualifiedShare::try_from(candidate)
}

/// Selects nearest-expiry active tradeable SPBFUT from BASE response.
pub fn select_tradeable_future(
    response: &FuturesResponse,
) -> Result<QualifiedFuture<'_>, QualificationDataError> {
    select_tradeable_future_at(response, OffsetDateTime::now_utc())
}

fn select_tradeable_future_at(
    response: &FuturesResponse,
    now: OffsetDateTime,
) -> Result<QualifiedFuture<'_>, QualificationDataError> {
    let mut selected: Option<(&FutureInstrument, OffsetDateTime)> = None;
    for instrument in response.instruments.iter().filter(|instrument| {
        instrument.class_code.as_deref() == Some("SPBFUT")
            && instrument.api_trade_available_flag == Some(true)
    }) {
        let Some(first_trade_date) = instrument.first_trade_date.as_deref() else {
            continue;
        };
        let Some(expiration_date) = instrument.expiration_date.as_deref() else {
            continue;
        };
        let Ok(activation) = OffsetDateTime::parse(first_trade_date, &Rfc3339) else {
            continue;
        };
        let Ok(expiration) = OffsetDateTime::parse(expiration_date, &Rfc3339) else {
            continue;
        };
        let earlier_than_selected =
            selected
                .as_ref()
                .is_none_or(|(current, current_expiration)| {
                    (expiration, instrument.ticker.as_deref())
                        < (*current_expiration, current.ticker.as_deref())
                });
        if activation <= now && now < expiration && earlier_than_selected {
            selected = Some((instrument, expiration));
        }
    }
    let candidate = selected
        .map(|(instrument, _)| instrument)
        .ok_or(QualificationDataError::FutureNotFound)?;
    QualifiedFuture::try_from(candidate)
}

#[derive(Debug, Error)]
pub enum QualificationDataError {
    #[error("expected one tradeable SBER/TQBR instrument, found none")]
    SberNotFound,
    #[error("expected one tradeable SBER/TQBR instrument, found multiple")]
    AmbiguousSber,
    #[error("no tradeable SPBFUT instrument in BASE response")]
    FutureNotFound,
    #[error("invalid RQ1 share: {0}")]
    InvalidShare(&'static str),
    #[error("invalid RQ1 future: {0}")]
    InvalidFuture(&'static str),
    #[error("invalid RQ2 order book: {0}")]
    InvalidOrderBook(&'static str),
    #[error("subscription acknowledgement does not match expected instrument UID/status")]
    InvalidAcknowledgement,
    #[error("inconsistent exact futures economics: {0}")]
    FuturesEconomics(#[source] FuturesEconomicsError),
    #[error("invalid qualification subscription: {0}")]
    Subscription(#[from] SubscriptionRegistryError),
    #[error("failed to encode typed qualification message: {0}")]
    WebSocket(#[from] WebSocketError),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketDataRequest {
    SubscribeTradesRequest(SubscribeTradesRequest),
    SubscribeOrderBookRequest(SubscribeOrderBookRequest),
    SubscribeInfoRequest(SubscribeInfoRequest),
    SubscribeLastPriceRequest(SubscribeLastPriceRequest),
    PingSettings(PingSettings),
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscribeTradesRequest {
    pub subscription_action: SubscriptionAction,
    pub instruments: Vec<TradeInstrumentRequest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TradeInstrumentRequest {
    pub instrument_id: String,
    pub trade_source: TradeSource,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscribeOrderBookRequest {
    pub subscription_action: SubscriptionAction,
    pub instruments: Vec<OrderBookInstrumentRequest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OrderBookInstrumentRequest {
    pub instrument_id: String,
    pub depth: u32,
    pub order_book_type: OrderBookType,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscribeInfoRequest {
    pub subscription_action: SubscriptionAction,
    pub instruments: Vec<InstrumentRequest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscribeLastPriceRequest {
    pub subscription_action: SubscriptionAction,
    pub instruments: Vec<InstrumentRequest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstrumentRequest {
    pub instrument_id: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum SubscriptionAction {
    #[serde(rename = "SUBSCRIPTION_ACTION_SUBSCRIBE")]
    Subscribe,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum TradeSource {
    #[serde(rename = "TRADE_SOURCE_ALL")]
    All,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum OrderBookType {
    #[serde(rename = "ORDERBOOK_TYPE_ALL")]
    All,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PingSettings {
    pub ping_delay_ms: u32,
}

pub fn rq2_market_data_subscriptions(
    instrument_uid: &str,
) -> Result<Vec<DesiredSubscription>, QualificationDataError> {
    if instrument_uid.trim().is_empty() || instrument_uid.chars().any(char::is_control) {
        return Err(QualificationDataError::InvalidShare(
            "instrument UID is empty or contains controls",
        ));
    }
    let instrument = || InstrumentRequest {
        instrument_id: instrument_uid.to_owned(),
    };
    let trades = MarketDataRequest::SubscribeTradesRequest(SubscribeTradesRequest {
        subscription_action: SubscriptionAction::Subscribe,
        instruments: vec![TradeInstrumentRequest {
            instrument_id: instrument_uid.to_owned(),
            trade_source: TradeSource::All,
        }],
    });
    let book = MarketDataRequest::SubscribeOrderBookRequest(SubscribeOrderBookRequest {
        subscription_action: SubscriptionAction::Subscribe,
        instruments: vec![OrderBookInstrumentRequest {
            instrument_id: instrument_uid.to_owned(),
            depth: 10,
            order_book_type: OrderBookType::All,
        }],
    });
    let info = MarketDataRequest::SubscribeInfoRequest(SubscribeInfoRequest {
        subscription_action: SubscriptionAction::Subscribe,
        instruments: vec![instrument()],
    });
    let last_price = MarketDataRequest::SubscribeLastPriceRequest(SubscribeLastPriceRequest {
        subscription_action: SubscriptionAction::Subscribe,
        instruments: vec![instrument()],
    });
    let ping = MarketDataRequest::PingSettings(PingSettings {
        ping_delay_ms: 5_000,
    });

    Ok(vec![
        desired(
            "rq2-trades",
            &trades,
            "subscribe_trades_response",
            instrument_uid,
        )?,
        desired(
            "rq2-order-book",
            &book,
            "subscribe_order_book_response",
            instrument_uid,
        )?,
        desired("rq2-info", &info, "subscribe_info_response", instrument_uid)?,
        desired(
            "rq2-last-price",
            &last_price,
            "subscribe_last_price_response",
            instrument_uid,
        )?,
        DesiredSubscription::without_ack(
            SubscriptionId::new("rq2-ping-settings")?,
            ProviderMessage::from_serializable(&ping)?,
        ),
    ])
}

fn desired(
    id: &str,
    request: &MarketDataRequest,
    acknowledgement: &str,
    instrument_uid: &str,
) -> Result<DesiredSubscription, QualificationDataError> {
    Ok(DesiredSubscription::new(
        SubscriptionId::new(id)?,
        ProviderMessage::from_serializable(request)?,
        AckKey::new(acknowledgement)?,
    )
    .with_expected_instrument_uid(instrument_uid))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct MarketDataStreamMessage {
    pub subscribe_trades_response: Option<TradesSubscriptionResponse>,
    pub subscribe_order_book_response: Option<OrderBookSubscriptionResponse>,
    pub subscribe_info_response: Option<InfoSubscriptionResponse>,
    pub subscribe_last_price_response: Option<LastPriceSubscriptionResponse>,
    pub trade: Option<TradeMessage>,
    pub orderbook: Option<OrderBookMessage>,
    pub trading_status: Option<TradingStatusMessage>,
    pub last_price: Option<LastPriceMessage>,
    pub ping: Option<PingMessage>,
}

impl MarketDataStreamMessage {
    pub fn acknowledgement_keys(&self) -> Vec<&'static str> {
        let mut keys = Vec::with_capacity(4);
        if self.subscribe_trades_response.is_some() {
            keys.push("subscribe_trades_response");
        }
        if self.subscribe_order_book_response.is_some() {
            keys.push("subscribe_order_book_response");
        }
        if self.subscribe_info_response.is_some() {
            keys.push("subscribe_info_response");
        }
        if self.subscribe_last_price_response.is_some() {
            keys.push("subscribe_last_price_response");
        }
        keys
    }

    pub const fn has_market_event(&self) -> bool {
        self.trade.is_some()
            || self.orderbook.is_some()
            || self.trading_status.is_some()
            || self.last_price.is_some()
    }

    pub fn validate_acknowledgement_uids(
        &self,
        expected_uid: &str,
    ) -> Result<(), QualificationDataError> {
        let groups: [&[SubscriptionStatus]; 4] = [
            self.subscribe_trades_response
                .as_ref()
                .map_or(&[], |response| response.trade_subscriptions.as_slice()),
            self.subscribe_order_book_response
                .as_ref()
                .map_or(&[], |response| response.order_book_subscriptions.as_slice()),
            self.subscribe_info_response
                .as_ref()
                .map_or(&[], |response| response.info_subscriptions.as_slice()),
            self.subscribe_last_price_response
                .as_ref()
                .map_or(&[], |response| response.last_price_subscriptions.as_slice()),
        ];
        if groups.into_iter().flatten().any(|status| {
            status.instrument_uid.as_deref() != Some(expected_uid)
                || status.subscription_status != "SUBSCRIPTION_STATUS_SUCCESS"
        }) {
            return Err(QualificationDataError::InvalidAcknowledgement);
        }
        Ok(())
    }
}

impl ProviderMessage {
    pub fn decode_qualification_market_data(
        &self,
    ) -> Result<MarketDataStreamMessage, WebSocketError> {
        self.decode()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TradesSubscriptionResponse {
    pub tracking_id: Option<String>,
    #[serde(default)]
    pub trade_subscriptions: Vec<SubscriptionStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OrderBookSubscriptionResponse {
    pub tracking_id: Option<String>,
    #[serde(default)]
    pub order_book_subscriptions: Vec<SubscriptionStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct InfoSubscriptionResponse {
    pub tracking_id: Option<String>,
    #[serde(default)]
    pub info_subscriptions: Vec<SubscriptionStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LastPriceSubscriptionResponse {
    pub tracking_id: Option<String>,
    #[serde(default)]
    pub last_price_subscriptions: Vec<SubscriptionStatus>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SubscriptionStatus {
    pub figi: Option<String>,
    pub instrument_uid: Option<String>,
    pub subscription_status: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TradeMessage {
    pub figi: String,
    pub instrument_uid: String,
    pub direction: String,
    pub price: Quotation,
    #[serde(deserialize_with = "deserialize_i64_string_or_number")]
    pub quantity: i64,
    pub time: String,
    pub trade_source: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OrderBookMessage {
    pub figi: String,
    pub instrument_uid: String,
    pub depth: u32,
    pub is_consistent: bool,
    pub time: String,
    #[serde(default)]
    pub bids: Vec<OrderBookLevel>,
    #[serde(default)]
    pub asks: Vec<OrderBookLevel>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct OrderBookLevel {
    pub price: Quotation,
    #[serde(deserialize_with = "deserialize_i64_string_or_number")]
    pub quantity: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct TradingStatusMessage {
    pub figi: String,
    pub instrument_uid: String,
    pub trading_status: String,
    pub time: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LastPriceMessage {
    pub figi: String,
    pub instrument_uid: String,
    pub price: Quotation,
    pub time: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PingMessage {
    pub time: String,
    pub stream_id: Option<String>,
    pub ping_request_time: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quotation_accepts_provider_int64_string_without_float() {
        let quotation = serde_json::from_value::<Quotation>(json!({
            "units": "833",
            "nano": 550_000_000
        }));
        let quotation = match quotation {
            Ok(quotation) => quotation,
            Err(error) => panic!("unexpected quotation error: {error}"),
        };
        assert_eq!(quotation.exact().units(), 833);
        assert_eq!(quotation.exact().nano(), 550_000_000);
        assert_eq!(quotation.fixed_point().total_nanos(), 833_550_000_000);
        assert!(
            serde_json::from_value::<Quotation>(json!({
                "units": "0",
                "nano": 1_000_000_000
            }))
            .is_err()
        );
    }

    #[test]
    fn rq2_helpers_are_typed_and_include_four_acks_plus_ping() {
        let desired = match rq2_market_data_subscriptions("instrument-uid") {
            Ok(desired) => desired,
            Err(error) => panic!("unexpected subscription error: {error}"),
        };
        assert_eq!(desired.len(), 5);
        assert_eq!(
            desired
                .iter()
                .filter(|subscription| subscription.acknowledgement.is_some())
                .count(),
            4
        );
        assert!(desired[4].acknowledgement.is_none());
        assert_eq!(
            desired[0]
                .request
                .as_value()
                .get("subscribe_trades_request")
                .and_then(|request| request.get("subscription_action")),
            Some(&json!("SUBSCRIPTION_ACTION_SUBSCRIBE"))
        );
    }

    #[test]
    fn typed_stream_message_reports_ack_and_exact_trade() {
        let decoded = serde_json::from_value::<MarketDataStreamMessage>(json!({
            "subscribe_trades_response": {
                "tracking_id": "track",
                "trade_subscriptions": [{
                    "instrument_uid": "uid",
                    "subscription_status": "SUBSCRIPTION_STATUS_SUCCESS"
                }]
            },
            "trade": {
                "figi": "BBG",
                "instrument_uid": "uid",
                "direction": "TRADE_DIRECTION_BUY",
                "price": {"units": "321", "nano": 500000000},
                "quantity": "2",
                "time": "2026-08-22T10:00:00Z",
                "trade_source": "TRADE_SOURCE_ALL"
            }
        }));
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) => panic!("unexpected stream decode error: {error}"),
        };
        assert_eq!(
            decoded.acknowledgement_keys(),
            vec!["subscribe_trades_response"]
        );
        assert!(decoded.validate_acknowledgement_uids("uid").is_ok());
        assert!(decoded.validate_acknowledgement_uids("wrong-uid").is_err());
        let trade = match decoded.trade {
            Some(trade) => trade,
            None => panic!("typed trade missing"),
        };
        assert_eq!(trade.price.fixed_point().total_nanos(), 321_500_000_000);
    }

    #[test]
    fn typed_order_book_snapshot_is_exact_and_nonempty() {
        let decoded = serde_json::from_value::<OrderBookResponse>(json!({
            "figi": "BBG",
            "instrumentUid": "uid",
            "depth": 10,
            "orderbookTs": "2026-08-22T10:00:00Z",
            "bids": [{"price": {"units": "321", "nano": 500000000}, "quantity": "2"}],
            "asks": []
        }));
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(error) => panic!("unexpected order book decode error: {error}"),
        };
        assert!(decoded.validate_rq2("uid").is_ok());
        assert_eq!(
            decoded.bids[0].price.fixed_point().total_nanos(),
            321_500_000_000
        );
        assert_eq!(decoded.bids[0].quantity, 2);
    }

    #[test]
    fn share_catalogue_ignores_unrelated_missing_economics() {
        let response = serde_json::from_value::<SharesResponse>(json!({
            "instruments": [
                {"ticker": "UNRELATED", "classCode": "TQBR"},
                share_json()
            ]
        }));
        let response = match response {
            Ok(response) => response,
            Err(error) => panic!("unexpected shares decode error: {error}"),
        };
        let selected = match select_sber(&response) {
            Ok(selected) => selected,
            Err(error) => panic!("unexpected SBER selection error: {error}"),
        };
        assert_eq!(selected.ticker, "SBER");
        assert_eq!(
            selected.min_price_increment.fixed_point().total_nanos(),
            1_000_000
        );
    }

    #[test]
    fn selected_share_missing_material_fields_fails_closed() {
        for missing in [
            "figi",
            "uid",
            "currency",
            "lot",
            "minPriceIncrement",
            "apiTradeAvailableFlag",
        ] {
            let mut share = share_json();
            let object = match share.as_object_mut() {
                Some(object) => object,
                None => panic!("share fixture must be an object"),
            };
            object.remove(missing);
            let response = serde_json::from_value::<SharesResponse>(json!({
                "instruments": [share]
            }));
            let response = match response {
                Ok(response) => response,
                Err(error) => panic!("wire optionality must deserialize {missing}: {error}"),
            };
            assert!(matches!(
                select_sber(&response),
                Err(QualificationDataError::InvalidShare(_))
            ));
        }
    }

    #[test]
    fn future_selector_uses_parsed_active_lifecycle_and_nearest_expiry() {
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": [
                future_json("EXPIRED", "2025-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
                future_json("LATER", "2026-01-01T00:00:00Z", "2027-06-01T00:00:00Z"),
                future_json("NEAREST", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z"),
                future_json("PENDING", "2027-01-01T00:00:00Z", "2027-02-01T00:00:00Z")
            ]
        }));
        let response = match response {
            Ok(response) => response,
            Err(error) => panic!("unexpected futures decode error: {error}"),
        };
        let now = match OffsetDateTime::parse("2026-08-23T00:00:00Z", &Rfc3339) {
            Ok(now) => now,
            Err(error) => panic!("unexpected timestamp error: {error}"),
        };
        let selected = match select_tradeable_future_at(&response, now) {
            Ok(selected) => selected,
            Err(error) => panic!("unexpected future selection error: {error}"),
        };
        assert_eq!(selected.ticker, "NEAREST");
        assert_eq!(selected.underlying_id, "underlying-NEAREST");
    }

    #[test]
    fn future_catalogue_ignores_unrelated_missing_economics() {
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": [
                {
                    "ticker": "EXPIRED-INCOMPLETE",
                    "classCode": "SPBFUT",
                    "apiTradeAvailableFlag": true,
                    "firstTradeDate": "2025-01-01T00:00:00Z",
                    "expirationDate": "2026-01-01T00:00:00Z"
                },
                future_json("ACTIVE", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z")
            ]
        }));
        let response = match response {
            Ok(response) => response,
            Err(error) => panic!("unexpected futures decode error: {error}"),
        };
        let now = timestamp("2026-08-23T00:00:00Z");
        let selected = match select_tradeable_future_at(&response, now) {
            Ok(selected) => selected,
            Err(error) => panic!("unexpected future selection error: {error}"),
        };
        assert_eq!(selected.ticker, "ACTIVE");
    }

    #[test]
    fn selected_nearest_future_missing_economics_fails_closed() {
        let mut nearest = future_json(
            "NEAREST-INCOMPLETE",
            "2026-01-01T00:00:00Z",
            "2027-03-01T00:00:00Z",
        );
        let object = match nearest.as_object_mut() {
            Some(object) => object,
            None => panic!("future fixture must be an object"),
        };
        object.remove("minPriceIncrement");
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": [
                nearest,
                future_json("LATER", "2026-01-01T00:00:00Z", "2027-06-01T00:00:00Z")
            ]
        }));
        let response = match response {
            Ok(response) => response,
            Err(error) => panic!("wire optionality must deserialize future: {error}"),
        };
        assert!(matches!(
            select_tradeable_future_at(&response, timestamp("2026-08-23T00:00:00Z")),
            Err(QualificationDataError::InvalidFuture(_))
        ));
    }

    #[test]
    fn non_ascii_provider_underlying_keeps_display_and_uses_authoritative_uid() {
        let mut future = future_json("KCQ6", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z");
        let object = future
            .as_object_mut()
            .expect("future fixture must be an object");
        object.insert("basicAsset".to_string(), json!("Кофе"));
        object.insert(
            "basicAssetPositionUid".to_string(),
            json!("f6d6c6b8-4f98-4ca2-8d47-a7d17e7154cb"),
        );
        object.insert("basicAssetUid".to_string(), json!("lower-priority-uid"));
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": [future]
        }))
        .expect("provider future must deserialize");

        let selected = select_tradeable_future_at(&response, timestamp("2026-08-23T00:00:00Z"))
            .expect("authoritative ASCII UID must qualify");

        assert_eq!(selected.basic_asset, Some("Кофе"));
        assert_eq!(
            selected.underlying_id,
            "f6d6c6b8-4f98-4ca2-8d47-a7d17e7154cb"
        );
    }

    #[test]
    fn basic_asset_uid_is_used_when_position_uid_is_not_valid_ascii() {
        let mut future = future_json("KCQ6", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z");
        let object = future
            .as_object_mut()
            .expect("future fixture must be an object");
        object.insert("basicAssetPositionUid".to_string(), json!("Кофе-ID"));
        object.insert("basicAssetUid".to_string(), json!("basic-asset-uid"));
        let instrument = serde_json::from_value::<FutureInstrument>(future)
            .expect("provider future must deserialize");

        let selected = QualifiedFuture::try_from(&instrument)
            .expect("secondary authoritative ASCII UID must qualify");

        assert_eq!(selected.underlying_id, "basic-asset-uid");
    }

    #[test]
    fn unicode_basic_asset_fallback_is_lossless_stable_and_preserved() {
        let mut futures = [
            future_json("KCQ6", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z"),
            future_json("KCU6", "2026-01-01T00:00:00Z", "2027-06-01T00:00:00Z"),
        ];
        for future in &mut futures {
            let object = future
                .as_object_mut()
                .expect("future fixture must be an object");
            object.insert("basicAsset".to_string(), json!("Кофе"));
            object.remove("basicAssetPositionUid");
            object.remove("basicAssetUid");
        }
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": futures
        }))
        .expect("provider futures must deserialize");
        let first = QualifiedFuture::try_from(&response.instruments[0])
            .expect("Unicode basic asset must encode");
        let second = QualifiedFuture::try_from(&response.instruments[1])
            .expect("same Unicode basic asset must encode across expiries");

        assert_eq!(first.basic_asset, Some("Кофе"));
        assert_eq!(second.basic_asset, Some("Кофе"));
        assert_eq!(first.underlying_id, "TINVEST_BA_HEX_D09AD0BED184D0B5");
        assert_eq!(second.underlying_id, first.underlying_id);
    }

    #[test]
    fn selected_future_with_all_underlying_sources_empty_fails_closed() {
        let mut future = future_json("KCQ6", "2026-01-01T00:00:00Z", "2027-03-01T00:00:00Z");
        let object = future
            .as_object_mut()
            .expect("future fixture must be an object");
        object.insert("basicAsset".to_string(), json!("  "));
        object.remove("basicAssetPositionUid");
        object.insert("basicAssetUid".to_string(), json!(""));
        let response = serde_json::from_value::<FuturesResponse>(json!({
            "instruments": [future]
        }))
        .expect("wire-optional underlying sources must deserialize");

        assert!(matches!(
            select_tradeable_future_at(&response, timestamp("2026-08-23T00:00:00Z")),
            Err(QualificationDataError::InvalidFuture(_))
        ));
    }

    fn share_json() -> serde_json::Value {
        json!({
            "figi": "BBG004730N88",
            "ticker": "SBER",
            "classCode": "TQBR",
            "uid": "e6123145-9665-43e0-8413-cd61b8aa9b13",
            "currency": "rub",
            "lot": 10,
            "minPriceIncrement": {"units": "0", "nano": 1000000},
            "apiTradeAvailableFlag": true
        })
    }

    fn timestamp(value: &str) -> OffsetDateTime {
        match OffsetDateTime::parse(value, &Rfc3339) {
            Ok(timestamp) => timestamp,
            Err(error) => panic!("unexpected timestamp error: {error}"),
        }
    }

    fn future_json(ticker: &str, activation: &str, expiration: &str) -> serde_json::Value {
        json!({
            "figi": format!("figi-{ticker}"),
            "ticker": ticker,
            "classCode": "SPBFUT",
            "uid": format!("uid-{ticker}"),
            "currency": "rub",
            "lot": 1,
            "minPriceIncrement": {"units": "0", "nano": 1000000},
            "apiTradeAvailableFlag": true,
            "basicAsset": "TEST",
            "basicAssetPositionUid": format!("underlying-{ticker}"),
            "assetType": "commodity",
            "firstTradeDate": activation,
            "expirationDate": expiration
        })
    }
}
