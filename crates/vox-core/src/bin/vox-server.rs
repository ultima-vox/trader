//! Starts the Vox application API.
//!
//! `vox-core` composes the process: it reads configuration, decides which application ports
//! are attached, and serves the boundary. #11 runtime health is attached from the accepted
//! `RuntimeHealth` contract. Account reads attach only when a broker read port exists; until
//! #17 supplies a connection they stay unavailable rather than inventing balances.
//!
//! TLS is terminated in front of this process (reverse proxy or platform load balancer);
//! the server itself binds plain HTTP on the loopback address by default, so a
//! misconfiguration cannot expose it unencrypted to a network.

use std::net::SocketAddr;

use anyhow::{Context, anyhow};
use tracing::info;
use tracing_subscriber::EnvFilter;
use vox_api::contract::scope::{BrokerEnvironment, ProviderDto};
use vox_api::{AppState, ProcessRuntime};
use vox_core::{CoreConfig, CoreRuntime};
use vox_domain::Environment;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .try_init()
        .map_err(|error| anyhow!("initialize tracing: {error}"))?;

    let config = CoreConfig::from_env().context("load Vox Core configuration")?;
    let runtime = CoreRuntime::new(config);
    let environment = broker_environment(config.environment())?;

    // #11 runtime health is attached from the accepted contract. Account and execution
    // ports stay empty until a broker connection exists; they refuse rather than invent data.
    let state = AppState::detached(ProviderDto::TInvest, environment).with_runtime(
        std::sync::Arc::new(ProcessRuntime::starting(ProviderDto::TInvest, environment)),
    );
    let app = vox_api::router(state);

    let address: SocketAddr = std::env::var("VOX_API_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()
        .context("parse VOX_API_BIND")?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind {address}"))?;

    info!(
        %address,
        environment = ?config.environment(),
        readiness = ?runtime.readiness().state(),
        "Vox application API listening"
    );
    axum::serve(listener, app)
        .await
        .context("serve the Vox application API")
}

/// Maps the process environment onto the broker environment the API speaks.
///
/// `Environment::Paper` is a trading mode, not a broker environment, and has no runtime
/// yet: refusing to start is better than serving a scope the contracts cannot express.
fn broker_environment(environment: Environment) -> anyhow::Result<BrokerEnvironment> {
    match environment {
        Environment::Sandbox => Ok(BrokerEnvironment::Sandbox),
        Environment::Live => Ok(BrokerEnvironment::Production),
        Environment::Paper => Err(anyhow!(
            "PAPER is a trading mode owned by #23/#29, not a broker environment; \
             the API serves SANDBOX and PRODUCTION only"
        )),
    }
}
