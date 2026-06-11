use crate::app_state::AppState;
use mrd_ipc::{
    CapabilitySnapshot, CapabilityStatus, MediaProfile, ScenarioEvaluation,
    ScenarioEvaluationStatus,
};
use mrd_proto::DeviceId;
use std::sync::Arc;

/// Validate capability and peer prerequisites before starting a session.
pub async fn preflight_session_start(
    app_state: &Arc<AppState>,
    target_device_id: &DeviceId,
    transport_kind: &str,
    requested_profile: Option<&MediaProfile>,
    require_lan_peer: bool,
) -> Result<(), String> {
    let snapshot = app_state.cached_capability_snapshot().await;
    app_state.refresh_capability_snapshot_in_background();

    ensure_transport_preflight(&snapshot, transport_kind)?;

    if let Some(profile) = requested_profile {
        let scenario_id = scenario_id_for_profile(profile);
        let evaluation = crate::capabilities::evaluate_scenario_profile_against_snapshot(
            &snapshot,
            scenario_id,
            Some(profile.clone()),
        );
        if matches!(evaluation.status, ScenarioEvaluationStatus::Blocked) {
            return Err(format_preflight_evaluation_failure(&evaluation));
        }
    }

    if require_lan_peer {
        let discovery = app_state.lan_discovery.snapshot().await;
        if !discovery
            .peers
            .iter()
            .any(|peer| &peer.device_id == target_device_id)
        {
            return Err(format!(
                "LAN peer {} was not found during session preflight.",
                target_device_id.0
            ));
        }
    }

    Ok(())
}

fn ensure_transport_preflight(
    snapshot: &CapabilitySnapshot,
    transport_kind: &str,
) -> Result<(), String> {
    let capability_id = transport_capability_id(transport_kind);
    let Some(capability) = snapshot
        .capabilities
        .iter()
        .find(|item| item.id == capability_id)
    else {
        return Err(format!(
            "{capability_id} is not advertised by local service capability preflight."
        ));
    };

    if capability_status_runs(&capability.status) {
        return Ok(());
    }

    Err(format!(
        "{} preflight failed: {}",
        capability.id,
        capability.reason.clone().unwrap_or_else(|| {
            format!("status {:?} cannot start this session.", capability.status)
        })
    ))
}

fn transport_capability_id(transport_kind: &str) -> &'static str {
    let kind = transport_kind.to_ascii_lowercase();
    if kind.contains("webrtc") {
        "transport.webrtc"
    } else if kind.contains("quic_datagram") {
        "transport.quic_datagram"
    } else if kind.contains("quic") {
        "transport.quic"
    } else {
        "transport.loopback"
    }
}

/// Select the closest built-in scenario id for media profile preflight.
pub fn scenario_id_for_profile(profile: &MediaProfile) -> &'static str {
    if cfg!(target_os = "macos") && profile.codec.eq_ignore_ascii_case("hevc") {
        return "lan.macos.hevc.2k144";
    }
    if cfg!(target_os = "macos") && profile.codec.eq_ignore_ascii_case("h264") {
        return "lan.macos.2k144";
    }
    if profile.width >= 3840 || profile.height >= 2160 {
        "quality.4k60"
    } else if profile.height >= 1600 && profile.fps >= 165 {
        "lan.1600p165"
    } else if profile.width >= 2560 && profile.height >= 1440 && profile.fps >= 144 {
        "lan.2k144"
    } else {
        "interactive.1080p60"
    }
}

fn format_preflight_evaluation_failure(evaluation: &ScenarioEvaluation) -> String {
    let mut parts = vec![format!(
        "Scenario {} was blocked by session preflight.",
        evaluation.scenario_id
    )];
    if !evaluation.missing_capabilities.is_empty() {
        parts.push(format!(
            "missing capabilities: {}",
            evaluation.missing_capabilities.join(", ")
        ));
    }
    for reason in &evaluation.reasons {
        if reason.severity == "error" {
            parts.push(reason.message.clone());
        }
    }
    parts.join(" ")
}

fn capability_status_runs(status: &CapabilityStatus) -> bool {
    matches!(
        status,
        CapabilityStatus::Available
            | CapabilityStatus::Usable
            | CapabilityStatus::Supported
            | CapabilityStatus::Degraded
    )
}
