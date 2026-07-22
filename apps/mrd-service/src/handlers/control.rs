use crate::app_state::AppState;
use mrd_ipc::{IpcResponse, TransportPolicyConfig, TransportPolicySnapshot};
use mrd_proto::SessionId;
use std::sync::Arc;

/// Apply a transport policy request and return the selected route snapshot.
pub fn set_transport_policy(session_id: SessionId, policy: TransportPolicyConfig) -> IpcResponse {
    IpcResponse::TransportPolicyUpdated {
        snapshot: transport_policy_snapshot(Some(session_id), &policy),
    }
}

/// Return the service-owned control channel counters for a session.
pub async fn control_channel_snapshot(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    let snapshot = app_state.control_input().lock().await.snapshot(session_id);
    IpcResponse::ControlChannelSnapshot { snapshot }
}

fn transport_policy_snapshot(
    session_id: Option<SessionId>,
    policy: &TransportPolicyConfig,
) -> TransportPolicySnapshot {
    let mut candidates = Vec::new();
    if policy.allow_lan_quic {
        candidates.push("quic".to_string());
    }
    if policy.allow_webrtc {
        candidates.push("webrtc".to_string());
    }

    let preferred = policy.preferred_transport.as_deref();
    let selected = match preferred {
        Some("quic") if policy.allow_lan_quic => "quic",
        Some("webrtc") if policy.allow_webrtc => "webrtc",
        _ if policy.mode == "wan" && policy.allow_webrtc => "webrtc",
        _ if policy.allow_lan_quic => "quic",
        _ if policy.allow_webrtc => "webrtc",
        _ => "none",
    };

    let relay_required = selected == "webrtc" && policy.mode == "wan" && policy.allow_relay;
    let fallback_reason = preferred
        .filter(|preferred| *preferred != selected)
        .map(|preferred| {
            format!("{preferred} was requested but is not allowed by the active transport policy.")
        });

    TransportPolicySnapshot {
        session_id,
        mode: policy.mode.clone(),
        selected_transport: selected.to_string(),
        candidate_transports: candidates,
        relay_required,
        reason: Some(match selected {
            "quic" => "LAN/high-refresh route selected QUIC datagram media.".to_string(),
            "webrtc" if relay_required => {
                "WAN route selected WebRTC with relay allowed.".to_string()
            }
            "webrtc" => "WebRTC route selected by transport policy.".to_string(),
            _ => "No transport is allowed by the active transport policy.".to_string(),
        }),
        fallback_reason,
    }
}
