use std::collections::BTreeSet;
use std::error::Error;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use vox_api::application::{ConnectionAdministration, ConnectionRequestContext, RuntimeQueries};
use vox_api::contract::capability::{Capability, CapabilitySet, UnavailableCapability};
use vox_api::contract::connections::{
    BindBrokerAccountRequest, BrokerAccountBindingDto, BrokerConnectionMetadataDto,
    ChangeExecutionAuthorizationRequest, ConnectionCapabilityDto, ConnectionDetailsDto,
    ConnectionHealthDto, ConnectionHealthReasonDto, ConnectionHealthStateDto,
    CreateBrokerConnectionRequest, CredentialClassDto, CredentialRotationResultDto,
    CredentialScopeDto, CredentialStatusDto, DiscoveredBrokerAccountDto, ExecutionAuthorizationDto,
    ExecutionAuthorizationModeDto, RotateCredentialRequest,
};
use vox_api::contract::instrument::InstrumentIdentityDto;
use vox_api::contract::market::InstrumentSummaryDto;
use vox_api::contract::money::Decimal;
use vox_api::contract::runtime::{ReasonCodeDto, RuntimeHealthDto, RuntimeStateDto};
use vox_api::contract::scope::{BrokerEnvironment, ExecutionScope, ProviderDto, TradingMode};
use vox_api::error::{ApiError, ErrorCategory};
use vox_api::{AppState, SnapshotMarketProjection};
use vox_connections::{Permission, SecretBytes, SqliteConnectionRepository, UserId};
use vox_core::auth::{AuthState, SessionBootstrap, authenticated_actor_middleware};

