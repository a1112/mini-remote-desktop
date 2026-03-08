// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod device_info;
mod benchmark;
mod frame_sink;
mod render_host;
mod realtime_client;
mod realtime_management;
mod realtime_runtime;
mod webrtc_host;
mod webrtc_media;
mod webrtc_session;
mod render_window_registry;
mod render_surface_catalog;
mod session_lifecycle;
mod session_runtime;
#[cfg(test)]
mod quic_transport_harness;

use device_info::HardwareInfo;
use frame_sink::{DecodedFrameSink, DecodedFrameSnapshot};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_observability::{MediaProbeEvent, PipelineProbeSnapshot, ProbeRegistry};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::encode_message;
use mrd_signal_proto::{IceCandidate, SessionDescription, SignalMessage};
use realtime_management::{RealtimeManagementClient, RealtimeStatus};
use realtime_runtime::{RealtimeRegistration, RealtimeRuntime};
use render_host::{
    render_host_snapshot_with, RenderHost, RenderHostSnapshot, RendererSnapshotResponse,
};
use render_surface_catalog::RenderSurfaceDescriptor;
use render_window_registry::{RenderWindowContext, RenderWindowRegistry};
use session_lifecycle::{SessionLifecycleCoordinator, SessionLifecycleSnapshot, SurfaceSourceBinding};
use session_runtime::sync_session_runtime;
use serde::{Deserialize, Serialize};
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
    webrtc_host: std::sync::Arc<Mutex<WebrtcHost>>,
    webrtc_sessions: std::sync::Arc<Mutex<WebrtcSessionCoordinator>>,
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
struct WebrtcHostSnapshotResponse {
    local_offer: Option<String>,
    remote_offer: Option<String>,
    local_answer: Option<String>,
    remote_answer: Option<String>,
    remote_ice_count: usize,
    remote_video_track_count: usize,
    remote_rtp_packet_count: u64,
    last_remote_codec: Option<String>,
    remote_h264_access_unit_count: u64,
    last_remote_access_unit_bytes: usize,
    decoded_frame_count: u64,
    last_decoded_width: usize,
    last_decoded_height: usize,
    last_decoded_pixel_format: Option<String>,
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
    webrtc_host: WebrtcHostSnapshotResponse,
    webrtc_signaling: Option<WebrtcSessionSnapshotResponse>,
}

