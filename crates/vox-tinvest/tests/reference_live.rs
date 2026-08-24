use std::error::Error;

use vox_tinvest::reference::{
    AssetFundamentalsRequest, AssetsRequest, BondEventsRequest, CapabilityRegistry,
    ConsensusForecastsResponse, CriticalDataError, EmptyRequest, FavoriteGroupsRequest,
    FavoritesRequest, FindInstrumentRequest, IdRequest, InsiderDealsRequest, InstrumentExchange,
    InstrumentIdRequest, InstrumentRequest, InstrumentStatus, InstrumentsRequest, NewsRequest,
    OptionsByRequest, PageRequest, PagedRequest, PeriodRequest, ProviderCurrentDate,
    ProviderInstrumentType, RequiredPeriodRequest, RiskRatesRequest, Timestamp,
    TradingSchedulesError, TradingSchedulesRequest, TradingSchedulesResult, next_page,
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

fn by_uid(
    uid: &str,
) -> Result<InstrumentRequest<'_>, vox_tinvest::reference::RequestValidationError> {
    InstrumentRequest::by_uid(uid)
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

fn present(value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn unavailable(method: &'static str, reason: &str, registry: &mut CapabilityRegistry) {
    registry.mark_provider_data_unavailable(method);
    println!("UNAVAILABLE {method}: {reason}");
}

fn forecast_sample(response: &ConsensusForecastsResponse) -> Option<&str> {
    response
        .items
        .iter()
        .filter_map(|item| item.uid.as_deref())
        .find(|uid| !uid.trim().is_empty())
}

#[test]
fn forecast_sample_uses_provider_consensus_uid() -> Result<(), serde_json::Error> {
    let response: ConsensusForecastsResponse =
        serde_json::from_str(r#"{"items":[{}, {"uid":"provider-forecast-uid"}]}"#)?;
    assert_eq!(forecast_sample(&response), Some("provider-forecast-uid"));
    Ok(())
}

#[test]
fn empty_consensus_is_explicitly_unavailable() -> Result<(), serde_json::Error> {
    let response: ConsensusForecastsResponse = serde_json::from_str(r#"{"items":[]}"#)?;
    assert_eq!(forecast_sample(&response), None);
    Ok(())
}

#[test]
fn provider_sourced_forecast_404_is_not_capability_gated() {
    assert_eq!(
        vox_tinvest::reference::capability_state_for_http_status(404),
        None
    );
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
    let share = shares
        .instruments
        .iter()
        .find(|instrument| present(&instrument.uid) && present(&instrument.ticker))
        .ok_or("Shares has no row with UID and ticker")?;
    let share_uid = required(&share.uid, "share.uid")?;
    let share_ticker = required(&share.ticker, "share.ticker")?;
    qualified(
        "ShareBy",
        client.share_by(&by_uid(share_uid)?).await,
        &mut capabilities,
    )?;
    qualified(
        "GetInstrumentBy",
        client.get_instrument_by(&by_uid(share_uid)?).await,
        &mut capabilities,
    )?;
    qualified(
        "FindInstrument",
        client
            .find_instrument(
                &FindInstrumentRequest::new(share_ticker)?
                    .with_instrument_kind(&ProviderInstrumentType::Share),
            )
            .await,
        &mut capabilities,
    )?;

    let bonds = qualified("Bonds", client.bonds(&catalogue).await, &mut capabilities)?
        .ok_or("Bonds unexpectedly gated")?;
    let bond = bonds
        .instruments
        .iter()
        .find(|instrument| present(&instrument.uid))
        .ok_or("Bonds has no row with UID")?;
    let bond_uid = required(&bond.uid, "bond.uid")?;
    qualified(
        "BondBy",
        client.bond_by(&by_uid(bond_uid)?).await,
        &mut capabilities,
    )?;

    let etfs = qualified("Etfs", client.etfs(&catalogue).await, &mut capabilities)?
        .ok_or("Etfs unexpectedly gated")?;
    let etf = etfs
        .instruments
        .iter()
        .find(|instrument| present(&instrument.uid))
        .ok_or("Etfs has no row with UID")?;
    let etf_uid = required(&etf.uid, "etf.uid")?;
    qualified(
        "EtfBy",
        client.etf_by(&by_uid(etf_uid)?).await,
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
        .iter()
        .find(|instrument| present(&instrument.uid))
        .ok_or("Currencies has no row with UID")?;
    let currency_uid = required(&currency.uid, "currency.uid")?;
    qualified(
        "CurrencyBy",
        client.currency_by(&by_uid(currency_uid)?).await,
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
        .iter()
        .find(|instrument| present(&instrument.uid))
        .ok_or("Futures has no row with UID")?;
    let future_uid = required(&future.uid, "future.uid")?;
    qualified(
        "FutureBy",
        client.future_by(&by_uid(future_uid)?).await,
        &mut capabilities,
    )?;
    qualified(
        "GetFuturesMargin",
        client
            .get_futures_margin(&InstrumentIdRequest::new(future_uid)?)
            .await,
        &mut capabilities,
    )?;

    let notes = qualified(
        "StructuredNotes",
        client.structured_notes(&catalogue).await,
        &mut capabilities,
    )?;
    if let Some(note) = notes
        .as_ref()
        .and_then(|response| response.instruments.iter().find(|item| present(&item.uid)))
    {
        let note_uid = required(&note.uid, "structured_note.uid")?;
        qualified(
            "StructuredNoteBy",
            client.structured_note_by(&by_uid(note_uid)?).await,
            &mut capabilities,
        )?;
    } else {
        unavailable(
            "StructuredNoteBy",
            "no provider-supplied structured-note UID",
            &mut capabilities,
        );
    }
    let dfas = qualified(
        "Dfas",
        client.dfas(&EmptyRequest::default()).await,
        &mut capabilities,
    )?;
    if let Some(dfa) = dfas
        .as_ref()
        .and_then(|response| response.instruments.iter().find(|item| present(&item.uid)))
    {
        let dfa_uid = required(&dfa.uid, "dfa.uid")?;
        qualified(
            "DfaBy",
            client.dfa_by(&by_uid(dfa_uid)?).await,
            &mut capabilities,
        )?;
    } else {
        unavailable("DfaBy", "no provider-supplied DFA UID", &mut capabilities);
    }
    qualified(
        "Indicatives",
        client.indicatives(&EmptyRequest::default()).await,
        &mut capabilities,
    )?;

    if let Some(option_source) = shares
        .instruments
        .iter()
        .find(|instrument| present(&instrument.uid) && present(&instrument.asset_uid))
    {
        let option_source_uid = required(&option_source.uid, "option_source.uid")?;
        let basic_asset_uid = required(&option_source.asset_uid, "option_source.asset_uid")?;
        let options = qualified(
            "OptionsBy",
            client
                .options_by(
                    &OptionsByRequest::new(basic_asset_uid)?
                        .with_basic_asset_position_uid(option_source.position_uid.as_deref())
                        .with_basic_instrument_id(Some(option_source_uid)),
                )
                .await,
            &mut capabilities,
        )?;
        if let Some(option) = options
            .as_ref()
            .and_then(|response| response.instruments.iter().find(|item| present(&item.uid)))
        {
            let option_uid = required(&option.uid, "option.uid")?;
            qualified(
                "OptionBy",
                client.option_by(&by_uid(option_uid)?).await,
                &mut capabilities,
            )?;
        } else {
            unavailable(
                "OptionBy",
                "no provider-supplied option UID",
                &mut capabilities,
            );
        }
    } else {
        unavailable(
            "OptionsBy",
            "share catalogue has no authoritative basic asset UID",
            &mut capabilities,
        );
        unavailable(
            "OptionBy",
            "OptionsBy lacked authoritative basic asset UID",
            &mut capabilities,
        );
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
    let asset = assets
        .assets
        .iter()
        .find(|asset| present(&asset.uid))
        .ok_or("GetAssets has no row with UID")?;
    let asset_uid = required(&asset.uid, "asset.uid")?;
    qualified(
        "GetAssetBy",
        client.get_asset_by(&IdRequest::new(asset_uid)?).await,
        &mut capabilities,
    )?;

    let page = PageRequest::new(20, 0)?;
    let mut brands = qualified(
        "GetBrands",
        client
            .get_brands(&PagedRequest {
                paging: page.clone(),
            })
            .await,
        &mut capabilities,
    )?
    .ok_or("GetBrands unexpectedly gated")?;
    if let Some(next) = brands
        .paging
        .as_ref()
        .map(|paging| next_page(&page, paging))
        .transpose()?
        .flatten()
        && let Some(mut second) = qualified(
            "GetBrands",
            client.get_brands(&PagedRequest { paging: next }).await,
            &mut capabilities,
        )?
    {
        brands.brands.append(&mut second.brands);
    }
    if let Some(brand) = brands.brands.iter().find(|brand| present(&brand.uid)) {
        let brand_uid = required(&brand.uid, "brand.uid")?;
        qualified(
            "GetBrandBy",
            client.get_brand_by(&IdRequest::new(brand_uid)?).await,
            &mut capabilities,
        )?;
    } else {
        unavailable(
            "GetBrandBy",
            "no provider-supplied brand UID",
            &mut capabilities,
        );
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

    let share_period = PeriodRequest::new(share_uid, Some(&from), Some(&to))?;
    qualified(
        "GetDividends",
        client.get_dividends(&share_period).await,
        &mut capabilities,
    )?;
    let bond_period = PeriodRequest::new(bond_uid, Some(&from), Some(&to))?;
    qualified(
        "GetBondCoupons",
        client.get_bond_coupons(&bond_period).await,
        &mut capabilities,
    )?;
    qualified(
        "GetAccruedInterests",
        client
            .get_accrued_interests(&RequiredPeriodRequest::new(bond_uid, &from, &to)?)
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetBondEvents",
        client
            .get_bond_events(&BondEventsRequest::new(
                bond_uid,
                Some(&from),
                Some(&to),
                "EVENT_TYPE_UNSPECIFIED",
            )?)
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetRiskRates",
        client
            .get_risk_rates(&RiskRatesRequest::new(&[share_uid, future_uid])?)
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssetFundamentals",
        client
            .get_asset_fundamentals(&AssetFundamentalsRequest::new(&[asset_uid])?)
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssetReports",
        client.get_asset_reports(&share_period).await,
        &mut capabilities,
    )?;

    let consensus_page = PageRequest::new(20, 0)?;
    let mut consensus = qualified(
        "GetConsensusForecasts",
        client
            .get_consensus_forecasts(&PagedRequest {
                paging: consensus_page.clone(),
            })
            .await,
        &mut capabilities,
    )?;
    if let Some(first) = consensus.as_mut()
        && let Some(next) = first
            .page
            .as_ref()
            .map(|paging| next_page(&consensus_page, paging))
            .transpose()?
            .flatten()
        && let Some(mut second) = qualified(
            "GetConsensusForecasts",
            client
                .get_consensus_forecasts(&PagedRequest { paging: next })
                .await,
            &mut capabilities,
        )?
    {
        first.items.append(&mut second.items);
    }
    if let Some(forecast_id) = consensus.as_ref().and_then(forecast_sample) {
        qualified(
            "GetForecastBy",
            client
                .get_forecast_by(&InstrumentIdRequest::new(forecast_id)?)
                .await,
            &mut capabilities,
        )?;
    } else {
        unavailable(
            "GetForecastBy",
            "no provider-supplied consensus UID",
            &mut capabilities,
        );
    }
    let insider_request = InsiderDealsRequest::first(share_uid, 20)?;
    let insider = qualified(
        "GetInsiderDeals",
        client.get_insider_deals(&insider_request).await,
        &mut capabilities,
    )?;
    if let Some(cursor) = insider
        .as_ref()
        .and_then(|response| response.next_cursor.as_deref())
        .filter(|cursor| !cursor.trim().is_empty())
    {
        qualified(
            "GetInsiderDeals",
            client
                .get_insider_deals(&insider_request.after(cursor))
                .await,
            &mut capabilities,
        )?;
    }
    let news_request = NewsRequest::first(20)?;
    let news = qualified("News", client.news(&news_request).await, &mut capabilities)?;
    if let Some(cursor) = news.as_ref().and_then(|response| {
        response
            .has_next
            .unwrap_or(false)
            .then_some(response.next_cursor.as_ref())
            .flatten()
            .cloned()
    }) {
        qualified(
            "News",
            client.news(&news_request.after(cursor)).await,
            &mut capabilities,
        )?;
    }
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
