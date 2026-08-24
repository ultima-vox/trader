use std::error::Error;

use vox_tinvest::reference::{
    AssetFundamentalsRequest, AssetsRequest, BondEventsRequest, CapabilityRegistry,
    CriticalDataError, EmptyRequest, FavoriteGroupsRequest, FavoritesRequest,
    FindInstrumentRequest, IdRequest, InsiderDealsRequest, InstrumentExchange, InstrumentIdRequest,
    InstrumentIdType, InstrumentRequest, InstrumentStatus, InstrumentsRequest, NewsRequest,
    OptionsByRequest, PageRequest, PagedRequest, PeriodRequest, ProviderCurrentDate,
    ProviderInstrumentType, RiskRatesRequest, Timestamp, TradingSchedulesError,
    TradingSchedulesRequest, TradingSchedulesResult,
};
use vox_tinvest::{ProviderResponse, RestError, RestErrorKind, SecretToken, TInvestRestClient};

fn qualified<T>(
    method: &'static str,
    result: Result<ProviderResponse<T>, RestError>,
    registry: &mut CapabilityRegistry,
) -> Result<Option<T>, Box<dyn Error>> {
    match result {
        Ok(response) => {
            println!("QUALIFIED {method}");
            Ok(Some(response.into_body()))
        }
        Err(error) => {
            let gated = match error.kind() {
                RestErrorKind::Provider(provider) => {
                    registry.record_provider_http(method, provider.http_status())
                }
                _ => false,
            };
            if gated {
                println!("GATED {method} {:?}", registry.state(method));
                Ok(None)
            } else {
                Err(Box::new(error))
            }
        }
    }
}

fn qualified_trading_schedules(
    result: Result<TradingSchedulesResult, TradingSchedulesError>,
    registry: &mut CapabilityRegistry,
) -> Result<Option<TradingSchedulesResult>, Box<dyn Error>> {
    match result {
        Ok(response) => {
            println!("QUALIFIED TradingSchedules");
            Ok(Some(response))
        }
        Err(error) => {
            let gated = error
                .rest_error()
                .and_then(|error| match error.kind() {
                    RestErrorKind::Provider(provider) => Some(
                        registry.record_provider_http("TradingSchedules", provider.http_status()),
                    ),
                    _ => None,
                })
                .unwrap_or(false);
            if gated {
                println!(
                    "GATED TradingSchedules {:?}",
                    registry.state("TradingSchedules")
                );
                Ok(None)
            } else {
                Err(Box::new(error))
            }
        }
    }
}

fn by_uid(uid: &str) -> InstrumentRequest<'_> {
    InstrumentRequest {
        id_type: InstrumentIdType::InstrumentIdTypeUid,
        class_code: None,
        id: uid,
    }
}

fn required<'a>(
    value: &'a Option<String>,
    field: &'static str,
) -> Result<&'a str, CriticalDataError> {
    value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CriticalDataError::Missing(field))
}

