use std::sync::Arc;

use mrd_application::ports::SessionSnapshot;
use mrd_ipc::{
    AuditLogQuery, ControlChannelSnapshot, IpcResponse, RuntimeSnapshot, ScenarioEvaluation,
    ScenarioEvaluationReason, ScenarioEvaluationStatus, SessionBootstrap, SessionRuntimeSnapshot,
    TelemetryBundle,
};
use mrd_proto::{DeviceId, SessionId};

use crate::app_state::AppState;

pub async fn runtime_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    let sessions = app_state.sessions.lock().await;
    let devices = app_state.devices.lock().await;

    let session_snapshots = sessions
        .list_all()
        .into_iter()
        .map(|snap| session_snapshot_to_ipc(&snap))
        .collect();

    let device_id = devices.get_local_device().map(|(id, _)| id.clone());

    IpcResponse::RuntimeSnapshot {
        snapshot: RuntimeSnapshot {
            sessions: session_snapshots,
            device_id,
            is_registered: devices.is_registered(),
        },
    }
}

pub async fn audit_log(app_state: &Arc<AppState>, query: AuditLogQuery) -> IpcResponse {
    let audit_log = app_state.audit_log.lock().await;
    IpcResponse::AuditLog {
        events: audit_log.query(&query),
    }
}

pub async fn capability_snapshot(app_state: &Arc<AppState>) -> IpcResponse {
    let snapshot = app_state.cached_capability_snapshot().await;
    app_state.refresh_capability_snapshot_in_background();
    IpcResponse::CapabilitySnapshot { snapshot }
}

pub async fn evaluate_scenario_profile(
    app_state: &Arc<AppState>,
    scenario_id: String,
    peer_device_id: Option<DeviceId>,
    requested_profile: Option<mrd_ipc::MediaProfile>,
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

pub async fn control_channel_snapshot(
    app_state: &Arc<AppState>,
    session_id: SessionId,
) -> IpcResponse {
    let snapshot: ControlChannelSnapshot =
        app_state.control_input().lock().await.snapshot(session_id);
    IpcResponse::ControlChannelSnapshot { snapshot }
}

pub fn telemetry_bundle(run_id: String, session_id: Option<SessionId>) -> IpcResponse {
    IpcResponse::TelemetryBundle {
        bundle: TelemetryBundle {
            run_id,
            session_id,
            metrics: Vec::new(),
            event_count: 0,
            log_count: 0,
            artifacts: Vec::new(),
        },
    }
}

pub fn service_health() -> IpcResponse {
    IpcResponse::ServiceHealth {
        status: mrd_ipc::ServiceStatus {
            running: true,
            healthy: true,
            pid: Some(std::process::id()),
        },
    }
}

fn session_snapshot_to_ipc(snap: &SessionSnapshot) -> SessionRuntimeSnapshot {
    let role = if snap.target_device_id.is_some() {
        "controller"
    } else if snap.source_device_id.is_some() {
        "agent"
    } else {
        "unknown"
    }
    .to_string();

    SessionRuntimeSnapshot {
        session_id: snap.session_id.clone(),
        role,
        state: snap.lifecycle_state.as_str().to_string(),
        transport_kind: snap.transport.clone(),
        local_bootstrap: if snap.local_listen_addr.is_some() || snap.local_server_name.is_some() {
            Some(SessionBootstrap {
                listen_addr: snap.local_listen_addr.clone(),
                server_name: snap.local_server_name.clone(),
                cert_der: snap.local_cert_der_b64.clone(),
            })
        } else {
            None
        },
        remote_bootstrap: if snap.remote_listen_addr.is_some() || snap.remote_server_name.is_some()
        {
            Some(SessionBootstrap {
                listen_addr: snap.remote_listen_addr.clone(),
                server_name: snap.remote_server_name.clone(),
                cert_der: snap.remote_cert_der_b64.clone(),
            })
        } else {
            None
        },
        last_error: snap.last_error.clone(),
        sender_active: snap.sender_active,
        receiver_active: snap.receiver_active,
    }
}

fn peer_not_found_evaluation(scenario_id: String, peer_device_id: DeviceId) -> ScenarioEvaluation {
    ScenarioEvaluation {
        scenario_id,
        status: ScenarioEvaluationStatus::Skipped,
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
