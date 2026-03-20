// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_settings;
mod benchmark;
mod ipc_client;
mod service_manager;
mod device_info;
mod frame_sink;
mod quic_host;
mod quic_session;
#[cfg(test)]
mod quic_transport_harness;
mod realtime_client;
mod realtime_management;
mod realtime_runtime;
mod render_host;
mod render_surface_catalog;
mod render_window_registry;
mod session_lifecycle;
mod session_runtime;
mod webrtc_host;
mod webrtc_media;
mod webrtc_session;

use app_settings::{
    default_settings_path, load_settings, save_settings, AppSettings, DecodePolicy,
};
use device_info::HardwareInfo;
use frame_sink::{DecodedFrameSink, DecodedFrameSnapshot};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_decode_nvdec::probe_runtime as probe_nvdec_runtime;
use mrd_observability::{MediaProbeEvent, PipelineProbeSnapshot, ProbeRegistry};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::encode_message;
use mrd_ipc;
use mrd_signal_proto::{IceCandidate, SessionDescription, SignalMessage};
use quic_host::{QuicHost, QuicHostSnapshot};
use quic_session::{QuicSessionCoordinator, QuicSessionSnapshot};
use realtime_management::{RealtimeManagementClient, RealtimeStatus};
use realtime_runtime::{RealtimeRegistration, RealtimeRuntime};
use render_host::{
    render_host_snapshot_with, RenderHost, RenderHostSnapshot, RendererSnapshotResponse,
};
use render_surface_catalog::RenderSurfaceDescriptor;
use render_window_registry::{RenderWindowContext, RenderWindowRegistry};
use serde::{Deserialize, Serialize};
use session_lifecycle::{
    SessionLifecycleCoordinator, SessionLifecycleSnapshot, SurfaceSourceBinding,
};
use session_runtime::sync_session_runtime;
use std::collections::HashMap;
use tauri::Manager;
use tokio::sync::Mutex;
use webrtc_host::{WebrtcHost, WebrtcHostSnapshot};
use webrtc_session::{WebrtcSessionCoordinator, WebrtcSessionSnapshot};

#[derive(Clone)]
struct AppState {
    frame_sink: std::sync::Arc<std::sync::Mutex<DecodedFrameSink>>,
    render_host: std::sync::Arc<std::sync::Mutex<RenderHost>>,
    render_windows: std::sync::Arc<std::sync::Mutex<RenderWindowRegistry>>,
    session_lifecycle: std::sync::Arc<std::sync::Mutex<SessionLifecycleCoordinator>>,
    realtime_runtime: RealtimeRuntime,
    settings_path: std::path::PathBuf,
    webrtc_host: std::sync::Arc<Mutex<WebrtcHost>>,
    webrtc_sessions: std::sync::Arc<Mutex<WebrtcSessionCoordinator>>,
    quic_host: std::sync::Arc<Mutex<QuicHost>>,
    quic_sessions: std::sync::Arc<Mutex<QuicSessionCoordinator>>,
    /// Service lifecycle manager - shared singleton for mrd-service
    service_manager: std::sync::Arc<std::sync::Mutex<service_manager::ServiceManager>>,
}

/// 设备注册响应
#[derive(Debug, Serialize, Deserialize)]
struct DeviceRegistrationResponse {
    device_id: String,
    device_name: String,
    access_token: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RealtimeRegistrationResponse {
    handle: u64,
    device_id: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WebrtcSessionSnapshotResponse {
    local_offer: Option<String>,
    remote_offer: Option<String>,
    remote_answer: Option<String>,
    remote_ice_candidates: Vec<IceCandidate>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct QuicSessionSnapshotResponse {
    transport: String,
    source_device_id: Option<String>,
    target_device_id: Option<String>,
    local_listen_addr: Option<String>,
    local_server_name: Option<String>,
    local_cert_der_b64: Option<String>,
    remote_listen_addr: Option<String>,
    remote_server_name: Option<String>,
    remote_cert_der_b64: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WebrtcHostSnapshotResponse {
    local_offer: Option<String>,
    remote_offer: Option<String>,
    local_answer: Option<String>,
    remote_answer: Option<String>,
    remote_ice_count: usize,
    remote_video_track_count: usize,
    remote_rtp_packet_count: u64,
    last_remote_codec: Option<String>,
    last_remote_payload_type: Option<u8>,
    last_remote_fmtp_line: Option<String>,
    remote_h264_access_unit_count: u64,
    last_remote_access_unit_bytes: usize,
    recent_remote_access_unit_bytes: Vec<usize>,
    recent_remote_access_unit_keyframes: Vec<bool>,
    decoded_frame_count: u64,
    last_decoded_width: usize,
    last_decoded_height: usize,
    last_decoded_pixel_format: Option<String>,
    decode_policy: Option<String>,
    preferred_decode_backend: Option<String>,
    active_decode_backend: Option<String>,
    decode_backend_reason: Option<String>,
    decode_fallback_count: u64,
    last_decode_fallback_reason: Option<String>,
    decode_error_count: u64,
    last_decode_error: Option<String>,
    available_video_source_ids: Vec<String>,
    local_video_track_count: usize,
    captured_frame_count: u64,
    sent_access_unit_count: u64,
    sent_rtp_bytes: u64,
    zero_write_access_unit_count: u64,
    sender_running: bool,
    peer_connection_state: Option<String>,
    ice_connection_state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DecodedFrameSnapshotResponse {
    frame_count: u64,
    width: usize,
    height: usize,
    pixel_format: String,
    bytes: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RenderHostSnapshotResponse {
    attached: bool,
    surface_count: usize,
    attached_surface_ids: Vec<String>,
    frame: Option<DecodedFrameSnapshotResponse>,
    preview_data_url: Option<String>,
    renderer_backend: Option<String>,
    renderer_snapshot: Option<RendererSnapshotResponse>,
    surface_source_bindings: Vec<SurfaceSourceBindingResponseResponse>,
    available_source_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RenderWindowContextResponse {
    label: String,
    session_id: String,
    surface_id: String,
    role: String,
    renderer_attached: bool,
    session_window_count: usize,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct RenderSurfaceDescriptorResponse {
    surface_id: String,
    name: String,
    role: String,
    current: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SurfaceSourceBindingResponseResponse {
    surface_id: String,
    source_id: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SessionLifecycleSnapshotResponse {
    session_id: String,
    current_surface_id: Option<String>,
    surfaces: Vec<RenderSurfaceDescriptorResponse>,
    available_source_ids: Vec<String>,
    surface_source_bindings: Vec<SurfaceSourceBindingResponseResponse>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct SessionRuntimeSnapshotResponse {
    lifecycle: SessionLifecycleSnapshotResponse,
    render_host: RenderHostSnapshotResponse,
    webrtc_host: Option<WebrtcHostSnapshotResponse>,
    quic_host: Option<QuicHostSnapshotResponse>,
    webrtc_signaling: Option<WebrtcSessionSnapshotResponse>,
    quic_signaling: Option<QuicSessionSnapshotResponse>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct QuicHostSnapshotResponse {
    transport: String,
    local_addr: Option<String>,
    peer_addr: Option<String>,
    remote_datagram_count: u64,
    remote_access_unit_count: u64,
    decoded_frame_count: u64,
    last_decoded_width: usize,
    last_decoded_height: usize,
    last_decoded_pixel_format: Option<String>,
    sent_access_unit_count: u64,
    sender_running: bool,
    receiver_running: bool,
    active_decode_backend: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NvdecCapabilityProbeResponse {
    codec: String,
    bit_depth_minus8: u8,
    chroma_format: i32,
    runtime_supported: bool,
    runtime_reason: String,
    wired_supported: bool,
    wired_reason: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NvdecRuntimeProbeResponse {
    backend: String,
    summary: String,
    checked_items: Vec<String>,
    capability_probes: Vec<NvdecCapabilityProbeResponse>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DecodePolicyResponse {
    decode_policy: String,
}

/// Tauri 命令：获取硬件信息
#[tauri::command]
fn get_hardware_info() -> Result<HardwareInfo, String> {
    Ok(device_info::get_hardware_info())
}

#[tauri::command]
fn nvdec_runtime_probe() -> Result<NvdecRuntimeProbeResponse, String> {
    Ok(nvdec_runtime_probe_response())
}

#[tauri::command]
async fn decode_policy(state: tauri::State<'_, AppState>) -> Result<DecodePolicyResponse, String> {
    Ok(decode_policy_with(state.webrtc_host.as_ref()).await)
}

#[tauri::command]
async fn set_decode_policy(
    state: tauri::State<'_, AppState>,
    decode_policy: String,
) -> Result<DecodePolicyResponse, String> {
    let decode_policy = parse_decode_policy(&decode_policy)?;
    set_decode_policy_with(
        &state.settings_path,
        state.webrtc_host.as_ref(),
        decode_policy,
    )
    .await
}

#[tauri::command]
async fn realtime_status() -> Result<RealtimeStatus, String> {
    RealtimeManagementClient::from_env().status().await
}

#[tauri::command]
async fn realtime_start() -> Result<RealtimeStatus, String> {
    RealtimeManagementClient::from_env().start().await
}

#[tauri::command]
async fn realtime_stop() -> Result<RealtimeStatus, String> {
    RealtimeManagementClient::from_env().stop().await
}

#[tauri::command]
async fn realtime_restart() -> Result<RealtimeStatus, String> {
    RealtimeManagementClient::from_env().restart().await
}

// mrd-service lifecycle commands
#[tauri::command]
async fn service_start(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().start().await
        })
    }).await.map_err(|e| e.to_string())?;

    Ok(true)
}

#[tauri::command]
async fn service_stop(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().stop().await
        })
    }).await.map_err(|e| e.to_string())?;

    Ok(true)
}

#[tauri::command]
async fn service_status(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    let is_running = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().is_running().await
        })
    }).await.map_err(|e| e.to_string())?;

    Ok(is_running)
}

#[tauri::command]
async fn service_health_check(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().health_check().await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn service_wait_for_healthy(state: tauri::State<'_, AppState>, timeout_secs: u64) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().wait_for_healthy(timeout_secs).await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn service_restart_with_backoff(state: tauri::State<'_, AppState>, max_attempts: u32) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().restart_with_backoff(max_attempts).await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())?;

    Ok(true)
}

#[tauri::command]
async fn service_pid(state: tauri::State<'_, AppState>) -> Result<Option<u32>, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            Ok(manager.lock().unwrap().pid().await)
        })
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn service_restart(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().restart().await
        })
    }).await.map_err(|e| e.to_string())?;

    Ok(true)
}

