use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, anyhow};
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use tower_http::services::{ServeDir, ServeFile};
use vox_api::binding::{AccountBinding, AccountBindingResolver, BindingError};
use vox_api::contract::scope::{BrokerEnvironment as ApiBrokerEnvironment, ProviderDto};
use vox_api::{AccountReadAdapter, AppState, ConnectionAdministrationAdapter};
use vox_connections::{
    ConnectionId, ConnectionRepository, ConnectionService, Permission, SecretBytes,
    SecurityContext, SqliteConnectionRepository, SqliteSecretStore, StaticKeyProvider, UserId,
};
use vox_domain::Environment;
use vox_runtime::OpaqueRef;
use vox_tinvest::connection_provider::TInvestConnectionProvider;
use vox_tinvest::{StoredTInvestClientFactory, StoredTInvestReadPort};

use crate::CoreConfig;
use crate::auth::{AuthState, SessionBootstrap, authenticated_actor_middleware};
use crate::production_runtime::ProductionRuntimeRegistry;

pub type ProductionSecretStore = SqliteSecretStore<StaticKeyProvider>;
pub type ProductionConnectionService =
    ConnectionService<SqliteConnectionRepository, ProductionSecretStore, TInvestConnectionProvider>;
pub type ProductionClientFactory = StoredTInvestClientFactory<
    SqliteConnectionRepository,
    ProductionSecretStore,
    TInvestConnectionProvider,
>;

pub struct ServerConfig {
    pub core: CoreConfig,
    pub bind: SocketAddr,
    pub platform_database_path: PathBuf,
    pub secret_database_path: PathBuf,
    pub runtime_database_directory: PathBuf,
    pub frontend_directory: Option<PathBuf>,
    pub key_provider: StaticKeyProvider,
    pub bootstrap: SessionBootstrap,
    pub tinvest_enabled: bool,
}

impl ServerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let core = CoreConfig::from_env().context("load core environment")?;
        let bind: SocketAddr = std::env::var("VOX_API_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
            .parse()
            .context("parse VOX_API_BIND")?;
        let platform_database_path = env_path("VOX_PLATFORM_DB", "data/vox-platform.sqlite3")?;
        let secret_database_path = env_path("VOX_SECRET_DB", "data/vox-secrets.sqlite3")?;
        let runtime_database_directory = env_path("VOX_RUNTIME_DB_DIR", "data/runtime")?;
        let key_provider =
            StaticKeyProvider::from_hex_environment("VOX_KEK_ACTIVE_VERSION", "VOX_KEK_HEX_V")
                .context("load external KEK keyring")?;
        let user_id = UserId::parse(required_env("VOX_BOOTSTRAP_USER_ID")?)
            .context("parse VOX_BOOTSTRAP_USER_ID")?;
        let bootstrap_credential = SecretBytes::new(required_env("VOX_BOOTSTRAP_CREDENTIAL")?)
            .context("load bootstrap credential")?;
        let expires_at_unix_ms = required_env("VOX_BOOTSTRAP_EXPIRES_UNIX_MS")?
            .parse::<i64>()
            .context("parse VOX_BOOTSTRAP_EXPIRES_UNIX_MS")?;
        let tinvest_enabled = optional_bool("VOX_TINVEST_ENABLED", true)?;
        let frontend_directory = optional_env_path("VOX_FRONTEND_DIR")?;
        let cookie_secure = optional_bool("VOX_SESSION_COOKIE_SECURE", true)?;
        if !cookie_secure && !bind.ip().is_loopback() {
            return Err(anyhow!(
                "VOX_SESSION_COOKIE_SECURE=false is allowed only on a loopback API bind"
            ));
        }
        Ok(Self {
            core,
            bind,
            platform_database_path,
            secret_database_path,
            runtime_database_directory,
            key_provider,
            bootstrap: SessionBootstrap {
                user_id,
                display_name: "Vox bootstrap operator".to_owned(),
                permissions: bootstrap_permissions(),
                bootstrap_credential,
                expires_at_unix_ms,
                cookie_secure,
            },
            frontend_directory,
            tinvest_enabled,
        })
    }
}

