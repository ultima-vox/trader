use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use vox_connections::{
    BrokerAccessLevel, BrokerAccountStatus, BrokerAccountType, BrokerEnvironment,
    BrokerProviderPort, ConnectionCapability, ConnectionHealthState, ConnectionRepository,
    ConnectionService, CreateConnectionRequest, CredentialClass, CredentialContext, CredentialRef,
    CredentialScope, CredentialStatus, ExecutionAuthorizationMode, ExecutionPurpose, KeyMaterial,
    KeyProvider, KeyProviderError, Permission, ProviderAccountFact, ProviderDiscovery,
    ProviderError, ProviderErrorKind, ProviderId, Role, RoleId, SecretBytes, SecretStore,
    SecretStoreError, SecurityContext, ServiceError, SqliteConnectionRepository, SqliteSecretStore,
    StaticKeyProvider, User, UserId, VoxAccountId,
};

fn temp_db(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "vox-connections-{name}-{}.sqlite",
        uuid::Uuid::new_v4()
    ))
}

fn key(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn keys(current: u32, entries: &[(u32, u8)]) -> Result<StaticKeyProvider, KeyProviderError> {
    StaticKeyProvider::new(
        current,
        entries
            .iter()
            .map(|(version, byte)| (*version, key(*byte)))
            .collect(),
    )
}

fn secret(value: &str) -> Result<SecretBytes, SecretStoreError> {
    SecretBytes::new(value.as_bytes().to_vec())
}

fn context(environment: BrokerEnvironment) -> CredentialContext {
    CredentialContext {
        provider: ProviderId::tinvest(),
        environment,
    }
}

#[test]
fn envelope_never_persists_plaintext_and_wrong_key_fails_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("envelope");
    let credential_ref = CredentialRef::new();
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 7)])?)?;
    let token = "production-token-that-must-never-reach-db";
    store.put(
        &credential_ref,
        &context(BrokerEnvironment::Production),
        secret(token)?,
        1,
    )?;

    let database = fs::read(&path)?;
    assert!(
        !database
            .windows(token.len())
            .any(|window| window == token.as_bytes())
    );
    assert_eq!(
        store
            .get(&credential_ref, &context(BrokerEnvironment::Production))?
            .expose_secret(),
        token.as_bytes()
    );
    assert!(matches!(
        store.get(&credential_ref, &context(BrokerEnvironment::Sandbox)),
        Err(SecretStoreError::ContextMismatch)
    ));

    let wrong = SqliteSecretStore::open(&path, keys(1, &[(1, 9)])?)?;
    assert!(matches!(
        wrong.get(&credential_ref, &context(BrokerEnvironment::Production)),
        Err(SecretStoreError::AuthenticationFailed)
    ));
    drop(wrong);
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute(
        "UPDATE encrypted_credentials SET ciphertext = zeroblob(length(ciphertext))",
        [],
    )?;
    drop(connection);
    assert!(matches!(
        store.get(&credential_ref, &context(BrokerEnvironment::Production)),
        Err(SecretStoreError::AuthenticationFailed)
    ));
    drop(store);
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn key_rotation_rewraps_under_new_external_key() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("rewrap");
    let credential_ref = CredentialRef::new();
    let original = SqliteSecretStore::open(&path, keys(1, &[(1, 1), (2, 2)])?)?;
    original.put(
        &credential_ref,
        &context(BrokerEnvironment::Sandbox),
        secret("sandbox-secret")?,
        10,
    )?;
    drop(original);

    let rotating = SqliteSecretStore::open(&path, keys(2, &[(1, 1), (2, 2)])?)?;
    rotating.rewrap(&credential_ref, &context(BrokerEnvironment::Sandbox), 20)?;
    drop(rotating);

    let new_only = SqliteSecretStore::open(&path, keys(2, &[(2, 2)])?)?;
    assert_eq!(
        new_only
            .get(&credential_ref, &context(BrokerEnvironment::Sandbox))?
            .expose_secret(),
        b"sandbox-secret"
    );
    drop(new_only);
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn corrupt_wrapped_dek_fails_authentication() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("wrapped-dek");
    let credential_ref = CredentialRef::new();
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 2)])?)?;
    let context = context(BrokerEnvironment::Sandbox);
    store.put(&credential_ref, &context, secret("sandbox-secret")?, 1)?;
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute(
        "UPDATE encrypted_credentials SET wrapped_dek = zeroblob(length(wrapped_dek))",
        [],
    )?;
    drop(connection);
    assert!(matches!(
        store.get(&credential_ref, &context),
        Err(SecretStoreError::AuthenticationFailed)
    ));
    drop(store);
    fs::remove_file(path)?;
    Ok(())
}

