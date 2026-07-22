use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use mrd_proto::{DeviceId, SessionId};

use crate::app_state::AppState;

use super::peer_format::format_peer_transports;
use super::protocol::{
    LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT, LAN_DISPLAY_MODE_CONTROL_TRANSPORT,
    LAN_INPUT_CONTROL_TRANSPORT, LAN_REMOTE_POWER_CONTROL_TRANSPORT,
};

pub(super) async fn peer_control_addr_with_capture_source_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    peer_control_addr_with_transport_capability(
        app_state,
        peer_device_id,
        LAN_CAPTURE_SOURCE_CONTROL_TRANSPORT,
        "capture source control",
    )
    .await
}

pub(super) async fn peer_control_addr_with_display_mode_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    peer_control_addr_with_transport_capability(
        app_state,
        peer_device_id,
        LAN_DISPLAY_MODE_CONTROL_TRANSPORT,
        "display mode control",
    )
    .await
}

pub(super) async fn peer_control_addr_with_input_control_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    peer_control_addr_with_transport_capability(
        app_state,
        peer_device_id,
        LAN_INPUT_CONTROL_TRANSPORT,
        "input control",
    )
    .await
}

pub(super) async fn peer_control_addr_with_remote_power_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
) -> Result<SocketAddr> {
    peer_control_addr_with_transport_capability(
        app_state,
        peer_device_id,
        LAN_REMOTE_POWER_CONTROL_TRANSPORT,
        "remote power control",
    )
    .await
}

async fn peer_control_addr_with_transport_capability(
    app_state: &Arc<AppState>,
    peer_device_id: &DeviceId,
    required_transport: &str,
    label: &str,
) -> Result<SocketAddr> {
    let target = app_state
        .lan_discovery
        .peer_control_addr(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    let peer_transports = app_state
        .lan_discovery
        .peer_transports(peer_device_id)
        .await
        .with_context(|| format!("LAN peer not found: {}", peer_device_id.0))?;
    if !peer_transports
        .iter()
        .any(|transport| transport.eq_ignore_ascii_case(required_transport))
    {
        anyhow::bail!(
            "LAN peer does not advertise required {label} [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            required_transport,
            peer_device_id.0,
            format_peer_transports(&peer_transports)
        );
    }
    Ok(target)
}

pub(super) async fn session_remote_peer(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<DeviceId> {
    let sessions = app_state.sessions.lock().await;
    let snapshot = sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    snapshot
        .target_device_id
        .clone()
        .or_else(|| snapshot.source_device_id.clone())
        .with_context(|| format!("session has no remote peer: {}", session_id.0))
}

pub(super) async fn local_device_id(app_state: &Arc<AppState>) -> Result<String> {
    let devices = app_state.devices.lock().await;
    devices
        .get_local_device()
        .map(|(id, _)| id.0.clone())
        .context("local device is not registered")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
    use mrd_proto::{DeviceId, SessionId};

    use crate::app_state::AppState;

    use super::super::protocol::LAN_INPUT_CONTROL_TRANSPORT;
    use super::super::{
        now_ms, LanAnnouncement, DISCOVERY_APP_ID, DISCOVERY_MAGIC, PROTOCOL_VERSION,
    };

    async fn upsert_peer_with_transports(app_state: &Arc<AppState>, transports: Vec<String>) {
        app_state
            .lan_discovery
            .upsert_peer(
                LanAnnouncement {
                    magic: DISCOVERY_MAGIC.to_string(),
                    app_id: DISCOVERY_APP_ID.to_string(),
                    instance_id: "target-instance".to_string(),
                    device_id: "target-device".to_string(),
                    device_name: "Target Device".to_string(),
                    device_type: "rdesk".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    discovery_port: 21116,
                    transports,
                    service_build_id: None,
                    media_protocol_version: None,
                    media_capabilities: Vec::new(),
                    mac_address: None,
                    timestamp_ms: now_ms(),
                },
                "127.0.0.1:21116".parse().unwrap(),
            )
            .await;
    }

    fn session_snapshot(
        session_id: &SessionId,
        source_device_id: Option<DeviceId>,
        target_device_id: Option<DeviceId>,
    ) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id,
            target_device_id,
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Connected,
            last_error: None,
            sender_active: false,
            receiver_active: true,
        }
    }

    #[tokio::test]
    async fn input_control_peer_lookup_rejects_peer_without_transport_capability() {
        let app_state = Arc::new(AppState::new());
        upsert_peer_with_transports(&app_state, vec!["quic".to_string()]).await;

        let error = super::peer_control_addr_with_input_control_capability(
            &app_state,
            &DeviceId("target-device".to_string()),
        )
        .await
        .expect_err("missing input control transport should be rejected");

        let message = error.to_string();
        assert!(message.contains(LAN_INPUT_CONTROL_TRANSPORT));
        assert!(message.contains("target-device supports quic"));
    }

    #[tokio::test]
    async fn input_control_peer_lookup_returns_discovered_control_endpoint() {
        let app_state = Arc::new(AppState::new());
        upsert_peer_with_transports(
            &app_state,
            vec!["quic".to_string(), LAN_INPUT_CONTROL_TRANSPORT.to_string()],
        )
        .await;

        let addr = super::peer_control_addr_with_input_control_capability(
            &app_state,
            &DeviceId("target-device".to_string()),
        )
        .await
        .expect("input control peer endpoint");

        assert_eq!(addr.to_string(), "127.0.0.1:21116");
    }

    #[tokio::test]
    async fn session_remote_peer_prefers_target_then_source_device() {
        let app_state = Arc::new(AppState::new());
        let target_session = SessionId("target-session".to_string());
        let source_session = SessionId("source-session".to_string());
        app_state.sessions.lock().await.insert(
            target_session.clone(),
            session_snapshot(
                &target_session,
                Some(DeviceId("source-device".to_string())),
                Some(DeviceId("target-device".to_string())),
            ),
        );
        app_state.sessions.lock().await.insert(
            source_session.clone(),
            session_snapshot(
                &source_session,
                Some(DeviceId("source-device".to_string())),
                None,
            ),
        );

        assert_eq!(
            super::session_remote_peer(&app_state, &target_session)
                .await
                .expect("target peer")
                .0,
            "target-device"
        );
        assert_eq!(
            super::session_remote_peer(&app_state, &source_session)
                .await
                .expect("source peer")
                .0,
            "source-device"
        );
    }
}
