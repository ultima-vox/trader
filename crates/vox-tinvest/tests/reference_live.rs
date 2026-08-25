use std::error::Error;

use prost_types::Timestamp;
use tonic::Code;
use vox_tinvest::generated::v1;
use vox_tinvest::reference::{
    AssetUid, CapabilityRegistry, ForecastProviderInconsistency, InstrumentUid, catalogue_request,
    consensus_asset_uids, forecast_instrument_candidates, fundamentals_request,
    insider_deals_request, instrument_by_uid,
};
use vox_tinvest::{
    GrpcCredential, GrpcError, GrpcErrorKind, GrpcResponse, SecretToken, TInvestGrpcClient,
};

fn qualified<T>(
    method: &'static str,
    result: Result<GrpcResponse<T>, GrpcError>,
    registry: &mut CapabilityRegistry,
) -> Result<Option<T>, Box<dyn Error>> {
    match result {
        Ok(response) => {
            println!("QUALIFIED {method}");
            Ok(Some(response.body))
        }
        Err(error) => {
            let gated = match &error.kind {
                GrpcErrorKind::Provider(provider) => {
                    registry.record_provider_code(method, provider.code)
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

fn uid(value: &str) -> Result<InstrumentUid, Box<dyn Error>> {
    Ok(InstrumentUid::new(value.to_owned())?)
}

fn period() -> (Timestamp, Timestamp) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    (
        Timestamp {
            seconds: now,
            nanos: 0,
        },
        Timestamp {
            seconds: now + 366 * 86_400,
            nanos: 0,
        },
    )
}

#[tokio::test]
#[ignore = "requires TINVEST_TOKEN; complete generated-contract safe-read qualification"]
async fn current_reference_surface_qualifies_over_grpc() -> Result<(), Box<dyn Error>> {
    let token = SecretToken::new(std::env::var("TINVEST_TOKEN")?)?;
    let client = TInvestGrpcClient::production(GrpcCredential::Production(token))?;
    let mut capabilities = CapabilityRegistry::default();
    let catalogue = catalogue_request();
    let (from, to) = period();

    let shares = qualified("Shares", client.shares(catalogue).await, &mut capabilities)?
        .ok_or("Shares unexpectedly gated")?;
    let share = shares
        .instruments
        .iter()
        .find(|item| !item.uid.is_empty() && !item.asset_uid.is_empty())
        .ok_or("Shares has no authoritative instrument+asset identity")?;
    let share_uid = uid(&share.uid)?;
    let share_asset_uid = AssetUid::new(share.asset_uid.clone())?;
    qualified(
        "ShareBy",
        client.share_by(instrument_by_uid(&share_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "GetInstrumentBy",
        client
            .get_instrument_by(instrument_by_uid(&share_uid))
            .await,
        &mut capabilities,
    )?;
    qualified(
        "FindInstrument",
        client
            .find_instrument(v1::FindInstrumentRequest {
                query: share.ticker.clone(),
                instrument_kind: Some(v1::InstrumentType::Share as i32),
                api_trade_available_flag: None,
            })
            .await,
        &mut capabilities,
    )?;

    let bonds = qualified("Bonds", client.bonds(catalogue).await, &mut capabilities)?
        .ok_or("Bonds unexpectedly gated")?;
    let bond_uid = uid(&bonds
        .instruments
        .iter()
        .find(|item| !item.uid.is_empty())
        .ok_or("Bonds has no UID")?
        .uid)?;
    qualified(
        "BondBy",
        client.bond_by(instrument_by_uid(&bond_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "GetBondCoupons",
        client
            .get_bond_coupons(v1::GetBondCouponsRequest {
                instrument_id: bond_uid.as_str().to_owned(),
                from: Some(from),
                to: Some(to),
                ..Default::default()
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetBondEvents",
        client
            .get_bond_events(v1::GetBondEventsRequest {
                instrument_id: bond_uid.as_str().to_owned(),
                from: Some(from),
                to: Some(to),
                r#type: 0,
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAccruedInterests",
        client
            .get_accrued_interests(v1::GetAccruedInterestsRequest {
                instrument_id: bond_uid.as_str().to_owned(),
                from: Some(from),
                to: Some(to),
                ..Default::default()
            })
            .await,
        &mut capabilities,
    )?;

    let currencies = qualified(
        "Currencies",
        client.currencies(catalogue).await,
        &mut capabilities,
    )?
    .ok_or("Currencies unexpectedly gated")?;
    if let Some(item) = currencies
        .instruments
        .iter()
        .find(|item| !item.uid.is_empty())
    {
        qualified(
            "CurrencyBy",
            client
                .currency_by(instrument_by_uid(&uid(&item.uid)?))
                .await,
            &mut capabilities,
        )?;
    }

    let etfs = qualified("Etfs", client.etfs(catalogue).await, &mut capabilities)?
        .ok_or("Etfs unexpectedly gated")?;
    if let Some(item) = etfs.instruments.iter().find(|item| !item.uid.is_empty()) {
        qualified(
            "EtfBy",
            client.etf_by(instrument_by_uid(&uid(&item.uid)?)).await,
            &mut capabilities,
        )?;
    }

    let futures = qualified(
        "Futures",
        client.futures(catalogue).await,
        &mut capabilities,
    )?
    .ok_or("Futures unexpectedly gated")?;
    let future_uid = uid(&futures
        .instruments
        .iter()
        .find(|item| !item.uid.is_empty())
        .ok_or("Futures has no UID")?
        .uid)?;
    qualified(
        "FutureBy",
        client.future_by(instrument_by_uid(&future_uid)).await,
        &mut capabilities,
    )?;
    qualified(
        "GetFuturesMargin",
        client
            .get_futures_margin(v1::GetFuturesMarginRequest {
                instrument_id: future_uid.as_str().to_owned(),
                ..Default::default()
            })
            .await,
        &mut capabilities,
    )?;

    let options = qualified(
        "OptionsBy",
        client
            .options_by(v1::FilterOptionsRequest {
                basic_asset_uid: Some(share_asset_uid.as_str().to_owned()),
                basic_asset_position_uid: None,
                basic_instrument_id: None,
            })
            .await,
        &mut capabilities,
    )?;
    if let Some(option) = options
        .as_ref()
        .and_then(|body| body.instruments.iter().find(|item| !item.uid.is_empty()))
    {
        qualified(
            "OptionBy",
            client
                .option_by(instrument_by_uid(&uid(&option.uid)?))
                .await,
            &mut capabilities,
        )?;
    } else {
        capabilities.mark_provider_data_unavailable("OptionBy");
        println!("UNAVAILABLE OptionBy: OptionsBy returned no candidate");
    }

    let notes = qualified(
        "StructuredNotes",
        client.structured_notes(catalogue).await,
        &mut capabilities,
    )?;
    if let Some(note) = notes
        .as_ref()
        .and_then(|body| body.instruments.iter().find(|item| !item.uid.is_empty()))
    {
        qualified(
            "StructuredNoteBy",
            client
                .structured_note_by(instrument_by_uid(&uid(&note.uid)?))
                .await,
            &mut capabilities,
        )?;
    } else {
        capabilities.mark_provider_data_unavailable("StructuredNoteBy");
    }

    let dfas = qualified(
        "Dfas",
        client.dfas(v1::DfasRequest {}).await,
        &mut capabilities,
    )?;
    if let Some(dfa) = dfas
        .as_ref()
        .and_then(|body| body.instruments.iter().find(|item| !item.uid.is_empty()))
    {
        qualified(
            "DfaBy",
            client.dfa_by(instrument_by_uid(&uid(&dfa.uid)?)).await,
            &mut capabilities,
        )?;
    } else {
        capabilities.mark_provider_data_unavailable("DfaBy");
    }
    qualified(
        "Indicatives",
        client.indicatives(v1::IndicativesRequest {}).await,
        &mut capabilities,
    )?;

    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    qualified(
        "TradingSchedules",
        client
            .trading_schedules(v1::TradingSchedulesRequest {
                exchange: None,
                from: Some(Timestamp {
                    seconds: now + 86_400,
                    nanos: 0,
                }),
                to: Some(Timestamp {
                    seconds: now + 8 * 86_400,
                    nanos: 0,
                }),
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetDividends",
        client
            .get_dividends(v1::GetDividendsRequest {
                instrument_id: share_uid.as_str().to_owned(),
                from: Some(from),
                to: Some(to),
                ..Default::default()
            })
            .await,
        &mut capabilities,
    )?;

    let asset = qualified(
        "GetAssetBy",
        client
            .get_asset_by(v1::AssetRequest {
                id: share_asset_uid.as_str().to_owned(),
            })
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssets",
        client.get_assets(v1::AssetsRequest::default()).await,
        &mut capabilities,
    )?;
    qualified(
        "GetFavorites",
        client
            .get_favorites(v1::GetFavoritesRequest::default())
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetFavoriteGroups",
        client
            .get_favorite_groups(v1::GetFavoriteGroupsRequest::default())
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetCountries",
        client.get_countries(v1::GetCountriesRequest {}).await,
        &mut capabilities,
    )?;

    let brands = qualified(
        "GetBrands",
        client
            .get_brands(v1::GetBrandsRequest {
                paging: Some(v1::Page {
                    limit: 100,
                    page_number: 0,
                }),
            })
            .await,
        &mut capabilities,
    )?;
    if let Some(brand) = brands
        .as_ref()
        .and_then(|body| body.brands.iter().find(|item| !item.uid.is_empty()))
    {
        qualified(
            "GetBrandBy",
            client
                .get_brand_by(v1::GetBrandRequest {
                    id: brand.uid.clone(),
                })
                .await,
            &mut capabilities,
        )?;
    } else {
        capabilities.mark_provider_data_unavailable("GetBrandBy");
    }
    qualified(
        "GetAssetFundamentals",
        client
            .get_asset_fundamentals(fundamentals_request(std::slice::from_ref(
                &share_asset_uid,
            ))?)
            .await,
        &mut capabilities,
    )?;
    qualified(
        "GetAssetReports",
        client
            .get_asset_reports(v1::GetAssetReportsRequest {
                instrument_id: share_uid.as_str().to_owned(),
                from: Some(from),
                to: Some(to),
            })
            .await,
        &mut capabilities,
    )?;

    let consensus = qualified(
        "GetConsensusForecasts",
        client
            .get_consensus_forecasts(v1::GetConsensusForecastsRequest {
                paging: Some(v1::Page {
                    limit: 100,
                    page_number: 0,
                }),
            })
            .await,
        &mut capabilities,
    )?;
    if let Some(consensus) = consensus {
        let asset_uids = consensus_asset_uids(&consensus);
        let mut candidate_count = 0;
        let mut qualified_forecast = false;
        for asset_uid in &asset_uids {
            let resolved = client
                .get_asset_by(v1::AssetRequest {
                    id: asset_uid.as_str().to_owned(),
                })
                .await?;
            let Some(asset) = resolved.body.asset else {
                continue;
            };
            for candidate in forecast_instrument_candidates(&asset) {
                candidate_count += 1;
                match client
                    .get_forecast_by(v1::GetForecastRequest {
                        instrument_id: candidate.as_str().to_owned(),
                    })
                    .await
                {
                    Ok(_) => {
                        println!("QUALIFIED GetForecastBy");
                        qualified_forecast = true;
                        break;
                    }
                    Err(GrpcError {
                        kind: GrpcErrorKind::Provider(provider),
                        ..
                    }) if provider.code == Code::NotFound => {}
                    Err(error) => return Err(Box::<dyn Error>::from(error)),
                }
            }
            if qualified_forecast {
                break;
            }
        }
        if !qualified_forecast {
            capabilities.mark_provider_inconsistency("GetForecastBy");
            return Err(Box::<dyn Error>::from(ForecastProviderInconsistency {
                asset_uids,
                candidate_count,
            }));
        }
    } else {
        capabilities.mark_provider_data_unavailable("GetForecastBy");
    }

    qualified(
        "GetRiskRates",
        client
            .get_risk_rates(v1::RiskRatesRequest {
                instrument_id: vec![share_uid.as_str().to_owned()],
            })
            .await,
        &mut capabilities,
    )?;
    let insider = insider_deals_request(&share_uid, 100, None)?;
    let insider_response = qualified(
        "GetInsiderDeals",
        client.get_insider_deals(insider).await,
        &mut capabilities,
    )?;
    if let Some(next_cursor) = insider_response.and_then(|response| response.next_cursor) {
        qualified(
            "GetInsiderDeals",
            client
                .get_insider_deals(insider_deals_request(&share_uid, 100, Some(next_cursor))?)
                .await,
            &mut capabilities,
        )?;
    }
    let news = qualified(
        "News",
        client
            .news(v1::NewsRequest {
                cursor: None,
                limit: Some(1000),
            })
            .await,
        &mut capabilities,
    )?;
    if let Some(next_cursor) = news.and_then(|response| response.next_cursor) {
        qualified(
            "News",
            client
                .news(v1::NewsRequest {
                    cursor: Some(next_cursor),
                    limit: Some(1000),
                })
                .await,
            &mut capabilities,
        )?;
    }

    let _ = asset;
    Ok(())
}
