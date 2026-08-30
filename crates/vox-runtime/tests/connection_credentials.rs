use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use vox_connections::{
    BrokerAccessLevel, BrokerAccountStatus, BrokerAccountType, BrokerEnvironment,
    BrokerProviderPort, ConnectionCapability, ConnectionRepository, ConnectionService,
    CreateConnectionRequest, CredentialClass, CredentialScope, ExecutionAuthorizationMode,
    Permission, ProviderAccountFact, ProviderDiscovery, ProviderError, ProviderId, Role, RoleId,
    SecretBytes, SecurityContext, SqliteConnectionRepository, SqliteSecretStore, StaticKeyProvider,
    User, UserId, VoxAccountId,
};
use vox_runtime::{
    CredentialResolverPort, OpaqueRef, Provider, RuntimeEnvironment, RuntimeExecutionPurpose,
    RuntimeScope, StoredCredentialResolver,
};

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
            credential_class: CredentialClass::Unknown,
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
async fn runtime_rechecks_exact_execution_authorization_before_each_dispatch_gate()
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
            Permission::BindAccounts,
            Permission::EnableAutomatedProductionExecution,
            Permission::EmergencyHalt,
        ]),
    };
    repository.put_user(&user)?;
    repository.put_role(&role)?;
    repository.grant_role(&user.id, &role.id)?;
    let service = Arc::new(ConnectionService::new(
        repository.clone(),
        secret_store,
        ProviderStub,
    ));
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
    service.bind_account(
        &security,
        &connection.id,
        "broker-account",
        VoxAccountId::new(),
    )?;
    let scope = RuntimeScope::new(
        Provider::TInvest,
        RuntimeEnvironment::Production,
        "broker-account",
        OpaqueRef::new(connection.id.as_str())?,
        OpaqueRef::new(connection.credential_ref.as_str())?,
    )?;
    let resolver = StoredCredentialResolver::new(service.clone());

    assert!(!resolver.resolve(&scope).await?.execution_authorized);
    assert!(
        resolver
            .authorize_execution(&scope, RuntimeExecutionPurpose::ProductionAutomated)
            .await
            .is_err()
    );

    service.set_execution_authorization(
        &security,
        &connection.id,
        "broker-account",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    resolver
        .authorize_execution(&scope, RuntimeExecutionPurpose::ProductionAutomated)
        .await?;

    service.set_execution_authorization(
        &security,
        &connection.id,
        "broker-account",
        ExecutionAuthorizationMode::Disabled,
    )?;
    assert!(
        resolver
            .authorize_execution(&scope, RuntimeExecutionPurpose::ProductionAutomated)
            .await
            .is_err(),
        "same resolver must observe revocation without restart"
    );

    let mismatched = RuntimeScope::new(
        Provider::TInvest,
        RuntimeEnvironment::Sandbox,
        "broker-account",
        OpaqueRef::new(connection.id.as_str())?,
        OpaqueRef::new(connection.credential_ref.as_str())?,
    )?;
    assert!(resolver.resolve(&mismatched).await.is_err());

    drop(resolver);
    drop(service);
    drop(repository);
    std::fs::remove_file(path)?;
    Ok(())
}
