use crate::app_state::AppState;
use mrd_ipc::{IpcResponse, MediaProfile, ScenarioEvaluation, ScenarioEvaluationReason};
use mrd_proto::DeviceId;
use std::sync::Arc;

/// Return the cached local capability snapshot and trigger a background refresh.
pub async fn capability_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    let snapshot = app_state.cached_capability_snapshot().await;
    app_state.refresh_capability_snapshot_in_background();
    IpcResponse::CapabilitySnapshot { snapshot }
}

/// Evaluate a requested scenario/profile against current local capabilities.
pub async fn evaluate_scenario_profile(
    app_state: &Arc<AppState>,
    scenario_id: String,
    peer_device_id: Option<DeviceId>,
    requested_profile: Option<MediaProfile>,
) -> IpcResponse {
    if let Some(peer_device_id) = peer_device_id {
        let snapshot = app_state.lan_discovery.snapshot().await;
        if !snapshot
            .peers
            .iter()
            .any(|peer| peer.device_id == peer_device_id)
        {
            return IpcResponse::ScenarioProfileEvaluated {
                evaluation: peer_not_found_evaluation(scenario_id, peer_device_id),
            };
        }
    }

    let snapshot = app_state.cached_capability_snapshot().await;
    app_state.refresh_capability_snapshot_in_background();
    IpcResponse::ScenarioProfileEvaluated {
        evaluation: crate::capabilities::evaluate_scenario_profile_against_snapshot(
            &snapshot,
            &scenario_id,
            requested_profile,
        ),
    }
}

/// Return the capability snapshot advertised by one discovered LAN peer.
pub async fn peer_capability_snapshot(
    app_state: &Arc<AppState>,
    peer_device_id: DeviceId,
) -> IpcResponse {
    let snapshot = app_state.lan_discovery.snapshot().await;
    let capability_snapshot = snapshot
        .peers
        .iter()
        .find(|peer| peer.device_id == peer_device_id)
        .map(crate::capabilities::peer_capability_snapshot);
    IpcResponse::PeerCapabilitySnapshot {
        peer_device_id,
        snapshot: capability_snapshot,
    }
}

fn peer_not_found_evaluation(scenario_id: String, peer_device_id: DeviceId) -> ScenarioEvaluation {
    ScenarioEvaluation {
        scenario_id,
        status: mrd_ipc::ScenarioEvaluationStatus::Skipped,
        selected_profile: None,
        transport_kind: None,
        reasons: vec![ScenarioEvaluationReason {
            code: "peer_not_found".to_string(),
            severity: "warning".to_string(),
            message: format!("LAN peer {} is not currently discovered.", peer_device_id.0),
            capability_id: None,
        }],
        required_capabilities: Vec::new(),
        missing_capabilities: Vec::new(),
        fallback_profile: None,
    }
}