// Service guard command - starts monitoring the service
#[tauri::command]
async fn service_start_guard(state: tauri::State<'_, AppState>) -> Result<String, String> {
    use service_manager::{ServiceGuard, ServiceGuardConfig};
    use std::sync::Arc;

    let config = ServiceGuardConfig::default();
    let guard = ServiceGuard::new(config).map_err(|e| e.to_string())?;

    // Start the guard in the background
    let handle = guard.start();

    // Return a handle ID (in a real implementation, you'd store this)
    Ok(format!("Guard started with handle: {:?}", handle))
}

// ============================================================================
// IPC-based commands (migrated to use mrd-service)
// ============================================================================

/// Register device via IPC (migrated version)
#[tauri::command]
async fn ipc_register_device(
    device_id: String,
    device_name: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::DeviceId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::RegisterDevice {
        device_id: DeviceId(device_id),
        device_name,
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::DeviceRegistered { device_id } => Ok(device_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// List devices via IPC (migrated version)
#[tauri::command]
async fn ipc_list_devices() -> Result<Vec<mrd_ipc::DeviceInfo>, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::ListDevices).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::DeviceList { devices } => Ok(devices),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start session via IPC (migrated version)
#[tauri::command]
async fn ipc_start_session(
    session_id: String,
    target_device_id: String,
    transport_kind: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::{SessionId, DeviceId};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::StartSession {
        session_id: SessionId(session_id),
        target_device_id: DeviceId(target_device_id),
        transport_kind,
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Accept session via IPC (migrated version)
#[tauri::command]
async fn ipc_accept_session(
    session_id: String,
    source_device_id: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::{SessionId, DeviceId};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::AcceptSession {
        session_id: SessionId(session_id),
        source_device_id: DeviceId(source_device_id),
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionAccepted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Stop session via IPC (migrated version)
#[tauri::command]
async fn ipc_stop_session(
    session_id: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::StopSession {
        session_id: SessionId(session_id),
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionStopped { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get session snapshot via IPC (migrated version)
#[tauri::command]
async fn ipc_session_snapshot(
    session_id: String,
) -> Result<mrd_ipc::SessionRuntimeSnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::SessionRuntimeSnapshot {
        session_id: SessionId(session_id),
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionSnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start sender via IPC (migrated version)
#[tauri::command]
async fn ipc_start_sender(
    session_id: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::StartSender {
        session_id: SessionId(session_id),
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SenderStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start receiver via IPC (migrated version)
#[tauri::command]
async fn ipc_start_receiver(
    session_id: String,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::StartReceiver {
        session_id: SessionId(session_id),
    }).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ReceiverStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

#[tauri::command]
async fn realtime_register(
    state: tauri::State<'_, AppState>,
    role: String,
    device_id: Option<String>,
    name: String,
) -> Result<RealtimeRegistrationResponse, String> {
    realtime_register_with(&state.realtime_runtime, role, device_id, name).await
}

// Example of migrated realtime command using IPC
#[tauri::command]
async fn realtime_register_via_ipc(
    role: String,
    device_id: Option<String>,
    name: String,
) -> Result<RealtimeRegistrationResponse, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let device_id_for_register = device_id.unwrap_or_else(|| {
        // Generate a device ID if not provided
        use std::time::SystemTime;
        format!("device-{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis())
    });

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::RegisterDevice {
        device_id: mrd_proto::DeviceId(device_id_for_register),
        device_name: name,
    }).await.map_err(|e| format!("IPC error: {}", e))?;

    match response {
        IpcResponse::DeviceRegistered { device_id } => {
            Ok(RealtimeRegistrationResponse {
                handle: 0,  // TODO: Return actual handle from service
                device_id: device_id.0,
            })
        }
        IpcResponse::Error { code, message } => {
            Err(format!("{}: {}", code, message))
        }
        _ => Err("Unexpected response".to_string()),
    }
}

#[tauri::command]
async fn realtime_request_session(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    target_device_id: String,
    transport: Option<String>,
    quic_listen_addr: Option<String>,
    quic_server_name: Option<String>,
    quic_cert_der_b64: Option<String>,
) -> Result<(), String> {
    if transport.as_deref() == Some("quic_quinn") {
        let source_device_id = state.realtime_runtime.device_id(handle).await?;
        state
            .quic_sessions
            .lock()
            .await
            .request_session(
                SessionId(session_id.clone()),
                source_device_id,
                DeviceId(target_device_id.clone()),
                "quic_quinn".into(),
                quic_listen_addr.clone(),
                quic_server_name.clone(),
                quic_cert_der_b64.clone(),
            )?;
    }
    realtime_request_session_with(
        &state.realtime_runtime,
        handle,
        session_id,
        target_device_id,
        transport,
        quic_listen_addr,
        quic_server_name,
        quic_cert_der_b64,
    )
    .await
}

#[tauri::command]
async fn realtime_accept_session(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    transport: Option<String>,
    quic_listen_addr: Option<String>,
    quic_server_name: Option<String>,
    quic_cert_der_b64: Option<String>,
) -> Result<(), String> {
    let (transport, quic_listen_addr, quic_server_name, quic_cert_der_b64) =
        if transport.as_deref() == Some("quic_quinn")
            && (quic_listen_addr.is_none()
                || quic_server_name.is_none()
                || quic_cert_der_b64.is_none())
        {
            prepare_quic_accept_with(
                state.quic_host.as_ref(),
                state.quic_sessions.as_ref(),
                SessionId(session_id.clone()),
            )
            .await?
        } else {
            (
                transport.unwrap_or_else(|| "webrtc".into()),
                quic_listen_addr,
                quic_server_name,
                quic_cert_der_b64,
            )
        };
    realtime_accept_session_with(
        &state.realtime_runtime,
        handle,
        session_id.clone(),
        Some(transport.clone()),
        quic_listen_addr,
        quic_server_name,
        quic_cert_der_b64,
    )
    .await?;

    if transport == "quic_quinn" {
        spawn_quic_accept_completion(state.quic_host.clone(), SessionId(session_id));
    }
    Ok(())
}

#[tauri::command]
async fn realtime_drain_events(
    state: tauri::State<'_, AppState>,
    handle: u64,
) -> Result<Vec<String>, String> {
    let events = drain_realtime_events_with(&state.realtime_runtime, handle).await?;
    events
        .into_iter()
        .map(|event| encode_message(&event).map_err(|e| format!("编码 realtime 事件失败: {}", e)))
        .collect()
}

#[tauri::command]
async fn realtime_send_offer(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    state
        .realtime_runtime
        .send_offer(
            handle,
            SessionDescription {
                session_id: SessionId(session_id),
                sdp,
            },
        )
        .await
}

#[tauri::command]
async fn realtime_send_answer(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    state
        .realtime_runtime
        .send_answer(
            handle,
            SessionDescription {
                session_id: SessionId(session_id),
                sdp,
            },
        )
        .await
}

#[tauri::command]
async fn realtime_send_ice_candidate(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) -> Result<(), String> {
    state
        .realtime_runtime
        .send_ice_candidate(
            handle,
            IceCandidate {
                session_id: SessionId(session_id),
                candidate,
                sdp_mid,
                sdp_mline_index,
            },
        )
        .await
}

#[tauri::command]
async fn webrtc_create_local_offer(
    state: tauri::State<'_, AppState>,
    session_id: String,
    sdp: String,
) -> Result<String, String> {
    let description =
        webrtc_create_local_offer_with(state.webrtc_sessions.as_ref(), session_id, sdp).await?;
    Ok(description.sdp)
}

#[tauri::command]
async fn webrtc_apply_remote_answer(
    state: tauri::State<'_, AppState>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    webrtc_apply_remote_answer_with(state.webrtc_sessions.as_ref(), session_id, sdp).await
}

#[tauri::command]
async fn webrtc_apply_remote_ice_candidate(
    state: tauri::State<'_, AppState>,
    session_id: String,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) -> Result<(), String> {
    webrtc_apply_remote_ice_candidate_with(
        state.webrtc_sessions.as_ref(),
        session_id,
        candidate,
        sdp_mid,
        sdp_mline_index,
    )
    .await
}

#[tauri::command]
async fn webrtc_sync_realtime_events(
    state: tauri::State<'_, AppState>,
    handle: u64,
) -> Result<WebrtcSessionSnapshotResponse, String> {
    webrtc_sync_realtime_events_with(
        &state.realtime_runtime,
        state.webrtc_sessions.as_ref(),
        handle,
    )
    .await
}

#[tauri::command]
async fn webrtc_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<WebrtcSessionSnapshotResponse>, String> {
    Ok(webrtc_snapshot_with(state.webrtc_sessions.as_ref(), session_id).await)
}

#[tauri::command]
async fn quic_session_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<QuicSessionSnapshotResponse>, String> {
    Ok(quic_snapshot_with(state.quic_sessions.as_ref(), session_id).await)
}

#[tauri::command]
async fn quic_host_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<QuicHostSnapshotResponse>, String> {
    Ok(quic_host_snapshot_with(state.quic_host.as_ref(), session_id).await)
}

// Example of migrated command using IPC (for demonstration)
#[tauri::command]
async fn quic_session_snapshot_via_ipc(
    session_id: String,
) -> Result<Option<mrd_ipc::SessionRuntimeSnapshot>, String> {
    use mrd_ipc::IpcRequest;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::SessionRuntimeSnapshot {
        session_id: SessionId(session_id),
    }).await.map_err(|e| format!("IPC error: {}", e))?;

    match response {
        mrd_ipc::IpcResponse::SessionSnapshot { snapshot } => Ok(Some(snapshot)),
        mrd_ipc::IpcResponse::Error { code, message } => {
            Err(format!("{}: {}", code, message))
        }
        _ => Err("Unexpected response".to_string()),
    }
}

#[tauri::command]
async fn webrtc_host_create_offer(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let description =
        webrtc_host_create_offer_with(state.webrtc_host.as_ref(), session_id.clone()).await?;
    webrtc_create_local_offer_with(
        state.webrtc_sessions.as_ref(),
        session_id,
        description.sdp.clone(),
    )
    .await?;
    Ok(description.sdp)
}

#[tauri::command]
async fn webrtc_host_apply_remote_offer(
    state: tauri::State<'_, AppState>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    webrtc_host_apply_remote_offer_with(
        state.webrtc_host.as_ref(),
        session_id.clone(),
        sdp.clone(),
    )
    .await?;
    state
        .webrtc_sessions
        .lock()
        .await
        .apply_remote_offer(SessionId(session_id), sdp)
}

#[tauri::command]
async fn webrtc_host_create_answer(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let description =
        webrtc_host_create_answer_with(state.webrtc_host.as_ref(), session_id.clone()).await?;
    state
        .webrtc_sessions
        .lock()
        .await
        .apply_remote_answer(SessionId(session_id), description.sdp.clone())?;
    Ok(description.sdp)
}

#[tauri::command]
async fn webrtc_host_apply_remote_answer(
    state: tauri::State<'_, AppState>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    webrtc_host_apply_remote_answer_with(
        state.webrtc_host.as_ref(),
        session_id.clone(),
        sdp.clone(),
    )
    .await?;
    webrtc_apply_remote_answer_with(state.webrtc_sessions.as_ref(), session_id, sdp).await
}

#[tauri::command]
async fn webrtc_host_apply_remote_ice_candidate(
    state: tauri::State<'_, AppState>,
    session_id: String,
    candidate: String,
    sdp_mid: Option<String>,
    sdp_mline_index: Option<u16>,
) -> Result<(), String> {
    webrtc_host_apply_remote_ice_candidate_with(
        state.webrtc_host.as_ref(),
        session_id.clone(),
        candidate.clone(),
        sdp_mid.clone(),
        sdp_mline_index,
    )
    .await?;
    webrtc_apply_remote_ice_candidate_with(
        state.webrtc_sessions.as_ref(),
        session_id,
        candidate,
        sdp_mid,
        sdp_mline_index,
    )
    .await
}

#[tauri::command]
async fn webrtc_host_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<WebrtcHostSnapshotResponse>, String> {
    Ok(webrtc_host_snapshot_with(state.webrtc_host.as_ref(), session_id).await)
}

// Example of migrated WebRTC command using IPC
#[tauri::command]
async fn webrtc_session_list_via_ipc() -> Result<Vec<String>, String> {
    use mrd_ipc::IpcRequest;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::ListDevices).await
        .map_err(|e| format!("IPC error: {}", e))?;

    match response {
        mrd_ipc::IpcResponse::DeviceList { devices } => {
            Ok(devices.into_iter().map(|d| d.device_id.0).collect())
        }
        mrd_ipc::IpcResponse::Error { code, message } => {
            Err(format!("{}: {}", code, message))
        }
        _ => Err("Unexpected response".to_string()),
    }
}

#[tauri::command]
async fn session_runtime_probe_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<PipelineProbeSnapshot>, String> {
    let session_id = SessionId(session_id);
    if let Some(snapshot) = state.webrtc_host.lock().await.probe_snapshot(&session_id) {
        return Ok(Some(snapshot));
    }
    let host = state.quic_host.lock().await;
    Ok(host.probe_snapshot(&session_id))
}

#[tauri::command]
async fn session_runtime_probe_recent_events(
    state: tauri::State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<MediaProbeEvent>, String> {
    let session_id = SessionId(session_id);
    let limit = limit.unwrap_or(64);
    let host = state.webrtc_host.lock().await;
    let events = host.probe_recent_events(&session_id, limit);
    if !events.is_empty() {
        return Ok(events);
    }
    let host = state.quic_host.lock().await;
    Ok(host.probe_recent_events(&session_id, limit))
}

#[tauri::command]
async fn webrtc_host_start_embedded_desktop_sender(
    state: tauri::State<'_, AppState>,
    session_id: String,
    fps: Option<u32>,
) -> Result<(), String> {
    state
        .webrtc_host
        .lock()
        .await
        .start_embedded_desktop_sender(SessionId(session_id), fps.unwrap_or(15))
        .await
}

#[tauri::command]
async fn webrtc_host_stop_embedded_video_sender(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .webrtc_host
        .lock()
        .await
        .stop_embedded_video_sender(&SessionId(session_id))
        .await
}

#[tauri::command]
async fn quic_host_start_embedded_desktop_sender(
    state: tauri::State<'_, AppState>,
    session_id: String,
    fps: Option<u32>,
) -> Result<(), String> {
    state
        .quic_host
        .lock()
        .await
        .start_embedded_desktop_sender(SessionId(session_id), fps.unwrap_or(15))
        .await
}

#[tauri::command]
async fn quic_host_stop_embedded_video_sender(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .quic_host
        .lock()
        .await
        .stop_embedded_video_sender(&SessionId(session_id))
        .await
}

#[tauri::command]
async fn decoded_frame_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<DecodedFrameSnapshotResponse>, String> {
    Ok(decoded_frame_snapshot_with(
        state.frame_sink.as_ref(),
        session_id,
    ))
}

#[tauri::command]
async fn decoded_frame_preview(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    decoded_frame_preview_with(state.frame_sink.as_ref(), session_id)
}

#[tauri::command]
async fn render_host_attach_session(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let window_handle = current_window_handle(&window)?;
    let surface_id = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .context_for_label(&window.app_handle(), window.label())
        .map(|context| context.surface_id)
        .unwrap_or_else(|| "surface-1".to_string());
    let session_id = SessionId(session_id);
    state
        .render_host
        .lock()
        .expect("lock render host")
        .attach_session(session_id.clone(), surface_id, window_handle)?;
    {
        let mut lifecycle = state
            .session_lifecycle
            .lock()
            .expect("lock session lifecycle");
        let mut render_host = state.render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
    }
    state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .set_renderer_attached(&window.app_handle(), window.label(), true);
    Ok(())
}

#[tauri::command]
async fn bind_current_render_window_surface(
    window: tauri::Window,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    surface_id: String,
) -> Result<(), String> {
    let window_label = window.label().to_string();
    let window_handle = current_window_handle(&window)?;
    let (session_id, previous_surface_id) = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .rebind_window_surface(&app, &window_label, surface_id.clone())?;

    if let Some(previous_surface_id) = previous_surface_id {
        let remaining_count = state
            .render_windows
            .lock()
            .expect("lock render window registry")
            .surface_window_count(&app, &session_id, &previous_surface_id);
        if remaining_count == 0 {
            state
                .render_host
                .lock()
                .expect("lock render host")
                .detach_surface(&session_id, &previous_surface_id);
        }
    }

    state
        .render_host
        .lock()
        .expect("lock render host")
        .attach_session(session_id.clone(), surface_id, window_handle)?;
    {
        let mut lifecycle = state
            .session_lifecycle
            .lock()
            .expect("lock session lifecycle");
        let mut render_host = state.render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
    }
    state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .set_renderer_attached(&app, &window_label, true);
    Ok(())
}

#[tauri::command]
async fn render_host_detach_session(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    state
        .render_host
        .lock()
        .expect("lock render host")
        .detach_session(&SessionId(session_id));
    Ok(())
}

#[tauri::command]
async fn render_host_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<RenderHostSnapshotResponse, String> {
    let session_id = SessionId(session_id);
    {
        let mut lifecycle = state
            .session_lifecycle
            .lock()
            .expect("lock session lifecycle");
        let mut render_host = state.render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
    }
    let snapshot = render_host_snapshot_with(state.render_host.as_ref(), session_id.0)?;
    Ok(render_host_snapshot_response(snapshot))
}

#[tauri::command]
async fn bind_render_surface_source(
    state: tauri::State<'_, AppState>,
    session_id: String,
    surface_id: String,
    source_id: String,
) -> Result<(), String> {
    let session_id = SessionId(session_id);
    state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .bind_surface_source(session_id.clone(), surface_id, source_id)?;
    {
        let mut lifecycle = state
            .session_lifecycle
            .lock()
            .expect("lock session lifecycle");
        let mut render_host = state.render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
    }
    Ok(())
}

#[tauri::command]
async fn session_lifecycle_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<SessionLifecycleSnapshotResponse, String> {
    let session_id = SessionId(session_id);
    {
        let mut lifecycle = state
            .session_lifecycle
            .lock()
            .expect("lock session lifecycle");
        let mut render_host = state.render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
    }
    let snapshot = state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .snapshot(&session_id);
    Ok(session_lifecycle_snapshot_response(snapshot))
}

#[tauri::command]
async fn session_runtime_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<SessionRuntimeSnapshotResponse, String> {
    session_runtime_snapshot_with(
        state.session_lifecycle.as_ref(),
        state.render_host.as_ref(),
        state.webrtc_host.as_ref(),
        state.quic_host.as_ref(),
        state.webrtc_sessions.as_ref(),
        state.quic_sessions.as_ref(),
        SessionId(session_id),
    )
    .await
}

#[tauri::command]
async fn session_runtime_sync_realtime(
    state: tauri::State<'_, AppState>,
    handle: u64,
) -> Result<Option<SessionRuntimeSnapshotResponse>, String> {
    let Some(session_id) = apply_realtime_events_to_session_coordinators(
        &state.realtime_runtime,
        state.webrtc_sessions.as_ref(),
        state.quic_sessions.as_ref(),
        handle,
    )
    .await?
    else {
        return Ok(None);
    };
    let local_device_id = state.realtime_runtime.device_id(handle).await?;
    sync_quic_host_from_session_snapshot_with(
        state.quic_host.as_ref(),
        state.quic_sessions.as_ref(),
        &local_device_id,
        &session_id,
    )
    .await?;

    session_runtime_snapshot_with(
        state.session_lifecycle.as_ref(),
        state.render_host.as_ref(),
        state.webrtc_host.as_ref(),
        state.quic_host.as_ref(),
        state.webrtc_sessions.as_ref(),
        state.quic_sessions.as_ref(),
        session_id,
    )
    .await
    .map(Some)
}

#[tauri::command]
fn open_render_window(app: tauri::AppHandle, session_id: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    let surface_id = state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .create_surface(SessionId(session_id.clone()), None)
        .surface_id;
    let result = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .open_window(&app, SessionId(session_id), Some(surface_id));
    result
}

#[tauri::command]
fn open_render_surface_window(
    app: tauri::AppHandle,
    session_id: String,
    surface_id: String,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .ensure_surface(SessionId(session_id.clone()), surface_id.clone());
    let result = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .open_window(&app, SessionId(session_id), Some(surface_id));
    result
}

#[tauri::command]
fn list_render_windows(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Vec<RenderWindowContextResponse>, String> {
    let state = app.state::<AppState>();
    let windows = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .list_window_contexts(&app, &SessionId(session_id))
        .into_iter()
        .map(render_window_context_response)
        .collect();
    Ok(windows)
}

#[tauri::command]
fn close_render_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let result = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .close_window(&app, &label);
    result
}

#[tauri::command]
fn render_window_context(
    window: tauri::Window,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<RenderWindowContextResponse>, String> {
    let label = window.label().to_string();
    let context = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .context_for_label(&app, &label)
        .map(render_window_context_response);
    Ok(context)
}

#[tauri::command]
fn list_render_surfaces(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Vec<RenderSurfaceDescriptorResponse>, String> {
    let session_id = SessionId(session_id);
    let lifecycle = state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle");
    let current_surface_id = lifecycle.current_surface_id(&session_id);
    let surfaces = lifecycle
        .list_surfaces(&session_id)
        .into_iter()
        .map(|surface| render_surface_descriptor_response(surface, current_surface_id.as_deref()))
        .collect();
    Ok(surfaces)
}

#[tauri::command]
fn create_render_surface(
    state: tauri::State<'_, AppState>,
    session_id: String,
    name: Option<String>,
) -> Result<RenderSurfaceDescriptorResponse, String> {
    let surface = state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .create_surface(SessionId(session_id), name);
    let current_surface_id = surface.surface_id.clone();
    Ok(render_surface_descriptor_response(
        surface,
        Some(current_surface_id.as_str()),
    ))
}

#[tauri::command]
fn select_current_render_surface(
    state: tauri::State<'_, AppState>,
    session_id: String,
    surface_id: String,
) -> Result<(), String> {
    state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .select_current_surface(SessionId(session_id), surface_id)
}

#[tauri::command]
fn current_render_surface(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<String>, String> {
    Ok(state
        .session_lifecycle
        .lock()
        .expect("lock session lifecycle")
        .current_surface_id(&SessionId(session_id)))
}

/// Tauri 命令：设备注册
///
/// 调用后端 API 进行设备注册，后端根据主板序列号生成设备ID
#[tauri::command]
async fn register_device(
    motherboard_serial: String,
    hostname: String,
    os_version: String,
    device_name: Option<String>,
) -> Result<DeviceRegistrationResponse, String> {
    // 构建注册请求
    let client = reqwest::Client::new();
    let mut payload = HashMap::new();
    payload.insert("motherboard_serial", motherboard_serial);
    payload.insert("hostname", hostname);
    payload.insert("os_version", os_version);

    if let Some(name) = device_name {
        payload.insert("device_name", name);
    }

    // 调用后端 API
    let response = client
        .post("http://127.0.0.1:9530/api/v1/devices/register")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("连接服务器失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "未知错误".to_string());
        return Err(format!("注册失败 ({}): {}", status, error_text));
    }

    response
        .json::<DeviceRegistrationResponse>()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))
}

/// Tauri 命令：检查设备是否已注册
#[tauri::command]
async fn check_device_registration(motherboard_serial: String) -> Result<bool, String> {
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://127.0.0.1:9530/api/v1/devices/check/{}",
            motherboard_serial
        ))
        .send()
        .await
        .map_err(|e| format!("连接服务器失败: {}", e))?;

    Ok(response.status().is_success())
}

async fn realtime_register_with(
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

async fn realtime_request_session_with(
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

async fn realtime_accept_session_with(
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

async fn drain_realtime_events_with(
    runtime: &RealtimeRuntime,
    handle: u64,
) -> Result<Vec<SignalMessage>, String> {
    runtime.drain_events(handle).await
}

async fn prepare_quic_accept_with(
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

fn spawn_quic_accept_completion(quic_host: std::sync::Arc<Mutex<QuicHost>>, session_id: SessionId) {
    tokio::spawn(async move {
        let _ = quic_host.lock().await.accept_peer(session_id).await;
    });
}

fn parse_backend_role(role: &str) -> Result<BackendRole, String> {
    match role {
        "controller" => Ok(BackendRole::Controller),
        "agent" => Ok(BackendRole::Agent),
        other => Err(format!("不支持的 realtime role: {}", other)),
    }
}

fn realtime_registration_response(
    registration: RealtimeRegistration,
) -> RealtimeRegistrationResponse {
    RealtimeRegistrationResponse {
        handle: registration.handle,
        device_id: registration.device_id.0,
    }
}

fn nvdec_runtime_probe_response() -> NvdecRuntimeProbeResponse {
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
                chroma_format: capability.chroma_format,
                runtime_supported: capability.runtime_supported,
                runtime_reason: capability.runtime_reason,
                wired_supported: capability.wired_supported,
                wired_reason: capability.wired_reason,
            })
            .collect(),
    }
}

fn parse_decode_policy(value: &str) -> Result<DecodePolicy, String> {
    match value {
        "auto" => Ok(DecodePolicy::Auto),
        "software" => Ok(DecodePolicy::Software),
        "d3d11va" => Ok(DecodePolicy::D3d11va),
        "nvdec" => Ok(DecodePolicy::Nvdec),
        other => Err(format!("未知 decode policy: {other}")),
    }
}

async fn decode_policy_with(host: &Mutex<WebrtcHost>) -> DecodePolicyResponse {
    let host = host.lock().await;
    DecodePolicyResponse {
        decode_policy: host.decode_policy().as_str().to_string(),
    }
}

async fn set_decode_policy_with(
    settings_path: &std::path::Path,
    host: &Mutex<WebrtcHost>,
    decode_policy: DecodePolicy,
) -> Result<DecodePolicyResponse, String> {
    save_settings(settings_path, &AppSettings { decode_policy })?;
    let mut host = host.lock().await;
    host.set_decode_policy(decode_policy);
    Ok(DecodePolicyResponse {
        decode_policy: host.decode_policy().as_str().to_string(),
    })
}

fn webrtc_snapshot_response(snapshot: &WebrtcSessionSnapshot) -> WebrtcSessionSnapshotResponse {
    WebrtcSessionSnapshotResponse {
        local_offer: snapshot.local_offer.clone(),
        remote_offer: snapshot.remote_offer.clone(),
        remote_answer: snapshot.remote_answer.clone(),
        remote_ice_candidates: snapshot.remote_ice_candidates.clone(),
    }
}

fn quic_snapshot_response(snapshot: &QuicSessionSnapshot) -> QuicSessionSnapshotResponse {
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

fn webrtc_host_snapshot_response(snapshot: &WebrtcHostSnapshot) -> WebrtcHostSnapshotResponse {
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
        recent_remote_access_unit_keyframes: snapshot.recent_remote_access_unit_keyframes.clone(),
        decoded_frame_count: snapshot.decoded_frame_count,
        last_decoded_width: snapshot.last_decoded_width,
        last_decoded_height: snapshot.last_decoded_height,
        last_decoded_pixel_format: snapshot.last_decoded_pixel_format.clone(),
        decode_policy: snapshot.decode_policy.clone(),
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
        peer_connection_state: snapshot.peer_connection_state.clone(),
        ice_connection_state: snapshot.ice_connection_state.clone(),
    }
}

fn quic_host_snapshot_response(snapshot: &QuicHostSnapshot) -> QuicHostSnapshotResponse {
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

fn decoded_frame_snapshot_response(
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

fn render_host_snapshot_response(snapshot: RenderHostSnapshot) -> RenderHostSnapshotResponse {
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
        renderer_backend: snapshot.renderer_backend,
        renderer_snapshot: snapshot.renderer_snapshot,
        surface_source_bindings: snapshot
            .surface_source_bindings
            .into_iter()
            .map(|binding| SurfaceSourceBindingResponseResponse {
                surface_id: binding.surface_id,
                source_id: binding.source_id,
            })
            .collect(),
        available_source_ids: snapshot.available_source_ids,
    }
}

fn render_window_context_response(context: RenderWindowContext) -> RenderWindowContextResponse {
    RenderWindowContextResponse {
        label: context.label,
        session_id: context.session_id,
        surface_id: context.surface_id,
        role: context.role,
        renderer_attached: context.renderer_attached,
        session_window_count: context.session_window_count,
    }
}

fn render_surface_descriptor_response(
    surface: RenderSurfaceDescriptor,
    current_surface_id: Option<&str>,
) -> RenderSurfaceDescriptorResponse {
    RenderSurfaceDescriptorResponse {
        current: current_surface_id == Some(surface.surface_id.as_str()),
        surface_id: surface.surface_id,
        name: surface.name,
        role: surface.role,
    }
}

fn surface_source_binding_response(
    binding: SurfaceSourceBinding,
) -> SurfaceSourceBindingResponseResponse {
    SurfaceSourceBindingResponseResponse {
        surface_id: binding.surface_id,
        source_id: binding.source_id,
    }
}

fn session_lifecycle_snapshot_response(
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

fn current_window_handle(window: &tauri::Window) -> Result<isize, String> {
    #[cfg(target_os = "windows")]
    {
        return window
            .hwnd()
            .map(|hwnd| hwnd.0)
            .map_err(|error| format!("获取窗口句柄失败: {error}"));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = window;
        Err("当前平台不支持窗口句柄渲染目标".to_string())
    }
}

async fn webrtc_create_local_offer_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
    sdp: String,
) -> Result<SessionDescription, String> {
    coordinator
        .lock()
        .await
        .create_local_offer(SessionId(session_id), sdp)
}

async fn webrtc_apply_remote_answer_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    coordinator
        .lock()
        .await
        .apply_remote_answer(SessionId(session_id), sdp)
}

async fn webrtc_apply_remote_ice_candidate_with(
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

async fn webrtc_sync_realtime_events_with(
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

async fn apply_realtime_events_to_session_coordinators(
    runtime: &RealtimeRuntime,
    webrtc_sessions: &Mutex<WebrtcSessionCoordinator>,
    quic_sessions: &Mutex<QuicSessionCoordinator>,
    handle: u64,
) -> Result<Option<SessionId>, String> {
    let events = runtime.drain_events(handle).await?;
    let mut last_session_id: Option<SessionId> = None;

    {
        let mut webrtc = webrtc_sessions.lock().await;
        let mut quic = quic_sessions.lock().await;
        for event in events {
            match event {
                SignalMessage::SessionRequest(request) => {
                    last_session_id = Some(request.session_id.clone());
                    if request.transport == "quic_quinn" {
                        quic.request_session(
                            request.session_id,
                            request.source_device_id,
                            request.target_device_id,
                            request.transport,
                            request.quic_listen_addr,
                            request.quic_server_name,
                            request.quic_cert_der_b64,
                        )?;
                    }
                }
                SignalMessage::SessionAccept(accept) => {
                    last_session_id = Some(accept.session_id.clone());
                    if accept.transport == "quic_quinn" {
                        quic.accept_session(
                            accept.session_id,
                            accept.transport,
                            accept.quic_listen_addr,
                            accept.quic_server_name,
                            accept.quic_cert_der_b64,
                        )?;
                    }
                }
                SignalMessage::WebrtcOffer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    webrtc.apply_remote_offer(description.session_id, description.sdp)?;
                }
                SignalMessage::WebrtcAnswer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    webrtc.apply_remote_answer(description.session_id, description.sdp)?;
                }
                SignalMessage::IceCandidate(candidate) => {
                    last_session_id = Some(candidate.session_id.clone());
                    webrtc.apply_remote_ice_candidate(candidate.session_id.clone(), candidate)?;
                }
                _ => {}
            }
        }
    }

    Ok(last_session_id)
}

async fn sync_quic_host_from_session_snapshot_with(
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

async fn session_runtime_snapshot_with(
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

    let render_host_snapshot = render_host_snapshot_with(render_host, session_id.0.clone())?;
    let webrtc_host_snapshot = webrtc_host_snapshot_with(webrtc_host, session_id.0.clone()).await;
    let quic_host_snapshot = quic_host_snapshot_with(quic_host, session_id.0.clone()).await;
    let webrtc_signaling = webrtc_snapshot_with(webrtc_sessions, session_id.0.clone()).await;
    let quic_signaling = quic_snapshot_with(quic_sessions, session_id.0.clone()).await;

    Ok(SessionRuntimeSnapshotResponse {
        lifecycle: session_lifecycle_snapshot_response(lifecycle_snapshot),
        render_host: render_host_snapshot_response(render_host_snapshot),
        webrtc_host: webrtc_host_snapshot,
        quic_host: quic_host_snapshot,
        webrtc_signaling,
        quic_signaling,
    })
}

async fn webrtc_snapshot_with(
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    session_id: String,
) -> Option<WebrtcSessionSnapshotResponse> {
    let sessions = coordinator.lock().await;
    sessions
        .snapshot(&SessionId(session_id))
        .map(webrtc_snapshot_response)
}

async fn quic_snapshot_with(
    coordinator: &Mutex<QuicSessionCoordinator>,
    session_id: String,
) -> Option<QuicSessionSnapshotResponse> {
    let sessions = coordinator.lock().await;
    sessions
        .snapshot(&SessionId(session_id))
        .map(quic_snapshot_response)
}

async fn webrtc_host_create_offer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Result<SessionDescription, String> {
    host.lock().await.create_offer(SessionId(session_id)).await
}

async fn webrtc_host_apply_remote_offer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    host.lock()
        .await
        .apply_remote_offer(SessionId(session_id), sdp)
        .await
}

async fn webrtc_host_create_answer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Result<SessionDescription, String> {
    host.lock().await.create_answer(SessionId(session_id)).await
}

async fn webrtc_host_apply_remote_answer_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    host.lock()
        .await
        .apply_remote_answer(SessionId(session_id), sdp)
        .await
}

async fn webrtc_host_apply_remote_ice_candidate_with(
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

async fn webrtc_host_snapshot_with(
    host: &Mutex<WebrtcHost>,
    session_id: String,
) -> Option<WebrtcHostSnapshotResponse> {
    let host = host.lock().await;
    host.snapshot(&SessionId(session_id))
        .map(|snapshot| webrtc_host_snapshot_response(&snapshot))
}

async fn quic_host_snapshot_with(
    host: &Mutex<QuicHost>,
    session_id: String,
) -> Option<QuicHostSnapshotResponse> {
    let host = host.lock().await;
    host.snapshot(&SessionId(session_id))
        .map(|snapshot| quic_host_snapshot_response(&snapshot))
}

fn decoded_frame_snapshot_with(
    sink: &std::sync::Mutex<DecodedFrameSink>,
    session_id: String,
) -> Option<DecodedFrameSnapshotResponse> {
    sink.lock()
        .expect("lock decoded frame sink")
        .snapshot(&SessionId(session_id))
        .map(decoded_frame_snapshot_response)
}

fn decoded_frame_preview_with(
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

fn main() {
    let frame_sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
    let probe_registry = ProbeRegistry::default();
    let render_host = std::sync::Arc::new(std::sync::Mutex::new(
        RenderHost::with_frame_sink_and_probes(frame_sink.clone(), Some(probe_registry.clone())),
    ));
    let render_windows =
        std::sync::Arc::new(std::sync::Mutex::new(RenderWindowRegistry::default()));
    let session_lifecycle =
        std::sync::Arc::new(std::sync::Mutex::new(SessionLifecycleCoordinator::default()));
    let settings_path = default_settings_path();
    let settings = load_settings(&settings_path).unwrap_or_else(|error| {
        eprintln!("failed to load app settings: {error}");
        AppSettings::default()
    });
    let mut webrtc_host =
        WebrtcHost::with_frame_sink_and_probes(frame_sink.clone(), probe_registry);
    webrtc_host.set_decode_policy(settings.decode_policy);
    let quic_host = QuicHost::with_frame_sink(frame_sink.clone());

    // Create shared service manager
    let service_manager = std::sync::Arc::new(std::sync::Mutex::new(
        service_manager::ServiceManager::new()
            .expect("failed to create ServiceManager")
    ));

    tauri::Builder::default()
        .manage(AppState {
            frame_sink: frame_sink.clone(),
            render_host,
            render_windows,
            session_lifecycle,
            realtime_runtime: RealtimeRuntime::from_env(),
            settings_path,
            webrtc_host: std::sync::Arc::new(Mutex::new(webrtc_host)),
            webrtc_sessions: std::sync::Arc::new(Mutex::new(WebrtcSessionCoordinator::default())),
            quic_host: std::sync::Arc::new(Mutex::new(quic_host)),
            quic_sessions: std::sync::Arc::new(Mutex::new(QuicSessionCoordinator::default())),
            service_manager,
        })
        .invoke_handler(tauri::generate_handler![
            get_hardware_info,
            nvdec_runtime_probe,
            decode_policy,
            set_decode_policy,
            register_device,
            check_device_registration,
            realtime_status,
            realtime_start,
            realtime_stop,
            realtime_restart,
            realtime_register,
            realtime_request_session,
            realtime_accept_session,
            realtime_drain_events,
            realtime_send_offer,
            realtime_send_answer,
            realtime_send_ice_candidate,
            webrtc_create_local_offer,
            webrtc_apply_remote_answer,
            webrtc_apply_remote_ice_candidate,
            webrtc_sync_realtime_events,
            webrtc_snapshot,
            quic_session_snapshot,
            quic_host_snapshot,
            webrtc_host_create_offer,
            webrtc_host_apply_remote_offer,
            webrtc_host_create_answer,
            webrtc_host_apply_remote_answer,
            webrtc_host_apply_remote_ice_candidate,
            webrtc_host_snapshot,
            session_runtime_probe_snapshot,
            session_runtime_probe_recent_events,
            webrtc_host_start_embedded_desktop_sender,
            webrtc_host_stop_embedded_video_sender,
            quic_host_start_embedded_desktop_sender,
            quic_host_stop_embedded_video_sender,
            decoded_frame_snapshot,
            decoded_frame_preview,
            render_host_attach_session,
            bind_render_surface_source,
            session_lifecycle_snapshot,
            session_runtime_snapshot,
            session_runtime_sync_realtime,
            bind_current_render_window_surface,
            render_host_detach_session,
            render_host_snapshot,
            open_render_window,
            open_render_surface_window,
            list_render_windows,
            close_render_window,
            render_window_context,
            list_render_surfaces,
            create_render_surface,
            select_current_render_surface,
            current_render_surface,
            // Service lifecycle commands
            service_start,
            service_stop,
            service_status,
            service_health_check,
            service_wait_for_healthy,
            service_restart_with_backoff,
            service_pid,
            service_start_guard,
            // IPC-based commands (migrated to use mrd-service)
            ipc_register_device,
            ipc_list_devices,
            ipc_start_session,
            ipc_accept_session,
            ipc_stop_session,
            ipc_session_snapshot,
            ipc_start_sender,
            ipc_start_receiver
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{
        apply_realtime_events_to_session_coordinators,
        benchmark::{
            write_benchmark_artifacts, BenchmarkManifest, BenchmarkPaths, BenchmarkSummary,
        },
        decode_policy_with, decoded_frame_preview_with, decoded_frame_snapshot_with,
        drain_realtime_events_with, nvdec_runtime_probe_response, quic_host_snapshot_with,
        prepare_quic_accept_with, quic_snapshot_with, realtime_accept_session_with, realtime_register_with,
        realtime_request_session_with, render_host_snapshot_response,
        session_runtime_snapshot_with, set_decode_policy_with, spawn_quic_accept_completion,
        sync_quic_host_from_session_snapshot_with, webrtc_apply_remote_answer_with,
        webrtc_apply_remote_ice_candidate_with, webrtc_create_local_offer_with,
        webrtc_host_apply_remote_answer_with, webrtc_host_apply_remote_offer_with,
        webrtc_host_create_answer_with, webrtc_host_create_offer_with, webrtc_host_snapshot_with,
        webrtc_snapshot_with, webrtc_sync_realtime_events_with,
    };
    use crate::{
        app_settings::DecodePolicy, frame_sink::DecodedFrameSink, quic_host::QuicHost,
        quic_session::QuicSessionCoordinator, realtime_runtime::RealtimeRuntime,
        render_host::RenderHost, session_lifecycle::SessionLifecycleCoordinator,
        webrtc_host::WebrtcHost, webrtc_session::WebrtcSessionCoordinator,
    };
    use axum::{
        extract::ws::{Message, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::{SinkExt, StreamExt};
    use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
    use mrd_proto::{DeviceId, SessionId};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_signal_proto::SignalMessage;
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

    #[tokio::test]
    async fn decode_policy_helpers_roundtrip_persisted_policy() {
        let host = Mutex::new(WebrtcHost::default());
        let settings_path = std::env::temp_dir().join(format!(
            "decode-policy-test-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        ));

        let initial = decode_policy_with(&host).await;
        assert_eq!(initial.decode_policy, "auto");

        let updated = set_decode_policy_with(&settings_path, &host, DecodePolicy::Nvdec)
            .await
            .expect("set decode policy");
        assert_eq!(updated.decode_policy, "nvdec");

        let reread = decode_policy_with(&host).await;
        assert_eq!(reread.decode_policy, "nvdec");
        let _ = std::fs::remove_file(settings_path);
    }

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

    #[derive(Default)]
    struct RoutedSignalingState {
        peers: HashMap<DeviceId, mpsc::UnboundedSender<String>>,
        routes: HashMap<SessionId, (DeviceId, DeviceId)>,
    }

    async fn routed_ws_handler(
        ws: WebSocketUpgrade,
        axum::extract::State(state): axum::extract::State<Arc<Mutex<RoutedSignalingState>>>,
    ) -> impl IntoResponse {
        ws.on_upgrade(move |socket| handle_routed_socket(socket, state))
    }

    async fn handle_routed_socket(
        socket: WebSocket,
        state: Arc<Mutex<RoutedSignalingState>>,
    ) {
        let (mut sender, mut receiver) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let mut current_device: Option<DeviceId> = None;

        let send_task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if sender.send(Message::Text(message.into())).await.is_err() {
                    break;
                }
            }
        });

        while let Some(Ok(Message::Text(raw))) = receiver.next().await {
            let signal = decode_message(&raw).expect("decode routed signal");
            match signal {
                SignalMessage::Register(register) => {
                    let device_id = register.device_id.expect("registered device id");
                    state.lock().await.peers.insert(device_id.clone(), tx.clone());
                    current_device = Some(device_id.clone());
                    let ack = encode_message(&SignalMessage::Registered(
                        mrd_signal_proto::RegisteredResponse { device_id },
                    ))
                    .expect("encode routed register ack");
                    let _ = tx.send(ack);
                }
                SignalMessage::SessionRequest(request) => {
                    state.lock().await.routes.insert(
                        request.session_id.clone(),
                        (
                            request.source_device_id.clone(),
                            request.target_device_id.clone(),
                        ),
                    );
                    if let Some(peer) = state.lock().await.peers.get(&request.target_device_id) {
                        let payload = encode_message(&SignalMessage::SessionRequest(request))
                            .expect("encode routed request");
                        let _ = peer.send(payload);
                    }
                }
                SignalMessage::SessionAccept(accept) => {
                    let Some(current_device) = current_device.clone() else {
                        continue;
                    };
                    let peer = {
                        let guard = state.lock().await;
                        guard
                            .routes
                            .get(&accept.session_id)
                            .and_then(|(controller, agent)| {
                                if *agent == current_device {
                                    Some(controller.clone())
                                } else if *controller == current_device {
                                    Some(agent.clone())
                                } else {
                                    None
                                }
                            })
                    };
                    if let Some(peer) = peer {
                        if let Some(target) = state.lock().await.peers.get(&peer) {
                            let payload = encode_message(&SignalMessage::SessionAccept(accept))
                                .expect("encode routed accept");
                            let _ = target.send(payload);
                        }
                    }
                }
                SignalMessage::WebrtcOffer(_)
                | SignalMessage::WebrtcAnswer(_)
                | SignalMessage::IceCandidate(_)
                | SignalMessage::Registered(_) => {}
            }
        }

        if let Some(device_id) = current_device {
            state.lock().await.peers.remove(&device_id);
        }
        send_task.abort();
    }

    async fn spawn_routed_server() -> String {
        let state = Arc::new(Mutex::new(RoutedSignalingState::default()));
        let app = Router::new()
            .route("/ws", get(routed_ws_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind realtime routed test server");
        let addr = listener.local_addr().expect("routed test server addr");

        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve routed helper test ws");
        });

        format!("ws://{}/ws", addr)
    }

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

    #[tokio::test]
    async fn syncing_realtime_events_records_quic_session_metadata() {
        let runtime = RealtimeRuntime::new(spawn_server().await);
        let webrtc_sessions = Mutex::new(WebrtcSessionCoordinator::default());
        let quic_sessions = Mutex::new(QuicSessionCoordinator::default());

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
            "session-quic-1".into(),
            "agent-1".into(),
            Some("quic_quinn".into()),
            Some("127.0.0.1:5000".into()),
            Some("localhost".into()),
            Some("AQID".into()),
        )
        .await
        .expect("request quic session through helper");
        realtime_accept_session_with(
            &runtime,
            registration.handle,
            "session-quic-1".into(),
            Some("quic_quinn".into()),
            Some("127.0.0.1:6000".into()),
            Some("localhost".into()),
            Some("BAUG".into()),
        )
        .await
        .expect("accept quic session through helper");

        let session_id = apply_realtime_events_to_session_coordinators(
            &runtime,
            &webrtc_sessions,
            &quic_sessions,
            registration.handle,
        )
        .await
        .expect("apply realtime events")
        .expect("quic session id");

        assert_eq!(session_id.0, "session-quic-1");
        let quic_snapshot = quic_snapshot_with(&quic_sessions, "session-quic-1".into())
            .await
            .expect("quic session snapshot");
        assert_eq!(quic_snapshot.transport, "quic_quinn");
        assert_eq!(
            quic_snapshot.local_listen_addr.as_deref(),
            Some("127.0.0.1:5000")
        );
        assert_eq!(
            quic_snapshot.remote_listen_addr.as_deref(),
            Some("127.0.0.1:6000")
        );
        assert!(
            webrtc_snapshot_with(&webrtc_sessions, "session-quic-1".into())
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn webrtc_host_helpers_complete_offer_answer_roundtrip() {
        let controller_host = Mutex::new(WebrtcHost::with_frame_sink(std::sync::Arc::new(
            std::sync::Mutex::new(DecodedFrameSink::default()),
        )));
        let agent_host = Mutex::new(WebrtcHost::with_frame_sink(std::sync::Arc::new(
            std::sync::Mutex::new(DecodedFrameSink::default()),
        )));

        let offer = webrtc_host_create_offer_with(&controller_host, "session-3".into())
            .await
            .expect("controller create offer");
        webrtc_host_apply_remote_offer_with(&agent_host, "session-3".into(), offer.sdp.clone())
            .await
            .expect("agent apply remote offer");

        let answer = webrtc_host_create_answer_with(&agent_host, "session-3".into())
            .await
            .expect("agent create answer");
        webrtc_host_apply_remote_answer_with(
            &controller_host,
            "session-3".into(),
            answer.sdp.clone(),
        )
        .await
        .expect("controller apply remote answer");

        let controller_snapshot = webrtc_host_snapshot_with(&controller_host, "session-3".into())
            .await
            .expect("controller host snapshot");
        let agent_snapshot = webrtc_host_snapshot_with(&agent_host, "session-3".into())
            .await
            .expect("agent host snapshot");

        assert!(controller_snapshot.local_offer.is_some());
        assert!(controller_snapshot.remote_answer.is_some());
        assert_eq!(controller_snapshot.remote_video_track_count, 0);
        assert_eq!(controller_snapshot.remote_h264_access_unit_count, 0);
        assert_eq!(controller_snapshot.last_remote_access_unit_bytes, 0);
        assert_eq!(controller_snapshot.decoded_frame_count, 0);
        assert_eq!(controller_snapshot.last_decoded_width, 0);
        assert_eq!(controller_snapshot.last_decoded_height, 0);
        assert_eq!(controller_snapshot.last_decoded_pixel_format, None);
        assert!(agent_snapshot.remote_offer.is_some());
        assert!(agent_snapshot.local_answer.is_some());
    }

    #[tokio::test]
    async fn session_runtime_quic_reports_transport_and_frame_delivery() {
        let session_id = SessionId("session-quic-runtime".into());
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let render_host = std::sync::Arc::new(std::sync::Mutex::new(RenderHost::with_frame_sink(
            sink.clone(),
        )));
        let lifecycle =
            std::sync::Arc::new(std::sync::Mutex::new(SessionLifecycleCoordinator::default()));
        let webrtc_host = Mutex::new(WebrtcHost::default());
        let mut agent_quic_host = QuicHost::default();
        let mut controller_quic_host = QuicHost::with_frame_sink(sink.clone());
        let webrtc_sessions = Mutex::new(WebrtcSessionCoordinator::default());
        let quic_sessions = Mutex::new(QuicSessionCoordinator::default());

        let bootstrap = agent_quic_host
            .prepare_listener(session_id.clone(), "127.0.0.1:0")
            .await
            .expect("prepare quic listener");
        controller_quic_host
            .connect_to_peer(
                session_id.clone(),
                "127.0.0.1:0",
                &bootstrap,
                "h264_software",
            )
            .await
            .expect("connect quic controller");
        agent_quic_host
            .accept_peer(session_id.clone())
            .await
            .expect("accept quic controller");
        agent_quic_host
            .start_test_video_sender_with_backend(session_id.clone(), 16, 16, 30, "openh264")
            .await
            .expect("start quic sender");
        controller_quic_host
            .wait_for_first_frame(&session_id, Duration::from_secs(5))
            .await
            .expect("wait for quic frame");

        quic_sessions
            .lock()
            .await
            .request_session(
                session_id.clone(),
                DeviceId("controller-1".into()),
                DeviceId("agent-1".into()),
                "quic_quinn".into(),
                Some(bootstrap.listen_addr.to_string()),
                Some(bootstrap.server_name.clone()),
                Some(base64::engine::general_purpose::STANDARD.encode(&bootstrap.cert_der)),
            )
            .expect("record quic request session");
        quic_sessions
            .lock()
            .await
            .accept_session(
                session_id.clone(),
                "quic_quinn".into(),
                Some(bootstrap.listen_addr.to_string()),
                Some(bootstrap.server_name.clone()),
                Some(base64::engine::general_purpose::STANDARD.encode(&bootstrap.cert_der)),
            )
            .expect("record quic accept session");

        let quic_host = Mutex::new(controller_quic_host);
        let snapshot = session_runtime_snapshot_with(
            lifecycle.as_ref(),
            render_host.as_ref(),
            &webrtc_host,
            &quic_host,
            &webrtc_sessions,
            &quic_sessions,
            session_id.clone(),
        )
        .await
        .expect("session runtime snapshot");

        assert!(snapshot.webrtc_host.is_none());
        assert!(snapshot.webrtc_signaling.is_none());
        assert_eq!(
            snapshot
                .quic_signaling
                .as_ref()
                .expect("quic signaling snapshot")
                .transport,
            "quic_quinn"
        );
        assert!(
            snapshot
                .quic_host
                .as_ref()
                .expect("quic host snapshot")
                .decoded_frame_count
                > 0
        );
        assert!(
            decoded_frame_snapshot_with(sink.as_ref(), session_id.0.clone())
                .expect("decoded frame snapshot")
                .frame_count
                > 0
        );

        let quic_snapshot = quic_snapshot_with(&quic_sessions, session_id.0.clone())
            .await
            .expect("quic session snapshot");
        let host_snapshot = quic_host_snapshot_with(&quic_host, session_id.0.clone())
            .await
            .expect("quic host runtime snapshot");
        assert_eq!(quic_snapshot.transport, "quic_quinn");
        assert!(host_snapshot.remote_datagram_count > 0);
    }

    #[tokio::test]
    async fn realtime_quic_flow_connects_hosts_and_delivers_frames() {
        let signaling_url = spawn_routed_server().await;
        let controller_runtime = RealtimeRuntime::new(signaling_url.clone());
        let agent_runtime = RealtimeRuntime::new(signaling_url);

        let controller_registration = realtime_register_with(
            &controller_runtime,
            "controller".into(),
            Some("controller-1".into()),
            "controller".into(),
        )
        .await
        .expect("register controller runtime");
        let agent_registration = realtime_register_with(
            &agent_runtime,
            "agent".into(),
            Some("agent-1".into()),
            "agent".into(),
        )
        .await
        .expect("register agent runtime");

        let session_id = SessionId("session-quic-live-runtime".into());
        let controller_sink =
            std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let controller_quic_host = std::sync::Arc::new(Mutex::new(QuicHost::with_frame_sink(
            controller_sink.clone(),
        )));
        let agent_quic_host = std::sync::Arc::new(Mutex::new(QuicHost::default()));
        let controller_webrtc = Mutex::new(WebrtcSessionCoordinator::default());
        let agent_webrtc = Mutex::new(WebrtcSessionCoordinator::default());
        let controller_quic = Mutex::new(QuicSessionCoordinator::default());
        let agent_quic = Mutex::new(QuicSessionCoordinator::default());

        controller_quic
            .lock()
            .await
            .request_session(
                session_id.clone(),
                DeviceId("controller-1".into()),
                DeviceId("agent-1".into()),
                "quic_quinn".into(),
                None,
                None,
                None,
            )
            .expect("record controller quic request");
        realtime_request_session_with(
            &controller_runtime,
            controller_registration.handle,
            session_id.0.clone(),
            "agent-1".into(),
            Some("quic_quinn".into()),
            None,
            None,
            None,
        )
        .await
        .expect("send quic session request");

        let agent_session = apply_realtime_events_to_session_coordinators(
            &agent_runtime,
            &agent_webrtc,
            &agent_quic,
            agent_registration.handle,
        )
        .await
        .expect("apply agent realtime events")
        .expect("agent session id");
        assert_eq!(agent_session, session_id);

        let (transport, quic_listen_addr, quic_server_name, quic_cert_der_b64) =
            prepare_quic_accept_with(agent_quic_host.as_ref(), &agent_quic, session_id.clone())
                .await
                .expect("prepare quic accept");
        realtime_accept_session_with(
            &agent_runtime,
            agent_registration.handle,
            session_id.0.clone(),
            Some(transport),
            quic_listen_addr,
            quic_server_name,
            quic_cert_der_b64,
        )
        .await
        .expect("send quic session accept");
        spawn_quic_accept_completion(agent_quic_host.clone(), session_id.clone());

        let controller_session = apply_realtime_events_to_session_coordinators(
            &controller_runtime,
            &controller_webrtc,
            &controller_quic,
            controller_registration.handle,
        )
        .await
        .expect("apply controller realtime events")
        .expect("controller session id");
        assert_eq!(controller_session, session_id);
        sync_quic_host_from_session_snapshot_with(
            controller_quic_host.as_ref(),
            &controller_quic,
            &DeviceId(controller_registration.device_id.clone()),
            &session_id,
        )
        .await
        .expect("connect controller quic host");

        agent_quic_host
            .lock()
            .await
            .start_test_video_sender_with_backend(session_id.clone(), 16, 16, 30, "openh264")
            .await
            .expect("start agent quic sender");
        controller_quic_host
            .lock()
            .await
            .wait_for_first_frame(&session_id, Duration::from_secs(5))
            .await
            .expect("wait for controller quic frame");

        let host_snapshot =
            quic_host_snapshot_with(controller_quic_host.as_ref(), session_id.0.clone())
                .await
                .expect("controller quic host snapshot");
        assert!(host_snapshot.remote_datagram_count > 0);
        assert!(host_snapshot.decoded_frame_count > 0);
        assert!(
            controller_sink
                .lock()
                .expect("lock controller sink")
                .snapshot(&session_id)
                .map(|snapshot| snapshot.frame_count > 0)
                .unwrap_or(false)
        );
    }

    struct BenchmarkCapture {
        tick: u8,
        width: usize,
        height: usize,
    }

    impl FrameCapture for BenchmarkCapture {
        fn capture_frame(&mut self) -> Result<CapturedFrame, mrd_pipeline_core::PipelineError> {
            self.tick = self.tick.wrapping_add(1);
            let mut data = vec![0_u8; self.width * self.height * 4];
            for chunk in data.chunks_exact_mut(4) {
                chunk[0] = self.tick;
                chunk[1] = 64;
                chunk[2] = 192;
                chunk[3] = 255;
            }
            Ok(CapturedFrame {
                width: self.width,
                height: self.height,
                pixel_format: FramePixelFormat::Bgra32,
                timestamp_us: self.tick as u64 * 33_000,
                data,
            })
        }
    }

    enum BenchmarkEncoder {
        OpenH264(mrd_encode_openh264::OpenH264Encoder),
        Nvenc(mrd_encode_nvenc::NvencH264Encoder),
    }

    impl VideoEncoder for BenchmarkEncoder {
        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<mrd_pipeline_core::EncodedAccessUnit>, mrd_pipeline_core::PipelineError>
        {
            match self {
                Self::OpenH264(encoder) => encoder.encode(frame),
                Self::Nvenc(encoder) => encoder.encode(frame),
            }
        }
    }

    fn create_benchmark_encoder(
        backend: &str,
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<BenchmarkEncoder, mrd_pipeline_core::PipelineError> {
        match backend {
            "nvenc" => Ok(BenchmarkEncoder::Nvenc(
                mrd_encode_nvenc::NvencH264Encoder::new(width, height, fps)?,
            )),
            "nvenc_ll_p1" => Ok(BenchmarkEncoder::Nvenc(
                mrd_encode_nvenc::NvencH264Encoder::new_low_latency_p1(width, height, fps)?,
            )),
            "nvenc_hq_p5" => Ok(BenchmarkEncoder::Nvenc(
                mrd_encode_nvenc::NvencH264Encoder::new_high_quality_p5(width, height, fps)?,
            )),
            "nvenc_baseline" => Ok(BenchmarkEncoder::Nvenc(
                mrd_encode_nvenc::NvencH264Encoder::new_baseline(width, height, fps)?,
            )),
            "openh264" => Ok(BenchmarkEncoder::OpenH264(
                mrd_encode_openh264::OpenH264Encoder::new(width, height, fps)?,
            )),
            "openh264_speed" => Ok(BenchmarkEncoder::OpenH264(
                mrd_encode_openh264::OpenH264Encoder::new_speed(width, height, fps)?,
            )),
            other => Err(mrd_pipeline_core::PipelineError::message(format!(
                "unsupported benchmark encoder backend: {other}"
            ))),
        }
    }

    #[test]
    fn benchmark_encoder_accepts_variant_backends() {
        for backend in ["openh264_speed", "nvenc_ll_p1", "nvenc_hq_p5"] {
            let result = create_benchmark_encoder(backend, 128, 128, 30);
            assert!(
                !matches!(
                    result,
                    Err(ref error)
                        if error
                            .to_string()
                            .contains("unsupported benchmark encoder backend")
                ),
                "expected benchmark backend {backend} to be recognized"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn benchmark_run_writes_requested_artifacts() {
        ensure_rustls_crypto_provider();
        let artifact_root = std::env::var("MRD_BENCH_ARTIFACT_ROOT").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("mrd-bench-default")
                .display()
                .to_string()
        });
        let scenario =
            std::env::var("MRD_BENCH_SCENARIO").unwrap_or_else(|_| "quick.transport".into());
        let profile = std::env::var("MRD_BENCH_PROFILE")
            .unwrap_or_else(|_| "transport-webrtc-baseline".into());
        let transport = std::env::var("MRD_BENCH_TRANSPORT").unwrap_or_else(|_| "webrtc".into());
        let run_id = std::env::var("MRD_BENCH_RUN_ID").unwrap_or_else(|_| {
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unix epoch")
                .as_secs();
            format!("quick-webrtc-{ts}")
        });
        let date = std::env::var("MRD_BENCH_DATE").unwrap_or_else(|_| "2026-03-08".into());
        let width = std::env::var("MRD_BENCH_WIDTH")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1280);
        let height = std::env::var("MRD_BENCH_HEIGHT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(720);
        let fps = std::env::var("MRD_BENCH_FPS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(30);
        let duration_secs = std::env::var("MRD_BENCH_DURATION_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(20);
        let encode_backend =
            std::env::var("MRD_BENCH_ENCODE_BACKEND").unwrap_or_else(|_| "openh264".into());
        let decode_backend =
            std::env::var("MRD_BENCH_DECODE_BACKEND").unwrap_or_else(|_| "h264_software".into());
        let effective_encode_backend = if transport == "webrtc" && encode_backend == "nvenc" {
            "nvenc_baseline".to_string()
        } else {
            encode_backend.clone()
        };
        let git_commit = std::env::var("MRD_BENCH_GIT_COMMIT").unwrap_or_else(|_| "unknown".into());
        let session_id = SessionId("session-benchmark".into());

        if transport == "quic_quinn" {
            let outcome = crate::quic_transport_harness::run_quic_benchmark_pipeline(
                session_id.clone(),
                width,
                height,
                fps,
                duration_secs,
                &encode_backend,
                &decode_backend,
            )
            .await
            .expect("run quic benchmark pipeline");
            let manifest = BenchmarkManifest {
                run_id: run_id.clone(),
                scenario,
                transport: outcome.transport_label.clone(),
                capture_backend: "synthetic".into(),
                encode_backend: encode_backend.clone(),
                decode_backend: decode_backend.clone(),
                renderer_backend: "d3d11".into(),
                width: width as u32,
                height: height as u32,
                fps,
                duration_secs,
                git_commit,
            };
            let summary = BenchmarkSummary::from_transport_probes(
                &manifest,
                &outcome.sender_probe,
                &outcome.receiver_probe,
                true,
                outcome.sink_snapshot.frame_count > 0,
                outcome.first_frame_time_ms,
                0,
            );
            let paths =
                BenchmarkPaths::new(std::path::Path::new(&artifact_root), date, profile, run_id);
            write_benchmark_artifacts(
                &paths,
                &manifest,
                &summary,
                &session_id.0,
                &outcome.receiver_probe,
            )
            .expect("write quic benchmark artifacts");

            assert!(paths.summary_json.exists());
            assert!(paths.summary_csv.exists());
            assert!(paths.report_md.exists());
            return;
        }

        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let mut controller = WebrtcHost::with_frame_sink(sink.clone());
        controller.set_decode_policy(match decode_backend.as_str() {
            "h264_software" => crate::app_settings::DecodePolicy::Software,
            "d3d11va" => crate::app_settings::DecodePolicy::D3d11va,
            other => panic!("unsupported benchmark decode backend: {other}"),
        });
        let mut agent = WebrtcHost::default();

        agent
            .prepare_test_video_sender_with_backend(session_id.clone(), &effective_encode_backend)
            .await
            .expect("prepare benchmark sender track");

        let offer = agent
            .create_offer(session_id.clone())
            .await
            .expect("agent offer");
        controller
            .apply_remote_offer(session_id.clone(), offer.sdp)
            .await
            .expect("controller apply offer");
        let answer = controller
            .create_answer(session_id.clone())
            .await
            .expect("controller answer");
        agent
            .apply_remote_answer(session_id.clone(), answer.sdp)
            .await
            .expect("agent apply answer");
        agent
            .start_test_video_sender(
                session_id.clone(),
                BenchmarkCapture {
                    tick: 0,
                    width,
                    height,
                },
                create_benchmark_encoder(&effective_encode_backend, width, height, fps)
                    .expect("benchmark encoder"),
                Duration::from_millis((1000 / fps.max(1)) as u64),
            )
            .await
            .expect("start benchmark sender");

        let started_at = Instant::now();
        let first_frame_timeout_secs = if effective_encode_backend.starts_with("nvenc") {
            20
        } else {
            8
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(first_frame_timeout_secs);
        let first_frame_wait = loop {
            let snapshot = controller
                .snapshot(&session_id)
                .expect("controller snapshot");
            if snapshot.decoded_frame_count > 0 {
                break Ok(started_at.elapsed().as_secs_f64() * 1000.0);
            }
            if std::time::Instant::now() >= deadline {
                break Err(());
            }
            tokio::task::yield_now().await;
        };
        let first_frame_seen = first_frame_wait.is_ok();
        let first_frame_time_ms =
            first_frame_wait.unwrap_or_else(|_| started_at.elapsed().as_secs_f64() * 1000.0);

        tokio::time::sleep(Duration::from_secs(duration_secs)).await;

        let controller_snapshot = controller
            .snapshot(&session_id)
            .expect("controller snapshot");
        let agent_snapshot = agent.snapshot(&session_id).expect("agent snapshot");
        let controller_probe = controller
            .probe_snapshot(&session_id)
            .expect("controller probe");
        let agent_probe = agent.probe_snapshot(&session_id).expect("agent probe");
        let manifest = BenchmarkManifest {
            run_id: run_id.clone(),
            scenario,
            transport,
            capture_backend: "dxgi".into(),
            encode_backend: effective_encode_backend,
            decode_backend,
            renderer_backend: "d3d11".into(),
            width: width as u32,
            height: height as u32,
            fps,
            duration_secs,
            git_commit,
        };
        let summary = BenchmarkSummary::from_transport_probes(
            &manifest,
            &agent_probe,
            &controller_probe,
            true,
            first_frame_seen && controller_snapshot.decoded_frame_count > 0,
            first_frame_time_ms,
            agent_snapshot.zero_write_access_unit_count,
        );
        let paths =
            BenchmarkPaths::new(std::path::Path::new(&artifact_root), date, profile, run_id);
        write_benchmark_artifacts(
            &paths,
            &manifest,
            &summary,
            &session_id.0,
            &controller_probe,
        )
        .expect("write benchmark artifacts");

        assert!(paths.summary_json.exists());
        assert!(paths.summary_csv.exists());
        assert!(paths.report_md.exists());

        controller
            .stop_embedded_video_sender(&session_id)
            .await
            .expect("stop benchmark controller sender");
        agent
            .stop_embedded_video_sender(&session_id)
            .await
            .expect("stop benchmark agent sender");
        controller
            .close_session(&session_id)
            .await
            .expect("close benchmark controller session");
        agent
            .close_session(&session_id)
            .await
            .expect("close benchmark agent session");
    }

    #[tokio::test]
    async fn webrtc_nvenc_720p_benchmark_capture_delivers_remote_frames() {
        ensure_rustls_crypto_provider();
        let session_id = SessionId("session-benchmark-nvenc-720p".into());
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let mut controller = WebrtcHost::with_frame_sink(sink);
        let mut agent = WebrtcHost::default();

        agent
            .prepare_test_video_sender_with_backend(session_id.clone(), "nvenc_baseline")
            .await
            .expect("prepare benchmark sender track");

        let offer = agent
            .create_offer(session_id.clone())
            .await
            .expect("agent offer");
        controller
            .apply_remote_offer(session_id.clone(), offer.sdp)
            .await
            .expect("controller apply offer");
        let answer = controller
            .create_answer(session_id.clone())
            .await
            .expect("controller answer");
        agent
            .apply_remote_answer(session_id.clone(), answer.sdp)
            .await
            .expect("agent apply answer");
        agent
            .start_test_video_sender(
                session_id.clone(),
                BenchmarkCapture {
                    tick: 0,
                    width: 1280,
                    height: 720,
                },
                create_benchmark_encoder("nvenc_baseline", 1280, 720, 30)
                    .expect("nvenc baseline encoder"),
                Duration::from_millis(33),
            )
            .await
            .expect("start benchmark sender");

        let wait_result = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let snapshot = controller
                    .snapshot(&session_id)
                    .expect("controller snapshot");
                if snapshot.decoded_frame_count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        let controller_snapshot = controller
            .snapshot(&session_id)
            .expect("controller snapshot");
        let controller_probe = controller
            .probe_snapshot(&session_id)
            .expect("controller probe");
        assert!(
            wait_result.is_ok(),
            "expected benchmark-style WebRTC+NVENC path to deliver frames: snapshot={controller_snapshot:?} probe={controller_probe:?}"
        );
        assert!(controller_snapshot.decoded_frame_count > 0);
    }

    #[test]
    fn decoded_frame_snapshot_reports_latest_ingested_frame() {
        let sink = std::sync::Mutex::new(DecodedFrameSink::default());
        sink.lock().expect("lock decoded frame sink").ingest_frame(
            SessionId("session-9".into()),
            mrd_decode::DecodedFrame::cpu_rgb24(640, 360, vec![0; 640 * 360 * 3]),
        );

        let snapshot = decoded_frame_snapshot_with(&sink, "session-9".into()).expect("snapshot");

        assert_eq!(snapshot.frame_count, 1);
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);
        assert_eq!(snapshot.pixel_format, "Rgb24");
        assert_eq!(snapshot.bytes, 640 * 360 * 3);
    }

    #[test]
    fn decoded_frame_preview_encodes_png_data_url() {
        let sink = std::sync::Mutex::new(DecodedFrameSink::default());
        sink.lock().expect("lock decoded frame sink").ingest_frame(
            SessionId("session-preview".into()),
            mrd_decode::DecodedFrame::cpu_rgb24(
                2,
                2,
                vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
            ),
        );

        let preview = decoded_frame_preview_with(&sink, "session-preview".into())
            .expect("encode preview")
            .expect("preview exists");

        assert!(preview.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn render_host_snapshot_reports_attachment_and_preview() {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        sink.lock().expect("lock decoded frame sink").ingest_frame(
            SessionId("session-render".into()),
            mrd_decode::DecodedFrame::cpu_rgb24(2, 2, vec![255; 12]),
        );
        let mut render_host = RenderHost::with_frame_sink(sink);
        let _ =
            render_host.attach_session(SessionId("session-render".into()), "surface-1".into(), 0);

        let response = render_host_snapshot_response(
            render_host
                .snapshot(&SessionId("session-render".into()))
                .expect("render host snapshot"),
        );

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
}