pub struct ApplicationComposition {
    router: Router,
    pub connections: Arc<ProductionConnectionService>,
    pub repository: SqliteConnectionRepository,
    pub secret_store: ProductionSecretStore,
    pub client_factory: Arc<ProductionClientFactory>,
    pub runtime: Arc<ProductionRuntimeRegistry>,
    pub lifecycle_recovery: vox_connections::LifecycleRecoveryReport,
}

impl ApplicationComposition {
    pub async fn build(config: ServerConfig) -> anyhow::Result<Self> {
        if !config.tinvest_enabled {
            return Err(anyhow!(
                "VOX_TINVEST_ENABLED=false leaves no configured broker provider"
            ));
        }
        prepare_parent(&config.platform_database_path)?;
        prepare_parent(&config.secret_database_path)?;
        std::fs::create_dir_all(&config.runtime_database_directory)
            .context("create runtime DB directory")?;

        let repository = SqliteConnectionRepository::open(&config.platform_database_path)
            .context("open platform connection repository")?;
        let secret_store =
            SqliteSecretStore::open(&config.secret_database_path, config.key_provider)
                .context("open encrypted credential store")?;
        let now = now_unix_ms()?;
        let recovery_actor = config.bootstrap.user_id.clone();
        let auth = AuthState::open(
            &config.platform_database_path,
            repository.clone(),
            config.bootstrap,
            now,
            match broker_environment(config.core.environment())? {
                ApiBrokerEnvironment::Sandbox => Permission::SubmitSandboxOrders,
                ApiBrokerEnvironment::Production => Permission::SubmitProductionManualOrders,
            },
        )
        .context("initialize server-side authentication")?;
        let connections = Arc::new(ConnectionService::new(
            repository.clone(),
            secret_store.clone(),
            TInvestConnectionProvider,
        ));
        let recovery_security =
            SecurityContext::new(recovery_actor, "startup-lifecycle-recovery", now)?;
        let lifecycle_recovery = connections
            .recover_connection_lifecycle(&recovery_security)
            .await
            .context("recover connection lifecycle")?;

        let client_factory = Arc::new(StoredTInvestClientFactory::new(Arc::clone(&connections)));
        let stored_reads = Arc::new(StoredTInvestReadPort::new(Arc::clone(&client_factory)));
        let binding_resolver = Arc::new(StoredBindingResolver::new(repository.clone()));
        let account_reads = Arc::new(AccountReadAdapter::new(
            binding_resolver.clone(),
            stored_reads,
            OpaqueRef::new("credential:resolved-from-stored-binding")?,
        ));
        let runtime = Arc::new(ProductionRuntimeRegistry::new(
            repository.clone(),
            Arc::clone(&connections),
            Arc::clone(&client_factory),
            binding_resolver,
            config.runtime_database_directory,
            broker_environment(config.core.environment())?,
        ));
        let connection_admin = Arc::new(
            ConnectionAdministrationAdapter::new(Arc::clone(&connections))
                .with_lifecycle_observer(runtime.clone()),
        );
        let state = AppState::production(
            ProviderDto::TInvest,
            broker_environment(config.core.environment())?,
            runtime.clone(),
            account_reads,
            runtime.clone(),
            connection_admin,
            Arc::new(auth.clone()),
        )
        .with_risk_queries(runtime.clone())
        .with_risk_commands(runtime.clone());
        let api = vox_api::router(state)
            .layer(DefaultBodyLimit::max(128 * 1024))
            .layer(middleware::from_fn_with_state(
                auth,
                authenticated_actor_middleware,
            ));
        let router = with_static_bundle(api, config.frontend_directory.as_deref());

        Ok(Self {
            router,
            connections,
            repository,
            secret_store,
            client_factory,
            runtime,
            lifecycle_recovery,
        })
    }

    pub fn router(&self) -> Router {
        self.router.clone()
    }

