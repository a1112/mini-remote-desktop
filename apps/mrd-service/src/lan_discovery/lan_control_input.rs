use super::protocol::LanDiscoveryPacket;
use crate::app_state::AppState;
use anyhow::{Context, Result};
use mrd_ipc::{CaptureSource, ControlInputEvent, ControlInputLane, DisplayMode};
use mrd_proto::SessionId;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct LanControlInputDedupeKey {
    source_device_id: String,
    session_id: String,
    event_id: u64,
}

#[derive(Debug, Clone)]
pub(super) struct LanControlInputAckState {
    pub accepted: bool,
    pub message: Option<String>,
    pub lane: Option<ControlInputLane>,
    pub event_count: u32,
    timestamp_ms: u64,
}

pub async fn request_lan_control_input(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    event: ControlInputEvent,
) -> Result<crate::control_input::ControlInputResult> {
    let peer_device_id = super::session_remote_peer(app_state, session_id).await?;
    let target =
        super::peer_control_addr_with_input_control_capability(app_state, &peer_device_id).await?;
    let source_device_id = super::local_device_id(app_state).await?;
    let event_id = next_control_input_event_id();

    let socket = UdpSocket::bind(("0.0.0.0", 0))
        .await
        .context("failed to bind LAN control input UDP socket")?;
    let packet = LanDiscoveryPacket::ControlInput {
        magic: super::DISCOVERY_MAGIC.to_string(),
        app_id: super::DISCOVERY_APP_ID.to_string(),
        instance_id: app_state.lan_discovery.instance_id.clone(),
        session_id: session_id.0.clone(),
        source_device_id,
        event_id,
        event: event.clone(),
        timestamp_ms: super::now_ms(),
    };

    let mut buffer = vec![0_u8; super::DISCOVERY_PACKET_BUFFER_BYTES];
    let attempts = control_input_request_attempts(&event);
    for attempt in 0..attempts {
        super::send_packet(&socket, &packet, target).await?;

        let received = timeout(
            super::LAN_CONTROL_INPUT_ACK_TIMEOUT,
            socket.recv_from(&mut buffer),
        )
        .await;
        let (len, _) = match received {
            Ok(received) => received?,
            Err(_) if attempt + 1 < attempts => continue,
            Err(_) => {
                anyhow::bail!(
                    "LAN control input request timed out after {} attempt(s)",
                    attempts
                );
            }
        };

        let ack: LanDiscoveryPacket = serde_json::from_slice(&buffer[..len])?;
        match ack {
            LanDiscoveryPacket::ControlInputAck {
                magic,
                app_id,
                session_id: ack_session_id,
                event_id: ack_event_id,
                accepted,
                message,
                lane,
                event_count,
                ..
            } if super::is_valid_discovery_packet(&magic, &app_id)
                && ack_session_id == session_id.0
                && ack_event_id == event_id =>
            {
                if accepted {
                    return Ok(crate::control_input::ControlInputResult {
                        lane: lane.context("LAN peer accepted control input without lane")?,
                        event_count,
                    });
                } else {
                    anyhow::bail!(
                        "LAN peer rejected control input: {}",
                        message.unwrap_or_else(|| "unknown reason".to_string())
                    );
                }
            }
            _ => anyhow::bail!("unexpected LAN control input response"),
        };
    }

    anyhow::bail!(
        "LAN control input request timed out after {} attempt(s)",
        attempts
    )
}

fn next_control_input_event_id() -> u64 {
    super::LAN_CONTROL_INPUT_EVENT_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1)
}

fn control_input_request_attempts(event: &ControlInputEvent) -> usize {
    match event {
        ControlInputEvent::MouseMove { .. }
        | ControlInputEvent::MouseWheel { .. }
        | ControlInputEvent::MouseHorizontalWheel { .. } => {
            super::LAN_CONTROL_INPUT_REALTIME_ATTEMPTS
        }
        ControlInputEvent::MouseButton { .. }
        | ControlInputEvent::Key { .. }
        | ControlInputEvent::ReleaseAll => super::LAN_CONTROL_INPUT_RELIABLE_ATTEMPTS,
    }
}

