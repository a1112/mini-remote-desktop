use std::{net::SocketAddr, sync::Arc};

use mrd_ipc::{IpcRequest, IpcResponse, RemoteDevicePowerAction, ShutdownMode};
#[cfg(feature = "browser-webrtc-preview")]
use mrd_service::browser_webrtc_preview::sanitize_browser_preview_fps;
use mrd_service::{
    app_state::AppState,
    ipc_server::IpcServer,
    web_bridge::{dispatch_ipc_for_test, is_ipc_request_allowed, WebBridgeConfig},
};

#[test]
fn localhost_web_bridge_requires_token() {
    let bind = "127.0.0.1:9533".parse::<SocketAddr>().unwrap();
    let error = WebBridgeConfig::new_for_test(bind, None)
        .expect_err("localhost bridge without token must be rejected");

    assert!(error.to_string().contains("MRD_WEB_BRIDGE_TOKEN"));

    let config = WebBridgeConfig::new_for_test(bind, Some("test-token".to_string()))
        .expect("localhost bridge config with token");
    assert!(config.requires_token());
}

#[test]
fn lan_bound_web_bridge_requires_token() {
    let error = WebBridgeConfig::new_for_test("0.0.0.0:9533".parse::<SocketAddr>().unwrap(), None)
        .expect_err("LAN bridge without token must be rejected");

    assert!(error.to_string().contains("MRD_WEB_BRIDGE_TOKEN"));
}

#[test]
fn bridge_allows_only_browser_safe_ipc_requests() {
    assert!(is_ipc_request_allowed(&IpcRequest::CapabilitySnapshot));
    assert!(is_ipc_request_allowed(&IpcRequest::RefreshLanDiscovery));
    assert!(is_ipc_request_allowed(&IpcRequest::ServiceHealth));
    assert!(is_ipc_request_allowed(&IpcRequest::GetShellStatus));
    assert!(is_ipc_request_allowed(&IpcRequest::GetDevicePreferences));
    assert!(is_ipc_request_allowed(
        &IpcRequest::UpdateDevicePreference {
            device_id: mrd_proto::DeviceId("agent-device".to_string()),
            update: mrd_ipc::DevicePreferenceUpdate {
                favorite: Some(true),
                disabled: None,
                removed: None,
            },
        }
    ));
    assert!(is_ipc_request_allowed(
        &IpcRequest::RequestRemoteDevicePowerAction {
            device_id: mrd_proto::DeviceId("agent-device".to_string()),
            action: RemoteDevicePowerAction::Restart,
        }
    ));

    assert!(!is_ipc_request_allowed(&IpcRequest::ShutdownService {
        mode: ShutdownMode::Graceful
    }));
}

#[cfg(feature = "browser-webrtc-preview")]
#[test]
fn browser_webrtc_preview_allows_144_fps_followup_target() {
    assert_eq!(sanitize_browser_preview_fps(Some(120)), 120);
    assert_eq!(sanitize_browser_preview_fps(Some(144)), 144);
    assert_eq!(sanitize_browser_preview_fps(Some(249)), 144);
    assert_eq!(sanitize_browser_preview_fps(None), 120);
}

#[tokio::test]
async fn allowed_ipc_request_is_dispatched_to_service() {
    let server = IpcServer::new(Arc::new(AppState::new()));

    let response = dispatch_ipc_for_test(server, IpcRequest::CapabilitySnapshot).await;

    match response {
        IpcResponse::CapabilitySnapshot { snapshot } => {
            assert_eq!(snapshot.schema_version, 1);
        }
        other => panic!("expected capability snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn shell_status_is_available_through_web_bridge() {
    let server = IpcServer::new(Arc::new(AppState::new()));

    let response = dispatch_ipc_for_test(server, IpcRequest::GetShellStatus).await;

    match response {
        IpcResponse::ShellStatus { status } => {
            assert!(status.service_pid > 0);
        }
        other => panic!("expected shell status, got {other:?}"),
    }
}

#[tokio::test]
async fn blocked_ipc_request_returns_forbidden_error() {
    let server = IpcServer::new(Arc::new(AppState::new()));

    let response = dispatch_ipc_for_test(
        server,
        IpcRequest::ShutdownService {
            mode: ShutdownMode::Graceful,
        },
    )
    .await;

    match response {
        IpcResponse::Error { code, message } => {
            assert_eq!(code, "E_WEB_BRIDGE_FORBIDDEN");
            assert!(message.contains("ShutdownService"));
        }
        other => panic!("expected forbidden error, got {other:?}"),
    }
}
