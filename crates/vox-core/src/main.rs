use anyhow::{Context, anyhow};
use tracing::info;
use tracing_subscriber::EnvFilter;
use vox_core::{CoreConfig, CoreRuntime};

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
    info!(
        environment = ?config.environment(),
        readiness = ?runtime.readiness().state(),
        "Vox Core Rust foundation configuration valid"
    );
    Ok(())
}
