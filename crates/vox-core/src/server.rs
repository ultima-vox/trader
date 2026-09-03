//! Single canonical Vox Trader server entry point.
//!
//! Booting the application is one code path shared by the `vox-core` default binary and the
//! explicitly-named `vox-server` binary, so a clean `cargo run -p vox-core` and
//! `cargo run --bin vox-server` both start the real API. TLS is terminated in front of this
//! process (reverse proxy); the server binds plain HTTP on loopback by default.

use anyhow::{Context, anyhow};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::composition::{ApplicationComposition, ServerConfig};

/// Loads configuration, builds the production composition, binds, and serves until Ctrl-C.
pub async fn run_server() -> anyhow::Result<()> {
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