#[derive(Clone, Default)]
struct MockProvider {
    discoveries: SharedDiscoveries,
    environments: Arc<Mutex<Vec<BrokerEnvironment>>>,
}

type SharedDiscoveries = Arc<Mutex<HashMap<Vec<u8>, Result<ProviderDiscovery, ProviderError>>>>;

impl MockProvider {
    fn accept(&self, credential: &str, discovery: ProviderDiscovery) {
        let mut values = self
            .discoveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        values.insert(credential.as_bytes().to_vec(), Ok(discovery));
    }

    fn reject(&self, credential: &str, kind: ProviderErrorKind) {
        let mut values = self
            .discoveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        values.insert(
            credential.as_bytes().to_vec(),
            Err(ProviderError::new(kind, "provider rejected credential")),
        );
    }
}

#[async_trait]
impl BrokerProviderPort for MockProvider {
    async fn validate_and_discover(
        &self,
        provider: &ProviderId,
        environment: BrokerEnvironment,
        credential: &SecretBytes,
    ) -> Result<ProviderDiscovery, ProviderError> {
        if provider.as_str() != ProviderId::T_INVEST {
            return Err(ProviderError::new(
                ProviderErrorKind::UnsupportedProvider,
                "unsupported provider",
            ));
        }
        self.environments
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(environment);
        self.discoveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(credential.expose_secret())
            .cloned()
            .unwrap_or_else(|| {
                Err(ProviderError::new(
                    ProviderErrorKind::InvalidCredential,
                    "invalid credential",
                ))
            })
    }
}

fn discovery(account_ids: &[&str], production_orders: bool) -> ProviderDiscovery {
    let mut capabilities = BTreeSet::from([
        ConnectionCapability::AccountDiscovery,
        ConnectionCapability::PortfolioRead,
        ConnectionCapability::PositionsRead,
        ConnectionCapability::OperationsRead,
        ConnectionCapability::StreamHealth,
    ]);
    if production_orders {
        capabilities.insert(ConnectionCapability::ProductionOrdersProviderAllowed);
    }
    ProviderDiscovery {
        credential_class: if production_orders {
            CredentialClass::FullAccess
        } else {
            CredentialClass::ReadOnly
        },
        credential_scope: CredentialScope::NotConfirmed,
        connection_capabilities: capabilities.clone(),
        accounts: account_ids
            .iter()
            .map(|account_id| ProviderAccountFact {
                provider_account_id: (*account_id).to_owned(),
                display_name: Some(format!("Account {account_id}")),
                account_type: BrokerAccountType::Brokerage,
                status: BrokerAccountStatus::Open,
                access_level: if production_orders {
                    BrokerAccessLevel::Full
                } else {
                    BrokerAccessLevel::ReadOnly
                },
                opened_at_unix_ms: Some(1),
                closed_at_unix_ms: None,
                accessible: true,
                capabilities: capabilities.clone(),
            })
            .collect(),
    }
}

fn bootstrap(
    repository: &SqliteConnectionRepository,
    permissions: BTreeSet<Permission>,
) -> Result<UserId, Box<dyn std::error::Error>> {
    let user = User {
        id: UserId::new(),
        display_name: "Operator".to_owned(),
        enabled: true,
    };
    let role = Role {
        id: RoleId::new(),
        name: format!("role-{}", uuid::Uuid::new_v4()),
        permissions,
    };
    repository.put_user(&user)?;
    repository.put_role(&role)?;
    repository.grant_role(&user.id, &role.id)?;
    Ok(user.id)
}

fn all_permissions() -> BTreeSet<Permission> {
    BTreeSet::from([
        Permission::ViewConnectionMetadata,
        Permission::ManageCredentials,
        Permission::DisableDeleteConnection,
        Permission::DiscoverAccounts,
        Permission::BindAccounts,
        Permission::ViewPortfolio,
        Permission::SubmitSandboxOrders,
        Permission::SubmitProductionManualOrders,
        Permission::EnableAutomatedProductionExecution,
        Permission::ChangeRiskPolicy,
        Permission::EmergencyHalt,
        Permission::SecurityAdmin,
    ])
}

