//! Test helper functions for legacy runtime integration tests
//!
//! These helpers were previously in main.rs and are used by the integration tests.
//! They provide thin wrapper functions for interacting with the legacy runtime components.

use crate::app_settings::{save_settings, AppSettings, DecodePolicy};
use crate::{
    DecodedFrameSink, DecodedFrameSnapshot,
    QuicHost, QuicHostSnapshot,
    QuicSessionCoordinator, QuicSessionSnapshot,
    RealtimeRegistration, RealtimeRuntime,
    RenderHost, RenderHostSnapshot, RenderSurfaceDescriptor,
    SessionLifecycleCoordinator, SessionLifecycleSnapshot, SurfaceSourceBinding,
    WebrtcHost, WebrtcHostSnapshot,
    WebrtcSessionCoordinator, WebrtcSessionSnapshot,
};

use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_decode::DecodedFrame;
use mrd_decode_nvdec::probe_runtime as probe_nvdec_runtime;
use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::encode_message;
use mrd_signal_proto::{IceCandidate, SessionDescription, SignalMessage};
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

// =============================================================================
// Response types for test helpers
// =============================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeRegistrationResponse {
    pub handle: u64,
    pub device_id: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WebrtcSessionSnapshotResponse {
    pub local_offer: Option<String>,
    pub remote_offer: Option<String>,
    pub remote_answer: Option<String>,
    pub remote_ice_candidates: Vec<IceCandidate>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuicSessionSnapshotResponse {
    pub transport: String,
    pub source_device_id: Option<String>,
    pub target_device_id: Option<String>,
    pub local_listen_addr: Option<String>,
    pub local_server_name: Option<String>,
    pub local_cert_der_b64: Option<String>,
    pub remote_listen_addr: Option<String>,
    pub remote_server_name: Option<String>,
    pub remote_cert_der_b64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WebrtcHostSnapshotResponse {
    pub local_offer: Option<String>,
    pub remote_offer: Option<String>,
    pub local_answer: Option<String>,
    pub remote_answer: Option<String>,
    pub remote_ice_count: usize,
    pub remote_video_track_count: usize,
    pub remote_rtp_packet_count: u64,
    pub last_remote_codec: Option<String>,
    pub last_remote_payload_type: Option<u8>,
    pub last_remote_fmtp_line: Option<String>,
    pub remote_h264_access_unit_count: u64,
    pub last_remote_access_unit_bytes: usize,
    pub recent_remote_access_unit_bytes: Vec<usize>,
    pub recent_remote_access_unit_keyframes: Vec<usize>,
    pub decoded_frame_count: u64,
    pub last_decoded_width: usize,
    pub last_decoded_height: usize,
    pub last_decoded_pixel_format: Option<String>,
    pub decode_policy: String,
    pub preferred_decode_backend: Option<String>,
    pub active_decode_backend: Option<String>,
    pub decode_backend_reason: Option<String>,
    pub decode_fallback_count: u64,
    pub last_decode_fallback_reason: Option<String>,
    pub decode_error_count: u64,
    pub last_decode_error: Option<String>,
    pub available_video_source_ids: Vec<String>,
    pub local_video_track_count: usize,
    pub captured_frame_count: u64,
    pub sent_access_unit_count: u64,
    pub sent_rtp_bytes: u64,
    pub zero_write_access_unit_count: u64,
    pub sender_running: bool,
    pub peer_connection_state: String,
    pub ice_connection_state: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuicHostSnapshotResponse {
    pub transport: String,
    pub local_addr: Option<String>,
    pub peer_addr: Option<String>,
    pub remote_datagram_count: u64,
    pub remote_access_unit_count: u64,
    pub decoded_frame_count: u64,
    pub last_decoded_width: usize,
    pub last_decoded_height: usize,
    pub last_decoded_pixel_format: Option<String>,
    pub sent_access_unit_count: u64,
    pub sender_running: bool,
    pub receiver_running: bool,
    pub active_decode_backend: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecodedFrameSnapshotResponse {
    pub frame_count: u64,
    pub width: usize,
    pub height: usize,
    pub pixel_format: String,
    pub bytes: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RenderHostSnapshotResponse {
    pub attached: bool,
    pub surface_count: usize,
    pub attached_surface_ids: Vec<String>,
    pub frame: Option<DecodedFrameSnapshotResponse>,
    pub preview_data_url: Option<String>,
    pub renderer_backend: String,
    pub renderer_snapshot: Option<String>,
    pub surface_source_bindings: Vec<SurfaceSourceBindingResponse>,
    pub available_source_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceSourceBindingResponse {
    pub surface_id: String,
    pub source_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RenderSurfaceDescriptorResponse {
    pub current: bool,
    pub surface_id: String,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionLifecycleSnapshotResponse {
    pub session_id: String,
    pub current_surface_id: Option<String>,
    pub surfaces: Vec<RenderSurfaceDescriptorResponse>,
    pub available_source_ids: Vec<String>,
    pub surface_source_bindings: Vec<SurfaceSourceBindingResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SessionRuntimeSnapshotResponse {
    pub lifecycle: SessionLifecycleSnapshotResponse,
    pub render_host: RenderHostSnapshotResponse,
    pub webrtc_host: Option<WebrtcHostSnapshotResponse>,
    pub quic_host: Option<QuicHostSnapshotResponse>,
    pub webrtc_signaling: Option<WebrtcSessionSnapshotResponse>,
    pub quic_signaling: Option<QuicSessionSnapshotResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DecodePolicyResponse {
    pub decode_policy: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NvdecCapabilityProbeResponse {
    pub codec: String,
    pub bit_depth_minus8: u8,
    pub chroma_format: u8,
    pub runtime_supported: bool,
    pub runtime_reason: Option<String>,
    pub wired_supported: bool,
    pub wired_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NvdecRuntimeProbeResponse {
    pub backend: String,
    pub summary: String,
    pub checked_items: Vec<String>,
    pub capability_probes: Vec<NvdecCapabilityProbeResponse>,
}

// =============================================================================
// Response conversion functions
// =============================================================================

pub fn realtime_registration_response(
    registration: RealtimeRegistration,
) -> RealtimeRegistrationResponse {
    RealtimeRegistrationResponse {
        handle: registration.handle,
        device_id: registration.device_id.0,
    }
}

pub fn webrtc_snapshot_response(snapshot: &WebrtcSessionSnapshot) -> WebrtcSessionSnapshotResponse {
    WebrtcSessionSnapshotResponse {
        local_offer: snapshot.local_offer.clone(),
        remote_offer: snapshot.remote_offer.clone(),
        remote_answer: snapshot.remote_answer.clone(),
        remote_ice_candidates: snapshot.remote_ice_candidates.clone(),
    }
}

pub fn quic_snapshot_response(snapshot: &QuicSessionSnapshot) -> QuicSessionSnapshotResponse {
    QuicSessionSnapshotResponse {
        transport: snapshot.transport.clone(),
        source_device_id: snapshot.source_device_id.clone(),
        target_device_id: snapshot.target_device_id.clone(),
        local_listen_addr: snapshot.local_listen_addr.clone(),
        local_server_name: snapshot.local_server_name.clone(),
        local_cert_der_b64: snapshot.local_cert_der_b64.clone(),
        remote_listen_addr: snapshot.remote_listen_addr.clone(),
        remote_server_name: snapshot.remote_server_name.clone(),
        remote_cert_der_b64: snapshot.remote_cert_der_b64.clone(),
    }
}

pub fn webrtc_host_snapshot_response(snapshot: &WebrtcHostSnapshot) -> WebrtcHostSnapshotResponse {
    WebrtcHostSnapshotResponse {
        local_offer: snapshot.local_offer.clone(),
        remote_offer: snapshot.remote_offer.clone(),
        local_answer: snapshot.local_answer.clone(),
        remote_answer: snapshot.remote_answer.clone(),
        remote_ice_count: snapshot.remote_ice_count,
        remote_video_track_count: snapshot.remote_video_track_count,
        remote_rtp_packet_count: snapshot.remote_rtp_packet_count,
        last_remote_codec: snapshot.last_remote_codec.clone(),
        last_remote_payload_type: snapshot.last_remote_payload_type,
        last_remote_fmtp_line: snapshot.last_remote_fmtp_line.clone(),
        remote_h264_access_unit_count: snapshot.remote_h264_access_unit_count,
        last_remote_access_unit_bytes: snapshot.last_remote_access_unit_bytes,
        recent_remote_access_unit_bytes: snapshot.recent_remote_access_unit_bytes.clone(),
        recent_remote_access_unit_keyframes: snapshot
            .recent_remote_access_unit_keyframes
            .iter()
            .map(|&b| b as usize)
            .collect(),
        decoded_frame_count: snapshot.decoded_frame_count,
        last_decoded_width: snapshot.last_decoded_width,
        last_decoded_height: snapshot.last_decoded_height,
        last_decoded_pixel_format: snapshot.last_decoded_pixel_format.clone(),
        decode_policy: snapshot.decode_policy.as_deref().unwrap_or("auto").to_string(),
        preferred_decode_backend: snapshot.preferred_decode_backend.clone(),
        active_decode_backend: snapshot.active_decode_backend.clone(),
        decode_backend_reason: snapshot.decode_backend_reason.clone(),
        decode_fallback_count: snapshot.decode_fallback_count,
        last_decode_fallback_reason: snapshot.last_decode_fallback_reason.clone(),
        decode_error_count: snapshot.decode_error_count,
        last_decode_error: snapshot.last_decode_error.clone(),
        available_video_source_ids: snapshot.available_video_source_ids.clone(),
        local_video_track_count: snapshot.local_video_track_count,
        captured_frame_count: snapshot.captured_frame_count,
        sent_access_unit_count: snapshot.sent_access_unit_count,
        sent_rtp_bytes: snapshot.sent_rtp_bytes,
        zero_write_access_unit_count: snapshot.zero_write_access_unit_count,
        sender_running: snapshot.sender_running,
        peer_connection_state: snapshot.peer_connection_state.clone().unwrap_or_default(),
        ice_connection_state: snapshot.ice_connection_state.clone().unwrap_or_default(),
    }
}

pub fn quic_host_snapshot_response(snapshot: &QuicHostSnapshot) -> QuicHostSnapshotResponse {
    QuicHostSnapshotResponse {
        transport: snapshot.transport.clone(),
        local_addr: snapshot.local_addr.clone(),
        peer_addr: snapshot.peer_addr.clone(),
        remote_datagram_count: snapshot.remote_datagram_count,
        remote_access_unit_count: snapshot.remote_access_unit_count,
        decoded_frame_count: snapshot.decoded_frame_count,
        last_decoded_width: snapshot.last_decoded_width,
        last_decoded_height: snapshot.last_decoded_height,
        last_decoded_pixel_format: snapshot.last_decoded_pixel_format.clone(),
        sent_access_unit_count: snapshot.sent_access_unit_count,
        sender_running: snapshot.sender_running,
        receiver_running: snapshot.receiver_running,
        active_decode_backend: snapshot.active_decode_backend.clone(),
        last_error: snapshot.last_error.clone(),
    }
}

pub fn decoded_frame_snapshot_response(
    snapshot: &DecodedFrameSnapshot,
) -> DecodedFrameSnapshotResponse {
    DecodedFrameSnapshotResponse {
        frame_count: snapshot.frame_count,
        width: snapshot.width,
        height: snapshot.height,
        pixel_format: match snapshot.pixel_format {
            mrd_decode::PixelFormat::Rgb24 => "Rgb24".to_string(),
            mrd_decode::PixelFormat::D3d11Texture => "D3d11Texture".to_string(),
        },
        bytes: snapshot.bytes,
    }
}

pub fn render_host_snapshot_response(snapshot: crate::render_host::RenderHostSnapshot) -> RenderHostSnapshotResponse {
    RenderHostSnapshotResponse {
        attached: snapshot.attached,
        surface_count: snapshot.surface_count,
        attached_surface_ids: snapshot.attached_surface_ids,
        frame: snapshot.frame.map(|frame| DecodedFrameSnapshotResponse {
            frame_count: frame.frame_count,
            width: frame.width,
            height: frame.height,
            pixel_format: frame.pixel_format,
            bytes: frame.bytes,
        }),
        preview_data_url: snapshot.preview_data_url,
        renderer_backend: snapshot.renderer_backend.unwrap_or_default(),
        renderer_snapshot: snapshot.renderer_snapshot.map(|s| format!("{:?}", s)),
        surface_source_bindings: snapshot
            .surface_source_bindings
            .into_iter()
            .map(|binding| SurfaceSourceBindingResponse {
                surface_id: binding.surface_id,
                source_id: binding.source_id,
            })
            .collect(),
        available_source_ids: snapshot.available_source_ids,
    }
}

pub fn render_surface_descriptor_response(
    surface: crate::render_surface_catalog::RenderSurfaceDescriptor,
    current_surface_id: Option<&str>,
) -> RenderSurfaceDescriptorResponse {
    RenderSurfaceDescriptorResponse {
        current: current_surface_id == Some(surface.surface_id.as_str()),
        surface_id: surface.surface_id,
        name: surface.name,
        role: surface.role,
    }
}

pub fn surface_source_binding_response(
    binding: SurfaceSourceBinding,
) -> SurfaceSourceBindingResponse {
    SurfaceSourceBindingResponse {
        surface_id: binding.surface_id,
        source_id: binding.source_id,
    }
}

pub fn session_lifecycle_snapshot_response(
    snapshot: SessionLifecycleSnapshot,
) -> SessionLifecycleSnapshotResponse {
    let current_surface_id = snapshot.current_surface_id.clone();
    SessionLifecycleSnapshotResponse {
        session_id: snapshot.session_id,
        current_surface_id: current_surface_id.clone(),
        surfaces: snapshot
            .surfaces
            .into_iter()
            .map(|surface| {
                render_surface_descriptor_response(surface, current_surface_id.as_deref())
            })
            .collect(),
        available_source_ids: snapshot.available_source_ids,
        surface_source_bindings: snapshot
            .surface_source_bindings
            .into_iter()
            .map(surface_source_binding_response)
            .collect(),
    }
}

pub fn nvdec_runtime_probe_response() -> NvdecRuntimeProbeResponse {
    let probe = probe_nvdec_runtime();
    NvdecRuntimeProbeResponse {
        backend: probe.backend.to_string(),
        summary: probe.summary,
        checked_items: probe
            .checked_items
            .into_iter()
            .map(str::to_string)
            .collect(),
        capability_probes: probe
            .capability_probes
            .into_iter()
            .map(|capability| NvdecCapabilityProbeResponse {
                codec: capability.codec,
                bit_depth_minus8: capability.bit_depth_minus8,
                chroma_format: capability.chroma_format as u8,
                runtime_supported: capability.runtime_supported,
                runtime_reason: if capability.runtime_reason.is_empty() {
                    None
                } else {
                    Some(capability.runtime_reason)
                },
                wired_supported: capability.wired_supported,
                wired_reason: if capability.wired_reason.is_empty() {
                    None
                } else {
                    Some(capability.wired_reason)
                },
            })
            .collect(),
    }
}

// =============================================================================
// Parse utility functions
// =============================================================================

pub fn parse_backend_role(role: &str) -> Result<BackendRole, String> {
    match role {
        "controller" => Ok(BackendRole::Controller),
        "agent" => Ok(BackendRole::Agent),
        other => Err(format!("不支持的 realtime role: {}", other)),
    }
}

pub fn parse_decode_policy(value: &str) -> Result<DecodePolicy, String> {
    match value {
        "auto" => Ok(DecodePolicy::Auto),
        "software" => Ok(DecodePolicy::Software),
        "d3d11va" => Ok(DecodePolicy::D3d11va),
        "nvdec" => Ok(DecodePolicy::Nvdec),
        other => Err(format!("未知 decode policy: {other}")),
    }
}

// =============================================================================
// Realtime runtime helper functions
// =============================================================================

pub async fn realtime_register_with(
    runtime: &RealtimeRuntime,
    role: String,
    device_id: Option<String>,
    name: String,
) -> Result<RealtimeRegistrationResponse, String> {
    let registration = runtime
        .register(parse_backend_role(&role)?, device_id.map(DeviceId), name)
        .await?;

    Ok(realtime_registration_response(registration))
}

pub async fn realtime_request_session_with(
    runtime: &RealtimeRuntime,
    handle: u64,
    session_id: String,
    target_device_id: String,
    transport: Option<String>,
    quic_listen_addr: Option<String>,
    quic_server_name: Option<String>,
    quic_cert_der_b64: Option<String>,
) -> Result<(), String> {
    runtime
        .request_session_with_transport(
            handle,
            SessionId(session_id),
            DeviceId(target_device_id),
            transport.unwrap_or_else(|| "webrtc".into()),
            quic_listen_addr,
            quic_server_name,
            quic_cert_der_b64,
        )
        .await
}

pub async fn realtime_accept_session_with(
    runtime: &RealtimeRuntime,
    handle: u64,
    session_id: String,
    transport: Option<String>,
    quic_listen_addr: Option<String>,
    quic_server_name: Option<String>,
    quic_cert_der_b64: Option<String>,
) -> Result<(), String> {
    runtime
        .accept_session_with_transport(
            handle,
            SessionId(session_id),
            transport.unwrap_or_else(|| "webrtc".into()),
            quic_listen_addr,
            quic_server_name,
            quic_cert_der_b64,
        )
        .await
}

pub async fn drain_realtime_events_with(
    runtime: &RealtimeRuntime,
    handle: u64,
) -> Result<Vec<SignalMessage>, String> {
    runtime.drain_events(handle).await
}

pub async fn apply_realtime_events_to_session_coordinators(
    runtime: &RealtimeRuntime,
    webrtc_sessions: &Mutex<WebrtcSessionCoordinator>,
    quic_sessions: &Mutex<QuicSessionCoordinator>,
    handle: u64,
) -> Result<Option<SessionId>, String> {
    let events = runtime.drain_events(handle).await?;
    let mut last_session_id: Option<SessionId> = None;
    for event in events {
        match event {
            SignalMessage::SessionRequest(request) => {
                if request.transport == "quic_quinn" {
                    quic_sessions.lock().await.accept_session(
                        request.session_id.clone(),
                        request.transport.clone(),
                        request.quic_listen_addr.clone(),
                        request.quic_server_name.clone(),
                        request.quic_cert_der_b64.clone(),
                    )?;
                } else {
                    webrtc_sessions.lock().await.create_local_offer(
                        request.session_id.clone(),
                        String::new(),
                    )?;
                }
                last_session_id = Some(request.session_id);
            }
            SignalMessage::SessionAccept(accept) => {
                if accept.transport == "quic_quinn" {
                    quic_sessions.lock().await.request_session(
                        accept.session_id.clone(),
                        mrd_proto::DeviceId("test-source".to_string()),
                        mrd_proto::DeviceId("test-target".to_string()),
                        accept.transport.clone(),
                        accept.quic_listen_addr.clone(),
                        accept.quic_server_name.clone(),
                        accept.quic_cert_der_b64.clone(),
                    )?;
                } else {
                    webrtc_sessions.lock().await.apply_remote_offer(
                        accept.session_id.clone(),
                        String::new(),
                    )?;
                }
                last_session_id = Some(accept.session_id);
            }
            SignalMessage::WebrtcOffer(offer) => {
                webrtc_sessions.lock().await.apply_remote_offer(
                    offer.session_id.clone(),
                    offer.sdp,
                )?;
                last_session_id = Some(offer.session_id);
            }
            SignalMessage::WebrtcAnswer(answer) => {
                webrtc_sessions.lock().await.apply_remote_answer(
                    answer.session_id.clone(),
                    answer.sdp,
                )?;
                last_session_id = Some(answer.session_id);
            }
            SignalMessage::IceCandidate(candidate) => {
                let coordinator = if candidate.session_id.0.starts_with("quic") {
                    continue;
                } else {
                    &webrtc_sessions
                };
                coordinator.lock().await.apply_remote_ice_candidate(
                    candidate.session_id.clone(),
                    IceCandidate {
                        session_id: candidate.session_id.clone(),
                        candidate: candidate.candidate,
                        sdp_mid: candidate.sdp_mid,
                        sdp_mline_index: candidate.sdp_mline_index,
                    },
                )?;
                last_session_id = Some(candidate.session_id);
            }
            SignalMessage::Registered(_) => {}
            SignalMessage::Register(_) => {}
        }
    }

    Ok(last_session_id)
}

// =============================================================================
// QUIC host/session helper functions
// =============================================================================

pub async fn prepare_quic_accept_with(
    quic_host: &Mutex<QuicHost>,
    quic_sessions: &Mutex<QuicSessionCoordinator>,
    session_id: SessionId,
) -> Result<(String, Option<String>, Option<String>, Option<String>), String> {
    use base64::Engine;

    let bootstrap = quic_host
        .lock()
        .await
        .prepare_listener(session_id.clone(), "127.0.0.1:0")
        .await?;
    quic_sessions.lock().await.accept_session(
        session_id,
        "quic_quinn".into(),
        Some(bootstrap.listen_addr.to_string()),
        Some(bootstrap.server_name.clone()),
        Some(base64::engine::general_purpose::STANDARD.encode(&bootstrap.cert_der)),
    )?;
    Ok((
        "quic_quinn".into(),
        Some(bootstrap.listen_addr.to_string()),
        Some(bootstrap.server_name),
        Some(base64::engine::general_purpose::STANDARD.encode(&bootstrap.cert_der)),
    ))
}

pub fn spawn_quic_accept_completion(quic_host: std::sync::Arc<Mutex<QuicHost>>, session_id: SessionId) {
    tokio::spawn(async move {
        let _ = quic_host.lock().await.accept_peer(session_id).await;
    });
}

pub async fn sync_quic_host_from_session_snapshot_with(
    quic_host: &Mutex<QuicHost>,
    quic_sessions: &Mutex<QuicSessionCoordinator>,
    local_device_id: &DeviceId,
    session_id: &SessionId,
) -> Result<(), String> {
    let snapshot = {
        let sessions = quic_sessions.lock().await;
        sessions.snapshot(session_id).cloned()
    };
    let Some(snapshot) = snapshot else {
        return Ok(());
    };
    if snapshot.transport != "quic_quinn" {
        return Ok(());
    }
    if snapshot.source_device_id.as_deref() != Some(local_device_id.0.as_str()) {
        return Ok(());
    }
    let remote_listen_addr = match snapshot.remote_listen_addr {
        Some(value) => value,
        None => return Ok(()),
    };
    let remote_server_name = match snapshot.remote_server_name {
        Some(value) => value,
        None => return Ok(()),
    };
    let remote_cert_der_b64 = match snapshot.remote_cert_der_b64 {
        Some(value) => value,
        None => return Ok(()),
    };

    {
        let host = quic_host.lock().await;
        if host.snapshot(session_id).is_some() {
            return Ok(());
        }
    }

    let cert_der = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(remote_cert_der_b64)
            .map_err(|error| format!("decode remote QUIC cert failed: {error}"))?
    };
    quic_host
        .lock()
        .await
        .connect_to_peer(
            session_id.clone(),
            "127.0.0.1:0",
            &mrd_transport_quic_quinn::QuinnServerBootstrap {
                transport: "quic_quinn",
                listen_addr: remote_listen_addr
                    .parse()
                    .map_err(|error| format!("parse remote QUIC listen addr failed: {error}"))?,
                server_name: remote_server_name,
                cert_der,
            },
            "h264_software",
        )
        .await
}

pub async fn quic_snapshot_with(
    coordinator: &Mutex<QuicSessionCoordinator>,
    session_id: String,
) -> Option<QuicSessionSnapshotResponse> {
    let sessions = coordinator.lock().await;
    sessions
        .snapshot(&SessionId(session_id))
        .map(quic_snapshot_response)
}

pub async fn quic_host_snapshot_with(
    host: &Mutex<QuicHost>,
    session_id: String,
) -> Option<QuicHostSnapshotResponse> {
    let host = host.lock().await;
    host.snapshot(&SessionId(session_id))
        .map(|snapshot| quic_host_snapshot_response(&snapshot))
}

// =============================================================================
// WebRTC session/host helper functions
// =============================================================================

pub async fn webrtc_create_local_offer_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
    sdp: String,
) -> Result<SessionDescription, String> {
    coordinator
        .lock()
        .await
        .create_local_offer(SessionId(session_id), sdp)
}

pub async fn webrtc_apply_remote_answer_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    coordinator
        .lock()
        .await
        .apply_remote_answer(SessionId(session_id), sdp)
}

pub async fn webrtc_apply_remote_ice_candidate_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) -> Result<(), String> {
    coordinator.lock().await.apply_remote_ice_candidate(
        SessionId(session_id.clone()),
        IceCandidate {
            session_id: SessionId(session_id),
            candidate,
            sdp_mid,
            sdp_mline_index,
        },
    )
}

pub async fn webrtc_sync_realtime_events_with(
    runtime: &RealtimeRuntime,
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    handle: u64,
) -> Result<WebrtcSessionSnapshotResponse, String> {
    let quic_sessions = Mutex::new(QuicSessionCoordinator::default());
    let session_id =
        apply_realtime_events_to_session_coordinators(runtime, coordinator, &quic_sessions, handle)
            .await?
            .ok_or_else(|| "未收到可应用的 webrtc 事件".to_string())?;
    let sessions = coordinator.lock().await;
    let snapshot = sessions
        .snapshot(&session_id)
        .ok_or_else(|| format!("未找到会话协商快照: {}", session_id.0))?;
    Ok(webrtc_snapshot_response(snapshot))
}

pub async fn webrtc_snapshot_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
) -> Option<WebrtcSessionSnapshotResponse> {
    let sessions = coordinator.lock().await;
    sessions
        .snapshot(&SessionId(session_id))
        .map(webrtc_snapshot_response)
}

pub async fn webrtc_host_create_offer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Result<SessionDescription, String> {
    host.lock().await.create_offer(SessionId(session_id)).await
}

pub async fn webrtc_host_apply_remote_offer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    host.lock()
        .await
        .apply_remote_offer(SessionId(session_id), sdp)
        .await
}

pub async fn webrtc_host_create_answer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Result<SessionDescription, String> {
    host.lock().await.create_answer(SessionId(session_id)).await
}

pub async fn webrtc_host_apply_remote_answer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    host.lock()
        .await
        .apply_remote_answer(SessionId(session_id), sdp)
        .await
}

pub async fn webrtc_host_apply_remote_ice_candidate_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) -> Result<(), String> {
    host.lock()
        .await
        .apply_remote_ice_candidate(
            SessionId(session_id.clone()),
            IceCandidate {
                session_id: SessionId(session_id),
                candidate,
                sdp_mid,
                sdp_mline_index,
            },
        )
        .await
}

pub async fn webrtc_host_snapshot_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Option<WebrtcHostSnapshotResponse> {
    let host = host.lock().await;
    host.snapshot(&SessionId(session_id))
        .map(|snapshot| webrtc_host_snapshot_response(&snapshot))
}

// =============================================================================
// Session runtime helper functions
// =============================================================================

use crate::session_runtime::sync_session_runtime;

pub async fn session_runtime_snapshot_with(
    lifecycle: &std::sync::Mutex<SessionLifecycleCoordinator>,
    render_host: &std::sync::Mutex<RenderHost>,
    webrtc_host: &Mutex<WebrtcHost>,
    quic_host: &Mutex<QuicHost>,
    webrtc_sessions: &Mutex<WebrtcSessionCoordinator>,
    quic_sessions: &Mutex<QuicSessionCoordinator>,
    session_id: SessionId,
) -> Result<SessionRuntimeSnapshotResponse, String> {
    let lifecycle_snapshot = {
        let mut lifecycle = lifecycle.lock().expect("lock session lifecycle");
        let mut render_host = render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
        lifecycle.snapshot(&session_id)
    };

    let render_host_snapshot = render_host_snapshot_with(render_host, session_id.0.clone()).await?;
    let webrtc_host_snapshot = webrtc_host_snapshot_with(webrtc_host, session_id.0.clone()).await;
    let quic_host_snapshot = quic_host_snapshot_with(quic_host, session_id.0.clone()).await;
    let webrtc_signaling = webrtc_snapshot_with(webrtc_sessions, session_id.0.clone()).await;
    let quic_signaling = quic_snapshot_with(quic_sessions, session_id.0.clone()).await;

    Ok(SessionRuntimeSnapshotResponse {
        lifecycle: session_lifecycle_snapshot_response(lifecycle_snapshot),
        render_host: render_host_snapshot,
        webrtc_host: webrtc_host_snapshot,
        quic_host: quic_host_snapshot,
        webrtc_signaling,
        quic_signaling,
    })
}

// =============================================================================
// Decoded frame helper functions
// =============================================================================

pub fn decoded_frame_snapshot_with(
    sink: &std::sync::Mutex<DecodedFrameSink>,
    session_id: String,
) -> Option<DecodedFrameSnapshotResponse> {
    sink.lock()
        .expect("lock decoded frame sink")
        .snapshot(&SessionId(session_id))
        .map(decoded_frame_snapshot_response)
}

pub fn decoded_frame_preview_with(
    sink: &std::sync::Mutex<DecodedFrameSink>,
    session_id: String,
) -> Result<Option<String>, String> {
    let latest_frame = {
        let sink = sink.lock().expect("lock decoded frame sink");
        sink.latest_frame(&SessionId(session_id)).cloned()
    };

    let Some(frame) = latest_frame else {
        return Ok(None);
    };

    let Some(rgb) = frame.cpu_bytes() else {
        return Ok(None);
    };
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            rgb,
            frame.width as u32,
            frame.height as u32,
            ColorType::Rgb8.into(),
        )
        .map_err(|error| format!("encode decoded frame preview failed: {error}"))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);
    Ok(Some(format!("data:image/png;base64,{encoded}")))
}

// =============================================================================
// Render host helper functions
// =============================================================================

pub async fn render_host_snapshot_with(
    render_host: &std::sync::Mutex<RenderHost>,
    session_id: String,
) -> Result<RenderHostSnapshotResponse, String> {
    let mut host = render_host.lock().expect("lock render host");
    let snapshot = host.snapshot(&SessionId(session_id))?;
    Ok(render_host_snapshot_response(snapshot))
}

// =============================================================================
// Settings helper functions
// =============================================================================

pub async fn set_decode_policy_with(
    settings_path: &std::path::Path,
    decode_policy: DecodePolicy,
) -> Result<DecodePolicyResponse, String> {
    save_settings(settings_path, &AppSettings { decode_policy })?;
    Ok(DecodePolicyResponse {
        decode_policy: decode_policy.as_str().to_string(),
    })
}