pub(super) async fn accept_or_replay_lan_control_input(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    source_device_id: &str,
    event_id: u64,
    event: &ControlInputEvent,
) -> LanControlInputAckState {
    let now = super::now_ms();
    let key = (event_id != 0).then(|| LanControlInputDedupeKey {
        source_device_id: source_device_id.to_string(),
        session_id: session_id.0.clone(),
        event_id,
    });
    if let Some(key) = key.as_ref() {
        let mut cache = app_state.lan_discovery.recent_control_inputs.lock().await;
        prune_recent_control_inputs(&mut cache, now);
        if let Some(cached) = cache.get(key).cloned() {
            return cached;
        }
    }

    let ack_state = match accept_lan_control_input(app_state, session_id, event).await {
        Ok(result) => LanControlInputAckState {
            accepted: true,
            message: Some("injected".to_string()),
            lane: Some(result.lane),
            event_count: result.event_count,
            timestamp_ms: now,
        },
        Err(error) => LanControlInputAckState {
            accepted: false,
            message: Some(error.to_string()),
            lane: None,
            event_count: 0,
            timestamp_ms: now,
        },
    };

    if let Some(key) = key {
        let mut cache = app_state.lan_discovery.recent_control_inputs.lock().await;
        cache.insert(key, ack_state.clone());
        prune_recent_control_inputs(&mut cache, now);
    }

    ack_state
}

async fn accept_lan_control_input(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    event: &ControlInputEvent,
) -> Result<crate::control_input::ControlInputResult> {
    {
        let sessions = app_state.sessions.lock().await;
        let snapshot = sessions
            .get(session_id)
            .with_context(|| format!("session not found: {}", session_id.0))?;
        if snapshot.lifecycle_state.is_terminal() {
            anyhow::bail!(
                "control input rejected for {} session",
                snapshot.lifecycle_state
            );
        }
        if !snapshot.sender_active {
            anyhow::bail!(
                "control input rejected until target session has an active sender: {}",
                session_id.0
            );
        }
    }

    let event = crate::control_input::map_control_input_event_for_target_geometry(
        event,
        control_input_target_geometry(app_state, session_id).await,
    );

    app_state
        .control_input()
        .lock()
        .await
        .handle_event(&event)
        .map_err(Into::into)
}

async fn control_input_target_geometry(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Option<crate::control_input::ControlInputTargetGeometry> {
    let selection = app_state.capture_sources.lock().await.get(session_id)?;
    let active_display_mode = app_state.display_modes.lock().await.active_mode(session_id);
    let (source_width, source_height) =
        control_input_source_size(&selection.source, active_display_mode.as_ref());
    let negotiation = app_state.media_profiles.lock().await.get(session_id);
    let frame_width = negotiation
        .as_ref()
        .map(|profile| profile.selected.width)
        .filter(|width| *width > 0)
        .unwrap_or(source_width);
    let frame_height = negotiation
        .as_ref()
        .map(|profile| profile.selected.height)
        .filter(|height| *height > 0)
        .unwrap_or(source_height);
    let (origin_x, origin_y) = control_input_source_origin(&selection.source);

    Some(crate::control_input::ControlInputTargetGeometry {
        frame_width,
        frame_height,
        source_width,
        source_height,
        origin_x,
        origin_y,
    })
}

fn control_input_source_size(
    source: &CaptureSource,
    active_display_mode: Option<&DisplayMode>,
) -> (u32, u32) {
    if is_display_capture_source(source) {
        if let Some(mode) = active_display_mode.filter(|mode| mode.width > 0 && mode.height > 0) {
            return (mode.width, mode.height);
        }
    }
    (source.width, source.height)
}

fn control_input_source_origin(source: &CaptureSource) -> (i32, i32) {
    if is_windows_display_capture_source(source) {
        crate::display_mode::display_origin_for_source_id(&source.id).unwrap_or((0, 0))
    } else {
        (0, 0)
    }
}

fn is_display_capture_source(source: &CaptureSource) -> bool {
    matches!(source.source_kind.as_str(), "display" | "display_shared")
}

fn is_windows_display_capture_source(source: &CaptureSource) -> bool {
    source.platform.eq_ignore_ascii_case("windows") && is_display_capture_source(source)
}

fn prune_recent_control_inputs(
    cache: &mut HashMap<LanControlInputDedupeKey, LanControlInputAckState>,
    now: u64,
) {
    let cutoff = now.saturating_sub(super::LAN_CONTROL_INPUT_DEDUPE_WINDOW_MS);
    cache.retain(|_, ack| ack.timestamp_ms >= cutoff);
    if cache.len() <= super::LAN_CONTROL_INPUT_DEDUPE_CACHE_LIMIT {
        return;
    }

    let remove_count = cache.len() - super::LAN_CONTROL_INPUT_DEDUPE_CACHE_LIMIT;
    let mut oldest = cache
        .iter()
        .map(|(key, ack)| (key.clone(), ack.timestamp_ms))
        .collect::<Vec<_>>();
    oldest.sort_by_key(|(_, timestamp_ms)| *timestamp_ms);
    for (key, _) in oldest.into_iter().take(remove_count) {
        cache.remove(&key);
    }
}
