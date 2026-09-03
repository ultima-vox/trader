//! Vox Trader canonical entry point.
//!
//! A clean `cargo run -p vox-core` starts the real server (see `vox_core::server::run_server`).
use vox_core::server::run_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_server().await
}