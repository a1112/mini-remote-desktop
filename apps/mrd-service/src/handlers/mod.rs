// Handler modules for mrd-service

pub mod device;
pub mod identity;
pub mod session;
pub mod telemetry;
pub mod transport;

#[cfg(test)]
mod tests {
    use crate::app_state::AppState;
    use mrd_ipc::{AuditLogQuery, IpcResponse};
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
}
