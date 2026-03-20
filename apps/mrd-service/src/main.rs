// mrd-service: Local session orchestration service
//
// This service runs as a local background process and handles:
// - Session orchestration (controller/agent)
// - Signaling communication
// - Media pipeline coordination
// - Transport management
// - IPC server for Rdesk UI shell

mod app_state;
mod handlers;
mod ipc_server;

use anyhow::Result;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;
use ipc_server::IpcServer;
use app_state::AppState;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    info!("mrd-service starting...");

    // Initialize application state
    let app_state = Arc::new(AppState::new());
    info!("Application state initialized");

    // Initialize IPC server with app state
    let ipc_server = IpcServer::new(app_state);
    info!("IPC server initialized");

    info!("mrd-service running (press Ctrl+C to stop)");

    // Start IPC server loop
    tokio::select! {
        result = ipc_server.run() => {
            if let Err(e) = result {
                eprintln!("IPC server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Shutdown requested");
        }
    }

    info!("mrd-service shutting down");
    Ok(())
}