/// Tauri 命令：获取硬件信息
#[tauri::command]
fn get_hardware_info() -> Result<HardwareInfo, String> {
    Ok(device_info::get_hardware_info())
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

#[tauri::command]
async fn realtime_register(
    state: tauri::State<'_, AppState>,
    role: String,
    device_id: Option<String>,
    name: String,
) -> Result<RealtimeRegistrationResponse, String> {
    realtime_register_with(&state.realtime_runtime, role, device_id, name).await
}

#[tauri::command]
async fn realtime_request_session(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
    target_device_id: String,
) -> Result<(), String> {
    realtime_request_session_with(
        &state.realtime_runtime,
        handle,
        session_id,
        target_device_id,
    )
    .await
}

#[tauri::command]
async fn realtime_accept_session(
    state: tauri::State<'_, AppState>,
    handle: u64,
    session_id: String,
) -> Result<(), String> {
    realtime_accept_session_with(&state.realtime_runtime, handle, session_id).await
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
async fn webrtc_host_create_offer(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let description =
        webrtc_host_create_offer_with(state.webrtc_host.as_ref(), session_id.clone()).await?;
    webrtc_create_local_offer_with(state.webrtc_sessions.as_ref(), session_id, description.sdp.clone())
        .await?;
    Ok(description.sdp)
}

#[tauri::command]
async fn webrtc_host_apply_remote_offer(
    state: tauri::State<'_, AppState>,
    session_id: String,
    sdp: String,
) -> Result<(), String> {
    webrtc_host_apply_remote_offer_with(state.webrtc_host.as_ref(), session_id.clone(), sdp.clone())
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
    webrtc_host_apply_remote_answer_with(state.webrtc_host.as_ref(), session_id.clone(), sdp.clone())
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

#[tauri::command]
async fn session_runtime_probe_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<PipelineProbeSnapshot>, String> {
    let host = state.webrtc_host.lock().await;
    Ok(host.probe_snapshot(&SessionId(session_id)))
}

#[tauri::command]
async fn session_runtime_probe_recent_events(
    state: tauri::State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<MediaProbeEvent>, String> {
    let host = state.webrtc_host.lock().await;
    Ok(host.probe_recent_events(&SessionId(session_id), limit.unwrap_or(64)))
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
async fn decoded_frame_snapshot(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Option<DecodedFrameSnapshotResponse>, String> {
    Ok(decoded_frame_snapshot_with(state.frame_sink.as_ref(), session_id))
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
        let mut render_host = state
            .render_host
            .lock()
            .expect("lock render host");
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
        let mut render_host = state
            .render_host
            .lock()
            .expect("lock render host");
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
        let mut render_host = state
            .render_host
            .lock()
            .expect("lock render host");
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
        let mut render_host = state
            .render_host
            .lock()
            .expect("lock render host");
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
        let mut render_host = state
            .render_host
            .lock()
            .expect("lock render host");
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
        state.webrtc_sessions.as_ref(),
        SessionId(session_id),
    )
    .await
}

#[tauri::command]
async fn session_runtime_sync_realtime(
    state: tauri::State<'_, AppState>,
    handle: u64,
) -> Result<Option<SessionRuntimeSnapshotResponse>, String> {
    let Some(session_id) = apply_realtime_events_to_webrtc_sessions(
        &state.realtime_runtime,
        state.webrtc_sessions.as_ref(),
        handle,
    )
    .await? else {
        return Ok(None);
    };

    session_runtime_snapshot_with(
        state.session_lifecycle.as_ref(),
        state.render_host.as_ref(),
        state.webrtc_host.as_ref(),
        state.webrtc_sessions.as_ref(),
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
        .register(
            parse_backend_role(&role)?,
            device_id.map(DeviceId),
            name,
        )
        .await?;

    Ok(realtime_registration_response(registration))
}

async fn realtime_request_session_with(
    runtime: &RealtimeRuntime,
    handle: u64,
    session_id: String,
    target_device_id: String,
) -> Result<(), String> {
    runtime
        .request_session(handle, SessionId(session_id), DeviceId(target_device_id))
        .await
}

async fn realtime_accept_session_with(
    runtime: &RealtimeRuntime,
    handle: u64,
    session_id: String,
) -> Result<(), String> {
    runtime.accept_session(handle, SessionId(session_id)).await
}

async fn drain_realtime_events_with(
    runtime: &RealtimeRuntime,
    handle: u64,
) -> Result<Vec<SignalMessage>, String> {
    runtime.drain_events(handle).await
}

fn parse_backend_role(role: &str) -> Result<BackendRole, String> {
    match role {
        "controller" => Ok(BackendRole::Controller),
        "agent" => Ok(BackendRole::Agent),
        other => Err(format!("不支持的 realtime role: {}", other)),
    }
}

fn realtime_registration_response(registration: RealtimeRegistration) -> RealtimeRegistrationResponse {
    RealtimeRegistrationResponse {
        handle: registration.handle,
        device_id: registration.device_id.0,
    }
}

fn webrtc_snapshot_response(snapshot: &WebrtcSessionSnapshot) -> WebrtcSessionSnapshotResponse {
    WebrtcSessionSnapshotResponse {
        local_offer: snapshot.local_offer.clone(),
        remote_offer: snapshot.remote_offer.clone(),
        remote_answer: snapshot.remote_answer.clone(),
        remote_ice_candidates: snapshot.remote_ice_candidates.clone(),
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
        remote_h264_access_unit_count: snapshot.remote_h264_access_unit_count,
        last_remote_access_unit_bytes: snapshot.last_remote_access_unit_bytes,
        decoded_frame_count: snapshot.decoded_frame_count,
        last_decoded_width: snapshot.last_decoded_width,
        last_decoded_height: snapshot.last_decoded_height,
        last_decoded_pixel_format: snapshot.last_decoded_pixel_format.clone(),
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

fn decoded_frame_snapshot_response(snapshot: &DecodedFrameSnapshot) -> DecodedFrameSnapshotResponse {
    DecodedFrameSnapshotResponse {
        frame_count: snapshot.frame_count,
        width: snapshot.width,
        height: snapshot.height,
        pixel_format: match snapshot.pixel_format {
            mrd_decode::PixelFormat::Rgb24 => "Rgb24".to_string(),
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
    let session_id = apply_realtime_events_to_webrtc_sessions(runtime, coordinator, handle)
        .await?
        .ok_or_else(|| "未收到可应用的 webrtc 事件".to_string())?;
    let sessions = coordinator.lock().await;
    let snapshot = sessions
        .snapshot(&session_id)
        .ok_or_else(|| format!("未找到会话协商快照: {}", session_id.0))?;
    Ok(webrtc_snapshot_response(snapshot))
}

async fn apply_realtime_events_to_webrtc_sessions(
    runtime: &RealtimeRuntime,
    coordinator: &Mutex<WebrtcSessionCoordinator>,
    handle: u64,
) -> Result<Option<SessionId>, String> {
    let events = runtime.drain_events(handle).await?;
    let mut last_session_id: Option<SessionId> = None;

    {
        let mut sessions = coordinator.lock().await;
        for event in events {
            match event {
                SignalMessage::WebrtcOffer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    sessions.apply_remote_offer(description.session_id, description.sdp)?;
                }
                SignalMessage::WebrtcAnswer(description) => {
                    last_session_id = Some(description.session_id.clone());
                    sessions.apply_remote_answer(description.session_id, description.sdp)?;
                }
                SignalMessage::IceCandidate(candidate) => {
                    last_session_id = Some(candidate.session_id.clone());
                    sessions.apply_remote_ice_candidate(candidate.session_id.clone(), candidate)?;
                }
                _ => {}
            }
        }
    }

    Ok(last_session_id)
}

async fn session_runtime_snapshot_with(
    lifecycle: &std::sync::Mutex<SessionLifecycleCoordinator>,
    render_host: &std::sync::Mutex<RenderHost>,
    webrtc_host: &Mutex<WebrtcHost>,
    webrtc_sessions: &Mutex<WebrtcSessionCoordinator>,
    session_id: SessionId,
) -> Result<SessionRuntimeSnapshotResponse, String> {
    let lifecycle_snapshot = {
        let mut lifecycle = lifecycle.lock().expect("lock session lifecycle");
        let mut render_host = render_host.lock().expect("lock render host");
        sync_session_runtime(&mut lifecycle, &mut render_host, &session_id)?;
        lifecycle.snapshot(&session_id)
    };

    let render_host_snapshot = render_host_snapshot_with(render_host, session_id.0.clone())?;
    let webrtc_host_snapshot = webrtc_host_snapshot_with(webrtc_host, session_id.0.clone())
        .await
        .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
    let webrtc_signaling = webrtc_snapshot_with(webrtc_sessions, session_id.0.clone()).await;

    Ok(SessionRuntimeSnapshotResponse {
        lifecycle: session_lifecycle_snapshot_response(lifecycle_snapshot),
        render_host: render_host_snapshot_response(render_host_snapshot),
        webrtc_host: webrtc_host_snapshot,
        webrtc_signaling,
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

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &frame.data,
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
    let render_windows = std::sync::Arc::new(std::sync::Mutex::new(RenderWindowRegistry::default()));
    let session_lifecycle =
        std::sync::Arc::new(std::sync::Mutex::new(SessionLifecycleCoordinator::default()));
    tauri::Builder::default()
        .manage(AppState {
            frame_sink: frame_sink.clone(),
            render_host,
            render_windows,
            session_lifecycle,
            realtime_runtime: RealtimeRuntime::from_env(),
            webrtc_host: std::sync::Arc::new(Mutex::new(WebrtcHost::with_frame_sink_and_probes(
                frame_sink,
                probe_registry,
            ))),
            webrtc_sessions: std::sync::Arc::new(Mutex::new(WebrtcSessionCoordinator::default())),
        })
        .invoke_handler(tauri::generate_handler![
            get_hardware_info,
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
            current_render_surface
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        benchmark::{write_benchmark_artifacts, BenchmarkManifest, BenchmarkPaths, BenchmarkSummary},
        decoded_frame_preview_with, decoded_frame_snapshot_with, drain_realtime_events_with, realtime_accept_session_with,
        realtime_register_with, render_host_snapshot_response,
        realtime_request_session_with, webrtc_apply_remote_answer_with,
        webrtc_apply_remote_ice_candidate_with, webrtc_create_local_offer_with,
        webrtc_host_apply_remote_answer_with, webrtc_host_apply_remote_offer_with,
        webrtc_host_create_answer_with, webrtc_host_create_offer_with,
        webrtc_host_snapshot_with, webrtc_snapshot_with, webrtc_sync_realtime_events_with,
    };
    use crate::{
        frame_sink::DecodedFrameSink, realtime_runtime::RealtimeRuntime, render_host::RenderHost, webrtc_host::WebrtcHost,
        webrtc_session::WebrtcSessionCoordinator,
    };
    use axum::{
        extract::ws::{Message, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::StreamExt;
    use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_proto::{DeviceId, SessionId};
    use mrd_signal_proto::SignalMessage;
    use std::sync::Once;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    fn ensure_rustls_crypto_provider() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
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

        let ack = encode_message(&SignalMessage::Registered(mrd_signal_proto::RegisteredResponse {
            device_id: DeviceId("controller-1".into()),
        }))
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
        )
        .await
        .expect("request session through helper");

        realtime_accept_session_with(&runtime, registration.handle, "session-1".into())
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

        let offer = webrtc_create_local_offer_with(
            &coordinator,
            "session-1".into(),
            "offer-sdp".into(),
        )
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

        let snapshot = webrtc_sync_realtime_events_with(
            &runtime,
            &coordinator,
            registration.handle,
        )
        .await
        .expect("sync realtime events");

        assert_eq!(snapshot.remote_offer.as_deref(), Some("offer-sdp"));
        assert_eq!(snapshot.local_offer, None);
        assert_eq!(snapshot.remote_answer.as_deref(), Some("answer-sdp"));
        assert_eq!(snapshot.remote_ice_candidates.len(), 1);
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
        webrtc_host_apply_remote_offer_with(
            &agent_host,
            "session-3".into(),
            offer.sdp.clone(),
        )
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
        ) -> Result<Vec<mrd_pipeline_core::EncodedAccessUnit>, mrd_pipeline_core::PipelineError> {
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
            "openh264" => Ok(BenchmarkEncoder::OpenH264(
                mrd_encode_openh264::OpenH264Encoder::new(width, height, fps)?,
            )),
            other => Err(mrd_pipeline_core::PipelineError::message(format!(
                "unsupported benchmark encoder backend: {other}"
            ))),
        }
    }

    #[tokio::test]
    async fn benchmark_run_writes_requested_artifacts() {
        ensure_rustls_crypto_provider();
        let artifact_root = std::env::var("MRD_BENCH_ARTIFACT_ROOT")
            .unwrap_or_else(|_| std::env::temp_dir().join("mrd-bench-default").display().to_string());
        let scenario = std::env::var("MRD_BENCH_SCENARIO").unwrap_or_else(|_| "quick.transport".into());
        let profile =
            std::env::var("MRD_BENCH_PROFILE").unwrap_or_else(|_| "transport-webrtc-baseline".into());
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
            )
            .await
            .expect("run quic benchmark pipeline");
            let manifest = BenchmarkManifest {
                run_id: run_id.clone(),
                scenario,
                transport,
                capture_backend: "synthetic".into(),
                encode_backend: encode_backend.clone(),
                decode_backend: "h264_software".into(),
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
            let paths = BenchmarkPaths::new(
                std::path::Path::new(&artifact_root),
                date,
                profile,
                run_id,
            );
            write_benchmark_artifacts(&paths, &manifest, &summary, &session_id.0, &outcome.receiver_probe)
                .expect("write quic benchmark artifacts");

            assert!(paths.summary_json.exists());
            assert!(paths.summary_csv.exists());
            assert!(paths.report_md.exists());
            return;
        }

        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let mut controller = WebrtcHost::with_frame_sink(sink.clone());
        let mut agent = WebrtcHost::default();

        agent
            .prepare_test_video_sender_with_backend(session_id.clone(), &encode_backend)
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
                create_benchmark_encoder(&encode_backend, width, height, fps)
                    .expect("benchmark encoder"),
                Duration::from_millis((1000 / fps.max(1)) as u64),
            )
            .await
            .expect("start benchmark sender");

        let started_at = Instant::now();
        let first_frame_timeout_secs = if encode_backend == "nvenc" { 20 } else { 8 };
        let first_frame_wait = tokio::time::timeout(Duration::from_secs(first_frame_timeout_secs), async {
            loop {
                let snapshot = controller.snapshot(&session_id).expect("controller snapshot");
                if snapshot.decoded_frame_count > 0 {
                    break started_at.elapsed().as_secs_f64() * 1000.0;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        let first_frame_seen = first_frame_wait.is_ok();
        let first_frame_time_ms = first_frame_wait
            .unwrap_or_else(|_| started_at.elapsed().as_secs_f64() * 1000.0);

        tokio::time::sleep(Duration::from_secs(duration_secs)).await;

        let controller_snapshot = controller.snapshot(&session_id).expect("controller snapshot");
        let agent_snapshot = agent.snapshot(&session_id).expect("agent snapshot");
        let controller_probe = controller.probe_snapshot(&session_id).expect("controller probe");
        let agent_probe = agent.probe_snapshot(&session_id).expect("agent probe");
        let manifest = BenchmarkManifest {
            run_id: run_id.clone(),
            scenario,
            transport,
            capture_backend: "dxgi".into(),
            encode_backend: encode_backend,
            decode_backend: "h264_software".into(),
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
        let paths = BenchmarkPaths::new(
            std::path::Path::new(&artifact_root),
            date,
            profile,
            run_id,
        );
        write_benchmark_artifacts(&paths, &manifest, &summary, &session_id.0, &controller_probe)
            .expect("write benchmark artifacts");

        assert!(paths.summary_json.exists());
        assert!(paths.summary_csv.exists());
        assert!(paths.report_md.exists());
    }

    #[tokio::test]
    #[ignore = "known reproduction: webrtc + nvenc 720p receives no keyframes yet"]
    async fn webrtc_nvenc_720p_benchmark_capture_delivers_remote_frames() {
        ensure_rustls_crypto_provider();
        let session_id = SessionId("session-benchmark-nvenc-720p".into());
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        let mut controller = WebrtcHost::with_frame_sink(sink);
        let mut agent = WebrtcHost::default();

        agent
            .prepare_test_video_sender_with_backend(session_id.clone(), "nvenc")
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
                create_benchmark_encoder("nvenc", 1280, 720, 30).expect("nvenc encoder"),
                Duration::from_millis(33),
            )
            .await
            .expect("start benchmark sender");

        let wait_result = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let snapshot = controller.snapshot(&session_id).expect("controller snapshot");
                if snapshot.decoded_frame_count > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;

        let controller_snapshot = controller.snapshot(&session_id).expect("controller snapshot");
        let controller_probe = controller.probe_snapshot(&session_id).expect("controller probe");
        assert!(
            wait_result.is_ok(),
            "expected benchmark-style WebRTC+NVENC path to deliver frames: snapshot={controller_snapshot:?} probe={controller_probe:?}"
        );
        assert!(controller_snapshot.decoded_frame_count > 0);
    }

    #[test]
    fn decoded_frame_snapshot_reports_latest_ingested_frame() {
        let sink = std::sync::Mutex::new(DecodedFrameSink::default());
        sink.lock()
            .expect("lock decoded frame sink")
            .ingest_frame(
                SessionId("session-9".into()),
                mrd_decode::DecodedFrame {
                    width: 640,
                    height: 360,
                    pixel_format: mrd_decode::PixelFormat::Rgb24,
                    data: vec![0; 640 * 360 * 3],
                },
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
        sink.lock()
            .expect("lock decoded frame sink")
            .ingest_frame(
                SessionId("session-preview".into()),
                mrd_decode::DecodedFrame {
                    width: 2,
                    height: 2,
                    pixel_format: mrd_decode::PixelFormat::Rgb24,
                    data: vec![
                        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255,
                    ],
                },
            );

        let preview = decoded_frame_preview_with(&sink, "session-preview".into())
            .expect("encode preview")
            .expect("preview exists");

        assert!(preview.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn render_host_snapshot_reports_attachment_and_preview() {
        let sink = std::sync::Arc::new(std::sync::Mutex::new(DecodedFrameSink::default()));
        sink.lock()
            .expect("lock decoded frame sink")
            .ingest_frame(
                SessionId("session-render".into()),
                mrd_decode::DecodedFrame {
                    width: 2,
                    height: 2,
                    pixel_format: mrd_decode::PixelFormat::Rgb24,
                    data: vec![255; 12],
                },
            );
        let mut render_host = RenderHost::with_frame_sink(sink);
        let _ = render_host.attach_session(SessionId("session-render".into()), "surface-1".into(), 0);

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
            .map_or(false, |value: &str| value.starts_with("data:image/png;base64,")));
    }
}
