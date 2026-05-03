// IPC contract tests
// Verify serialization/deserialization of all IPC messages

use mrd_ipc::{
    CaptureSource, CaptureSourceSelection, DeviceInfo, IpcRequest, IpcResponse, MediaProfile,
    MediaProfileNegotiation, SessionBootstrap, SessionRuntimeSnapshot,
};
use mrd_proto::{DeviceId, SessionId};

fn test_device_id() -> DeviceId {
    DeviceId("test-device".to_string())
}

fn test_session_id() -> SessionId {
    SessionId("test-session-123".to_string())
}

fn test_media_profile() -> MediaProfile {
    MediaProfile {
        width: 2560,
        height: 1440,
        fps: 144,
        bitrate_mbps: 64,
        codec: "h264".to_string(),
    }
}

fn test_capture_source() -> CaptureSource {
    CaptureSource {
        id: "windows:window:0x1234".to_string(),
        platform: "windows".to_string(),
        source_kind: "window".to_string(),
        title: "Target App".to_string(),
        class_name: "ApplicationFrameWindow".to_string(),
        width: 1280,
        height: 720,
        process_id: 4242,
        app_name: Some("Target App".to_string()),
        bundle_identifier: None,
        preview_data_url: Some("data:image/png;base64,AAAA".to_string()),
        preview_width: Some(320),
        preview_height: Some(180),
    }
}

#[test]
fn serialize_deserialize_register_device() {
    let request = IpcRequest::RegisterDevice {
        device_id: test_device_id(),
        device_name: "Test Device".to_string(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_list_devices() {
    let request = IpcRequest::ListDevices;

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_start_session() {
    let request = IpcRequest::StartSession {
        session_id: test_session_id(),
        target_device_id: test_device_id(),
        transport_kind: "quic".to_string(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_start_lan_remote_session_with_media_profile() {
    let request = IpcRequest::StartLanRemoteSession {
        session_id: test_session_id(),
        target_device_id: test_device_id(),
        transport_kind: "quic".to_string(),
        requested_profile: Some(test_media_profile()),
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("requested_profile"));
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_update_media_profile() {
    let request = IpcRequest::UpdateMediaProfile {
        session_id: test_session_id(),
        requested_profile: test_media_profile(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_list_remote_capture_sources() {
    let request = IpcRequest::ListRemoteCaptureSources {
        session_id: test_session_id(),
        include_previews: true,
        limit: Some(32),
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("ListRemoteCaptureSources"));
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_select_remote_capture_source() {
    let request = IpcRequest::SelectRemoteCaptureSource {
        session_id: test_session_id(),
        source_id: "windows:window:0x1234".to_string(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_accept_session() {
    let request = IpcRequest::AcceptSession {
        session_id: test_session_id(),
        source_device_id: test_device_id(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_start_sender() {
    let request = IpcRequest::StartSender {
        session_id: test_session_id(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_start_receiver() {
    let request = IpcRequest::StartReceiver {
        session_id: test_session_id(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_stop_session() {
    let request = IpcRequest::StopSession {
        session_id: test_session_id(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_session_runtime_snapshot_request() {
    let request = IpcRequest::SessionRuntimeSnapshot {
        session_id: test_session_id(),
    };

    let json = serde_json::to_string(&request).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(request, deserialized);
}

#[test]
fn serialize_deserialize_device_registered_response() {
    let response = IpcResponse::DeviceRegistered {
        device_id: test_device_id(),
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_device_list_response() {
    let response = IpcResponse::DeviceList {
        devices: vec![DeviceInfo {
            device_id: test_device_id(),
            device_name: "Test Device".to_string(),
            is_online: true,
        }],
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_session_snapshot_response() {
    let response = IpcResponse::SessionSnapshot {
        snapshot: SessionRuntimeSnapshot {
            session_id: test_session_id(),
            role: "controller".to_string(),
            state: "connected".to_string(),
            transport_kind: "quic".to_string(),
            local_bootstrap: Some(SessionBootstrap {
                listen_addr: Some("127.0.0.1:4433".to_string()),
                server_name: Some("localhost".to_string()),
                cert_der: Some("base64cert".to_string()),
            }),
            remote_bootstrap: None,
            last_error: None,
            sender_active: false,
            receiver_active: false,
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_error_response() {
    let response = IpcResponse::Error {
        code: "E001".to_string(),
        message: "Test error".to_string(),
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_media_profile_updated_response() {
    let negotiation = MediaProfileNegotiation {
        requested: MediaProfile {
            width: 3840,
            height: 2160,
            fps: 240,
            bitrate_mbps: 120,
            codec: "h264".to_string(),
        },
        selected: test_media_profile(),
        status: "downgraded".to_string(),
        reason: Some("clamped to LAN QUIC profile capability".to_string()),
    };
    let response = IpcResponse::MediaProfileUpdated {
        session_id: test_session_id(),
        negotiation: negotiation.clone(),
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized, response);
}

#[test]
fn serialize_deserialize_capture_source_list_response() {
    let response = IpcResponse::CaptureSourceList {
        session_id: test_session_id(),
        sources: vec![test_capture_source()],
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("preview_data_url"));
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized, response);
}

#[test]
fn serialize_deserialize_capture_source_selected_response() {
    let selection = CaptureSourceSelection {
        session_id: test_session_id(),
        source: test_capture_source(),
        status: "selected".to_string(),
        reason: None,
    };
    let response = IpcResponse::CaptureSourceSelected {
        session_id: test_session_id(),
        selection,
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized, response);
}

#[test]
fn serialize_deserialize_all_request_types() {
    let requests = vec![
        IpcRequest::RegisterDevice {
            device_id: test_device_id(),
            device_name: "Device".to_string(),
        },
        IpcRequest::ListDevices,
        IpcRequest::StartSession {
            session_id: test_session_id(),
            target_device_id: test_device_id(),
            transport_kind: "webrtc".to_string(),
        },
        IpcRequest::StartLanRemoteSession {
            session_id: test_session_id(),
            target_device_id: test_device_id(),
            transport_kind: "quic".to_string(),
            requested_profile: Some(test_media_profile()),
        },
        IpcRequest::UpdateMediaProfile {
            session_id: test_session_id(),
            requested_profile: test_media_profile(),
        },
        IpcRequest::ListRemoteCaptureSources {
            session_id: test_session_id(),
            include_previews: true,
            limit: Some(16),
        },
        IpcRequest::SelectRemoteCaptureSource {
            session_id: test_session_id(),
            source_id: "windows:window:0x1234".to_string(),
        },
        IpcRequest::AcceptSession {
            session_id: test_session_id(),
            source_device_id: test_device_id(),
        },
        IpcRequest::StartSender {
            session_id: test_session_id(),
        },
        IpcRequest::StartReceiver {
            session_id: test_session_id(),
        },
        IpcRequest::StopSession {
            session_id: test_session_id(),
        },
        IpcRequest::SessionRuntimeSnapshot {
            session_id: test_session_id(),
        },
        IpcRequest::StreamProbeEvents,
    ];

    for request in requests {
        let json = serde_json::to_string(&request).unwrap();
        let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(request, deserialized);
    }
}
