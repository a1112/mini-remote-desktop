// Handler modules for mrd-service

pub mod device;
pub mod identity;
pub mod session;
pub mod transport;

#[cfg(test)]
mod tests {
    use crate::app_state::AppState;
    use mrd_ipc::IpcResponse;
    use mrd_proto::DeviceId;
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
}
