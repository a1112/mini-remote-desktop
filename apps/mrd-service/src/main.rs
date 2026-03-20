// mrd-service: Local session orchestration service
//
// This service runs as a local background process and handles:
// - Session orchestration (controller/agent)
// - Signaling communication
// - Media pipeline coordination
// - Transport management
// - IPC server for Rdesk UI shell

use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    info!("mrd-service starting...");

    // TODO: Initialize IPC server
    // TODO: Initialize application layer
    // TODO: Run service loop

    info!("mrd-service running (press Ctrl+C to stop)");

    // Run until interrupted
    tokio::signal::ctrl_c().await?;
    info!("mrd-service shutting down");

    Ok(())
}
