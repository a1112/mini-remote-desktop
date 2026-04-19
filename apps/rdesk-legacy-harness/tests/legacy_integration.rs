//! Legacy runtime integration tests
//!
//! These tests preserve validation of the old direct-control runtime during the hard-cut migration.
//! Migrated from apps/Rdesk/src-tauri/src/main.rs #[cfg(test)] block.

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use rdesk_legacy_harness::test_helpers::*;
use rdesk_legacy_harness::{
    app_settings::{load_settings, DecodePolicy},
    benchmark::{write_benchmark_artifacts, BenchmarkManifest, BenchmarkPaths, BenchmarkSummary},
    frame_sink::DecodedFrameSink,
    quic_host::QuicHost,
    quic_session::QuicSessionCoordinator,
    realtime_runtime::RealtimeRuntime,
    render_host::RenderHost,
    session_lifecycle::SessionLifecycleCoordinator,
    webrtc_host::WebrtcHost,
    webrtc_session::WebrtcSessionCoordinator,
};
use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
use mrd_proto::{DeviceId, SessionId};
use mrd_signal_client::{decode_message, encode_message};
use mrd_signal_proto::{SignalMessage, IceCandidate, SessionDescription};
use std::{collections::HashMap, sync::Arc, sync::Once};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};

fn ensure_rustls_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

// =============================================================================
// Test: NVDEC runtime probe
// =============================================================================

#[test]
fn nvdec_runtime_probe_response_reports_capabilities() {
    let probe = nvdec_runtime_probe_response();

    assert_eq!(probe.backend, "windows-nvdec");
    assert!(probe
        .capability_probes
        .iter()
        .any(|capability| capability.codec == "h264" && capability.bit_depth_minus8 == 0));
    assert!(probe
        .capability_probes
        .iter()
        .any(|capability| capability.codec == "hevc" && capability.bit_depth_minus8 == 0));
    assert!(probe
        .capability_probes
        .iter()
        .any(|capability| { capability.codec == "hevc" && capability.bit_depth_minus8 == 2 }));
}

// =============================================================================
// Test: Decode policy helpers
// =============================================================================

#[tokio::test]
async fn decode_policy_helpers_roundtrip_persisted_policy() {
    let settings_path = std::env::temp_dir().join(format!(
        "decode-policy-test-{}.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));

    // After migration, decode policy is stored in settings and applied by mrd-service
    let updated = set_decode_policy_with(&settings_path, DecodePolicy::Nvdec)
        .await
        .expect("set decode policy");
    assert_eq!(updated.decode_policy, "nvdec");

    // Decode policy is now read from settings file, not from a runtime host
    let reread = load_settings(&settings_path).expect("load settings");
    assert_eq!(reread.decode_policy.as_str(), "nvdec");
    let _ = std::fs::remove_file(settings_path);
}

// =============================================================================
// WebSocket test server helpers
// =============================================================================

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(mut socket: WebSocket) {
    let Some(Ok(Message::Text(raw))) = socket.next().await else {
        return;
    };

    let message = decode_message(&raw).expect("decode register message");
    assert!(matches!(message, SignalMessage::Register(_)));

    let ack = encode_message(&SignalMessage::Registered(
        mrd_signal_proto::RegisteredResponse {
            device_id: DeviceId("controller-1".into()),
        },
    ))
    .expect("encode registered response");

    socket
        .send(Message::Text(ack.into()))
        .await
        .expect("send registered response");

    while let Some(Ok(Message::Text(raw))) = socket.next().await {
        let signal = decode_message(&raw).expect("decode session signal");
        let outbound = encode_message(&signal).expect("encode echoed session signal");
        socket
            .send(Message::Text(outbound.into()))
            .await
            .expect("echo session signal");
    }
}

async fn spawn_server() -> String {
    let app = Router::new().route("/ws", get(ws_handler));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind realtime helper test server");
    let addr = listener.local_addr().expect("test server addr");

    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve helper test ws");
    });

    format!("ws://{}/ws", addr)
}

// =============================================================================
// Test: Realtime helpers roundtrip
// =============================================================================

