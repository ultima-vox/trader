//! Starts the Vox application API.
//!
//! `vox-core` composes the process: it reads configuration, decides which application ports
//! are attached, and serves the boundary. #11 runtime health is attached from the accepted
//! `RuntimeHealth` contract. Stored broker connections, account reads, authorization-gated
//! execution, and lifecycle recovery are built by the same production composition used in tests.
//!
//! TLS is terminated in front of this process (reverse proxy or platform load balancer);
//! the server itself binds plain HTTP on the loopback address by default, so a
//! misconfiguration cannot expose it unencrypted to a network.

use anyhow::{Context, anyhow};
use tracing::info;
use tracing_subscriber::EnvFilter;
use vox_core::composition::{ApplicationComposition, ServerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .try_init()
        .map_err(|error| anyhow!("initialize tracing: {error}"))?;

    let config = ServerConfig::from_env().context("load Vox server configuration")?;
    let address = config.bind;
    let composition = ApplicationComposition::build(config)
        .await
        .context("build production application composition")?;
    let app = composition.router();
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind {address}"))?;

    info!(
        %address,
        lifecycle_recovery = ?composition.lifecycle_recovery,
        "Vox application API listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve the Vox application API")?;
    composition.shutdown().await;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
