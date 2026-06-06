use crate::app_state::AppState;
use mrd_ipc::{DevicePreferenceUpdate, IpcResponse, RemoteDevicePowerAction};
use mrd_proto::DeviceId;
use std::sync::Arc;

/// Register the local device in the service-owned device registry.
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

/// Return the service-owned local device list.
pub async fn list_devices(app_state: &Arc<AppState>) -> IpcResponse {
    let devices = app_state.devices.lock().await;
    let device_list = if let Some((id, name)) = devices.get_local_device() {
        vec![mrd_ipc::DeviceInfo {
            device_id: id.clone(),
            device_name: name.clone(),
            is_online: true,
        }]
    } else {
        vec![]
    };
    IpcResponse::DeviceList {
        devices: device_list,
    }
}

/// Return service-owned device preference flags.
pub async fn list_device_preferences(app_state: &Arc<AppState>) -> IpcResponse {
    let preferences = app_state.device_preferences.lock().await.list();
    IpcResponse::DevicePreferences { preferences }
}

/// Apply a partial service-owned preference update for one device.
pub async fn update_device_preference(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    update: DevicePreferenceUpdate,
) -> IpcResponse {
    let preference = app_state
        .device_preferences
        .lock()
        .await
        .update(device_id, update);
    IpcResponse::DevicePreferenceUpdated { preference }
}

/// Send a Wake-on-LAN magic packet for a known peer MAC address.
pub fn wake_on_lan(
    device_id: DeviceId,
    mac_address: String,
    broadcast_addr: Option<String>,
) -> IpcResponse {
    match crate::wake_on_lan::send_wake_on_lan(&mac_address, broadcast_addr.as_deref()) {
        Ok(result) => IpcResponse::WakeOnLanSent {
            device_id,
            mac_address: result.mac_address,
            broadcast_addr: result.broadcast_addr,
            packet_bytes: result.packet_bytes,
        },
        Err(error) => IpcResponse::Error {
            code: "E_WAKE_ON_LAN".to_string(),
            message: error.to_string(),
        },
    }
}

pub async fn request_remote_device_power_action(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    action: RemoteDevicePowerAction,
) -> IpcResponse {
    match crate::lan_discovery::request_lan_remote_device_power_action(
        app_state,
        &device_id,
        action.clone(),
    )
    .await
    {
        Ok(()) => IpcResponse::RemoteDevicePowerActionAccepted { device_id, action },
        Err(error) => IpcResponse::Error {
            code: "E_REMOTE_POWER".to_string(),
            message: error.to_string(),
        },
    }
}
