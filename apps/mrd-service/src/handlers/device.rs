use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use mrd_application::ports::SessionLifecycleState;
use mrd_ipc::{
    ControlInputEvent, DeviceActionKind, DeviceActionResult, DeviceDetailSnapshot,
    DeviceIdentitySnapshot, DeviceInfo, IpcResponse,
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

pub async fn detail_snapshot(app_state: &Arc<AppState>, device_id: DeviceId) -> IpcResponse {
    let local_device = app_state
        .devices
        .lock()
        .await
        .get_local_device()
        .map(|(id, name)| (id.clone(), name.clone()));
    let local_name = local_device
        .as_ref()
        .filter(|(id, _)| id == &device_id)
        .map(|(_, name)| name.clone());
    let is_local = local_name.is_some();

    let lan_peer = app_state
        .lan_discovery
        .snapshot()
        .await
        .peers
        .into_iter()
        .find(|peer| peer.device_id == device_id);

    let paired_devices = app_state.device_identities.lock().await.list();
    let paired = paired_devices
        .iter()
        .find(|identity| identity.device_id == device_id);

    let device_name = lan_peer
        .as_ref()
        .map(|peer| peer.device_name.clone())
        .or(local_name)
        .or_else(|| paired.map(|identity| identity.display_name.clone()));

    IpcResponse::DeviceDetail {
        detail: DeviceDetailSnapshot {
            device_id,
            device_name,
            is_local,
            is_online: is_local || lan_peer.is_some(),
            is_lan_peer: lan_peer.is_some(),
            is_paired: paired.is_some(),
            discovery_port: lan_peer.as_ref().map(|peer| peer.discovery_port),
            p2p_control_addr: lan_peer.as_ref().map(|peer| peer.p2p_control_addr.clone()),
            transports: lan_peer
                .as_ref()
                .map(|peer| peer.transports.clone())
                .unwrap_or_default(),
            media_capabilities: lan_peer
                .as_ref()
                .map(|peer| peer.media_capabilities.clone())
                .unwrap_or_default(),
            age_ms: lan_peer.as_ref().map(|peer| peer.age_ms),
            service_build_id: lan_peer
                .as_ref()
                .and_then(|peer| peer.service_build_id.clone()),
            media_protocol_version: lan_peer
                .as_ref()
                .and_then(|peer| peer.media_protocol_version),
        },
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
        DeviceActionKind::WakeOnLan => match app_state
            .lan_discovery
            .peer_wake_mac_address(&device_id)
            .await
        {
            Some(mac_address) => match send_wake_on_lan_magic_packet(&mac_address).await {
                Ok(()) => (
                    true,
                    true,
                    format!("Wake-on-LAN magic packet sent to {mac_address}."),
                ),
                Err(error) => (false, true, error.to_string()),
            },
            None if known_device => (
                false,
                false,
                "Wake-on-LAN requires the peer to advertise MRD_WAKE_MAC_ADDRESS.".to_string(),
            ),
            None => (
                false,
                false,
                "Device is not known to the local service.".to_string(),
            ),
        },
        DeviceActionKind::Disconnect => {
            return IpcResponse::DeviceActionRequested {
                result: disconnect_device_sessions(app_state, device_id, known_device).await,
            };
        }
        DeviceActionKind::RemoteTerminal
        | DeviceActionKind::Restart
        | DeviceActionKind::Shutdown => {
            match crate::lan_discovery::request_lan_device_action(app_state, &device_id, action)
                .await
            {
                Ok(result) => {
                    return IpcResponse::DeviceActionRequested { result };
                }
                Err(error) if known_device => (
                    false,
                    false,
                    format!("Remote device action command channel is unavailable: {error}"),
                ),
                Err(_) => (
                    false,
                    false,
                    "Device is not known to the local service.".to_string(),
                ),
            }
        }
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

async fn disconnect_device_sessions(
    app_state: &Arc<AppState>,
    device_id: DeviceId,
    known_device: bool,
) -> DeviceActionResult {
    let session_ids = {
        let sessions = app_state.sessions.lock().await;
        sessions
            .list_all()
            .into_iter()
            .filter(|session| {
                !matches!(
                    session.lifecycle_state,
                    SessionLifecycleState::Closed | SessionLifecycleState::Failed { .. }
                ) && (session.source_device_id.as_ref() == Some(&device_id)
                    || session.target_device_id.as_ref() == Some(&device_id))
            })
            .map(|session| session.session_id)
            .collect::<Vec<_>>()
    };

    for session_id in &session_ids {
        let mut sessions = app_state.sessions.lock().await;
        if let Some(snapshot) = sessions.get(session_id).cloned() {
            sessions.insert(
                session_id.clone(),
                mrd_application::ports::SessionSnapshot {
                    lifecycle_state: SessionLifecycleState::Closed,
                    last_error: None,
                    sender_active: false,
                    receiver_active: false,
                    ..snapshot
                },
            );
        }
        drop(sessions);
        if let Err(error) = app_state
            .control_input()
            .lock()
            .await
            .handle_event(&ControlInputEvent::ReleaseAll)
        {
            tracing::warn!(
                session_id = %session_id.0,
                %error,
                "failed to release active control input while disconnecting device"
            );
        }
        app_state.media_tasks.lock().await.abort_session(session_id);
        app_state.media_profiles.lock().await.remove(session_id);
        app_state.capture_sources.lock().await.remove(session_id);
        app_state
            .peer_media_capabilities
            .lock()
            .await
            .remove(session_id);
        #[cfg(windows)]
        app_state
            .media_surface_renderers
            .lock()
            .await
            .detach_session(session_id);
        app_state.media_pipelines.lock().await.remove(session_id);
    }

    DeviceActionResult {
        device_id,
        action: DeviceActionKind::Disconnect,
        accepted: !session_ids.is_empty(),
        supported: known_device || !session_ids.is_empty(),
        message: if session_ids.is_empty() && !known_device {
            "Device is not known to the local service.".to_string()
        } else if session_ids.is_empty() {
            "No active sessions are associated with this device.".to_string()
        } else {
            format!("Disconnected {} active session(s).", session_ids.len())
        },
    }
}

async fn send_wake_on_lan_magic_packet(mac_address: &str) -> anyhow::Result<()> {
    let packet = wake_on_lan_magic_packet(mac_address)?;
    let socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)).await?;
    socket.set_broadcast(true)?;
    socket
        .send_to(&packet, SocketAddrV4::new(Ipv4Addr::BROADCAST, 9))
        .await?;
    Ok(())
}

fn wake_on_lan_magic_packet(mac_address: &str) -> anyhow::Result<[u8; 102]> {
    let mac = parse_mac_address(mac_address)?;
    let mut packet = [0xFF_u8; 102];
    for chunk in packet[6..].chunks_exact_mut(6) {
        chunk.copy_from_slice(&mac);
    }
    Ok(packet)
}

fn parse_mac_address(mac_address: &str) -> anyhow::Result<[u8; 6]> {
    let hex = mac_address
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    if hex.len() != 12 {
        anyhow::bail!("Wake-on-LAN MAC address must contain 12 hex digits");
    }

    let mut mac = [0_u8; 6];
    for (index, chunk_start) in (0..hex.len()).step_by(2).enumerate() {
        mac[index] = u8::from_str_radix(&hex[chunk_start..chunk_start + 2], 16)?;
    }
    Ok(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_on_lan_magic_packet_repeats_mac_sixteen_times() {
        let packet = wake_on_lan_magic_packet("AA:bb-CC:dd-EE:ff").expect("magic packet");

        assert_eq!(&packet[..6], &[0xFF; 6]);
        for chunk in packet[6..].chunks_exact(6) {
            assert_eq!(chunk, &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        }
    }
}
