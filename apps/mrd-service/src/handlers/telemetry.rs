use crate::app_state::AppState;
use mrd_ipc::{AuditLogQuery, IpcResponse, TelemetryBundle};
use mrd_proto::SessionId;
use std::sync::Arc;

/// Query service-owned audit events.
pub async fn audit_log(app_state: &Arc<AppState>, query: AuditLogQuery) -> IpcResponse {
    let audit_log = app_state.audit_log.lock().await;
    IpcResponse::AuditLog {
        events: audit_log.query(&query),
    }
}

/// Return a compact telemetry bundle for a run/session.
///
/// The current service exposes the IPC contract before persistent telemetry
/// aggregation is wired in, so the bundle keeps the existing empty payload
/// semantics while centralizing the response construction.
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
