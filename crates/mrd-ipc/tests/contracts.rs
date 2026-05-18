// IPC contract tests
// Verify serialization/deserialization of all IPC messages

use mrd_ipc::{
    AdaptiveMediaConfig, AttachedRenderSurface, CapabilityConstraint, CapabilityConstraintStatus,
    CapabilityDomain, CapabilityItem, CapabilityPlatform, CapabilityProfile, CapabilitySnapshot,
    CapabilityStatus, CaptureSource, CaptureSourceSelection, DeviceInfo, IpcRequest, IpcResponse,
    MediaAdaptationSnapshot, MediaPipelineSnapshot, MediaProfile, MediaProfileNegotiation,
    MediaStageMetrics, SessionBootstrap, SessionRuntimeSnapshot,
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
        ..MediaProfile::default()
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

fn test_capability_snapshot() -> CapabilitySnapshot {
    CapabilitySnapshot {
        schema_version: 1,
        platform: CapabilityPlatform::Windows,
        service_version: "0.1.0".to_string(),
        capabilities: vec![CapabilityItem {
            id: "transport.quic_datagram".to_string(),
            domain: CapabilityDomain::Transport,
            label: "QUIC datagram media".to_string(),
            status: CapabilityStatus::Available,
            platform: CapabilityPlatform::Windows,
            reason: None,
            detail: None,
            requires: Vec::new(),
            conflicts_with: Vec::new(),
            depends_on: Vec::new(),
            fallback_ids: Vec::new(),
            last_probe_time_ms: Some(1_700_000_000_000),
        }],
        constraints: vec![CapabilityConstraint {
            id: "openh264_requires_cpu_input".to_string(),
            applies_to: vec![
                "encode.openh264".to_string(),
                "memory.d3d11_shared".to_string(),
            ],
            status: CapabilityConstraintStatus::Block,
            reason: "OpenH264 requires CPU-backed input".to_string(),
            fallback_ids: vec!["memory.cpu".to_string()],
        }],
        profiles: vec![
            CapabilityProfile {
                id: "lan.2k144".to_string(),
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 64,
                codec: "h264".to_string(),
                latency_budget_ms: None,
                min_stable_fps_ratio: Some(0.8),
                max_drop_ratio: Some(0.02),
                required_capabilities: vec![
                    "transport.quic_datagram".to_string(),
                    "transport.media_profile_control_v1".to_string(),
                ],
            },
            CapabilityProfile {
                id: "lan.1600p165".to_string(),
                width: 2560,
                height: 1600,
                fps: 165,
                bitrate_mbps: 80,
                codec: "h264".to_string(),
                latency_budget_ms: None,
                min_stable_fps_ratio: Some(0.8),
                max_drop_ratio: Some(0.02),
                required_capabilities: vec![
                    "transport.quic_datagram".to_string(),
                    "transport.media_profile_control_v1".to_string(),
                ],
            },
        ],
        updated_at_ms: 1_700_000_000_000,
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
fn serialize_deserialize_configure_media_adaptation() {
    let config = AdaptiveMediaConfig {
        enabled: true,
        mode: "keyframe_ladder".to_string(),
        ceiling_profile: Some(test_media_profile()),
        floor_profile: Some(MediaProfile {
            width: 1280,
            height: 720,
            fps: 60,
            bitrate_mbps: 10,
            codec: "h264".to_string(),
            ..MediaProfile::default()
        }),
        ladder: vec![test_media_profile()],
        downshift_cooldown_ms: 2_000,
        upshift_hold_ms: 5_000,
    };
    let request = IpcRequest::ConfigureMediaAdaptation {
        session_id: test_session_id(),
        config,
    };

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("ConfigureMediaAdaptation"));
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
fn serialize_deserialize_capability_snapshot_request() {
    let request = IpcRequest::CapabilitySnapshot;

    let json = serde_json::to_string(&request).unwrap();
    assert!(json.contains("CapabilitySnapshot"));
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
fn serialize_deserialize_capability_snapshot_response() {
    let response = IpcResponse::CapabilitySnapshot {
        snapshot: test_capability_snapshot(),
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("lan.2k144"));
    assert!(json.contains("lan.1600p165"));
    assert!(json.contains("transport.quic_datagram"));
    assert!(json.contains("\"platform\":\"windows\""));
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
            ..MediaProfile::default()
        },
        selected: test_media_profile(),
        status: "downgraded".to_string(),
        reason: Some("clamped to LAN QUIC profile capability".to_string()),
        selected_source_id: Some("windows:display:0".to_string()),
        selected_width: Some(2560),
        selected_height: Some(1440),
        downgrade_reason: Some("clamped to LAN QUIC profile capability".to_string()),
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
fn serialize_deserialize_render_surface_control_contracts() {
    let attach = IpcRequest::AttachRenderSurface {
        session_id: test_session_id(),
        surface_id: "surface-1".to_string(),
        backend: "d3d11".to_string(),
        window_handle: Some(0x1234),
    };
    let json = serde_json::to_string(&attach).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(attach, deserialized);

    let detach = IpcRequest::DetachRenderSurface {
        session_id: test_session_id(),
        surface_id: "surface-1".to_string(),
    };
    let json = serde_json::to_string(&detach).unwrap();
    let deserialized: IpcRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(detach, deserialized);

    let response = IpcResponse::RenderSurfaceAttached {
        session_id: test_session_id(),
        surface_id: "surface-1".to_string(),
    };
    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_media_pipeline_snapshot_contract() {
    let response = IpcResponse::MediaPipelineSnapshot {
        snapshot: MediaPipelineSnapshot {
            session_id: test_session_id(),
            attached_surfaces: vec![AttachedRenderSurface {
                surface_id: "surface-1".to_string(),
                backend: "d3d11".to_string(),
                window_handle: Some(0x1234),
            }],
            active_decoder: Some("nvdec".to_string()),
            active_renderer: Some("d3d11".to_string()),
            active_codec: Some("hevc".to_string()),
            active_codec_profile: Some("main".to_string()),
            active_bit_depth: Some(8),
            active_chroma_subsampling: Some("4:2:0".to_string()),
            active_pixel_format: Some("d3d11_shared_nv12".to_string()),
            active_hdr_enabled: Some(false),
            active_width: Some(2560),
            active_height: Some(1440),
            active_fps: Some(144),
            active_bitrate_mbps: Some(80),
            codec_fallback_reason: None,
            queue_depth: 1,
            dropped_frames: 2,
            render_queue_replacements: 1,
            render_lock_drops: 1,
            stage_metrics: vec![MediaStageMetrics {
                stage: "decode".to_string(),
                p50_ms: Some(1.0),
                p95_ms: Some(2.0),
            }],
            test_impairment: None,
            adaptation: Some(MediaAdaptationSnapshot {
                enabled: true,
                state: "stable".to_string(),
                ladder_index: 0,
                current_profile: test_media_profile(),
                target_profile: test_media_profile(),
                last_reason: Some("configured".to_string()),
                last_change_ms: 1_700_000_000_000,
                observed_fps: 144.0,
                drop_ratio: 0.0,
                queue_depth: 0,
            }),
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(response, deserialized);
}

#[test]
fn serialize_deserialize_media_adaptation_configured_response() {
    let response = IpcResponse::MediaAdaptationConfigured {
        session_id: test_session_id(),
        snapshot: MediaAdaptationSnapshot {
            enabled: true,
            state: "stable".to_string(),
            ladder_index: 0,
            current_profile: test_media_profile(),
            target_profile: test_media_profile(),
            last_reason: Some("configured".to_string()),
            last_change_ms: 1_700_000_000_000,
            observed_fps: 144.0,
            drop_ratio: 0.0,
            queue_depth: 0,
        },
    };

    let json = serde_json::to_string(&response).unwrap();
    assert!(json.contains("MediaAdaptationConfigured"));
    let deserialized: IpcResponse = serde_json::from_str(&json).unwrap();

    assert_eq!(response, deserialized);
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