    pub async fn shutdown(&self) {
        self.runtime.shutdown().await;
    }
}

#[derive(Clone)]
struct StoredBindingResolver {
    repository: SqliteConnectionRepository,
}

impl StoredBindingResolver {
    const fn new(repository: SqliteConnectionRepository) -> Self {
        Self { repository }
    }
}

impl AccountBindingResolver for StoredBindingResolver {
    fn resolve(
        &self,
        account_id: &str,
        broker_connection_id: &str,
    ) -> Result<AccountBinding, BindingError> {
        let connection_id = ConnectionId::parse(broker_connection_id.to_owned())
            .map_err(|_| BindingError::UnknownAccount)?;
        let connection = self
            .repository
            .connection(&connection_id)
            .map_err(|_| BindingError::Unavailable)?
            .ok_or(BindingError::UnknownAccount)?;
        let binding = self
            .repository
            .bindings(&connection_id)
            .map_err(|_| BindingError::Unavailable)?
            .into_iter()
            .find(|binding| binding.vox_account_id.as_str() == account_id && binding.enabled)
            .ok_or(BindingError::UnknownAccount)?;
        AccountBinding::new_with_credential_ref(
            binding.vox_account_id.as_str(),
            connection.id.as_str(),
            binding.provider_account_id,
            OpaqueRef::new(connection.credential_ref.as_str())
                .map_err(|_| BindingError::Unavailable)?,
        )
    }
}

fn broker_environment(environment: Environment) -> anyhow::Result<ApiBrokerEnvironment> {
    match environment {
        Environment::Sandbox => Ok(ApiBrokerEnvironment::Sandbox),
        Environment::Live => Ok(ApiBrokerEnvironment::Production),
        Environment::Paper => Err(anyhow!(
            "PAPER is a trading mode, not a T-Invest broker environment"
        )),
    }
}

fn bootstrap_permissions() -> BTreeSet<Permission> {
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

fn required_env(name: &'static str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| anyhow!("{name} is required and must be valid Unicode"))
}

fn env_path(name: &'static str, default: &'static str) -> anyhow::Result<PathBuf> {
    let value = match std::env::var(name) {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => default.to_owned(),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(anyhow!("{name} must be valid Unicode"));
        }
    };
    if value.trim().is_empty() {
        return Err(anyhow!("{name} cannot be empty"));
    }
    Ok(PathBuf::from(value))
}

fn optional_env_path(name: &'static str) -> anyhow::Result<Option<PathBuf>> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(PathBuf::from(value))),
        Ok(_) => Ok(None),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(anyhow!("{name} must be valid Unicode")),
    }
}

fn optional_bool(name: &'static str, default: bool) -> anyhow::Result<bool> {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(anyhow!("{name} must be true, false, 1, or 0")),
        },
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(anyhow!("{name} must be valid Unicode")),
    }
}

fn prepare_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create database parent {}", parent.display()))?;
    }
    Ok(())
}

fn now_unix_ms() -> anyhow::Result<i64> {
    i64::try_from(time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000)
        .context("system time outside supported millisecond range")
}

/// Serves the built frontend from the same origin as the API.
///
/// Vox Trader keeps the operator on one canonical origin: `cargo run -p vox-core` serves both
/// REST/WebSocket under `/api/v1` and the built SPA (`index.html` + `assets/`). API routes keep
/// precedence and their JSON error semantics; only non-API paths fall through to static files.
/// When no built frontend directory is configured, the API-only router is returned unchanged.
fn with_static_bundle(api: Router, frontend_directory: Option<&Path>) -> Router {
    let Some(directory) = frontend_directory else {
        return api;
    };
    if !directory.is_dir() {
        return api;
    }
    let index = ServeFile::new(directory.join("index.html"));
    let assets = ServeDir::new(directory.join("assets"));
    Router::new()
        .merge(api)
        .route_service("/", index.clone())
        .route_service("/index.html", index)
        .nest_service("/assets", assets)
}
