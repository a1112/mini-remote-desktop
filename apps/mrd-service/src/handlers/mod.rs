// Handler modules for mrd-service

pub mod capability;
pub mod control;
pub mod device;
pub mod identity;
pub mod lan;
pub mod preflight;
pub mod session;
pub mod shell;
pub mod telemetry;
pub mod transport;

#[cfg(test)]
mod tests {
    use crate::app_state::AppState;
    use mrd_ipc::{
        AuditLogQuery, CapabilityStatus, ControlChannelReliability, IpcResponse,
        RemoteDevicePowerAction, ScenarioEvaluationStatus, TransportPolicyConfig, UiDetachReason,
    };
    use mrd_proto::{DeviceId, SessionId};
    use std::sync::Arc;

    #[tokio::test]
    async fn device_handler_lists_registered_local_device() {
        let app_state = Arc::new(AppState::new());
        super::device::register_device(
            &app_state,
            DeviceId("local-device".to_string()),
            "Local Device".to_string(),
        )
        .await;

        let response = super::device::list_devices(&app_state).await;

        match response {
            IpcResponse::DeviceList { devices } => {
                assert_eq!(devices.len(), 1);
                assert_eq!(devices[0].device_id, DeviceId("local-device".to_string()));
                assert_eq!(devices[0].device_name, "Local Device");
                assert!(devices[0].is_online);
            }
            _ => panic!("expected device list response"),
        }
    }

    #[tokio::test]
    async fn lan_handler_returns_discovery_snapshot() {
        let app_state = Arc::new(AppState::new());

        let response = super::lan::lan_discovery_snapshot(&app_state).await;

        match response {
            IpcResponse::LanDiscoverySnapshot { snapshot } => {
                assert!(snapshot.enabled);
                assert!(snapshot.peers.is_empty());
            }
            _ => panic!("expected LAN discovery snapshot response"),
        }
    }

    #[tokio::test]
    async fn identity_handler_updates_pairing_state_and_snapshot() {
        let app_state = Arc::new(AppState::new());
        super::device::register_device(
            &app_state,
            DeviceId("local-device".to_string()),
            "Local Device".to_string(),
        )
        .await;
        let peer_device_id = DeviceId("peer-device".to_string());

        let pair_response = super::identity::pair_device(
            &app_state,
            peer_device_id.clone(),
            Some("sha256:peer".to_string()),
        )
        .await;

        match pair_response {
            IpcResponse::PairingUpdated { snapshot } => {
                assert_eq!(
                    snapshot.local_device_id,
                    Some(DeviceId("local-device".to_string()))
                );
                assert_eq!(snapshot.display_name.as_deref(), Some("Local Device"));
                assert_eq!(snapshot.paired_devices.len(), 1);
                assert_eq!(snapshot.paired_devices[0].device_id, peer_device_id);
                assert_eq!(
                    snapshot.paired_devices[0]
                        .certificate_fingerprint
                        .as_deref(),
                    Some("sha256:peer")
                );
                assert_eq!(snapshot.paired_devices[0].trust_status, "pending");
            }
            _ => panic!("expected pairing updated response"),
        }

        let approve_response =
            super::identity::approve_pairing(&app_state, peer_device_id.clone()).await;
        match approve_response {
            IpcResponse::PairingUpdated { snapshot } => {
                assert_eq!(snapshot.paired_devices.len(), 1);
                assert_eq!(snapshot.paired_devices[0].trust_status, "paired");
                assert_eq!(
                    snapshot.paired_devices[0]
                        .certificate_fingerprint
                        .as_deref(),
                    Some("sha256:peer")
                );
            }
            _ => panic!("expected pairing updated response"),
        }

        let revoke_response = super::identity::revoke_device(&app_state, peer_device_id).await;
        match revoke_response {
            IpcResponse::PairingUpdated { snapshot } => {
                assert_eq!(snapshot.paired_devices.len(), 1);
                assert_eq!(snapshot.paired_devices[0].trust_status, "revoked");
            }
            _ => panic!("expected pairing updated response"),
        }

        let snapshot = super::identity::identity_snapshot(&app_state).await;
        assert_eq!(
            snapshot.local_device_id,
            Some(DeviceId("local-device".to_string()))
        );
        assert_eq!(snapshot.display_name.as_deref(), Some("Local Device"));
        assert!(snapshot.consent_required);
    }

