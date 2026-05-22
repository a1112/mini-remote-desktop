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
mod browser_webcodecs_preview;
mod browser_webrtc_preview;
mod capabilities;
mod capture_source;
mod display_mode;
mod handlers;
mod ipc_server;
mod lan_discovery;
mod media_adaptation;
mod shell;
mod web_bridge;

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

    let lan_discovery_config = lan_discovery::LanDiscoveryConfig::from_env()?;

    // Initialize application state with tray
    let app_state = Arc::new(AppState::with_tray_and_lan_discovery_config(
        tray.clone(),
        lan_discovery_config,
    ));
    {
        let (device_id, device_name) = app_state::default_lan_device_identity();
        let mut devices = app_state.devices.lock().await;
        if let Some((registered_id, registered_name)) =
            devices.register_if_unregistered(device_id, device_name)
        {
            info!(
                "Default LAN device registered: {} ({})",
                registered_id.0, registered_name
            );
        }
    }
    {
        let tray_available = tray.lock().unwrap().is_available();
        let mut shell = app_state.shell.lock().await;
        shell.tray_available = tray_available;
    }
    info!("Application state initialized");
    app_state.refresh_capability_snapshot_in_background();

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
    let web_bridge_task = web_bridge::spawn_from_env(ipc_server.clone()).await?;

    info!("mrd-service running (press Ctrl+C to stop)");

    // Start IPC server loop
    tokio::select! {
        result = ipc_server.run() => {
            if let Err(e) = result {
                eprintln!("IPC server error: {}", e);
            }
        }
        result = web_bridge::wait_for_task(web_bridge_task) => {
            if let Err(e) = result {
                eprintln!("Web bridge error: {}", e);
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