const CONNECTION_ID: &str = "connection:frontend-e2e";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let bootstrap = std::env::var("VOX_FRONTEND_E2E_BOOTSTRAP")?;
    let database = std::env::temp_dir().join(format!(
        "vox-frontend-e2e-{}-{}.sqlite3",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let repository = SqliteConnectionRepository::open(&database)?;
    let auth = AuthState::open(
        &database,
        repository,
        SessionBootstrap {
            user_id: UserId::new(),
            display_name: "Frontend E2E operator".to_owned(),
            permissions: BTreeSet::from([
                Permission::ViewConnectionMetadata,
                Permission::ViewPortfolio,
                Permission::SubmitSandboxOrders,
            ]),
            bootstrap_credential: SecretBytes::new(bootstrap)?,
            expires_at_unix_ms: 4_102_444_800_000,
            cookie_secure: false,
        },
        1_700_000_000_000,
        Permission::SubmitSandboxOrders,
    )?;

    let market = Arc::new(SnapshotMarketProjection::new());
    market.publish_instrument(InstrumentSummaryDto {
        identity: InstrumentIdentityDto {
            provider: "tinvest".to_owned(),
            uid: "provider-diagnostic-uid".to_owned(),
            figi: None,
            ticker: "SBER".to_owned(),
            class_code: "TQBR".to_owned(),
        },
        name: "Сбербанк".to_owned(),
        instrument_type: "Акция".to_owned(),
        lot_size: 10,
        min_price_increment: Decimal::parse("0.01")?,
        currency: "RUB".to_owned(),
        tradable: true,
    });

    let mut state = AppState::detached(ProviderDto::TInvest, BrokerEnvironment::Sandbox);
    state.runtime = Some(Arc::new(FixtureRuntime));
    state.connections = Some(Arc::new(FixtureConnections::new()));
    state.authentication = Some(Arc::new(auth.clone()));
    state.market_data = Some(market);
    let app = vox_api::router(state)
        .layer(DefaultBodyLimit::max(128 * 1024))
        .layer(middleware::from_fn_with_state(
            auth,
            authenticated_actor_middleware,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:18100").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

struct FixtureRuntime;

#[async_trait]
impl RuntimeQueries for FixtureRuntime {
    async fn health(&self) -> Result<RuntimeHealthDto, ApiError> {
        Ok(health("process", false, 0))
    }

    async fn scoped_health(&self, scope: &ExecutionScope) -> Result<RuntimeHealthDto, ApiError> {
        match scope.account_id.as_str() {
            "account:alpha" => Ok(health("Alpha", true, 11)),
            "account:beta" => Ok(health("Beta", false, 22)),
            _ => Err(not_found()),
        }
    }

    async fn scopes(&self) -> Result<Vec<ExecutionScope>, ApiError> {
        Ok(vec![scope("account:alpha"), scope("account:beta")])
    }

    async fn capabilities(
        &self,
        account_id: Option<&str>,
    ) -> Result<Option<CapabilitySet>, ApiError> {
        Ok(Some(CapabilitySet {
            provider: ProviderDto::TInvest,
            environment: BrokerEnvironment::Sandbox,
            account_id: account_id.map(ToOwned::to_owned),
            supported: vec![
                Capability::RuntimeHealth,
                Capability::OrderExecution,
                Capability::MarketData,
            ],
            unavailable: vec![
                UnavailableCapability {
                    capability: Capability::ProtectionExecution,
                    reason: "fixture has no protection command port".to_owned(),
                    owner: "#10".to_owned(),
                },
                UnavailableCapability {
                    capability: Capability::RiskVerdict,
                    reason: "risk verdict fixture intentionally deferred".to_owned(),
                    owner: "#21".to_owned(),
                },
            ],
        }))
    }
}

fn scope(account_id: &str) -> ExecutionScope {
    ExecutionScope {
        provider: ProviderDto::TInvest,
        environment: BrokerEnvironment::Sandbox,
        broker_connection_id: CONNECTION_ID.to_owned(),
        account_id: account_id.to_owned(),
        trading_mode: TradingMode::Live,
    }
}

fn health(account: &str, ready: bool, epoch: u64) -> RuntimeHealthDto {
    RuntimeHealthDto {
        state: if ready {
            RuntimeStateDto::Ready
        } else {
            RuntimeStateDto::Halted
        },
        reason_code: if ready {
            ReasonCodeDto::ReconciliationComplete
        } else {
            ReasonCodeDto::ExecutionUnauthorized
        },
        reason: if ready {
            "selected scope ready".to_owned()
        } else {
            "selected scope blocks new exposure".to_owned()
        },
        provider: ProviderDto::TInvest,
        environment: BrokerEnvironment::Sandbox,
        account_display: account.to_owned(),
        runtime_epoch: epoch,
        connected: ready,
        last_successful_reconciliation_at_unix_ms: ready.then_some(1_700_000_000_000),
        reconciliation_age_ms: ready.then_some(0),
        unresolved_unknown_count: 0,
        open_order_count: 0,
        active_stop_count: 0,
        stream_states: Vec::new(),
        persistence_healthy: true,
        execution_authorized: ready,
        new_exposure_allowed: ready,
    }
}

struct FixtureConnections {
    metadata: BrokerConnectionMetadataDto,
    details: ConnectionDetailsDto,
}

impl FixtureConnections {
    fn new() -> Self {
        let metadata = BrokerConnectionMetadataDto {
            connection_id: CONNECTION_ID.to_owned(),
            provider: ProviderDto::TInvest,
            environment: BrokerEnvironment::Sandbox,
            display_label: "Real HTTP fixture".to_owned(),
            enabled: true,
            credential_status: CredentialStatusDto::Valid,
            credential_class: CredentialClassDto::Sandbox,
            credential_scope: CredentialScopeDto::AllAccessibleAccounts,
            capabilities: vec![
                ConnectionCapabilityDto::PortfolioRead,
                ConnectionCapabilityDto::SandboxOrders,
            ],
            health: ConnectionHealthDto {
                state: ConnectionHealthStateDto::Healthy,
                checked_at_unix_ms: Some(1_700_000_000_000),
                reason_code: ConnectionHealthReasonDto::None,
                safe_detail: None,
                retryable: false,
            },
            created_at_unix_ms: 1,
            updated_at_unix_ms: 2,
        };
        let accounts = [
            ("provider-alpha", "Alpha account", "account:alpha"),
            ("provider-beta", "Beta account", "account:beta"),
        ];
        let details = ConnectionDetailsDto {
            connection: metadata.clone(),
            accounts: accounts
                .iter()
                .map(|(provider_id, display, _)| DiscoveredBrokerAccountDto {
                    connection_id: CONNECTION_ID.to_owned(),
                    provider: ProviderDto::TInvest,
                    environment: BrokerEnvironment::Sandbox,
                    provider_account_id: (*provider_id).to_owned(),
                    display_name: Some((*display).to_owned()),
                    account_type: "BROKER".to_owned(),
                    account_status: "OPEN".to_owned(),
                    access_level: "FULL_ACCESS".to_owned(),
                    opened_at_unix_ms: None,
                    closed_at_unix_ms: None,
                    accessible: true,
                    capabilities: metadata.capabilities.clone(),
                    discovered_at_unix_ms: 3,
                })
                .collect(),
            bindings: accounts
                .iter()
                .enumerate()
                .map(
                    |(index, (provider_id, _, account_id))| BrokerAccountBindingDto {
                        binding_id: format!("binding:{}", index + 1),
                        connection_id: CONNECTION_ID.to_owned(),
                        provider: ProviderDto::TInvest,
                        environment: BrokerEnvironment::Sandbox,
                        provider_account_id: (*provider_id).to_owned(),
                        account_id: (*account_id).to_owned(),
                        enabled: true,
                        created_at_unix_ms: 4,
                        updated_at_unix_ms: 5,
                    },
                )
                .collect(),
            execution_authorizations: accounts
                .iter()
                .map(|(provider_id, _, _)| ExecutionAuthorizationDto {
                    connection_id: CONNECTION_ID.to_owned(),
                    provider_account_id: (*provider_id).to_owned(),
                    mode: ExecutionAuthorizationModeDto::ManualAllowed,
                    authorization_revision: 1,
                    changed_by: "fixture".to_owned(),
                    changed_at_unix_ms: 6,
                })
                .collect(),
        };
        Self { metadata, details }
    }
}

#[async_trait]
impl ConnectionAdministration for FixtureConnections {
    async fn list_connections(
        &self,
        _: &ConnectionRequestContext,
    ) -> Result<Vec<BrokerConnectionMetadataDto>, ApiError> {
        Ok(vec![self.metadata.clone()])
    }
    async fn connection_details(
        &self,
        _: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<ConnectionDetailsDto, ApiError> {
        (connection_id == CONNECTION_ID)
            .then(|| self.details.clone())
            .ok_or_else(not_found)
    }
    async fn accounts(
        &self,
        _: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<DiscoveredBrokerAccountDto>, ApiError> {
        Ok(self
            .connection_details(&fixture_context(), connection_id)
            .await?
            .accounts)
    }
    async fn bindings(
        &self,
        _: &ConnectionRequestContext,
        connection_id: &str,
    ) -> Result<Vec<BrokerAccountBindingDto>, ApiError> {
        Ok(self
            .connection_details(&fixture_context(), connection_id)
            .await?
            .bindings)
    }
    async fn create_connection(
        &self,
        _: &ConnectionRequestContext,
        _: CreateBrokerConnectionRequest,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        Err(read_only())
    }
    async fn revalidate_connection(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        Err(read_only())
    }
    async fn rotate_credential(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
        _: RotateCredentialRequest,
    ) -> Result<CredentialRotationResultDto, ApiError> {
        Err(read_only())
    }
    async fn disable_connection(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
    ) -> Result<BrokerConnectionMetadataDto, ApiError> {
        Err(read_only())
    }
    async fn delete_connection(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
    ) -> Result<(), ApiError> {
        Err(read_only())
    }
    async fn bind_account(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
        _: BindBrokerAccountRequest,
    ) -> Result<BrokerAccountBindingDto, ApiError> {
        Err(read_only())
    }
    async fn unbind_account(&self, _: &ConnectionRequestContext, _: &str) -> Result<(), ApiError> {
        Err(read_only())
    }
    async fn change_execution_authorization(
        &self,
        _: &ConnectionRequestContext,
        _: &str,
        _: ChangeExecutionAuthorizationRequest,
    ) -> Result<ExecutionAuthorizationDto, ApiError> {
        Err(read_only())
    }
}

fn fixture_context() -> ConnectionRequestContext {
    ConnectionRequestContext {
        actor: vox_api::application::AuthenticatedActor {
            user_id: "fixture".to_owned(),
        },
        correlation_id: "fixture".to_owned(),
        now_unix_ms: 1,
    }
}

fn read_only() -> ApiError {
    ApiError::new(
        ErrorCategory::Permission,
        "FIXTURE_READ_ONLY",
        "fixture is read-only",
    )
}

fn not_found() -> ApiError {
    ApiError::new(
        ErrorCategory::NotFound,
        "FIXTURE_SCOPE_NOT_FOUND",
        "fixture scope not found",
    )
}
