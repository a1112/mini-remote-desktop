// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod device_info;
mod frame_sink;
mod render_host;
mod realtime_client;
mod realtime_management;
mod realtime_runtime;
mod webrtc_host;
mod webrtc_media;
mod webrtc_session;
mod render_window_registry;

use device_info::HardwareInfo;
use frame_sink::{DecodedFrameSink, DecodedFrameSnapshot};
use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::encode_message;
use mrd_signal_proto::{IceCandidate, SessionDescription, SignalMessage};
use realtime_management::{RealtimeManagementClient, RealtimeStatus};
use realtime_runtime::{RealtimeRegistration, RealtimeRuntime};
use render_host::{
    render_host_snapshot_with, RenderHost, RenderHostSnapshot, RendererSnapshotResponse,
};
use render_window_registry::{RenderWindowContext, RenderWindowRegistry};
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
    state
        .render_host
        .lock()
        .expect("lock render host")
        .attach_session(SessionId(session_id), surface_id, window_handle)?;
    state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .set_renderer_attached(&window.app_handle(), window.label(), true);
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
    let snapshot = render_host_snapshot_with(state.render_host.as_ref(), session_id)?;
    Ok(render_host_snapshot_response(snapshot))
}

#[tauri::command]
fn open_render_window(app: tauri::AppHandle, session_id: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    let result = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .open_window(&app, SessionId(session_id));
    result
}

#[tauri::command]
fn list_render_windows(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Vec<String>, String> {
    let state = app.state::<AppState>();
    let labels = state
        .render_windows
        .lock()
        .expect("lock render window registry")
        .list_windows(&app, &SessionId(session_id));
    Ok(labels)
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

        let session_id = last_session_id.ok_or_else(|| "未收到可应用的 webrtc 事件".to_string())?;
        let snapshot = sessions
            .snapshot(&session_id)
            .ok_or_else(|| format!("未找到会话协商快照: {}", session_id.0))?;
        Ok(webrtc_snapshot_response(snapshot))
    }
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
    let render_host = std::sync::Arc::new(std::sync::Mutex::new(RenderHost::with_frame_sink(frame_sink.clone())));
    let render_windows = std::sync::Arc::new(std::sync::Mutex::new(RenderWindowRegistry::default()));
    tauri::Builder::default()
        .manage(AppState {
            frame_sink: frame_sink.clone(),
            render_host,
            render_windows,
            realtime_runtime: RealtimeRuntime::from_env(),
            webrtc_host: std::sync::Arc::new(Mutex::new(WebrtcHost::with_frame_sink(frame_sink))),
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
            decoded_frame_snapshot,
            decoded_frame_preview,
            render_host_attach_session,
            render_host_detach_session,
            render_host_snapshot,
            open_render_window,
            list_render_windows,
            close_render_window,
            render_window_context
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
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
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_proto::{DeviceId, SessionId};
    use mrd_signal_proto::SignalMessage;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

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