/// Opt-in current-contract qualification. Every call is a safe read. Gated
/// analytics/account methods report capability state without hiding DTO errors.
#[tokio::test]
#[ignore = "requires live T-Invest token; performs safe reads only"]
async fn current_reference_surface_decodes_live_read_only() -> Result<(), Box<dyn Error>> {
    let token = SecretToken::new(std::env::var("TINVEST_TOKEN")?)?;
    let client = TInvestRestClient::production(token)?;
    let mut capabilities = CapabilityRegistry::default();
    let catalogue = InstrumentsRequest {
        instrument_status: InstrumentStatus::InstrumentStatusBase,
        instrument_exchange: InstrumentExchange::InstrumentExchangeUnspecified,
    };
    let from = Timestamp::parse("2025-01-01T00:00:00Z")?;
    let to = Timestamp::parse("2026-12-31T23:59:59Z")?;
    let provider_current_date = ProviderCurrentDate::now_utc();
    let schedule_from = provider_current_date.timestamp_after_days(1)?;
    let schedule_to = provider_current_date.timestamp_after_days(8)?;

    let shares = qualified("Shares", client.shares(&catalogue).await, &mut capabilities)?
        .ok_or("Shares unexpectedly gated")?;
    let share = shares.instruments.first().ok_or("empty Shares response")?;
    let share_uid = required(&share.uid, "share.uid")?;
    let share_ticker = required(&share.ticker, "share.ticker")?;
    qualified(
        "ShareBy",
        client.share_by(&by_uid(share_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "GetInstrumentBy",
        client.get_instrument_by(&by_uid(share_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "FindInstrument",
        client
            .find_instrument(&FindInstrumentRequest {
                query: share_ticker,
                instrument_kind: Some(&ProviderInstrumentType::Share),
                api_trade_available_flag: None,
            })
            .await,
        &mut capabilities,
    )?;

    let bonds = qualified("Bonds", client.bonds(&catalogue).await, &mut capabilities)?
        .ok_or("Bonds unexpectedly gated")?;
    let bond = bonds.instruments.first().ok_or("empty Bonds response")?;
    let bond_uid = required(&bond.uid, "bond.uid")?;
    qualified(
        "BondBy",
        client.bond_by(&by_uid(bond_uid)).await,
        &mut capabilities,
    )?;

    let etfs = qualified("Etfs", client.etfs(&catalogue).await, &mut capabilities)?
        .ok_or("Etfs unexpectedly gated")?;
    let etf = etfs.instruments.first().ok_or("empty Etfs response")?;
    let etf_uid = required(&etf.uid, "etf.uid")?;
    qualified(
        "EtfBy",
        client.etf_by(&by_uid(etf_uid)).await,
        &mut capabilities,
    )?;

    let currencies = qualified(
        "Currencies",
        client.currencies(&catalogue).await,
        &mut capabilities,
    )?
    .ok_or("Currencies unexpectedly gated")?;
    let currency = currencies
        .instruments
        .first()
        .ok_or("empty Currencies response")?;
    let currency_uid = required(&currency.uid, "currency.uid")?;
    qualified(
        "CurrencyBy",
        client.currency_by(&by_uid(currency_uid)).await,
        &mut capabilities,
    )?;

    let futures = qualified(
        "Futures",
        client.futures(&catalogue).await,
        &mut capabilities,
    )?
    .ok_or("Futures unexpectedly gated")?;
    let future = futures
        .instruments
        .first()
        .ok_or("empty Futures response")?;
    let future_uid = required(&future.uid, "future.uid")?;
    qualified(
        "FutureBy",
        client.future_by(&by_uid(future_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "GetFuturesMargin",
        client
            .get_futures_margin(&InstrumentIdRequest {
                instrument_id: future_uid,
            })
            .await,
        &mut capabilities,
    )?;

    if let Some(notes) = qualified(
        "StructuredNotes",
        client.structured_notes(&catalogue).await,
        &mut capabilities,
    )? && let Some(note) = notes.instruments.first()
    {
        let note_uid = required(&note.uid, "structured_note.uid")?;
        qualified(
            "StructuredNoteBy",
            client.structured_note_by(&by_uid(note_uid)).await,
            &mut capabilities,
        )?;
    }
    if let Some(dfas) = qualified(
        "Dfas",
        client.dfas(&EmptyRequest::default()).await,
        &mut capabilities,
    )? && let Some(dfa) = dfas.instruments.first()
    {
        let dfa_uid = required(&dfa.uid, "dfa.uid")?;
        qualified(
            "DfaBy",
            client.dfa_by(&by_uid(dfa_uid)).await,
            &mut capabilities,
        )?;
    }
    qualified(
        "Indicatives",
        client.indicatives(&EmptyRequest::default()).await,
        &mut capabilities,
    )?;

    let basic_asset_uid = share.asset_uid.as_deref().unwrap_or(share_uid);
    if let Some(options) = qualified(
        "OptionsBy",
        client
            .options_by(&OptionsByRequest {
                basic_asset_uid,
                basic_asset_position_uid: share.position_uid.as_deref(),
                basic_instrument_id: Some(share_uid),
            })
            .await,
        &mut capabilities,
    )? && let Some(option) = options.instruments.first()
    {
        let option_uid = required(&option.uid, "option.uid")?;
        qualified(
            "OptionBy",
            client.option_by(&by_uid(option_uid)).await,
            &mut capabilities,
        )?;
    }

    let assets = qualified(
        "GetAssets",
        client
            .get_assets(&AssetsRequest {
                instrument_type: None,
                instrument_status: Some("INSTRUMENT_STATUS_BASE"),
            })
            .await,
        &mut capabilities,
    )?
    .ok_or("GetAssets unexpectedly gated")?;
    let asset = assets.assets.first().ok_or("empty GetAssets response")?;
    let asset_uid = required(&asset.uid, "asset.uid")?;
    qualified(
        "GetAssetBy",
        client.get_asset_by(&IdRequest { id: asset_uid }).await,
        &mut capabilities,
    )?;

    let page = PageRequest::new(20, 0)?;
    let brands = qualified(
        "GetBrands",
        client
            .get_brands(&PagedRequest {
                paging: page.clone(),
            })
            .await,
        &mut capabilities,
    )?
    .ok_or("GetBrands unexpectedly gated")?;
    if let Some(brand) = brands.brands.first() {
        let brand_uid = required(&brand.uid, "brand.uid")?;
        qualified(
            "GetBrandBy",
            client.get_brand_by(&IdRequest { id: brand_uid }).await,
            &mut capabilities,
        )?;
    }
    qualified(
        "GetCountries",
        client.get_countries(&EmptyRequest::default()).await,
        &mut capabilities,
    )?;
    qualified_trading_schedules(
        client
            .trading_schedules(&TradingSchedulesRequest {
                exchange: None,
                from: &schedule_from,
                to: &schedule_to,
            })
            .await,
        &mut capabilities,
    )?;

    let share_period = PeriodRequest {
        figi: None,
        instrument_id: share_uid,
        from: &from,
        to: &to,
    };
    qualified(
        "GetDividends",
        client.get_dividends(&share_period).await,
        &mut capabilities,
    )?;
    let bond_period = PeriodRequest {
        figi: None,
        instrument_id: bond_uid,
        from: &from,
        to: &to,
    };
    qualified(
        "GetBondCoupons",
        client.get_bond_coupons(&bond_period).await,
        &mut capabilities,
    )?;
    qualified(
        "GetAccruedInterests",
        client.get_accrued_interests(&bond_period).await,
        &mut capabilities,
    )?;
    qualified(
        "GetBondEvents",
        client
            .get_bond_events(&BondEventsRequest {
                instrument_id: bond_uid,
                from: &from,
                to: &to,
                event_type: "EVENT_TYPE_UNSPECIFIED",
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetRiskRates",
        client
            .get_risk_rates(&RiskRatesRequest {
                instrument_id: &[share_uid, future_uid],
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssetFundamentals",
        client
            .get_asset_fundamentals(&AssetFundamentalsRequest {
                assets: &[asset_uid],
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssetReports",
        client.get_asset_reports(&share_period).await,
        &mut capabilities,
    )?;

    let consensus = qualified(
        "GetConsensusForecasts",
        client
            .get_consensus_forecasts(&PagedRequest { paging: page })
            .await,
        &mut capabilities,
    )?;
    let forecast_id = consensus
        .as_ref()
        .and_then(|response| response.items.first())
        .and_then(|item| item.uid.as_deref())
        .unwrap_or(share_uid);
    qualified(
        "GetForecastBy",
        client
            .get_forecast_by(&InstrumentIdRequest {
                instrument_id: forecast_id,
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetInsiderDeals",
        client
            .get_insider_deals(&InsiderDealsRequest {
                instrument_id: share_uid,
                limit: 20,
                next_cursor: None,
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "News",
        client.news(&NewsRequest::first(20)?).await,
        &mut capabilities,
    )?;
    qualified(
        "GetFavorites",
        client
            .get_favorites(&FavoritesRequest { group_id: None })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetFavoriteGroups",
        client
            .get_favorite_groups(&FavoriteGroupsRequest {
                instrument_id: &[share_uid],
                excluded_group_id: &[],
            })
            .await,
        &mut capabilities,
    )?;

    Ok(())
}