    #[tokio::test]
    async fn telemetry_handler_returns_audit_log_and_empty_bundle_contract() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("session-a".to_string());
        {
            let mut audit_log = app_state.audit_log.lock().await;
            audit_log.record(
                "control_input",
                "accepted",
                Some(session_id.clone()),
                Some(DeviceId("local-device".to_string())),
                Some(DeviceId("peer-device".to_string())),
                Some("lan".to_string()),
                None,
                vec![("sequence".to_string(), "41".to_string())],
            );
            audit_log.record(
                "session",
                "started",
                Some(session_id.clone()),
                None,
                None,
                Some("lan".to_string()),
                None,
                Vec::new(),
            );
        }

        let audit_response = super::telemetry::audit_log(
            &app_state,
            AuditLogQuery {
                session_id: Some(session_id.clone()),
                action: Some("control_input".to_string()),
                limit: Some(10),
            },
        )
        .await;

        match audit_response {
            IpcResponse::AuditLog { events } => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].action, "control_input");
                assert_eq!(events[0].session_id, Some(session_id.clone()));
                assert_eq!(events[0].details[0].1, "41");
            }
            _ => panic!("expected audit log response"),
        }

        let bundle_response =
            super::telemetry::telemetry_bundle("run-a".to_string(), Some(session_id.clone()));

        match bundle_response {
            IpcResponse::TelemetryBundle { bundle } => {
                assert_eq!(bundle.run_id, "run-a");
                assert_eq!(bundle.session_id, Some(session_id));
                assert_eq!(bundle.event_count, 0);
                assert_eq!(bundle.log_count, 0);
                assert!(bundle.metrics.is_empty());
                assert!(bundle.artifacts.is_empty());
            }
            _ => panic!("expected telemetry bundle response"),
        }
    }

    #[tokio::test]
    async fn capability_handler_returns_cached_snapshot_and_peer_not_found_evaluation() {
        let app_state = Arc::new(AppState::new());
        let mut cached = crate::capabilities::local_capability_snapshot_static();
        cached.service_version = "handler-test".to_string();
        app_state
            .replace_capability_snapshot_for_test(cached.clone())
            .await;

        let snapshot_response = super::capability::capability_snapshot(&app_state).await;

        match snapshot_response {
            IpcResponse::CapabilitySnapshot { snapshot } => {
                assert_eq!(snapshot.service_version, "handler-test");
                assert!(snapshot
                    .capabilities
                    .iter()
                    .any(|capability| capability.status != CapabilityStatus::Unknown));
            }
            _ => panic!("expected capability snapshot response"),
        }

        let peer_device_id = DeviceId("missing-peer".to_string());
        let evaluation_response = super::capability::evaluate_scenario_profile(
            &app_state,
            "lan.2k144".to_string(),
            Some(peer_device_id.clone()),
            None,
        )
        .await;

        match evaluation_response {
            IpcResponse::ScenarioProfileEvaluated { evaluation } => {
                assert_eq!(evaluation.scenario_id, "lan.2k144");
                assert_eq!(evaluation.status, ScenarioEvaluationStatus::Skipped);
                assert_eq!(evaluation.reasons.len(), 1);
                assert_eq!(evaluation.reasons[0].code, "peer_not_found");
                assert!(evaluation.reasons[0].message.contains(&peer_device_id.0));
            }
            _ => panic!("expected scenario profile evaluation response"),
        }
    }

    #[tokio::test]
    async fn control_handler_returns_transport_policy_and_channel_snapshot() {
        let app_state = Arc::new(AppState::new());
        let session_id = SessionId("control-session".to_string());

        let policy_response = super::control::set_transport_policy(
            session_id.clone(),
            TransportPolicyConfig {
                mode: "wan".to_string(),
                preferred_transport: Some("quic".to_string()),
                allow_lan_quic: false,
                allow_webrtc: true,
                allow_relay: true,
            },
        );

        match policy_response {
            IpcResponse::TransportPolicyUpdated { snapshot } => {
                assert_eq!(snapshot.session_id, Some(session_id.clone()));
                assert_eq!(snapshot.selected_transport, "webrtc");
                assert_eq!(snapshot.candidate_transports, vec!["webrtc"]);
                assert!(snapshot.relay_required);
                assert_eq!(
                    snapshot.fallback_reason.as_deref(),
                    Some("quic was requested but is not allowed by the active transport policy.")
                );
            }
            _ => panic!("expected transport policy response"),
        }

        let snapshot_response =
            super::control::control_channel_snapshot(&app_state, session_id.clone()).await;

        match snapshot_response {
            IpcResponse::ControlChannelSnapshot { snapshot } => {
                assert_eq!(snapshot.session_id, session_id);
                assert_eq!(snapshot.reliable.name, "ctrl_rel");
                assert_eq!(
                    snapshot.reliable.reliability,
                    ControlChannelReliability::ReliableOrdered
                );
                assert_eq!(snapshot.realtime.name, "ctrl_rt");
                assert_eq!(
                    snapshot.realtime.reliability,
                    ControlChannelReliability::UnreliableRealtime
                );
            }
            _ => panic!("expected control channel snapshot response"),
        }
    }

    #[tokio::test]
    async fn shell_handler_tracks_ui_lifecycle_and_launcher_path() {
        let app_state = Arc::new(AppState::new());
        let launcher: crate::shell::UiLauncherPortRef = Arc::new(std::sync::Mutex::new(
            crate::shell::InMemoryUiLauncher::new(),
        ));

        let attach_response = super::shell::ui_attached(
            &app_state,
            &launcher,
            4242,
            Some("C:\\Program Files\\Rdesk\\Rdesk.exe".to_string()),
        )
        .await;
        assert!(matches!(attach_response, IpcResponse::Ack));

        let persisted_path = launcher
            .lock()
            .expect("launcher lock")
            .get_ui_path()
            .expect("launcher path");
        assert_eq!(
            persisted_path.as_deref(),
            Some(std::path::Path::new("C:\\Program Files\\Rdesk\\Rdesk.exe"))
        );

        let status_response = super::shell::shell_status(&app_state).await;
        match status_response {
            IpcResponse::ShellStatus { status } => {
                assert_eq!(status.ui_pid, Some(4242));
                assert_eq!(status.active_session_count, 0);
            }
            _ => panic!("expected shell status response"),
        }

        let detach_response =
            super::shell::ui_detached(&app_state, 4242, UiDetachReason::UserClose).await;
        assert!(matches!(detach_response, IpcResponse::Ack));

        let status_response = super::shell::shell_status(&app_state).await;
        match status_response {
            IpcResponse::ShellStatus { status } => {
                assert_eq!(status.ui_pid, None);
                assert_eq!(status.active_session_count, 0);
            }
            _ => panic!("expected shell status response"),
        }
    }

    #[test]
    fn device_handler_rejects_remote_power_without_agent_executor() {
        let response = super::device::request_remote_device_power_action(
            DeviceId("agent-device".to_string()),
            RemoteDevicePowerAction::Restart,
        );

        match response {
            IpcResponse::Error { code, message } => {
                assert_eq!(code, "E_REMOTE_POWER_UNSUPPORTED");
                assert!(message.contains("agent-device"));
                assert!(message.contains("restart"));
            }
            _ => panic!("expected remote power unsupported error"),
        }
    }

    #[tokio::test]
    async fn preflight_handler_rejects_missing_required_lan_peer() {
        let app_state = Arc::new(AppState::new());
        let target_device_id = DeviceId("missing-lan-peer".to_string());

        let error = super::preflight::preflight_session_start(
            &app_state,
            &target_device_id,
            "quic",
            None,
            true,
        )
        .await
        .expect_err("missing LAN peer should fail preflight");

        assert!(error.contains("LAN peer missing-lan-peer was not found"));
    }
}
