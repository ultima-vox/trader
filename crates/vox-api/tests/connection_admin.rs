use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Extension;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use vox_api::application::AuthenticatedActor;
use vox_api::contract::scope::{BrokerEnvironment as ApiEnvironment, ProviderDto};
use vox_api::{AppState, ConnectionAdministrationAdapter, router};
use vox_connections::{
    BrokerAccessLevel, BrokerAccountStatus, BrokerAccountType, BrokerEnvironment,
    BrokerProviderPort, ConnectionCapability, ConnectionRepository, ConnectionService,
    CredentialClass, CredentialScope, Permission, ProviderAccountFact, ProviderDiscovery,
    ProviderError, ProviderId, Role, RoleId, SecretBytes, SqliteConnectionRepository,
    SqliteSecretStore, StaticKeyProvider, User, UserId, VoxAccountId,
};

struct ProviderStub;

#[async_trait]
impl BrokerProviderPort for ProviderStub {
    async fn validate_and_discover(
        &self,
        _provider: &ProviderId,
        environment: BrokerEnvironment,
        _credential: &SecretBytes,
    ) -> Result<ProviderDiscovery, ProviderError> {
        let order_capability = match environment {
            BrokerEnvironment::Sandbox => ConnectionCapability::SandboxOrders,
            BrokerEnvironment::Production => ConnectionCapability::ProductionOrdersProviderAllowed,
        };
        let capabilities = BTreeSet::from([
            ConnectionCapability::AccountDiscovery,
            ConnectionCapability::PortfolioRead,
            order_capability,
        ]);
        Ok(ProviderDiscovery {
            credential_class: match environment {
                BrokerEnvironment::Sandbox => CredentialClass::Sandbox,
                BrokerEnvironment::Production => CredentialClass::Unknown,
            },
            credential_scope: CredentialScope::NotConfirmed,
            connection_capabilities: capabilities.clone(),
            accounts: vec![ProviderAccountFact {
                provider_account_id: "broker-account".to_owned(),
                display_name: Some("Sandbox account".to_owned()),
                account_type: BrokerAccountType::Brokerage,
                status: BrokerAccountStatus::Open,
                access_level: BrokerAccessLevel::Full,
                opened_at_unix_ms: Some(1),
                closed_at_unix_ms: None,
                accessible: true,
                capabilities,
            }],
        })
    }
}

#[tokio::test]
async fn connection_routes_onboard_bind_authorize_and_never_return_credential()
-> Result<(), Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!(
        "vox-api-connections-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let repository = SqliteConnectionRepository::open(&path)?;
    let secret_store = SqliteSecretStore::open(
        &path,
        StaticKeyProvider::new(1, BTreeMap::from([(1, [9_u8; 32])]))?,
    )?;
    let user = User {
        id: UserId::new(),
        display_name: "API operator".to_owned(),
        enabled: true,
    };
    let role = Role {
        id: RoleId::new(),
        name: "connection-admin".to_owned(),
        permissions: BTreeSet::from([
            Permission::ViewConnectionMetadata,
            Permission::ManageCredentials,
            Permission::DiscoverAccounts,
            Permission::BindAccounts,
            Permission::SubmitSandboxOrders,
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
    let state = AppState::detached(ProviderDto::TInvest, ApiEnvironment::Sandbox)
        .with_connections(Arc::new(ConnectionAdministrationAdapter::new(service)));
    let app = router(state).layer(Extension(AuthenticatedActor {
        user_id: user.id.as_str().to_owned(),
    }));

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/v1/broker-connections",
            serde_json::json!({
                "provider": "T_INVEST",
                "environment": "SANDBOX",
                "display_label": "Primary sandbox",
                "credential": "sandbox-credential-material"
            }),
        ))
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response_json(response).await?;
    let rendered = created.to_string();
    assert!(!rendered.contains("sandbox-credential-material"));
    assert!(created.get("credential_ref").is_none());
    let connection_id = created["connection_id"]
        .as_str()
        .ok_or("missing connection id")?;

    let accounts = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/broker-connections/{connection_id}/accounts"
            ))
            .body(Body::empty())?,
        )
        .await?;
    assert_eq!(accounts.status(), StatusCode::OK);
    let accounts = response_json(accounts).await?;
    assert_eq!(accounts[0]["provider_account_id"], "broker-account");

    let vox_account_id = VoxAccountId::new();
    let binding = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/v1/broker-connections/{connection_id}/bindings"),
            serde_json::json!({
                "provider_account_id": "broker-account",
                "account_id": vox_account_id.as_str()
            }),
        ))
        .await?;
    assert_eq!(binding.status(), StatusCode::CREATED);

    let authorization = app
        .clone()
        .oneshot(json_request(
            "PUT",
            &format!("/api/v1/broker-connections/{connection_id}/execution-authorization"),
            serde_json::json!({
                "provider_account_id": "broker-account",
                "mode": "MANUAL_ALLOWED",
                "expected_authorization_revision": 1
            }),
        ))
        .await?;
    assert_eq!(authorization.status(), StatusCode::OK);
    let authorization = response_json(authorization).await?;
    assert_eq!(authorization["mode"], "MANUAL_ALLOWED");

    let stale = app
        .clone()
        .oneshot(json_request(
            "PUT",
            &format!("/api/v1/broker-connections/{connection_id}/execution-authorization"),
            serde_json::json!({
                "provider_account_id": "broker-account",
                "mode": "DISABLED",
                "expected_authorization_revision": 1
            }),
        ))
        .await?;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(stale).await?["code"],
        "STALE_EXECUTION_AUTHORIZATION"
    );

    let details = app
        .oneshot(
            Request::get(format!("/api/v1/broker-connections/{connection_id}"))
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(details.status(), StatusCode::OK);
    let details = response_json(details).await?;
    assert_eq!(
        details["execution_authorizations"][0]["mode"],
        "MANUAL_ALLOWED"
    );

    drop(repository);
    std::fs::remove_file(path)?;
    Ok(())
}

fn json_request(method: &str, uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("valid test request")
}

async fn response_json(
    response: axum::response::Response,
) -> Result<serde_json::Value, Box<dyn Error>> {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
