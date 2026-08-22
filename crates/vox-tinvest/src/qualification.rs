//! Narrow live qualification surface for issue #12. This is deliberately not
//! the reference-data catalogue owned by issue #7.

use std::fmt;

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
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub uid: String,
    pub currency: String,
    pub lot: i64,
    pub min_price_increment: Quotation,
    pub api_trade_available_flag: bool,
}

impl ShareInstrument {
    pub fn validate_rq1(&self) -> Result<(), QualificationDataError> {
        if self.ticker.is_empty()
            || self.class_code.is_empty()
            || self.uid.is_empty()
            || self.currency.is_empty()
        {
            return Err(QualificationDataError::InvalidShare(
                "required identity field is empty",
            ));
        }
        if self.lot <= 0 {
            return Err(QualificationDataError::InvalidShare("lot must be positive"));
        }
        if self.min_price_increment.fixed_point().total_nanos() <= 0 {
            return Err(QualificationDataError::InvalidShare(
                "minimum price increment must be positive",
            ));
        }
        if !self.api_trade_available_flag {
            return Err(QualificationDataError::InvalidShare(
                "API trading flag is false",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct FuturesResponse {
    #[serde(default)]
    pub instruments: Vec<FutureInstrument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FutureInstrument {
    pub figi: String,
    pub ticker: String,
    pub class_code: String,
    pub uid: String,
    pub currency: String,
    pub lot: i64,
    pub min_price_increment: Quotation,
    pub api_trade_available_flag: bool,
    pub basic_asset: String,
    pub asset_type: String,
    pub first_trade_date: String,
    pub expiration_date: String,
}

impl FutureInstrument {
    pub fn validate_rq1(&self) -> Result<(), QualificationDataError> {
        if self.ticker.is_empty()
            || self.class_code != "SPBFUT"
            || self.uid.is_empty()
            || self.currency.is_empty()
            || self.basic_asset.is_empty()
            || self.asset_type.is_empty()
            || self.first_trade_date.is_empty()
            || self.expiration_date.is_empty()
        {
            return Err(QualificationDataError::InvalidFuture(
                "required identity/economics field is empty or class is not SPBFUT",
            ));
        }
        if self.lot <= 0 || !self.api_trade_available_flag {
            return Err(QualificationDataError::InvalidFuture(
                "lot must be positive and API trading enabled",
            ));
        }
        Ok(())
    }

    pub fn exact_economics(
        &self,
        margin: &FuturesMarginResponse,
    ) -> Result<FuturesEconomics, QualificationDataError> {
        self.validate_rq1()?;
        FuturesEconomics::new(
            self.min_price_increment.fixed_point(),
            margin.min_price_increment.fixed_point(),
            margin.min_price_increment_amount.fixed_point(),
        )
        .map_err(QualificationDataError::FuturesEconomics)
    }
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

pub fn select_sber(response: &SharesResponse) -> Result<&ShareInstrument, QualificationDataError> {
    let mut candidates = response.instruments.iter().filter(|instrument| {
        instrument.ticker == "SBER"
            && instrument.class_code == "TQBR"
            && instrument.api_trade_available_flag
    });
    let candidate = candidates
        .next()
        .ok_or(QualificationDataError::SberNotFound)?;
    if candidates.next().is_some() {
        return Err(QualificationDataError::AmbiguousSber);
    }
    candidate.validate_rq1()?;
    Ok(candidate)
}

/// Selects nearest-expiry active tradeable SPBFUT from BASE response.
pub fn select_tradeable_future(
    response: &FuturesResponse,
) -> Result<&FutureInstrument, QualificationDataError> {
    select_tradeable_future_at(response, OffsetDateTime::now_utc())
}

fn select_tradeable_future_at(
    response: &FuturesResponse,
    now: OffsetDateTime,
) -> Result<&FutureInstrument, QualificationDataError> {
    let mut selected: Option<(&FutureInstrument, OffsetDateTime)> = None;
    for instrument in response.instruments.iter().filter(|instrument| {
        instrument.class_code == "SPBFUT" && instrument.api_trade_available_flag
    }) {
        instrument.validate_rq1()?;
        let activation =
            OffsetDateTime::parse(&instrument.first_trade_date, &Rfc3339).map_err(|_| {
                QualificationDataError::InvalidFuture("first trade date is not RFC3339")
            })?;
        let expiration = OffsetDateTime::parse(&instrument.expiration_date, &Rfc3339)
            .map_err(|_| QualificationDataError::InvalidFuture("expiration date is not RFC3339"))?;
        let earlier_than_selected =
            selected
                .as_ref()
                .is_none_or(|(current, current_expiration)| {
                    (expiration, &instrument.ticker) < (*current_expiration, &current.ticker)
                });
        if activation <= now && now < expiration && earlier_than_selected {
            selected = Some((instrument, expiration));
        }
    }
    let candidate = selected
        .map(|(instrument, _)| instrument)
        .ok_or(QualificationDataError::FutureNotFound)?;
    candidate.validate_rq1()?;
    Ok(candidate)
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
            "assetType": "commodity",
            "firstTradeDate": activation,
            "expirationDate": expiration
        })
    }
}
