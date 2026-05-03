// mrd-service: Local session orchestration service
//
// This service runs as a local background process and handles:
// - Session orchestration (controller/agent)
// - Signaling communication
// - Media pipeline coordination
// - Transport management
// - IPC server for Rdesk UI shell
// - Shell lifecycle (UI launcher, tray, autostart)

mod app_state;
mod capture_source;
mod handlers;
mod ipc_server;
mod lan_discovery;
mod shell;

use anyhow::Result;
use app_state::AppState;
use ipc_server::IpcServer;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    info!("mrd-service starting...");

    // Initialize tray (Phase 4)
    let tray: Arc<std::sync::Mutex<dyn shell::TrayPort + Send + Sync>> = shell::default_tray();

    // Initialize application state with tray
    let app_state = Arc::new(AppState::with_tray(tray.clone()));
    {
        let tray_available = tray.lock().unwrap().is_available();
        let mut shell = app_state.shell.lock().await;
        shell.tray_available = tray_available;
    }
    info!("Application state initialized");

    match lan_discovery::start_lan_discovery(app_state.clone()).await {
        Ok(()) => info!("LAN peer discovery started"),
        Err(error) => {
            let mut shell = app_state.shell.lock().await;
            shell.last_error = Some(format!("LAN discovery failed: {error}"));
            info!("LAN peer discovery unavailable: {}", error);
        }
    }

    // Install tray with initial model
    let initial_model = shell::TrayModel::default();
    if let Err(e) = tray.lock().unwrap().install(initial_model) {
        info!("Tray not available: {}", e);
    } else {
        info!("Tray installed");
    }

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

    // Shutdown tray
    info!("Shutting down tray...");
    let _ = tray.lock().unwrap().shutdown();

    info!("mrd-service shutting down");
    Ok(())
}
