use crate::app_state::AppState;
use mrd_ipc::{DeviceIdentitySnapshot, IpcResponse};
use mrd_proto::DeviceId;
use std::sync::Arc;

/// Create or refresh a pending pairing record for a peer device.
pub async fn pair_device(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    certificate_fingerprint: Option<String>,
) -> IpcResponse {
    app_state
        .device_identities
        .lock()
        .await
        .upsert(device_id, certificate_fingerprint, "pending");
    IpcResponse::PairingUpdated {
        snapshot: identity_snapshot(app_state).await,
    }
}

/// Mark a known peer identity as paired while preserving its pinned fingerprint.
pub async fn approve_pairing(app_state: &Arc<AppState>, device_id: DeviceId) -> IpcResponse {
    app_state
        .device_identities
        .lock()
        .await
        .upsert(device_id, None, "paired");
    IpcResponse::PairingUpdated {
        snapshot: identity_snapshot(app_state).await,
    }
}

/// Revoke trust for a peer identity.
pub async fn revoke_device(app_state: &Arc<AppState>, device_id: DeviceId) -> IpcResponse {
    app_state.device_identities.lock().await.revoke(&device_id);
    IpcResponse::PairingUpdated {
        snapshot: identity_snapshot(app_state).await,
    }
}

/// Return the current local identity and known peer trust state.
pub async fn identity_snapshot(app_state: &Arc<AppState>) -> DeviceIdentitySnapshot {
    let devices = app_state.devices.lock().await;
    let (local_device_id, display_name) = devices
        .get_local_device()
        .map(|(device_id, name)| (Some(device_id.clone()), Some(name.clone())))
        .unwrap_or((None, None));
    drop(devices);
    let paired_devices = app_state.device_identities.lock().await.list();
    DeviceIdentitySnapshot {
        local_device_id,
        display_name,
        certificate_fingerprint: None,
        consent_required: true,
        paired_devices,
    }
}

/// Wrap the identity snapshot in its IPC response contract.
pub async fn get_device_identity_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    IpcResponse::DeviceIdentitySnapshot {
        snapshot: identity_snapshot(app_state).await,
    }
}
