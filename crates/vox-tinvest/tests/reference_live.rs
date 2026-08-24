use vox_tinvest::reference::{
    EmptyRequest, InstrumentExchange, InstrumentStatus, InstrumentsRequest, PageRequest,
    PagedRequest,
};
use vox_tinvest::{SecretToken, TInvestRestClient};

/// Opt-in, read-only qualification. Run with `TINVEST_TOKEN` explicitly set.
#[tokio::test]
#[ignore = "requires live T-Invest token; performs safe reads only"]
async fn current_reference_surface_decodes_live_read_only() -> Result<(), Box<dyn std::error::Error>>
{
    let token = SecretToken::new(std::env::var("TINVEST_TOKEN")?)?;
    let client = TInvestRestClient::production(token)?;
    let catalogue = InstrumentsRequest {
        instrument_status: InstrumentStatus::InstrumentStatusBase,
        instrument_exchange: InstrumentExchange::InstrumentExchangeUnspecified,
    };

    let shares = client.shares(&catalogue).await?.into_body();
    assert!(!shares.instruments.is_empty());
    let countries = client
        .get_countries(&EmptyRequest::default())
        .await?
        .into_body();
    assert!(!countries.countries.is_empty());
    let brands = client
        .get_brands(&PagedRequest {
            paging: PageRequest::new(10, 0)?,
        })
        .await?
        .into_body();
    assert!(brands.paging.total_count >= 0);
    Ok(())
}
