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
    let healthy = app_state.security_is_healthy() && !app_state.shutdown.is_requested();
    IpcResponse::ServiceHealth {
        status: ServiceStatus {
            running: true,
            healthy,
            pid: Some(std::process::id()),
            agent_readiness: active_console_agent_readiness(app_state),
        },
    }
}

#[cfg(windows)]
fn active_console_agent_readiness(app_state: &AppState) -> Option<mrd_ipc::AgentReadinessStatus> {
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;

    let console = unsafe { WTSGetActiveConsoleSessionId() };
    let console_active = console != 0 && console != u32::MAX;
    let snapshot = console_active
        .then(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
                .and_then(|now| app_state.agent_registry.active_for_session_at(console, now))
        })
        .flatten();
    Some(project_agent_readiness(
        console_active,
        snapshot.as_ref().map(|agent| {
            (
                agent.identity.protocol_minor,
                agent.health,
                &agent.capabilities.capabilities,
            )
        }),
    ))
}

#[cfg(not(windows))]
fn active_console_agent_readiness(_app_state: &AppState) -> Option<mrd_ipc::AgentReadinessStatus> {
    None
}

#[cfg(any(windows, test))]
fn project_agent_readiness(
    console_active: bool,
    registration: Option<(
        u16,
        crate::agent_runtime::AgentHealth,
        &std::collections::BTreeSet<mrd_agent_ipc::AgentCapability>,
    )>,
) -> mrd_ipc::AgentReadinessStatus {
    use crate::agent_runtime::AgentHealth;
    use mrd_agent_ipc::AgentCapability;

    let registration = registration.filter(|_| console_active);
    let advertised = |capability| {
        registration.is_some_and(|(_, _, capabilities)| capabilities.contains(&capability))
    };
    mrd_ipc::AgentReadinessStatus {
        console_active,
        registered: registration.is_some(),
        heartbeat_fresh: registration.is_some_and(|(_, health, _)| health == AgentHealth::Healthy),
        capture_advertised: advertised(AgentCapability::Capture),
        consent_advertised: advertised(AgentCapability::Consent),
        input_advertised: advertised(AgentCapability::Input),
        protocol_minor: registration.map(|(minor, _, _)| minor),
    }
}

#[cfg(test)]
mod agent_readiness_tests {
    use super::{project_agent_readiness, service_health};
    use crate::{agent_runtime::AgentHealth, app_state::AppState};
    use mrd_agent_ipc::AgentCapability;
    use mrd_ipc::{AgentReadinessStatus, IpcResponse};
    use std::collections::BTreeSet;

    fn unregistered(console_active: bool) -> AgentReadinessStatus {
        AgentReadinessStatus {
            console_active,
            registered: false,
            heartbeat_fresh: false,
            capture_advertised: false,
            consent_advertised: false,
            input_advertised: false,
            protocol_minor: None,
        }
    }

    #[test]
    fn missing_registration_does_not_claim_readiness_or_protocol() {
        assert_eq!(project_agent_readiness(true, None), unregistered(true));
    }

    #[test]
    fn no_active_console_does_not_project_a_registration() {
        let capabilities = BTreeSet::from([AgentCapability::Capture, AgentCapability::Consent]);
        assert_eq!(
            project_agent_readiness(false, Some((5, AgentHealth::Healthy, &capabilities))),
            unregistered(false)
        );
    }

    #[test]
    fn stale_heartbeat_preserves_registration_and_advertised_capabilities() {
        let capabilities = BTreeSet::from([
            AgentCapability::Capture,
            AgentCapability::Consent,
            AgentCapability::Input,
        ]);
        assert_eq!(
            project_agent_readiness(true, Some((5, AgentHealth::Unresponsive, &capabilities))),
            AgentReadinessStatus {
                console_active: true,
                registered: true,
                heartbeat_fresh: false,
                capture_advertised: true,
                consent_advertised: true,
                input_advertised: true,
                protocol_minor: Some(5),
            }
        );
    }

    #[test]
    fn empty_capabilities_do_not_claim_capture_consent_or_input() {
        let status =
            project_agent_readiness(true, Some((5, AgentHealth::Healthy, &BTreeSet::new())));
        assert!(status.registered && status.heartbeat_fresh);
        assert!(
            !status.capture_advertised && !status.consent_advertised && !status.input_advertised
        );
        assert_eq!(status.protocol_minor, Some(5));
    }

    #[test]
    fn capabilities_are_projected_independently() {
        for capability in [
            AgentCapability::Capture,
            AgentCapability::Consent,
            AgentCapability::Input,
        ] {
            let capabilities = BTreeSet::from([capability]);
            let status =
                project_agent_readiness(true, Some((5, AgentHealth::Healthy, &capabilities)));
            assert!(status.registered && status.heartbeat_fresh);
            assert_eq!(
                status.capture_advertised,
                capability == AgentCapability::Capture
            );
            assert_eq!(
                status.consent_advertised,
                capability == AgentCapability::Consent
            );
            assert_eq!(
                status.input_advertised,
                capability == AgentCapability::Input
            );
        }
    }

    #[test]
    fn readiness_does_not_change_service_health_semantics() {
        let state = AppState::new();
        for unhealthy in [false, true] {
            if unhealthy {
                state.mark_security_unhealthy();
            }
            let IpcResponse::ServiceHealth { status } = service_health(&state) else {
                panic!("expected service health");
            };
            assert!(status.running);
            assert_eq!(status.healthy, !unhealthy);
            #[cfg(windows)]
            assert!(!status.agent_readiness.unwrap().registered);
            #[cfg(not(windows))]
            assert!(status.agent_readiness.is_none());
        }
    }
}

/// Placeholder response for the reserved probe event streaming endpoint.
pub fn stream_probe_events() -> IpcResponse {
    IpcResponse::Error {
        code: "E501".to_string(),
        message: "Probe streaming not implemented yet".to_string(),
    }
}
