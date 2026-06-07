use std::sync::Arc;

use mrd_ipc::{
    DeviceActionKind, DeviceActionResult, DeviceIdentitySnapshot, DeviceInfo, IpcResponse,
};
use mrd_proto::DeviceId;

use crate::app_state::AppState;

pub async fn register_device(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    device_name: String,
) -> IpcResponse {
    tracing::info!("Registering device: {} ({})", device_id.0, device_name);
    let mut devices = app_state.devices.lock().await;
    devices.register(device_id.clone(), device_name);
    IpcResponse::DeviceRegistered { device_id }
}

pub async fn list_devices(app_state: &Arc<AppState>) -> IpcResponse {
    let devices = app_state.devices.lock().await;
    let device_list = if let Some((id, name)) = devices.get_local_device() {
        vec![DeviceInfo {
            device_id: id.clone(),
            device_name: name.clone(),
            is_online: true,
        }]
    } else {
        Vec::new()
    };
    IpcResponse::DeviceList {
        devices: device_list,
    }
}

pub async fn pair_device(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    certificate_fingerprint: Option<String>,
) -> DeviceIdentitySnapshot {
    app_state
        .device_identities
        .lock()
        .await
        .upsert(device_id, certificate_fingerprint, "pending");
    identity_snapshot(app_state).await
}

pub async fn approve_pairing(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
) -> DeviceIdentitySnapshot {
    app_state
        .device_identities
        .lock()
        .await
        .upsert(device_id, None, "paired");
    identity_snapshot(app_state).await
}

pub async fn revoke_device(
    app_state: &Arc<AppState>,
    device_id: &DeviceId,
) -> DeviceIdentitySnapshot {
    app_state.device_identities.lock().await.revoke(device_id);
    identity_snapshot(app_state).await
}

pub async fn local_device_id(app_state: &Arc<AppState>) -> Option<DeviceId> {
    app_state
        .devices
        .lock()
        .await
        .get_local_device()
        .map(|(device_id, _)| device_id.clone())
}

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

pub async fn request_device_action(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    action: DeviceActionKind,
) -> IpcResponse {
    let known_device = app_state
        .lan_discovery
        .snapshot()
        .await
        .peers
        .iter()
        .any(|peer| peer.device_id == device_id)
        || local_device_id(app_state)
            .await
            .as_ref()
            .is_some_and(|local_id| local_id == &device_id)
        || app_state
            .device_identities
            .lock()
            .await
            .list()
            .iter()
            .any(|identity| identity.device_id == device_id);

    let (accepted, supported, message) = match action {
        DeviceActionKind::WakeOnLan => (
            known_device,
            false,
            if known_device {
                "Wake-on-LAN provider is reserved; no MAC address binding is available yet."
                    .to_string()
            } else {
                "Device is not known to the local service.".to_string()
            },
        ),
        DeviceActionKind::RemoteTerminal => (
            false,
            false,
            "Remote terminal requires a service-owned command channel and consent flow."
                .to_string(),
        ),
        DeviceActionKind::Restart => (
            false,
            false,
            "Remote restart requires a service-owned privileged action channel.".to_string(),
        ),
        DeviceActionKind::Shutdown => (
            false,
            false,
            "Remote shutdown requires a service-owned privileged action channel.".to_string(),
        ),
    };

    IpcResponse::DeviceActionRequested {
        result: DeviceActionResult {
            device_id,
            action,
            accepted,
            supported,
            message,
        },
    }
}
