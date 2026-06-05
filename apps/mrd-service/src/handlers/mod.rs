// Handler modules for mrd-service

pub mod device;
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
}