#[tokio::test]
async fn production_onboarding_is_multi_account_bound_and_default_off()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("service");
    let repository = SqliteConnectionRepository::open(&path)?;
    let secret_store = SqliteSecretStore::open(&path, keys(1, &[(1, 4)])?)?;
    let provider = MockProvider::default();
    provider.accept(
        "full-production-token",
        discovery(&["broker-a", "broker-b"], true),
    );
    provider.accept(
        "rotated-production-token",
        discovery(&["broker-a", "broker-b"], true),
    );
    provider.accept("second-production-token", discovery(&["broker-c"], false));
    let actor = bootstrap(&repository, all_permissions())?;
    let security = SecurityContext::new(actor.clone(), "correlation-create", 100)?;
    let service = ConnectionService::new(repository.clone(), secret_store, provider);

    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Primary production".to_owned(),
            },
            secret("full-production-token")?,
        )
        .await?;
    assert_eq!(repository.accounts(&connection.id)?.len(), 2);
    for account in repository.accounts(&connection.id)? {
        assert_eq!(
            repository
                .authorization(&connection.id, &account.provider_account_id)?
                .ok_or("missing authorization")?
                .mode,
            ExecutionAuthorizationMode::Disabled
        );
    }

    let vox_account = VoxAccountId::new();
    service.bind_account(&security, &connection.id, "broker-b", vox_account.clone())?;
    let resolved = service.validate_read_access(&connection.id, "broker-b", &vox_account)?;
    assert_eq!(resolved.target.provider_account_id, "broker-b");
    assert!(matches!(
        service.validate_execution_access(
            &connection.id,
            "broker-b",
            &vox_account,
            ExecutionPurpose::ProductionManual,
            None,
        ),
        Err(ServiceError::ExecutionUnauthorized)
    ));
    assert!(
        service
            .validate_read_access(&connection.id, "broker-a", &vox_account)
            .is_err()
    );

    let authorization = service.set_execution_authorization(
        &security,
        &connection.id,
        "broker-b",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    assert_eq!(
        authorization.mode,
        ExecutionAuthorizationMode::AutomatedAllowed
    );
    let rotation = service
        .rotate_credential(
            &SecurityContext::new(actor.clone(), "correlation-rotate", 200)?,
            &connection.id,
            secret("rotated-production-token")?,
        )
        .await?;
    assert!(rotation.reconnect_required);
    service
        .create_connection(
            &SecurityContext::new(actor, "correlation-second", 300)?,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Read-only production".to_owned(),
            },
            secret("second-production-token")?,
        )
        .await?;
    assert_eq!(service.list_connections(&security)?.len(), 2);

    let serialized = serde_json::to_string(&rotation.connection)?;
    assert!(!serialized.contains("full-production-token"));
    assert!(!serialized.contains("rotated-production-token"));
    let database = fs::read(&path)?;
    assert!(
        !database
            .windows("full-production-token".len())
            .any(|window| window == b"full-production-token")
    );
    assert!(
        !database
            .windows("rotated-production-token".len())
            .any(|window| window == b"rotated-production-token")
    );
    assert!(
        !database
            .windows("second-production-token".len())
            .any(|window| window == b"second-production-token")
    );
    let audit = repository.audit_records()?;
    let audit_json = serde_json::to_string(&audit)?;
    assert!(!audit_json.contains("production-token"));
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn rbac_rejects_credential_and_execution_changes() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("rbac");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 5)])?)?;
    let provider = MockProvider::default();
    provider.accept("valid", discovery(&["account"], true));
    let viewer = bootstrap(
        &repository,
        BTreeSet::from([Permission::ViewConnectionMetadata]),
    )?;
    let service = ConnectionService::new(repository.clone(), store, provider.clone());
    let security = SecurityContext::new(viewer, "viewer-correlation", 1)?;
    let result = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Denied".to_owned(),
            },
            secret("valid")?,
        )
        .await;
    assert!(matches!(
        result,
        Err(vox_connections::ServiceError::PermissionDenied(
            Permission::ManageCredentials
        ))
    ));
    assert!(repository.list_connections()?.is_empty());
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn sandbox_binding_and_credential_lifecycle_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("sandbox-lifecycle");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 9)])?)?;
    let provider = MockProvider::default();
    provider.accept("sandbox-valid", sandbox_discovery(&["sandbox-account"]));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider);
    let security = SecurityContext::new(actor, "sandbox-lifecycle", 10)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Sandbox,
                label: "Sandbox".to_owned(),
            },
            secret("sandbox-valid")?,
        )
        .await?;
    assert_eq!(
        repository
            .authorization(&connection.id, "sandbox-account")?
            .ok_or("missing authorization")?
            .mode,
        ExecutionAuthorizationMode::Disabled
    );
    let vox_account = VoxAccountId::new();
    let binding = service.bind_account(
        &security,
        &connection.id,
        "sandbox-account",
        vox_account.clone(),
    )?;
    let authorization = service.set_execution_authorization(
        &security,
        &connection.id,
        "sandbox-account",
        ExecutionAuthorizationMode::ManualAllowed,
    )?;
    assert_eq!(
        authorization.mode,
        ExecutionAuthorizationMode::ManualAllowed
    );
    service.disable_connection(&security, &connection.id)?;
    let disabled = service.set_execution_authorization(
        &security,
        &connection.id,
        "sandbox-account",
        ExecutionAuthorizationMode::Disabled,
    )?;
    assert_eq!(disabled.mode, ExecutionAuthorizationMode::Disabled);
    service.disable_binding(&security, &binding.id)?;
    assert!(
        service
            .validate_read_access(&connection.id, "sandbox-account", &vox_account)
            .is_err()
    );
    service.delete_connection(&security, &connection.id)?;
    assert!(repository.connection(&connection.id)?.is_none());
    let actions = repository
        .audit_records()?
        .into_iter()
        .map(|event| event.action)
        .collect::<BTreeSet<_>>();
    assert!(actions.contains("CREDENTIAL_ADDED"));
    assert!(actions.contains("CREDENTIAL_DISABLED"));
    assert!(actions.contains("ACCOUNT_BINDING_DISABLED"));
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn revoked_account_access_closes_health_and_exact_target()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("revoke");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 6)])?)?;
    let provider = MockProvider::default();
    provider.accept("valid", discovery(&["account-a", "account-b"], false));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider.clone());
    let connection = service
        .create_connection(
            &SecurityContext::new(actor.clone(), "create", 1)?,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Read only".to_owned(),
            },
            secret("valid")?,
        )
        .await?;
    provider.accept("valid", discovery(&["account-b"], false));
    let updated = service
        .revalidate(
            &SecurityContext::new(actor.clone(), "revalidate", 2)?,
            &connection.id,
        )
        .await?;
    assert_eq!(
        updated.health.state,
        ConnectionHealthState::AccountAccessChanged
    );
    provider.reject("valid", ProviderErrorKind::ExpiredOrInactive);
    assert!(
        service
            .revalidate(&SecurityContext::new(actor, "expired", 3)?, &connection.id,)
            .await
            .is_err()
    );
    assert_eq!(
        repository
            .connection(&connection.id)?
            .ok_or("missing connection")?
            .health
            .state,
        ConnectionHealthState::InvalidCredential
    );
    assert!(
        service
            .set_execution_authorization(
                &SecurityContext::new(UserId::new(), "unauthorized", 4)?,
                &connection.id,
                "account-b",
                ExecutionAuthorizationMode::AutomatedAllowed,
            )
            .is_err()
    );
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn capability_downgrade_revokes_automated_authorization()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("capability-downgrade");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 8)])?)?;
    let provider = MockProvider::default();
    provider.accept("full", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider.clone());
    let security = SecurityContext::new(actor, "downgrade", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Downgrade".to_owned(),
            },
            secret("full")?,
        )
        .await?;
    service.bind_account(&security, &connection.id, "account", VoxAccountId::new())?;
    service.set_execution_authorization(
        &security,
        &connection.id,
        "account",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    provider.accept("full", discovery(&["account"], false));
    let updated = service
        .revalidate(
            &SecurityContext::new(security.actor, "downgrade-2", 2)?,
            &connection.id,
        )
        .await?;
    assert_eq!(
        updated.health.state,
        ConnectionHealthState::AccountAccessChanged
    );
    assert_eq!(
        repository
            .authorization(&connection.id, "account")?
            .ok_or("missing authorization")?
            .mode,
        ExecutionAuthorizationMode::Disabled
    );
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn production_token_cannot_resolve_for_mutation_while_vox_disabled()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("exec-gate");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 11)])?)?;
    let provider = MockProvider::default();
    provider.accept("full-production-token", discovery(&["broker-a"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider);
    let security = SecurityContext::new(actor, "exec-gate", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Full access still off".to_owned(),
            },
            secret("full-production-token")?,
        )
        .await?;
    let vox_account = VoxAccountId::new();
    service.bind_account(&security, &connection.id, "broker-a", vox_account.clone())?;
    let read = service.validate_read_access(&connection.id, "broker-a", &vox_account)?;
    assert_eq!(read.target.provider_account_id, "broker-a");
    let error = service
        .validate_execution_access(
            &connection.id,
            "broker-a",
            &vox_account,
            ExecutionPurpose::ProductionManual,
            None,
        )
        .expect_err("disabled authorization must not decrypt for mutation");
    assert!(matches!(error, ServiceError::ExecutionUnauthorized));
    let automated = service.validate_execution_access(
        &connection.id,
        "broker-a",
        &vox_account,
        ExecutionPurpose::ProductionAutomated,
        None,
    );
    assert!(matches!(
        automated,
        Err(ServiceError::ExecutionUnauthorized)
    ));
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn audit_insert_failure_does_not_change_authorization_or_binding()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("audit-fail");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 12)])?)?;
    let provider = MockProvider::default();
    provider.accept("full", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider);
    let security = SecurityContext::new(actor, "audit-fail", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Audit fail".to_owned(),
            },
            secret("full")?,
        )
        .await?;
    repository.fail_next_audit();
    assert!(
        service
            .bind_account(&security, &connection.id, "account", VoxAccountId::new())
            .is_err()
    );
    assert!(repository.bindings(&connection.id)?.is_empty());
    let vox_account = VoxAccountId::new();
    service.bind_account(&security, &connection.id, "account", vox_account)?;
    repository.fail_next_audit();
    assert!(
        service
            .set_execution_authorization(
                &security,
                &connection.id,
                "account",
                ExecutionAuthorizationMode::AutomatedAllowed,
            )
            .is_err()
    );
    assert_eq!(
        repository
            .authorization(&connection.id, "account")?
            .ok_or("missing authorization")?
            .mode,
        ExecutionAuthorizationMode::Disabled
    );
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn secret_delete_failure_leaves_pending_delete_tombstone()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("delete-tombstone");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 13)])?)?;
    let provider = MockProvider::default();
    provider.accept("token", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store.clone(), provider);
    let security = SecurityContext::new(actor, "delete-tombstone", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Tombstone".to_owned(),
            },
            secret("token")?,
        )
        .await?;
    service.disable_connection(&security, &connection.id)?;
    store.fail_next_delete();
    assert!(
        service
            .delete_connection(&security, &connection.id)
            .is_err()
    );
    let tombstone = repository
        .connection(&connection.id)?
        .ok_or("tombstone missing")?;
    assert!(!tombstone.enabled);
    assert_eq!(tombstone.credential_status, CredentialStatus::PendingDelete);
    service.delete_connection(&security, &connection.id)?;
    assert!(repository.connection(&connection.id)?.is_none());
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn secret_disable_failure_leaves_retryable_fail_closed_tombstone()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("disable-tombstone");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 16)])?)?;
    let provider = MockProvider::default();
    provider.accept("token", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store.clone(), provider);
    let security = SecurityContext::new(actor, "disable-tombstone", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Disable tombstone".to_owned(),
            },
            secret("token")?,
        )
        .await?;

    store.fail_next_disable();
    assert!(
        service
            .disable_connection(&security, &connection.id)
            .is_err()
    );
    let tombstone = repository
        .connection(&connection.id)?
        .ok_or("disable tombstone missing")?;
    assert!(!tombstone.enabled);
    assert_eq!(
        tombstone.credential_status,
        CredentialStatus::PendingDisable
    );
    assert!(
        service
            .validate_runtime_read_access(&connection.id, "account")
            .is_err()
    );

    let disabled = service.disable_connection(&security, &connection.id)?;
    assert_eq!(disabled.credential_status, CredentialStatus::Disabled);
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn onboarding_finalize_failure_keeps_durable_secret_owner()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("onboarding-tombstone");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 17)])?)?;
    let provider = MockProvider::default();
    provider.accept("token", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store.clone(), provider);
    let security = SecurityContext::new(actor, "onboarding-tombstone", 1)?;

    repository.fail_next_onboarding_finalize();
    store.fail_next_delete();
    assert!(
        service
            .create_connection(
                &security,
                CreateConnectionRequest {
                    provider: ProviderId::tinvest(),
                    environment: BrokerEnvironment::Production,
                    label: "Pending onboarding".to_owned(),
                },
                secret("token")?,
            )
            .await
            .is_err()
    );

    let pending = repository
        .list_connections()?
        .into_iter()
        .next()
        .ok_or("pending onboarding owner missing")?;
    assert!(!pending.enabled);
    assert_eq!(
        pending.credential_status,
        CredentialStatus::PendingValidation
    );
    assert_eq!(
        store
            .get(
                &pending.credential_ref,
                &context(BrokerEnvironment::Production)
            )?
            .expose_secret(),
        b"token"
    );
    service.delete_connection(&security, &pending.id)?;
    assert!(repository.connection(&pending.id)?.is_none());
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn authorization_revision_is_monotonic_and_invalidates_captured_grant()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("authorization-revision");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 18)])?)?;
    let provider = MockProvider::default();
    provider.accept("full", discovery(&["account"], true));
    let actor = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider);
    let security = SecurityContext::new(actor, "same-millisecond", 1)?;
    let connection = service
        .create_connection(
            &security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Revision".to_owned(),
            },
            secret("full")?,
        )
        .await?;
    service.bind_account(&security, &connection.id, "account", VoxAccountId::new())?;
    let enabled = service.set_execution_authorization(
        &security,
        &connection.id,
        "account",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    let captured = service.validate_runtime_execution_access(
        &connection.id,
        "account",
        ExecutionPurpose::ProductionAutomated,
        None,
    )?;
    assert_eq!(
        captured.authorization_revision,
        enabled.authorization_revision
    );

    let disabled = service.set_execution_authorization(
        &security,
        &connection.id,
        "account",
        ExecutionAuthorizationMode::Disabled,
    )?;
    assert_eq!(
        disabled.authorization_revision,
        enabled.authorization_revision + 1
    );
    assert!(matches!(
        service.validate_runtime_execution_access(
            &connection.id,
            "account",
            ExecutionPurpose::ProductionAutomated,
            Some(captured.authorization_revision),
        ),
        Err(ServiceError::StaleExecutionAuthorization)
    ));
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[tokio::test]
async fn emergency_halt_can_disable_automated_without_enable_privilege()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("emergency-halt");
    let repository = SqliteConnectionRepository::open(&path)?;
    let store = SqliteSecretStore::open(&path, keys(1, &[(1, 14)])?)?;
    let provider = MockProvider::default();
    provider.accept("full", discovery(&["account"], true));
    let admin = bootstrap(&repository, all_permissions())?;
    let service = ConnectionService::new(repository.clone(), store, provider);
    let admin_security = SecurityContext::new(admin, "admin", 1)?;
    let connection = service
        .create_connection(
            &admin_security,
            CreateConnectionRequest {
                provider: ProviderId::tinvest(),
                environment: BrokerEnvironment::Production,
                label: "Halt".to_owned(),
            },
            secret("full")?,
        )
        .await?;
    service.bind_account(
        &admin_security,
        &connection.id,
        "account",
        VoxAccountId::new(),
    )?;
    service.set_execution_authorization(
        &admin_security,
        &connection.id,
        "account",
        ExecutionAuthorizationMode::AutomatedAllowed,
    )?;
    let halt = bootstrap(&repository, BTreeSet::from([Permission::EmergencyHalt]))?;
    let disabled = service.set_execution_authorization(
        &SecurityContext::new(halt, "halt", 2)?,
        &connection.id,
        "account",
        ExecutionAuthorizationMode::Disabled,
    )?;
    assert_eq!(disabled.mode, ExecutionAuthorizationMode::Disabled);
    drop(service);
    drop(repository);
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn key_material_debug_is_redacted() {
    assert_eq!(
        format!("{:?}", KeyMaterial::new(key(1))),
        "KeyMaterial([REDACTED])"
    );
    let material = secret("do-not-print").unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(format!("{material:?}"), "SecretBytes([REDACTED])");
    assert_eq!(format!("{material}"), "[REDACTED]");
}

#[test]
fn external_key_provider_rejects_missing_material() {
    let variable = format!("VOX_TEST_MISSING_{}", uuid::Uuid::new_v4());
    assert!(matches!(
        StaticKeyProvider::from_hex_environment(&variable, "VOX_TEST_KEY_"),
        Err(KeyProviderError::MissingExternalKey)
    ));
}

struct MissingVersionProvider;

impl KeyProvider for MissingVersionProvider {
    fn active_key_version(&self) -> Result<u32, KeyProviderError> {
        Err(KeyProviderError::MissingVersion(1))
    }

    fn resolve_key_material(&self, version: u32) -> Result<KeyMaterial, KeyProviderError> {
        Err(KeyProviderError::MissingVersion(version))
    }

    fn rotate_key(&self, _version: u32, _material: KeyMaterial) -> Result<(), KeyProviderError> {
        Err(KeyProviderError::Unavailable)
    }
}

fn sandbox_discovery(account_ids: &[&str]) -> ProviderDiscovery {
    let mut value = discovery(account_ids, false);
    value.credential_class = CredentialClass::Sandbox;
    value
        .connection_capabilities
        .insert(ConnectionCapability::SandboxOrders);
    for account in &mut value.accounts {
        account
            .capabilities
            .insert(ConnectionCapability::SandboxOrders);
    }
    value
}

#[test]
fn missing_external_kek_fails_closed() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("missing-kek");
    let store = SqliteSecretStore::open(&path, MissingVersionProvider)?;
    assert_eq!(
        store.put(
            &CredentialRef::new(),
            &context(BrokerEnvironment::Production),
            secret("token")?,
            1,
        ),
        Err(SecretStoreError::KeyProvider(
            KeyProviderError::MissingVersion(1)
        ))
    );
    drop(store);
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn static_provider_requires_current_key() {
    assert!(matches!(
        StaticKeyProvider::new(2, BTreeMap::from([(1, key(1))])),
        Err(KeyProviderError::MissingVersion(2))
    ));
}

#[test]
fn environment_and_identity_deserialization_enforce_domain_invariants() {
    assert!(serde_json::from_str::<BrokerEnvironment>("\"PAPER\"").is_err());
    assert!(serde_json::from_str::<CredentialRef>("\"\"").is_err());
    assert!(serde_json::from_str::<CredentialRef>("\"credential:not-a-uuid\"").is_err());
}

#[test]
fn disabled_credential_fails_closed_until_delete() -> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("disable-secret");
    let provider = keys(1, &[(1, 7)])?;
    let store = SqliteSecretStore::open(&path, provider.clone())?;
    let credential_ref = CredentialRef::new();
    let context = context(BrokerEnvironment::Sandbox);
    store.put(&credential_ref, &context, secret("sandbox-token")?, 1)?;
    store.disable(&credential_ref)?;
    assert!(matches!(
        store.get(&credential_ref, &context),
        Err(SecretStoreError::Disabled)
    ));
    assert_eq!(
        store.rotate(&credential_ref, &context, secret("replacement")?, 2),
        Err(SecretStoreError::Disabled)
    );
    store.delete(&credential_ref)?;
    assert!(matches!(
        store.get(&credential_ref, &context),
        Err(SecretStoreError::NotFound)
    ));
    drop(store);
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn key_provider_rotates_and_keeps_historical_lookup() -> Result<(), Box<dyn std::error::Error>> {
    let provider = keys(1, &[(1, 3)])?;
    provider.rotate_key(2, KeyMaterial::new(key(4)))?;
    assert_eq!(provider.active_key_version()?, 2);
    assert!(provider.resolve_key_material(1).is_ok());
    assert!(provider.resolve_key_material(2).is_ok());
    assert_eq!(
        provider.rotate_key(2, KeyMaterial::new(key(5))),
        Err(KeyProviderError::NonMonotonicVersion)
    );
    assert_eq!(
        provider.rotate_key(1, KeyMaterial::new(key(5))),
        Err(KeyProviderError::NonMonotonicVersion)
    );
    Ok(())
}

#[test]
fn committed_security_contract_covers_required_cases() -> Result<(), Box<dyn std::error::Error>> {
    let contract: serde_json::Value = serde_json::from_str(include_str!(
        "../../../qualification/broker_connection_security_contracts.json"
    ))?;
    assert_eq!(contract["secret_envelope"]["algorithm"], "AES-256-GCM");
    assert_eq!(contract["permissions"].as_array().map(Vec::len), Some(12));
    assert_eq!(
        contract["qualification_cases"].as_array().map(Vec::len),
        Some(29)
    );
    Ok(())
}
