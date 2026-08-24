use anyhow::{Context, bail};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use vox_domain::InstrumentIdentity;
use vox_nautilus::{
    EquitySpec, FutureAssetClass, FutureSpec, InstrumentSpec, to_nautilus_equity,
    to_nautilus_future,
};
use vox_tinvest::qualification::{select_sber, select_tradeable_future};
use vox_tinvest::{SecretToken, TInvestRestClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = SecretToken::new(
        std::env::var("TINVEST_TOKEN").context("TINVEST_TOKEN is required for live RQ1")?,
    )?;
    let client = TInvestRestClient::production(token)?;

    let shares = client.qualification_shares().await?.into_body();
    let futures = client.qualification_futures().await?.into_body();
    let share = select_sber(&shares)?;
    let future = select_tradeable_future(&futures)?;
    let margin = client
        .qualification_futures_margin(future.uid)
        .await?
        .into_body();
    let economics = future.exact_economics(&margin)?;

    let share_lot = u64::try_from(share.lot).context("SBER lot must fit u64")?;
    let mapped_share = to_nautilus_equity(&EquitySpec {
        instrument: InstrumentSpec {
            identity: InstrumentIdentity::new(
                "tinvest",
                share.uid,
                Some(share.figi.to_owned()),
                share.ticker,
                share.class_code,
            )?,
            instrument_id: format!("{}-{}.TINVEST", share.ticker, share.class_code),
            raw_symbol: share.ticker.to_owned(),
            currency: share.currency.to_ascii_uppercase(),
            lot_size: share_lot,
            price_increment: share.min_price_increment.fixed_point(),
            ts_event_ns: 0,
            ts_init_ns: 0,
        },
    })?;

    let activation_ns = parse_timestamp_ns(future.first_trade_date)?;
    let expiration_ns = parse_timestamp_ns(future.expiration_date)?;
    let future_lot = u64::try_from(future.lot).context("future lot must fit u64")?;
    let mapped_future = to_nautilus_future(&FutureSpec {
        instrument: InstrumentSpec {
            identity: InstrumentIdentity::new(
                "tinvest",
                future.uid,
                Some(future.figi.to_owned()),
                future.ticker,
                future.class_code,
            )?,
            instrument_id: format!("{}-{}.TINVEST", future.ticker, future.class_code),
            raw_symbol: future.ticker.to_owned(),
            currency: future.currency.to_ascii_uppercase(),
            lot_size: future_lot,
            price_increment: future.min_price_increment.fixed_point(),
            ts_event_ns: 0,
            ts_init_ns: 0,
        },
        asset_class: map_asset_class(future.asset_type)?,
        exchange: None,
        underlying_id: future.underlying_id.to_string(),
        provider_underlying_name: future.basic_asset.map(str::to_owned),
        activation_ns,
        expiration_ns,
        economics,
    })?;

    if mapped_share.identity.uid() != share.uid
        || mapped_share.identity.figi() != Some(share.figi)
        || mapped_share.identity.class_code() != share.class_code
        || mapped_share.instrument.currency.to_string() != share.currency.to_ascii_uppercase()
        || mapped_share
            .instrument
            .lot_size
            .as_ref()
            .map(ToString::to_string)
            != Some(share_lot.to_string())
    {
        bail!("SBER source-to-runtime identity/economics round-trip mismatch");
    }
    if mapped_future.identity.uid() != future.uid
        || mapped_future.identity.figi() != Some(future.figi)
        || mapped_future.identity.class_code() != future.class_code
        || mapped_future.instrument.currency.to_string() != future.currency.to_ascii_uppercase()
        || mapped_future.instrument.lot_size.to_string() != future_lot.to_string()
        || mapped_future.instrument.multiplier.to_string()
            != mapped_future.money_per_point.to_string()
        || mapped_future.instrument.underlying != future.underlying_id.as_ref()
        || mapped_future.provider_underlying_name.as_deref() != future.basic_asset
    {
        bail!("future source-to-runtime identity/economics round-trip mismatch");
    }

    println!("RQ1 LIVE QUALIFICATION");
    println!(
        "SBER: id={} uid={} figi={} class={} currency={} lot={} tick_nanos={} precision={}",
        mapped_share.instrument.id,
        mapped_share.identity.uid(),
        mapped_share.identity.figi().unwrap_or("<missing>"),
        mapped_share.identity.class_code(),
        mapped_share.instrument.currency,
        share.lot,
        share.min_price_increment.fixed_point().total_nanos(),
        mapped_share.instrument.price_precision
    );
    println!(
        "FUTURE: id={} uid={} figi={} class={} currency={} asset_type={} provider_underlying={} underlying_id={} lot={} activation_ns={} expiration_ns={} tick_nanos={} tick_amount_nanos={} money_per_point={} multiplier={} initial_margin_buy_nanos={} initial_margin_sell_nanos={}",
        mapped_future.instrument.id,
        mapped_future.identity.uid(),
        mapped_future.identity.figi().unwrap_or("<missing>"),
        mapped_future.identity.class_code(),
        mapped_future.instrument.currency,
        future.asset_type,
        mapped_future
            .provider_underlying_name
            .as_deref()
            .unwrap_or("<missing>"),
        mapped_future.instrument.underlying,
        mapped_future.instrument.lot_size,
        activation_ns,
        expiration_ns,
        economics.min_price_increment().total_nanos(),
        economics.min_price_increment_amount().total_nanos(),
        mapped_future.money_per_point,
        mapped_future.instrument.multiplier,
        margin.initial_margin_on_buy.fixed_point().total_nanos(),
        margin.initial_margin_on_sell.fixed_point().total_nanos()
    );
    println!("PASS: live T-Invest RQ1 mapped into Nautilus without approximation");
    Ok(())
}

fn parse_timestamp_ns(value: &str) -> anyhow::Result<u64> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339)
        .with_context(|| format!("invalid provider timestamp {value}"))?
        .unix_timestamp_nanos();
    u64::try_from(timestamp).context("provider timestamp must be after Unix epoch and fit u64")
}

fn map_asset_class(value: &str) -> anyhow::Result<FutureAssetClass> {
    let normalized = value.trim().to_ascii_uppercase();
    match normalized.trim_start_matches("TYPE_") {
        "CURRENCY" => Ok(FutureAssetClass::Fx),
        "SECURITY" => Ok(FutureAssetClass::Equity),
        "COMMODITY" => Ok(FutureAssetClass::Commodity),
        "INDEX" => Ok(FutureAssetClass::Index),
        unsupported => bail!("unsupported T-Invest future asset type {unsupported}"),
    }
}
