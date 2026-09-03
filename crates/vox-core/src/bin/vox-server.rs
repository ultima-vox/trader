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
//!
//! This binary is the explicit server name; `cargo run -p vox-core` (the `main` binary)
//! shares the same `vox_core::server::run_server` boot path.

use vox_core::server::run_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_server().await
}