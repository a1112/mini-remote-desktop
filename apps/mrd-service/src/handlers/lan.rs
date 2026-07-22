use crate::app_state::AppState;
use mrd_ipc::IpcResponse;
use std::{sync::Arc, time::Duration};

const LAN_DISCOVERY_REFRESH_WAIT_MS: u64 = 450;

pub async fn lan_discovery_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    IpcResponse::LanDiscoverySnapshot {
        snapshot: app_state.lan_discovery.snapshot().await,
    }
}

pub async fn refresh_lan_discovery(app_state: &Arc<AppState>) -> IpcResponse {
    IpcResponse::LanDiscoverySnapshot {
        snapshot: app_state
            .lan_discovery
            .request_probe_and_wait(Duration::from_millis(LAN_DISCOVERY_REFRESH_WAIT_MS))
            .await,
    }
}
