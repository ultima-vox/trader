use std::error::Error;

use vox_connections::{
    BrokerEnvironment, BrokerProviderPort, ConnectionCapability, ProviderId, SecretBytes,
};
use vox_tinvest::connection_provider::TInvestConnectionProvider;

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN; read-only connection onboarding qualification"]
async fn sandbox_credential_validates_and_discovers_all_accounts() -> Result<(), Box<dyn Error>> {
    let token = std::env::var("TINVEST_SANDBOX_TOKEN")?;
    let credential = SecretBytes::new(token.into_bytes())?;
    let discovery = TInvestConnectionProvider
        .validate_and_discover(
            &ProviderId::tinvest(),
            BrokerEnvironment::Sandbox,
            &credential,
        )
        .await?;

    assert!(
        discovery
            .connection_capabilities
            .contains(&ConnectionCapability::AccountDiscovery)
    );
    assert!(!discovery.accounts.is_empty());
    assert!(discovery.accounts.iter().all(|account| {
        !account.provider_account_id.trim().is_empty()
            && (!account.accessible
                || account
                    .capabilities
                    .contains(&ConnectionCapability::PortfolioRead))
    }));
    Ok(())
}
