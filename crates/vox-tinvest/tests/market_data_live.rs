use std::collections::BTreeSet;
use std::error::Error;
use std::time::Duration;

use prost_types::Timestamp;
use vox_tinvest::generated::v1;
use vox_tinvest::market_data::{
    CanonicalCandle, CanonicalClosePrice, CanonicalLastPrice, CanonicalMarketValueInstrument,
    CanonicalTechAnalysisValue, CanonicalTrade, CanonicalTradingStatusFact,
    CanonicalUnaryOrderBook, MarketSubscription, MarketSubscriptionRegistry, SubscriptionCommand,
    SubscriptionKind, get_my_subscriptions_request,
};
use vox_tinvest::reference::catalogue_request;
use vox_tinvest::{GrpcCredential, SecretToken, TInvestGrpcClient};

fn period() -> (Timestamp, Timestamp) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    (
        Timestamp {
            seconds: now - 3_600,
            nanos: 0,
        },
        Timestamp {
            seconds: now,
            nanos: 0,
        },
    )
}

#[tokio::test]
#[ignore = "requires TINVEST_TOKEN; complete market-data unary+stream qualification"]
async fn complete_market_data_surface_qualifies_over_generated_grpc() -> Result<(), Box<dyn Error>>
{
    let token = SecretToken::new(std::env::var("TINVEST_TOKEN")?)?;
    let client = TInvestGrpcClient::production(GrpcCredential::Production(token))?;
    let instrument_uid = resolve_market_data_uid(&client).await?;
    let (from, to) = period();

    let candles = client
        .get_candles(v1::GetCandlesRequest {
            from: Some(from),
            to: Some(to),
            interval: v1::CandleInterval::CandleInterval1Min as i32,
            instrument_id: Some(instrument_uid.clone()),
            candle_source_type: Some(v1::get_candles_request::CandleSource::Exchange as i32),
            // Keep explicit exchange-source qualification. T-Invest error 30220 forbids sending
            // candle_source_type together with limit; the one-hour range bounds this response.
            limit: None,
            ..Default::default()
        })
        .await?
        .body;
    for candle in candles.candles {
        CanonicalCandle::from_historic(
            candle,
            instrument_uid.clone(),
            v1::CandleInterval::CandleInterval1Min as i32,
        )?;
    }
    println!("QUALIFIED GetCandles");

    let last_prices = client
        .get_last_prices(v1::GetLastPricesRequest {
            instrument_id: vec![instrument_uid.clone()],
            last_price_type: v1::LastPriceType::LastPriceExchange as i32,
            ..Default::default()
        })
        .await?
        .body;
    for value in last_prices.last_prices {
        CanonicalLastPrice::try_from(value)?;
    }
    println!("QUALIFIED GetLastPrices");

    CanonicalUnaryOrderBook::try_from(
        client
            .get_order_book(v1::GetOrderBookRequest {
                depth: 10,
                instrument_id: Some(instrument_uid.clone()),
                ..Default::default()
            })
            .await?
            .body,
    )?;
    println!("QUALIFIED GetOrderBook");

    CanonicalTradingStatusFact::try_from(
        client
            .get_trading_status(v1::GetTradingStatusRequest {
                instrument_id: Some(instrument_uid.clone()),
                ..Default::default()
            })
            .await?
            .body,
    )?;
    println!("QUALIFIED GetTradingStatus");

    let statuses = client
        .get_trading_statuses(v1::GetTradingStatusesRequest {
            instrument_id: vec![instrument_uid.clone()],
        })
        .await?
        .body;
    for status in statuses.trading_statuses {
        CanonicalTradingStatusFact::try_from(status)?;
    }
    println!("QUALIFIED GetTradingStatuses");

    let trades = client
        .get_last_trades(v1::GetLastTradesRequest {
            from: Some(from),
            to: Some(to),
            instrument_id: Some(instrument_uid.clone()),
            trade_source: v1::TradeSourceType::TradeSourceAll as i32,
            ..Default::default()
        })
        .await?
        .body;
    for trade in trades.trades {
        CanonicalTrade::try_from(trade)?;
    }
    println!("QUALIFIED GetLastTrades");

    let closes = client
        .get_close_prices(v1::GetClosePricesRequest {
            instruments: vec![v1::InstrumentClosePriceRequest {
                instrument_id: instrument_uid.clone(),
            }],
            instrument_status: None,
        })
        .await?
        .body;
    for close in closes.close_prices {
        CanonicalClosePrice::try_from(close)?;
    }
    println!("QUALIFIED GetClosePrices");

    let analysis = client
        .get_tech_analysis(v1::GetTechAnalysisRequest {
            indicator_type: v1::get_tech_analysis_request::IndicatorType::Sma as i32,
            instrument_uid: instrument_uid.clone(),
            from: Some(from),
            to: Some(to),
            interval: v1::get_tech_analysis_request::IndicatorInterval::OneMinute as i32,
            type_of_price: v1::get_tech_analysis_request::TypeOfPrice::Close as i32,
            length: 10,
            deviation: None,
            smoothing: None,
        })
        .await?
        .body;
    for value in analysis.technical_indicators {
        CanonicalTechAnalysisValue::try_from(value)?;
    }
    println!("QUALIFIED GetTechAnalysis");

    let values = client
        .get_market_values(v1::GetMarketValuesRequest {
            instrument_id: vec![instrument_uid.clone()],
            values: vec![v1::MarketValueType::InstrumentValueLastPrice as i32],
        })
        .await?
        .body;
    for value in values.instruments {
        CanonicalMarketValueInstrument::try_from(value)?;
    }
    println!("QUALIFIED GetMarketValues");

    qualify_stream(&client, &instrument_uid).await?;
    println!("QUALIFIED MarketDataStream");
    Ok(())
}

