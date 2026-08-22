use std::time::Duration;

use anyhow::{Context, bail};
use vox_tinvest::qualification::{
    MarketDataStreamMessage, rq2_market_data_subscriptions, select_sber,
};
use vox_tinvest::{
    ReconnectPolicy, SecretToken, StreamEvent, StreamHandle, SubscriptionRegistry,
    TInvestRestClient, TInvestWebSocket,
};

const EVENT_CAPACITY: usize = 64;
const PHASE_TIMEOUT: Duration = Duration::from_secs(45);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let raw_token =
        std::env::var("TINVEST_TOKEN").context("TINVEST_TOKEN is required for live RQ2")?;
    let token = SecretToken::new(raw_token)?;
    let rest = TInvestRestClient::production(token.clone())?;
    let shares = rest.qualification_shares().await?.into_body();
    let share = select_sber(&shares)?;
    let snapshot = rest
        .qualification_order_book(&share.uid, 10)
        .await?
        .into_body();
    snapshot.validate_rq2(&share.uid)?;

    let registry = SubscriptionRegistry::new(8)?;
    for subscription in rq2_market_data_subscriptions(&share.uid)? {
        registry.upsert(subscription).await?;
    }
    let reconnect =
        ReconnectPolicy::new(5, Duration::from_millis(250), Duration::from_secs(5), 2_000)?;
    let websocket = TInvestWebSocket::production(token)?;
    let mut stream = websocket
        .stream_supervisor(registry, reconnect, EVENT_CAPACITY)?
        .start()?;
    if stream.bounded_event_capacity() != EVENT_CAPACITY {
        bail!("stream event channel capacity is not bounded as configured");
    }

    let first_generation = wait_for_ready_market_event(&mut stream, 1, &share.uid).await?;
    stream.control().force_reconnect().await?;
    let second_generation =
        wait_for_ready_market_event(&mut stream, first_generation + 1, &share.uid).await?;
    stream.shutdown().await?;

    println!("RQ2 LIVE QUALIFICATION");
    println!("SBER: uid={} lot={}", share.uid, share.lot);
    println!(
        "REST: order_book_depth={} bids={} asks={}",
        snapshot.depth,
        snapshot.bids.len(),
        snapshot.asks.len()
    );
    println!(
        "PASS: generation {first_generation} subscribed; forced reconnect reached generation {second_generation}"
    );
    println!("PASS: post-reconnect market event received through bounded channel");
    Ok(())
}

async fn wait_for_ready_market_event(
    stream: &mut StreamHandle,
    minimum_generation: u64,
    expected_uid: &str,
) -> anyhow::Result<u64> {
    tokio::time::timeout(PHASE_TIMEOUT, async {
        let mut generation = 0;
        let mut ready = false;
        let mut market_event = false;
        loop {
            let event = stream
                .recv()
                .await
                .context("stream supervisor stopped before qualification completed")?;
            match event {
                StreamEvent::Connected { connection, .. }
                    if connection.generation >= minimum_generation =>
                {
                    generation = connection.generation;
                    ready = false;
                    market_event = false;
                }
                StreamEvent::SubscriptionsReady { connection }
                    if connection.generation == generation =>
                {
                    ready = true;
                }
                StreamEvent::Message {
                    connection,
                    message,
                } if connection.generation == generation => {
                    let decoded: MarketDataStreamMessage =
                        message.decode_qualification_market_data()?;
                    decoded.validate_acknowledgement_uids(expected_uid)?;
                    market_event |= validate_market_event(&decoded, expected_uid)?;
                }
                StreamEvent::Stopped { reason } => {
                    bail!("stream supervisor stopped early: {reason:?}");
                }
                _ => {}
            }
            if generation >= minimum_generation && ready && market_event {
                return Ok(generation);
            }
        }
    })
    .await
    .context("timed out waiting for subscriptions and market event")?
}

fn validate_market_event(
    message: &MarketDataStreamMessage,
    expected_uid: &str,
) -> anyhow::Result<bool> {
    if let Some(trade) = &message.trade {
        if trade.instrument_uid != expected_uid
            || trade.quantity <= 0
            || trade.price.fixed_point().total_nanos() <= 0
            || trade.time.is_empty()
        {
            bail!("invalid typed trade event");
        }
        return Ok(true);
    }
    if let Some(book) = &message.orderbook {
        if book.instrument_uid != expected_uid || !book.is_consistent || book.time.is_empty() {
            bail!("non-authoritative or untimestamped order book event");
        }
        return Ok(true);
    }
    if let Some(status) = &message.trading_status {
        if status.instrument_uid != expected_uid || status.time.is_empty() {
            bail!("untimestamped trading status event");
        }
        return Ok(true);
    }
    if let Some(last_price) = &message.last_price {
        if last_price.instrument_uid != expected_uid
            || last_price.price.fixed_point().total_nanos() <= 0
            || last_price.time.is_empty()
        {
            bail!("invalid typed last-price event");
        }
        return Ok(true);
    }
    Ok(false)
}
