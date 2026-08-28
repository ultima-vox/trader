use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use async_trait::async_trait;
use vox_connections::{
    BrokerAccessLevel, BrokerAccountStatus, BrokerAccountType, BrokerEnvironment,
    BrokerProviderPort, ConnectionCapability, ConnectionRepository, ConnectionService,
    CreateConnectionRequest, CredentialClass, CredentialScope, ExecutionAuthorizationMode,
    Permission, ProviderAccountFact, ProviderDiscovery, ProviderError, ProviderId, Role, RoleId,
    SecretBytes, SecurityContext, SqliteConnectionRepository, SqliteSecretStore, StaticKeyProvider,
    User, UserId, VoxAccountId,
};
use vox_runtime::connection_credentials::StoredCredentialResolver;
use vox_runtime::{CredentialResolverPort, OpaqueRef, Provider, RuntimeEnvironment, RuntimeScope};

struct ProviderStub;

#[async_trait]
impl BrokerProviderPort for ProviderStub {
    async fn validate_and_discover(
        &self,
        _provider: &ProviderId,
        _environment: BrokerEnvironment,
        _credential: &SecretBytes,
    ) -> Result<ProviderDiscovery, ProviderError> {
        let capabilities = BTreeSet::from([
            ConnectionCapability::PortfolioRead,
            ConnectionCapability::ProductionOrdersProviderAllowed,
        ]);
        Ok(ProviderDiscovery {
            credential_class: CredentialClass::FullAccess,
            credential_scope: CredentialScope::NotConfirmed,
            connection_capabilities: capabilities.clone(),
            accounts: vec![ProviderAccountFact {
                provider_account_id: "broker-account".to_owned(),
                display_name: None,
                account_type: BrokerAccountType::Brokerage,
                status: BrokerAccountStatus::Open,
                access_level: BrokerAccessLevel::Full,
                opened_at_unix_ms: None,
                closed_at_unix_ms: None,
                accessible: true,
                capabilities,
            }],
        })
    }
}

#[tokio::test]
async fn runtime_resolves_only_exact_bound_scope_and_separate_authorization()
-> Result<(), Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!(
        "vox-runtime-connections-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let repository = SqliteConnectionRepository::open(&path)?;
    let key_provider = StaticKeyProvider::new(1, BTreeMap::from([(1, [8_u8; 32])]))?;
    let secret_store = SqliteSecretStore::open(&path, key_provider)?;
    let user = User {
        id: UserId::new(),
        display_name: "Runtime operator".to_owned(),
        enabled: true,
    };
    let role = Role {
        id: RoleId::new(),
        name: "runtime-admin".to_owned(),
        permissions: BTreeSet::from([
            Permission::ManageCredentials,
            Permission::DiscoverAccounts,
            Permission::BindAccounts,
            Permission::EnableAutomatedProductionExecution,
        ]),
    };
    repository.put_user(&user)?;
    repository.put_role(&role)?;
    repository.grant_role(&user.id, &role.id)?;
    let service = ConnectionService::new(repository.clone(), secret_store, ProviderStub);
    let security = SecurityContext::new(user.id, "runtime-test", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Runtime connection".to_owned(),
            },
            SecretBytes::new(b"credential".to_vec())?,
        )
        .await?;
    let vox_account_id = VoxAccountId::new();
    service.bind_account(
        &security,
        &connection.id,
        "broker-account",
        vox_account_id.clone(),
    )?;
    let scope = RuntimeScope::new_bound(
        Provider::TInvest,
        RuntimeEnvironment::Production,
        vox_account_id.as_str(),
        "broker-account",
        OpaqueRef::new(connection.id.as_str())?,
        OpaqueRef::new(connection.credential_ref.as_str())?,
    )?;
    let resolver = StoredCredentialResolver::new(service);
    assert!(!resolver.resolve(&scope).await?.execution_authorized);

    let service = resolver.into_inner();
    service.set_execution_authorization(
        &security,
        &connection.id,
        "broker-account",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    let resolver = StoredCredentialResolver::new(service);
    assert!(resolver.resolve(&scope).await?.execution_authorized);
    let mismatched = RuntimeScope::new_bound(
        Provider::TInvest,
        RuntimeEnvironment::Sandbox,
        vox_account_id.as_str(),
        "broker-account",
        OpaqueRef::new(connection.id.as_str())?,
        OpaqueRef::new(connection.credential_ref.as_str())?,
    )?;
    assert!(resolver.resolve(&mismatched).await.is_err());
    drop(resolver);
    drop(repository);
    std::fs::remove_file(path)?;
    Ok(())
}