async fn qualify_stream(
    client: &TInvestGrpcClient,
    instrument_uid: &str,
) -> Result<(), Box<dyn Error>> {
    let mut registry = MarketSubscriptionRegistry::default();
    for kind in [
        SubscriptionKind::Candle {
            interval: v1::SubscriptionInterval::OneMinute as i32,
            waiting_close: true,
            source: Some(v1::get_candles_request::CandleSource::Exchange as i32),
        },
        SubscriptionKind::OrderBook {
            depth: 10,
            order_book_type: v1::OrderBookType::OrderbookTypeAll as i32,
        },
        SubscriptionKind::Trade {
            source: v1::TradeSourceType::TradeSourceAll as i32,
            with_open_interest: false,
        },
        SubscriptionKind::Info,
        SubscriptionKind::LastPrice,
    ] {
        registry.insert(MarketSubscription {
            instrument_id: instrument_uid.to_owned(),
            kind,
        })?;
    }
    let subscribe_requests = registry.subscribe_requests();
    let mut initial_requests = Vec::with_capacity(subscribe_requests.len() + 1);
    initial_requests.push(v1::MarketDataRequest {
        payload: Some(v1::market_data_request::Payload::PingSettings(
            v1::PingDelaySettings {
                ping_delay_ms: Some(120_000),
            },
        )),
    });
    initial_requests.extend(subscribe_requests);
    let mut stream = client.open_market_data_stream(16, initial_requests).await?;
    tokio::time::timeout(Duration::from_secs(45), async {
        while !registry.all_confirmed() {
            let response = stream.message().await?.ok_or("provider closed stream")?;
            for acknowledgement in
                registry.apply_command_response(&response, SubscriptionCommand::Subscribe)?
            {
                println!(
                    "ACK family={:?} instrument_uid={} status={} tracking_id={}",
                    acknowledgement.family,
                    acknowledgement.instrument_uid,
                    acknowledgement.provider_status,
                    acknowledgement.tracking_id
                );
            }
        }
        Ok::<(), Box<dyn Error>>(())
    })
    .await??;

    stream.send(get_my_subscriptions_request()).await?;
    let expected = registry.desired().cloned().collect::<BTreeSet<_>>();
    let mut broker_confirmed = BTreeSet::new();
    tokio::time::timeout(Duration::from_secs(45), async {
        while !expected.is_subset(&broker_confirmed) {
            let response = stream.message().await?.ok_or("provider closed stream")?;
            for acknowledgement in registry.parse_active_snapshot_response(&response)? {
                println!(
                    "ACTIVE family={:?} instrument_uid={} status={} tracking_id={}",
                    acknowledgement.family,
                    acknowledgement.instrument_uid,
                    acknowledgement.provider_status,
                    acknowledgement.tracking_id
                );
                broker_confirmed.insert(acknowledgement.subscription);
            }
        }
        Ok::<(), Box<dyn Error>>(())
    })
    .await??;
    Ok(())
}

async fn resolve_market_data_uid(client: &TInvestGrpcClient) -> Result<String, Box<dyn Error>> {
    if let Ok(instrument_uid) = std::env::var("TINVEST_MARKET_DATA_UID")
        && !instrument_uid.trim().is_empty()
    {
        return Ok(instrument_uid);
    }
    let mut candidates = client
        .shares(catalogue_request())
        .await?
        .body
        .instruments
        .into_iter()
        .filter(|share| {
            !share.uid.is_empty()
                && share.api_trade_available_flag
                && share.buy_available_flag
                && share.sell_available_flag
                && share.liquidity_flag
                && !share.blocked_tca_flag
                && !share.for_qual_investor_flag
                && share.first_1min_candle_date.is_some()
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|share| (share.ticker != "SBER", share.uid.clone()));
    let selected = candidates.into_iter().next().ok_or(
        "reference data contains no liquid API-tradable share for market-data qualification",
    )?;
    println!(
        "QUALIFICATION INSTRUMENT ticker={} class_code={} uid={}",
        selected.ticker, selected.class_code, selected.uid
    );
    Ok(selected.uid)
}
