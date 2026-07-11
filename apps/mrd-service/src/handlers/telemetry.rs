use crate::app_state::AppState;
use mrd_ipc::{AuditEventsQueryV2, AuditLogQuery, IpcResponse, ServiceStatus, TelemetryBundle};
use mrd_proto::SessionId;
use mrd_store_sqlite::StoreError;
use std::sync::Arc;

/// Query service-owned audit events.
pub async fn audit_log(app_state: &Arc<AppState>, query: AuditLogQuery) -> IpcResponse {
    let audit_log = app_state.audit_log.clone();
    match tokio::task::spawn_blocking(move || audit_log.query(&query)).await {
        Ok(Ok(events)) => IpcResponse::AuditLog { events },
        Ok(Err(StoreError::InvalidAuditQuery)) => IpcResponse::Error {
            code: "E_INVALID_AUDIT_QUERY".to_string(),
            message: "audit query is outside the supported bounds".to_string(),
        },
        Ok(Err(_)) | Err(_) => {
            app_state.mark_security_unhealthy();
            IpcResponse::Error {
                code: "E_SECURITY_STORE_UNAVAILABLE".to_string(),
                message: "authoritative security state is unavailable".to_string(),
            }
        }
    }
}

/// Query the durable, typed audit projection used by secure-remote product evidence.
pub async fn audit_events_v2(app_state: &Arc<AppState>, query: AuditEventsQueryV2) -> IpcResponse {
    let audit_log = app_state.audit_log.clone();
    match tokio::task::spawn_blocking(move || audit_log.query_v2(&query)).await {
        Ok(Ok(page)) => IpcResponse::AuditEventsV2 { page },
        Ok(Err(StoreError::InvalidAuditQuery)) => IpcResponse::Error {
            code: "E_INVALID_AUDIT_QUERY".to_string(),
            message: "audit query is outside the supported bounds".to_string(),
        },
        Ok(Err(_)) | Err(_) => {
            app_state.mark_security_unhealthy();
            IpcResponse::Error {
                code: "E_SECURITY_STORE_UNAVAILABLE".to_string(),
                message: "authoritative security state is unavailable".to_string(),
            }
        }
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

/// Return the basic service liveness contract used by UI/service probes.
pub fn service_health(app_state: &AppState) -> IpcResponse {
    let healthy = app_state.security_is_healthy();
    IpcResponse::ServiceHealth {
        status: ServiceStatus {
            running: true,
            healthy,
            pid: Some(std::process::id()),
        },
    }
}

/// Placeholder response for the reserved probe event streaming endpoint.
pub fn stream_probe_events() -> IpcResponse {
    IpcResponse::Error {
        code: "E501".to_string(),
        message: "Probe streaming not implemented yet".to_string(),
    }
}
