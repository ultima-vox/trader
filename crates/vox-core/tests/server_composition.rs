use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::path::PathBuf;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use vox_connections::{
    BindingId, BrokerAccessLevel, BrokerAccount, BrokerAccountBinding, BrokerAccountStatus,
    BrokerAccountType, BrokerConnection, BrokerEnvironment, ConnectionCapability, ConnectionHealth,
    ConnectionHealthReason, ConnectionHealthState, ConnectionId, ConnectionRepository,
    CredentialClass, CredentialContext, CredentialRef, CredentialScope, CredentialStatus,
    ExecutionAuthorization, ExecutionAuthorizationMode, ExecutionPurpose, Permission, ProviderId,
    SecretBytes, SecretStore, ServiceError, StaticKeyProvider, User, UserId, VoxAccountId,
};
use vox_core::CoreConfig;
use vox_core::auth::SessionBootstrap;
use vox_core::composition::{ApplicationComposition, ServerConfig};
use vox_domain::{CancelOrderCommand, ProviderOrderIdentityKind, RuntimeExecutionCommand};
use vox_tinvest::ConnectionFactoryError;

const SESSION: &str = "session-token-material-00000000000000000000000000000001";

#[tokio::test]
async fn production_composition_enforces_server_verified_session_and_rbac()
-> Result<(), Box<dyn Error>> {
    let root = temp_root("auth");
    let user_id = UserId::new();
    let mut server_config = config(
        &root,
        user_id.clone(),
        BTreeSet::from([Permission::ViewConnectionMetadata]),
    );
    server_config.bootstrap.cookie_secure = true;
    let composition = ApplicationComposition::build(server_config).await?;
    let app = composition.router();

    let anonymous = app
        .clone()
        .oneshot(Request::get("/api/v1/broker-connections").body(Body::empty())?)
        .await?;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let first_bootstrap = bootstrap_response(app.clone()).await?;
    assert_eq!(first_bootstrap.status(), StatusCode::OK);
    assert_eq!(first_bootstrap.headers()["cache-control"], "no-store");
    let first_cookie = first_bootstrap.headers()["set-cookie"].to_str()?;
    assert!(first_cookie.contains("HttpOnly"));
    assert!(first_cookie.contains("Secure"));
    assert!(first_cookie.contains("SameSite=Strict"));
    assert!(!first_cookie.contains(SESSION));
    let second_bootstrap = bootstrap_response(app.clone()).await?;
    assert_ne!(
        first_cookie,
        second_bootstrap.headers()["set-cookie"].to_str()?,
        "each bootstrap must issue fresh random session material"
    );
    let invalid = app
        .clone()
        .oneshot(
            Request::post("/api/v1/auth/session")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "bootstrap_credential": "invalid-bootstrap-credential-material"
                    })
                    .to_string(),
                ))?,
        )
        .await?;
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);

    let authenticated = app
        .clone()
        .oneshot(
            authenticated_request(app.clone(), "GET", "/api/v1/broker-connections", false).await?,
        )
        .await?;
    assert_eq!(authenticated.status(), StatusCode::OK);

    let no_csrf = app
        .clone()
        .oneshot(
            authenticated_request(app.clone(), "POST", "/api/v1/broker-connections", false).await?,
        )
        .await?;
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);

    composition.shutdown().await;
    drop(composition);

    // Restart must not silently re-enable a revoked/disabled principal.
    let repository =
        vox_connections::SqliteConnectionRepository::open(root.join("platform.sqlite3"))?;
    repository.put_user(&User {
        id: user_id.clone(),
        display_name: "revoked operator".to_owned(),
        enabled: false,
    })?;
    drop(repository);
    let restarted = ApplicationComposition::build(config(
        &root,
        user_id,
        BTreeSet::from([Permission::ViewConnectionMetadata]),
    ))
    .await?;
    let rejected = bootstrap_response(restarted.router()).await?;
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    restarted.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn production_composition_denies_missing_domain_permission() -> Result<(), Box<dyn Error>> {
    let root = temp_root("deny");
    let composition =
        ApplicationComposition::build(config(&root, UserId::new(), BTreeSet::new())).await?;
    let response = composition
        .router()
        .oneshot(
            authenticated_request(
                composition.router(),
                "GET",
                "/api/v1/broker-connections",
                false,
            )
            .await?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let credential_write = composition
        .router()
        .oneshot(
            json_authenticated_request(
                composition.router(),
                "POST",
                "/api/v1/broker-connections",
                serde_json::json!({
                    "provider": "T_INVEST",
                    "environment": "SANDBOX",
                    "display_label": "must fail before provider access",
                    "credential": "not-a-real-credential"
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(credential_write.status(), StatusCode::FORBIDDEN);

    let execution = composition
        .router()
        .oneshot(
            json_authenticated_request(
                composition.router(),
                "POST",
                "/api/v1/commands/order",
                serde_json::json!({}),
            )
            .await?,
        )
        .await?;
    assert_eq!(execution.status(), StatusCode::FORBIDDEN);
    composition.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn production_composition_requires_enable_permission_but_allows_emergency_downgrade()
-> Result<(), Box<dyn Error>> {
    let root = temp_root("execution-rbac");
    let user_id = UserId::new();
    let composition = ApplicationComposition::build(config(
        &root,
        user_id.clone(),
        BTreeSet::from([
            Permission::ViewConnectionMetadata,
            Permission::EmergencyHalt,
        ]),
    ))
    .await?;
    let (connection_id, provider_account_id) =
        seed_production_authorization(&composition, user_id)?;
    let stale_session = composition.client_factory.execution_session(
        &connection_id,
        &provider_account_id,
        ExecutionPurpose::ProductionAutomated,
    )?;
    let uri = format!(
        "/api/v1/broker-connections/{}/execution-authorization",
        connection_id.as_str()
    );

    let forbidden_enable = composition
        .router()
        .oneshot(
            json_authenticated_request(
                composition.router(),
                "PUT",
                &uri,
                serde_json::json!({
                    "provider_account_id": provider_account_id,
                    "mode": "AUTOMATED_ALLOWED",
                    "expected_authorization_revision": 7
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(forbidden_enable.status(), StatusCode::FORBIDDEN);

    let emergency_disable = composition
        .router()
        .oneshot(
            json_authenticated_request(
                composition.router(),
                "PUT",
                &uri,
                serde_json::json!({
                    "provider_account_id": provider_account_id,
                    "mode": "DISABLED",
                    "expected_authorization_revision": 7
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(emergency_disable.status(), StatusCode::OK);
    let body = response_json(emergency_disable).await?;
    assert_eq!(body["mode"], "DISABLED");
    assert_eq!(body["authorization_revision"], 8);
    let command = RuntimeExecutionCommand::CancelOrder(CancelOrderCommand {
        account_id: provider_account_id,
        order_id: "must-not-reach-provider".to_owned(),
        order_id_kind: Some(ProviderOrderIdentityKind::BrokerOrder),
    });
    assert!(matches!(
        stale_session.dispatch_once(&command).await,
        Err(ConnectionFactoryError::Connection(
            ServiceError::StaleExecutionAuthorization
        ))
    ));

    composition.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn production_composition_restart_restores_stored_scope_without_plaintext()
-> Result<(), Box<dyn Error>> {
    const TOKEN: &str = "offline-restart-broker-credential-material";
    let root = temp_root("offline-restart");
    let user_id = UserId::new();
    let permissions = BTreeSet::from([Permission::ViewConnectionMetadata]);
    let composition =
        ApplicationComposition::build(config(&root, user_id.clone(), permissions.clone())).await?;
    let (connection_id, provider_account_id) =
        seed_production_authorization_with_token(&composition, user_id.clone(), TOKEN)?;
    let session = composition
        .client_factory
        .read_session(&connection_id, &provider_account_id)?;
    assert_eq!(session.target.connection_id, connection_id);
    drop(session);
    composition.shutdown().await;
    drop(composition);

    for database in [root.join("platform.sqlite3"), root.join("secrets.sqlite3")] {
        let bytes = std::fs::read(database)?;
        assert!(
            !bytes
                .windows(TOKEN.len())
                .any(|window| window == TOKEN.as_bytes())
        );
    }

    let restarted = ApplicationComposition::build(config(&root, user_id, permissions)).await?;
    assert_eq!(restarted.lifecycle_recovery, Default::default());
    let details = restarted
        .router()
        .oneshot(
            authenticated_request(
                restarted.router(),
                "GET",
                &format!("/api/v1/broker-connections/{}", connection_id.as_str()),
                false,
            )
            .await?,
        )
        .await?;
    assert_eq!(details.status(), StatusCode::OK);
    let details = response_json(details).await?;
    assert_eq!(details["bindings"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        details["execution_authorizations"][0]["authorization_revision"],
        7
    );
    let session = restarted
        .client_factory
        .read_session(&connection_id, &provider_account_id)?;
    assert_eq!(session.target.provider_account_id, provider_account_id);
    restarted.shutdown().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires TINVEST_SANDBOX_TOKEN; read-only broker qualification plus local metadata writes"]
async fn live_sandbox_onboarding_read_and_restart_use_production_composition()
-> Result<(), Box<dyn Error>> {
    let token = std::env::var("TINVEST_SANDBOX_TOKEN")?;
    let root = temp_root("sandbox-live");
    let user_id = UserId::new();
    let permissions = BTreeSet::from([
        Permission::ViewConnectionMetadata,
        Permission::ManageCredentials,
        Permission::DiscoverAccounts,
        Permission::BindAccounts,
        Permission::ViewPortfolio,
        Permission::SubmitSandboxOrders,
    ]);
    let composition =
        ApplicationComposition::build(config(&root, user_id.clone(), permissions.clone())).await?;
    let app = composition.router();
    let created = app
        .clone()
        .oneshot(
            json_authenticated_request(
                app.clone(),
                "POST",
                "/api/v1/broker-connections",
                serde_json::json!({
                    "provider": "T_INVEST",
                    "environment": "SANDBOX",
                    "display_label": "live composition qualification",
                    "credential": token
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await?;
    assert!(created.get("credential_ref").is_none());
    let connection_id = created["connection_id"]
        .as_str()
        .ok_or("connection id missing")?;

    let accounts = app
        .clone()
        .oneshot(
            authenticated_request(
                app.clone(),
                "GET",
                &format!("/api/v1/broker-connections/{connection_id}/accounts"),
                false,
            )
            .await?,
        )
        .await?;
    assert_eq!(accounts.status(), StatusCode::OK);
    let accounts = response_json(accounts).await?;
    let provider_account_id = accounts
        .as_array()
        .and_then(|accounts| {
            accounts
                .iter()
                .find(|account| account["accessible"] == true)
        })
        .and_then(|account| account["provider_account_id"].as_str())
        .ok_or("no accessible sandbox account")?;
    let vox_account_id = vox_connections::VoxAccountId::new();
    let binding = app
        .clone()
        .oneshot(
            json_authenticated_request(
                app.clone(),
                "POST",
                &format!("/api/v1/broker-connections/{connection_id}/bindings"),
                serde_json::json!({
                    "provider_account_id": provider_account_id,
                    "account_id": vox_account_id.as_str()
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(binding.status(), StatusCode::CREATED);
    let authorization = app
        .clone()
        .oneshot(
            json_authenticated_request(
                app.clone(),
                "PUT",
                &format!("/api/v1/broker-connections/{connection_id}/execution-authorization"),
                serde_json::json!({
                    "provider_account_id": provider_account_id,
                    "mode": "MANUAL_ALLOWED",
                    "expected_authorization_revision": 1
                }),
            )
            .await?,
        )
        .await?;
    assert_eq!(authorization.status(), StatusCode::OK);
    let authorization = response_json(authorization).await?;
    assert_eq!(authorization["authorization_revision"], 2);

    let read_uri = format!(
        "/api/v1/accounts?provider=T_INVEST&environment=SANDBOX&broker_connection_id={connection_id}&account_id={}",
        vox_account_id.as_str()
    );
    let read = app
        .clone()
        .oneshot(authenticated_request(app.clone(), "GET", &read_uri, false).await?)
        .await?;
    let read_status = read.status();
    let read_body = response_json(read).await?;
    assert_eq!(read_status, StatusCode::OK, "read response: {read_body}");
    assert!(!read_body.as_array().ok_or("accounts response")?.is_empty());

    for path in ["portfolio", "positions"] {
        let uri = format!(
            "/api/v1/{path}?provider=T_INVEST&environment=SANDBOX&broker_connection_id={connection_id}&account_id={}",
            vox_account_id.as_str()
        );
        let response = app
            .clone()
            .oneshot(authenticated_request(app.clone(), "GET", &uri, false).await?)
            .await?;
        let status = response.status();
        let body = response_json(response).await?;
        assert_eq!(status, StatusCode::OK, "{path} response: {body}");
    }
    let execution_session = composition.client_factory.execution_session(
        &ConnectionId::parse(connection_id.to_owned())?,
        provider_account_id,
        ExecutionPurpose::SandboxMutation,
    )?;
    assert_eq!(execution_session.authorization_revision(), 2);
    drop(execution_session);

    let encrypted = std::fs::read(root.join("secrets.sqlite3"))?;
    assert!(
        !encrypted
            .windows(token.len())
            .any(|window| window == token.as_bytes())
    );
    composition.shutdown().await;
    drop(composition);

    let restarted = ApplicationComposition::build(config(&root, user_id, permissions)).await?;
    assert_eq!(restarted.lifecycle_recovery, Default::default());
    let details = restarted
        .router()
        .oneshot(
            authenticated_request(
                restarted.router(),
                "GET",
                &format!("/api/v1/broker-connections/{connection_id}"),
                false,
            )
            .await?,
        )
        .await?;
    assert_eq!(details.status(), StatusCode::OK);
    let details = response_json(details).await?;
    assert_eq!(details["bindings"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        details["execution_authorizations"][0]["authorization_revision"],
        2
    );
    restarted.shutdown().await;
    Ok(())
}

fn config(
    root: &std::path::Path,
    user_id: UserId,
    permissions: BTreeSet<Permission>,
) -> ServerConfig {
    ServerConfig {
        core: CoreConfig::default(),
        bind: "127.0.0.1:0".parse().expect("static socket address"),
        platform_database_path: root.join("platform.sqlite3"),
        secret_database_path: root.join("secrets.sqlite3"),
        runtime_database_directory: root.join("runtime"),
        key_provider: StaticKeyProvider::new(1, BTreeMap::from([(1, [73_u8; 32])]))
            .expect("static test keyring"),
        bootstrap: SessionBootstrap {
            user_id,
            display_name: "test operator".to_owned(),
            permissions,
            bootstrap_credential: SecretBytes::new(SESSION).expect("valid bootstrap credential"),
            expires_at_unix_ms: 4_102_444_800_000,
            cookie_secure: false,
        },
        tinvest_enabled: true,
    }
}

async fn authenticated_request(
    app: Router,
    method: &str,
    uri: &str,
    csrf: bool,
) -> Result<Request<Body>, Box<dyn Error>> {
    let (cookie, csrf_token) = session_credentials(app).await?;
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    if csrf {
        request = request.header("x-vox-csrf", csrf_token);
    }
    Ok(request.body(Body::empty())?)
}

async fn json_authenticated_request(
    app: Router,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) -> Result<Request<Body>, Box<dyn Error>> {
    let (cookie, csrf_token) = session_credentials(app).await?;
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .header("x-vox-csrf", csrf_token)
        .body(Body::from(body.to_string()))?)
}

async fn bootstrap_response(app: Router) -> Result<axum::response::Response, Box<dyn Error>> {
    Ok(app
        .oneshot(
            Request::post("/api/v1/auth/session")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "bootstrap_credential": SESSION }).to_string(),
                ))?,
        )
        .await?)
}

async fn session_credentials(app: Router) -> Result<(String, String), Box<dyn Error>> {
    let response = bootstrap_response(app).await?;
    if response.status() != StatusCode::OK {
        return Err(format!("session bootstrap failed: {}", response.status()).into());
    }
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .ok_or("session cookie missing")?
        .to_owned();
    let body = response_json(response).await?;
    let csrf = body["csrf_token"]
        .as_str()
        .ok_or("CSRF token missing")?
        .to_owned();
    Ok((cookie, csrf))
}

async fn response_json(
    response: axum::response::Response,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn seed_production_authorization(
    composition: &ApplicationComposition,
    actor: UserId,
) -> Result<(ConnectionId, String), Box<dyn Error>> {
    seed_production_authorization_with_token(
        composition,
        actor,
        "seeded-production-credential-material",
    )
}

fn seed_production_authorization_with_token(
    composition: &ApplicationComposition,
    actor: UserId,
    token: &str,
) -> Result<(ConnectionId, String), Box<dyn Error>> {
    let connection_id = ConnectionId::new();
    let credential_ref = CredentialRef::new();
    let provider = ProviderId::tinvest();
    let provider_account_id = "production-account".to_owned();
    let capabilities = BTreeSet::from([
        ConnectionCapability::PortfolioRead,
        ConnectionCapability::ProductionOrdersProviderAllowed,
    ]);
    let fingerprint = composition.secret_store.put(
        &credential_ref,
        &CredentialContext {
            provider: provider.clone(),
            environment: BrokerEnvironment::Production,
        },
        SecretBytes::new(token)?,
        1,
    )?;
    let connection = BrokerConnection {
        id: connection_id.clone(),
        provider: provider.clone(),
        environment: BrokerEnvironment::Production,
        credential_ref,
        label: "stored production connection".to_owned(),
        credential_fingerprint: fingerprint,
        credential_status: CredentialStatus::Valid,
        credential_class: CredentialClass::Unknown,
        credential_scope: CredentialScope::NotConfirmed,
        enabled: true,
        health: ConnectionHealth {
            state: ConnectionHealthState::Healthy,
            checked_at_unix_ms: Some(1),
            provider: provider.clone(),
            environment: BrokerEnvironment::Production,
            reason_code: ConnectionHealthReason::None,
            safe_detail: None,
            retryable: false,
        },
        capabilities: capabilities.clone(),
        created_at_unix_ms: 1,
        updated_at_unix_ms: 1,
    };
    let account = BrokerAccount {
        connection_id: connection_id.clone(),
        provider: provider.clone(),
        environment: BrokerEnvironment::Production,
        provider_account_id: provider_account_id.clone(),
        display_name: Some("production account".to_owned()),
        account_type: BrokerAccountType::Brokerage,
        status: BrokerAccountStatus::Open,
        access_level: BrokerAccessLevel::Full,
        opened_at_unix_ms: None,
        closed_at_unix_ms: None,
        accessible: true,
        capabilities,
        discovered_at_unix_ms: 1,
    };
    let authorization = ExecutionAuthorization {
        connection_id: connection_id.clone(),
        provider_account_id: provider_account_id.clone(),
        mode: ExecutionAuthorizationMode::AutomatedAllowed,
        authorization_revision: 7,
        changed_by: actor,
        changed_at_unix_ms: 1,
    };
    composition.repository.insert_connection(&connection)?;
    composition.repository.insert_onboarding(
        &connection,
        std::slice::from_ref(&account),
        std::slice::from_ref(&authorization),
        &[],
    )?;
    composition.repository.put_binding(&BrokerAccountBinding {
        id: BindingId::new(),
        connection_id: connection_id.clone(),
        provider,
        environment: BrokerEnvironment::Production,
        provider_account_id: provider_account_id.clone(),
        vox_account_id: VoxAccountId::new(),
        enabled: true,
        created_at_unix_ms: 1,
        updated_at_unix_ms: 1,
    })?;
    Ok((connection_id, provider_account_id))
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("vox-server-{label}-{}", uuid::Uuid::new_v4()))
}
