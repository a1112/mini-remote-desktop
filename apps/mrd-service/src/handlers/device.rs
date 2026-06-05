use crate::app_state::AppState;
use mrd_ipc::IpcResponse;
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