#[tokio::test]
async fn realtime_helpers_roundtrip_register_request_accept_and_drain_events() {
    let runtime = RealtimeRuntime::new(spawn_server().await);

    let registration = realtime_register_with(
        &runtime,
        "controller".into(),
        Some("controller-1".into()),
        "Rdesk".into(),
    )
    .await
    .expect("register realtime connection");

    realtime_request_session_with(
        &runtime,
        registration.handle,
        "session-1".into(),
        "agent-1".into(),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("request session through helper");

    realtime_accept_session_with(
        &runtime,
        registration.handle,
        "session-1".into(),
        None,
        None,
        None,
        None,
    )
    .await
    .expect("accept session through helper");

    let events = drain_realtime_events_with(&runtime, registration.handle)
        .await
        .expect("drain realtime events");

    assert_eq!(events.len(), 2);
    assert!(matches!(events[0], SignalMessage::SessionRequest(_)));
    assert!(matches!(events[1], SignalMessage::SessionAccept(_)));
}

// =============================================================================
// Test: WebRTC helpers record and report snapshot
// =============================================================================

#[tokio::test]
async fn webrtc_helpers_record_and_report_snapshot() {
    let coordinator = Mutex::new(WebrtcSessionCoordinator::default());

    let offer =
        webrtc_create_local_offer_with(&coordinator, "session-1".into(), "offer-sdp".into())
            .await
            .expect("create local offer");
    assert_eq!(offer.sdp, "offer-sdp");

    webrtc_apply_remote_answer_with(&coordinator, "session-1".into(), "answer-sdp".into())
        .await
        .expect("apply answer");
    webrtc_apply_remote_ice_candidate_with(
        &coordinator,
        "session-1".into(),
        "candidate:1 1 UDP 123 127.0.0.1 5000 typ host".into(),
        Some("0".into()),
        Some(0),
    )
    .await
    .expect("apply ice");

    let snapshot = webrtc_snapshot_with(&coordinator, "session-1".into())
        .await
        .expect("snapshot exists");
    assert_eq!(snapshot.local_offer.as_deref(), Some("offer-sdp"));
    assert_eq!(snapshot.remote_offer, None);
    assert_eq!(snapshot.remote_answer.as_deref(), Some("answer-sdp"));
    assert_eq!(snapshot.remote_ice_candidates.len(), 1);
}

// =============================================================================
// Test: Syncing realtime events applies offer/answer and ICE
// =============================================================================

#[tokio::test]
async fn syncing_realtime_events_applies_offer_answer_and_ice() {
    let runtime = RealtimeRuntime::new(spawn_server().await);
    let coordinator = Mutex::new(WebrtcSessionCoordinator::default());

    let registration = realtime_register_with(
        &runtime,
        "controller".into(),
        Some("controller-1".into()),
        "Rdesk".into(),
    )
    .await
    .expect("register realtime connection");

    runtime
        .send_offer(
            registration.handle,
            mrd_signal_proto::SessionDescription {
                session_id: SessionId("session-2".into()),
                sdp: "offer-sdp".into(),
            },
        )
        .await
        .expect("send offer");
    runtime
        .send_answer(
            registration.handle,
            mrd_signal_proto::SessionDescription {
                session_id: SessionId("session-2".into()),
                sdp: "answer-sdp".into(),
            },
        )
        .await
        .expect("send answer");
    runtime
        .send_ice_candidate(
            registration.handle,
            mrd_signal_proto::IceCandidate {
                session_id: SessionId("session-2".into()),
                candidate: "candidate:2 1 UDP 123 127.0.0.1 5001 typ host".into(),
                sdp_mid: Some("0".into()),
                sdp_mline_index: Some(0),
            },
        )
        .await
        .expect("send ice");

    let snapshot =
        webrtc_sync_realtime_events_with(&runtime, &coordinator, registration.handle)
            .await
            .expect("sync realtime events");

    assert_eq!(snapshot.remote_offer.as_deref(), Some("offer-sdp"));
    assert_eq!(snapshot.local_offer, None);
    assert_eq!(snapshot.remote_answer.as_deref(), Some("answer-sdp"));
    assert_eq!(snapshot.remote_ice_candidates.len(), 1);
}

// =============================================================================
// Test: Decoded frame snapshot reports latest ingested frame
// =============================================================================

#[test]
fn decoded_frame_snapshot_reports_latest_ingested_frame() {
    let sink = std::sync::Mutex::new(DecodedFrameSink::default());
    sink.lock().expect("lock decoded frame sink").ingest_frame(
        SessionId("session-9".into()),
        mrd_pipeline_core::DecodedFrame::from_cpu_rgb24(640, 360, 0, vec![0; 640 * 360 * 3]),
    );

    let snapshot = decoded_frame_snapshot_with(&sink, "session-9".into()).expect("snapshot");

    assert_eq!(snapshot.frame_count, 1);
    assert_eq!(snapshot.width, 640);
    assert_eq!(snapshot.height, 360);
    assert_eq!(snapshot.pixel_format, "Rgb24");
    assert_eq!(snapshot.bytes, 640 * 360 * 3);
}

// =============================================================================
// Test: Decoded frame preview encodes PNG data URL
// =============================================================================

#[test]
fn decoded_frame_preview_encodes_png_data_url() {
    let sink = std::sync::Mutex::new(DecodedFrameSink::default());
    sink.lock().expect("lock decoded frame sink").ingest_frame(
        SessionId("session-preview".into()),
        mrd_pipeline_core::DecodedFrame::from_cpu_rgb24(
            2,
            2,
            0,
            vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
        ),
    );

    let preview = decoded_frame_preview_with(&sink, "session-preview".into())
        .expect("encode preview")
        .expect("preview exists");

    assert!(preview.starts_with("data:image/png;base64,"));
}

// =============================================================================
// Test: Render host snapshot reports attachment and preview
// =============================================================================

#[test]
fn render_host_snapshot_reports_attachment_and_preview() {
    let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
    sink.lock().expect("lock decoded frame sink").ingest_frame(
        SessionId("session-render".into()),
        mrd_pipeline_core::DecodedFrame::from_cpu_rgb24(2, 2, 0, vec![255; 12]),
    );
    let mut render_host = RenderHost::with_frame_sink(sink);
    let _ =
        render_host.attach_session(SessionId("session-render".into()), "surface-1".into(), 0);

    let snapshot = render_host.snapshot(&SessionId("session-render".into()))
        .expect("render host snapshot");
    let response = render_host_snapshot_response(snapshot);

    assert!(response.attached);
    assert_eq!(response.surface_count, 1);
    assert_eq!(response.attached_surface_ids, vec!["surface-1".to_string()]);
    assert_eq!(response.frame.as_ref().map(|frame| frame.width), Some(2));
    assert!(response
        .preview_data_url
        .as_deref()
        .map_or(false, |value: &str| value
            .starts_with("data:image/png;base64,")));
}
