// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(unexpected_cfgs)]

mod app_settings;
mod device_info;
mod frame_sink;
mod ipc_client;
mod platform;
mod remote_display_surface;
mod render_probe;
mod render_window_registry;
mod resource_monitor;
mod service_manager;
mod test_harness;
mod test_orchestrator;
mod webrtc_host;
mod webrtc_media;

use app_settings::{
    default_settings_path, load_settings, save_settings, AppSettings, DecodePolicy,
};
use device_info::HardwareInfo;
use mrd_ipc;
use mrd_proto::SessionId;
use remote_display_surface::{
    NativeRenderSurfaceSnapshot, NativeSurfaceRect, RemoteDisplaySurfaceManager,
};
use render_window_registry::{PendingRenderWindow, RenderWindowContext, RenderWindowRegistry};
use resource_monitor::{ResourceMonitor, SystemResourceSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WebviewWindow, WebviewWindowBuilder,
};

const TRAY_ICON_ID: &str = "rdesk-tray";
const TRAY_MENU_SHOW_ID: &str = "rdesk-tray-show";
const TRAY_MENU_HIDE_ID: &str = "rdesk-tray-hide";
const TRAY_MENU_CENTER_ID: &str = "rdesk-tray-center";
const TRAY_MENU_QUIT_ID: &str = "rdesk-tray-quit";
const SINGLE_INSTANCE_ADDR: &str = "127.0.0.1:47631";
const LAN_E2E_AUTORUN_ENV: &str = "MRD_LAN_E2E_AUTORUN";
const LAN_E2E_TARGET_DEVICE_ID_ENV: &str = "MRD_LAN_E2E_TARGET_DEVICE_ID";
const LAN_E2E_TRANSPORT_ENV: &str = "MRD_LAN_E2E_TRANSPORT";
const LAN_E2E_TIMEOUT_MS_ENV: &str = "MRD_LAN_E2E_TIMEOUT_MS";
const LAN_E2E_MIN_DECODED_FRAMES_ENV: &str = "MRD_LAN_E2E_MIN_DECODED_FRAMES";
const LAN_E2E_MIN_FPS_ENV: &str = "MRD_LAN_E2E_MIN_FPS";
const LAN_E2E_STOP_ON_COMPLETE_ENV: &str = "MRD_LAN_E2E_STOP_ON_COMPLETE";
const LAN_E2E_REPORT_PATH_ENV: &str = "MRD_LAN_E2E_REPORT_PATH";

static APP_IS_QUITTING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    ShowWindow,
    HideWindow,
    CenterWindow,
    QuitUi,
}

#[derive(Clone)]
struct AppState {
    settings_path: std::path::PathBuf,
    // Service lifecycle manager - controls mrd-service
    service_manager: std::sync::Arc<std::sync::Mutex<service_manager::ServiceManager>>,
    // Test harness for end-to-end pipeline visualization
    test_harness: std::sync::Arc<std::sync::Mutex<test_harness::TestHarness>>,
    // Test orchestrator - unified test execution and management
    test_orchestrator: std::sync::Arc<test_orchestrator::TestOrchestrator>,
    // Lightweight resource sampler for the test workbench title bar
    resource_monitor: std::sync::Arc<std::sync::Mutex<ResourceMonitor>>,
    // Remote display windows: frameless web chrome plus optional native DX11 surface.
    render_window_registry: std::sync::Arc<std::sync::Mutex<RenderWindowRegistry>>,
    remote_display_surfaces: std::sync::Arc<std::sync::Mutex<RemoteDisplaySurfaceManager>>,
    // Local browser WebRTC preview host for the remote display window Web mode.
    webrtc_host: std::sync::Arc<tokio::sync::Mutex<webrtc_host::WebrtcHost>>,
}

/// 设备注册响应
#[derive(Debug, Serialize, Deserialize)]
struct DeviceRegistrationResponse {
    device_id: String,
    device_name: String,
    access_token: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct DecodePolicyResponse {
    decode_policy: String,
}

#[derive(Debug, Serialize)]
struct ClientDiagnostics {
    app_pid: u32,
    app_exe_path: Option<String>,
    current_dir: Option<String>,
    log_dir: String,
    service_exe_path: String,
    service_stdout_log: String,
    service_stderr_log: String,
}

#[derive(Debug, Serialize)]
struct BrowserWebrtcPreviewAnswer {
    session_id: String,
    answer_sdp: String,
}

fn ensure_rustls_crypto_provider() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

/// Tauri 命令：获取硬件信息
#[tauri::command]
fn get_hardware_info() -> Result<HardwareInfo, String> {
    Ok(device_info::get_hardware_info())
}

#[tauri::command]
async fn get_system_resource_snapshot(
    state: tauri::State<'_, AppState>,
) -> Result<SystemResourceSnapshot, String> {
    let service_pid = query_service_pid().await;
    let harness_running = state.test_harness.lock().unwrap().get_metrics().is_running;
    let (target_pid, target_name) = if harness_running {
        (Some(std::process::id()), "Rdesk Workbench")
    } else if service_pid.is_some() {
        (service_pid, "mrd-service")
    } else {
        (Some(std::process::id()), "Rdesk Workbench")
    };

    Ok(state
        .resource_monitor
        .lock()
        .unwrap()
        .snapshot_for_process(target_pid, target_name))
}

async fn query_service_pid() -> Option<u32> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    match client.send_request(IpcRequest::ServiceHealth).await {
        Ok(IpcResponse::ServiceHealth { status }) => status.pid,
        _ => None,
    }
}

#[tauri::command]
fn start_drag_window(window: WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|err| err.to_string())
}

#[tauri::command]
fn minimize_window(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
fn toggle_maximize_window(window: WebviewWindow) -> Result<bool, String> {
    if window.is_maximized().map_err(|err| err.to_string())? {
        window.unmaximize().map_err(|err| err.to_string())?;
        Ok(false)
    } else {
        window.maximize().map_err(|err| err.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
fn hide_to_tray(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
fn show_window(app_handle: AppHandle) -> Result<(), String> {
    show_main_window(&app_handle)
}

#[tauri::command]
fn center_window(window: WebviewWindow) -> Result<(), String> {
    window.center().map_err(|err| err.to_string())
}

#[tauri::command]
fn close_window(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
fn set_window_decorations(window: WebviewWindow, decorated: bool) -> Result<(), String> {
    window
        .set_decorations(decorated)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn apply_native_chrome(window: WebviewWindow) -> platform::NativeBackdropStatus {
    platform::configure_main_window(&window)
}

#[tauri::command]
fn open_remote_display_window(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
    surface_id: Option<String>,
) -> Result<RenderWindowContext, String> {
    let spec = {
        let mut registry = state.render_window_registry.lock().unwrap();
        registry.reserve_window(SessionId(session_id), surface_id)?
    };

    let context = {
        let mut registry = state.render_window_registry.lock().unwrap();
        let session_window_count = registry.register_window(
            spec.session_id.clone(),
            spec.label.clone(),
            spec.surface_id.clone(),
        );
        RenderWindowContext {
            label: spec.label.clone(),
            session_id: spec.session_id.0.clone(),
            surface_id: spec.surface_id.clone(),
            role: "controller".to_string(),
            renderer_attached: false,
            render_mode: "web".to_string(),
            native_surface_attached: false,
            session_window_count,
        }
    };

    let app_for_window = app.clone();
    let state_for_window = state.inner().clone();
    std::thread::spawn(move || {
        let build_app = app_for_window.clone();
        if let Err(error) = app_for_window.run_on_main_thread(move || {
            if let Err(error) = build_remote_display_window(&build_app, state_for_window, spec) {
                eprintln!("{error}");
            }
        }) {
            eprintln!("schedule remote display window failed: {error}");
        }
    });

    Ok(context)
}

fn build_remote_display_window(
    app: &AppHandle,
    state: AppState,
    spec: PendingRenderWindow,
) -> Result<(), String> {
    let label = spec.label.clone();
    let session_id = spec.session_id.0.clone();
    let window = WebviewWindowBuilder::new(app, spec.label.clone(), spec.url)
        .title(format!("Rdesk Display {}", spec.session_id.0))
        .decorations(false)
        .resizable(true)
        .inner_size(1280.0, 800.0)
        .min_inner_size(720.0, 420.0)
        .visible(false)
        .build()
        .map_err(|error| format!("create remote display window failed: {error}"))?;

    let cleanup_app = app.clone();
    let cleanup_state = state.clone();
    window.on_window_event(move |event| {
        if matches!(
            event,
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
        ) {
            cleanup_remote_display_window(
                &cleanup_app,
                &cleanup_state,
                &label,
                Some(session_id.clone()),
                "window_close",
            );
        }
    });

    window
        .show()
        .map_err(|error| format!("show remote display window failed: {error}"))?;
    let _ = window.set_focus();
    Ok(())
}

fn cleanup_remote_display_window(
    app: &AppHandle,
    state: &AppState,
    label: &str,
    _session_id_hint: Option<String>,
    reason: &'static str,
) {
    let window = app.get_webview_window(label);
    if let Err(error) = state
        .remote_display_surfaces
        .lock()
        .unwrap()
        .detach(label, window.as_ref())
    {
        eprintln!("remote display surface cleanup failed for {label}: {error}");
    }

    let removed = state
        .render_window_registry
        .lock()
        .unwrap()
        .remove_window_entry(label);

    let Some((session_id, remaining_windows)) = removed else {
        return;
    };
    let session_id = session_id.0;

    if remaining_windows == 0 {
        if is_local_pipeline_preview_session(&session_id) {
            stop_test_harness_async(state.test_harness.clone(), label.to_string());
        }
        stop_browser_webrtc_preview_async(state.webrtc_host.clone(), session_id.clone());
        stop_session_async(session_id, reason);
    }
}

fn is_local_pipeline_preview_session(session_id: &str) -> bool {
    session_id == "local-preview" || session_id.starts_with("local-display-test")
}

fn stop_test_harness_async(
    harness: std::sync::Arc<std::sync::Mutex<test_harness::TestHarness>>,
    label: String,
) {
    std::thread::spawn(move || {
        if let Err(error) = harness.lock().unwrap().stop() {
            eprintln!("test harness cleanup failed for {label}: {error}");
        }
    });
}

fn stop_browser_webrtc_preview_async(
    host: std::sync::Arc<tokio::sync::Mutex<webrtc_host::WebrtcHost>>,
    session_id: String,
) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Runtime::new() else {
            eprintln!("create runtime for browser webrtc preview cleanup failed: {session_id}");
            return;
        };

        rt.block_on(async move {
            let session_id = SessionId(session_id);
            let mut host = host.lock().await;
            if host.snapshot(&session_id).is_some() {
                if let Err(error) = host.close_session(&session_id).await {
                    eprintln!(
                        "browser webrtc preview cleanup failed for {}: {error}",
                        session_id.0
                    );
                }
            }
        });
    });
}

fn stop_session_async(session_id: String, reason: &'static str) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Runtime::new() else {
            eprintln!("create runtime for session cleanup failed: {session_id}");
            return;
        };

        rt.block_on(async move {
            use mrd_ipc::{IpcRequest, IpcResponse};
            let mut client = mrd_ipc::client::IpcClient::new();
            match client
                .send_request(IpcRequest::StopSession {
                    session_id: SessionId(session_id.clone()),
                })
                .await
            {
                Ok(IpcResponse::SessionStopped { .. }) => {}
                Ok(IpcResponse::Error { code, message }) => {
                    eprintln!(
                        "stop session on {reason} failed for {session_id}: {code}: {message}"
                    );
                }
                Ok(_) => {
                    eprintln!(
                        "stop session on {reason} returned unexpected response for {session_id}"
                    );
                }
                Err(error) => {
                    eprintln!("stop session on {reason} failed for {session_id}: {error}");
                }
            }
        });
    });
}

#[tauri::command]
fn list_remote_display_windows(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Vec<RenderWindowContext> {
    state
        .render_window_registry
        .lock()
        .unwrap()
        .list_window_contexts(&app, &SessionId(session_id))
}

#[tauri::command]
fn current_remote_display_window_context(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Option<RenderWindowContext> {
    state
        .render_window_registry
        .lock()
        .unwrap()
        .context_for_label(window.app_handle(), window.label())
}

#[tauri::command]
fn close_remote_display_window(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    label: String,
) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("remote display window not found: {label}"))?;
    cleanup_remote_display_window(&app, state.inner(), &label, None, "window_close_command");
    window
        .close()
        .map_err(|error| format!("close remote display window failed: {error}"))
}

#[tauri::command]
async fn browser_webrtc_preview_start(
    state: tauri::State<'_, AppState>,
    session_id: String,
    offer_sdp: String,
    fps: Option<u32>,
    h264_profile: Option<String>,
) -> Result<BrowserWebrtcPreviewAnswer, String> {
    ensure_rustls_crypto_provider();

    let session_id = SessionId(session_id);
    let fps = fps.unwrap_or(60).clamp(1, 144);
    let h264_profile = h264_profile.unwrap_or_else(|| "baseline".to_string());
    let encoded_access_units = state
        .test_harness
        .lock()
        .unwrap()
        .subscribe_encoded_access_units();
    let mut host = state.webrtc_host.lock().await;
    host.apply_remote_offer(session_id.clone(), offer_sdp)
        .await?;
    host.prepare_browser_h264_sender(session_id.clone(), fps, &h264_profile)
        .await?;
    let answer = host.create_answer(session_id.clone()).await?;
    host.start_encoded_access_unit_sender(session_id, fps, &h264_profile, encoded_access_units)
        .await?;

    Ok(BrowserWebrtcPreviewAnswer {
        session_id: answer.session_id.0,
        answer_sdp: answer.sdp,
    })
}

#[tauri::command]
async fn browser_webrtc_preview_stop(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    let session_id = SessionId(session_id);
    let mut host = state.webrtc_host.lock().await;
    if host.snapshot(&session_id).is_some() {
        host.close_session(&session_id).await?;
    }
    Ok(())
}

#[tauri::command]
fn configure_remote_display_native_surface(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
    rect: NativeSurfaceRect,
    enabled: bool,
    visible: Option<bool>,
) -> Result<NativeRenderSurfaceSnapshot, String> {
    let snapshot = state.remote_display_surfaces.lock().unwrap().configure(
        &window,
        rect,
        enabled,
        visible.unwrap_or(enabled),
    )?;

    let render_mode = match (snapshot.attached, snapshot.backend.as_str()) {
        (true, "macos") => "macos_native",
        (true, _) => "d3d11_native",
        (false, _) => "web",
    };
    let _ = state
        .render_window_registry
        .lock()
        .unwrap()
        .set_render_mode(
            window.app_handle(),
            window.label(),
            render_mode.to_string(),
            snapshot.attached,
        );

    Ok(snapshot)
}

#[tauri::command]
fn present_test_harness_frame_on_native_surface(
    window: WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let label = window.label().to_string();
    let target = state
        .remote_display_surfaces
        .lock()
        .unwrap()
        .render_target_handle(&label);
    let Some(target) = target else {
        return Ok(false);
    };

    present_native_probe_frame(target)
}

#[cfg(target_os = "macos")]
fn present_native_probe_frame(target: isize) -> Result<bool, String> {
    use mrd_render::{RenderTarget, RendererFactory};

    let factory = mrd_render_macos::MacosRendererFactory;
    let mut renderer = factory
        .create()
        .map_err(|error| format!("create Metal probe renderer failed: {error}"))?;
    renderer
        .attach_target(RenderTarget::WindowHandle(target))
        .map_err(|error| format!("attach Metal probe renderer failed: {error}"))?;
    renderer
        .upload_frame(build_native_probe_frame(640, 360))
        .map_err(|error| format!("present Metal probe frame failed: {error}"))?;

    let snapshot = renderer.snapshot();
    Ok(snapshot.attached_to_target && snapshot.uploaded_frame_count > 0)
}

#[cfg(windows)]
fn present_native_probe_frame(target: isize) -> Result<bool, String> {
    use mrd_render::{RenderTarget, RendererFactory};

    let factory = mrd_render_d3d11::D3d11RendererFactory;
    let mut renderer = factory
        .create()
        .map_err(|error| format!("create D3D11 probe renderer failed: {error}"))?;
    renderer
        .attach_target(RenderTarget::WindowHandle(target))
        .map_err(|error| format!("attach D3D11 probe renderer failed: {error}"))?;
    renderer
        .upload_frame(build_native_probe_frame(640, 360))
        .map_err(|error| format!("present D3D11 probe frame failed: {error}"))?;

    let snapshot = renderer.snapshot();
    Ok(snapshot.attached_to_target && snapshot.uploaded_frame_count > 0)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn present_native_probe_frame(_target: isize) -> Result<bool, String> {
    Ok(false)
}

fn build_native_probe_frame(width: usize, height: usize) -> mrd_render::RenderFrame {
    let mut bgra = vec![0_u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let checker = ((x / 40) + (y / 40)) % 2 == 0;
            let edge = x < 12 || y < 12 || x + 12 >= width || y + 12 >= height;
            let (r, g, b) = if edge {
                (255, 255, 255)
            } else if checker {
                (20, 210, 255)
            } else {
                (240, 70, 120)
            };
            bgra[offset] = b;
            bgra[offset + 1] = g;
            bgra[offset + 2] = r;
            bgra[offset + 3] = 255;
        }
    }

    mrd_render::RenderFrame::from_bgra32(width, height, bgra)
}

#[tauri::command]
fn get_client_diagnostics(state: tauri::State<'_, AppState>) -> ClientDiagnostics {
    let service_exe_path = state
        .service_manager
        .lock()
        .unwrap()
        .service_exe_path()
        .display()
        .to_string();

    ClientDiagnostics {
        app_pid: std::process::id(),
        app_exe_path: std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string()),
        current_dir: std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string()),
        log_dir: service_manager::runtime_log_dir().display().to_string(),
        service_exe_path,
        service_stdout_log: service_manager::service_stdout_log_path()
            .display()
            .to_string(),
        service_stderr_log: service_manager::service_stderr_log_path()
            .display()
            .to_string(),
    }
}

#[tauri::command]
fn open_diagnostics_folder() -> Result<(), String> {
    let log_dir = service_manager::runtime_log_dir();
    std::fs::create_dir_all(&log_dir).map_err(|error| error.to_string())?;

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("explorer.exe");
        command.arg(&log_dir);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(&log_dir);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&log_dir);
        command
    };

    command.spawn().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
fn automation_write_report(report: serde_json::Value) -> Result<Option<String>, String> {
    let Ok(path) = std::env::var(LAN_E2E_REPORT_PATH_ENV) else {
        return Ok(None);
    };
    let path = path.trim();
    if path.is_empty() {
        return Ok(None);
    }

    let path = std::path::PathBuf::from(path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create automation report directory failed: {error}"))?;
        }
    }

    let content = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("serialize automation report failed: {error}"))?;
    std::fs::write(&path, content)
        .map_err(|error| format!("write automation report failed: {error}"))?;
    Ok(Some(path.display().to_string()))
}

// nvdec_runtime_probe - moved to rdesk-legacy-harness package
#[tauri::command]
fn nvdec_runtime_probe() -> Result<serde_json::Value, String> {
    Err(
        "nvdec_runtime_probe moved to mrd-service - use rdesk-legacy-harness for testing"
            .to_string(),
    )
}

#[tauri::command]
async fn decode_policy(_state: tauri::State<'_, AppState>) -> Result<DecodePolicyResponse, String> {
    // Decode policy is now managed by mrd-service
    // Return current policy from settings
    Err("Use IPC to query decode policy from mrd-service".to_string())
}

#[tauri::command]
async fn set_decode_policy(
    state: tauri::State<'_, AppState>,
    decode_policy: String,
) -> Result<DecodePolicyResponse, String> {
    let decode_policy = parse_decode_policy(&decode_policy)?;
    // Save to settings only - actual policy application happens in mrd-service
    set_decode_policy_with(&state.settings_path, decode_policy).await
}

// ============================================================================
// Bootstrap Commands (Phase 6: bootstrap-only behavior)
// ============================================================================

/// Bootstrap mrd-service if not already running via IPC
/// Phase 6: This is the ONLY start method. Returns true if bootstrap was performed.
#[tauri::command]
async fn service_bootstrap_if_needed(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager
                .lock()
                .unwrap()
                .bootstrap_if_needed()
                .await
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Wait for service to be healthy (with timeout)
#[tauri::command]
async fn service_wait_for_healthy(
    state: tauri::State<'_, AppState>,
    timeout_secs: u64,
) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager
                .lock()
                .unwrap()
                .wait_for_healthy(timeout_secs)
                .await
                .map_err(|e| e.to_string())
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Check if this instance bootstrapped the service
#[tauri::command]
async fn service_did_bootstrap(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { Ok(manager.lock().unwrap().did_bootstrap().await) })
    })
    .await
    .map_err(|e| e.to_string())?
}

// ============================================================================
// Shell / Lifecycle Commands (Phase 2)
// ============================================================================

/// Register UI presence with mrd-service
#[tauri::command]
async fn shell_ui_attached() -> Result<(), String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::UiAttached {
            pid: std::process::id(),
            executable_path: std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(String::from)),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::Ack => Ok(()),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Notify mrd-service that UI is detaching
#[tauri::command]
async fn shell_ui_detached(reason: String) -> Result<(), String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let detach_reason = match reason.as_str() {
        "user_close" => mrd_ipc::UiDetachReason::UserClose,
        "user_quit" => mrd_ipc::UiDetachReason::UserQuit,
        "crash" => mrd_ipc::UiDetachReason::Crash,
        "connection_lost" => mrd_ipc::UiDetachReason::ConnectionLost,
        _ => return Err(format!("Unknown detach reason: {}", reason)),
    };

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::UiDetached {
            pid: std::process::id(),
            reason: detach_reason,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::Ack => Ok(()),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get shell/service status
#[tauri::command]
async fn shell_get_status() -> Result<mrd_ipc::ShellStatusSnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::GetShellStatus)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ShellStatus { status } => Ok(status),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Request service shutdown (Phase 2: returns error until fully implemented)
#[tauri::command]
async fn shell_shutdown_service(mode: String) -> Result<(), String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let shutdown_mode = match mode.as_str() {
        "graceful" => mrd_ipc::ShutdownMode::Graceful,
        "force" => mrd_ipc::ShutdownMode::Force,
        "after_sessions" => mrd_ipc::ShutdownMode::AfterSessions,
        _ => return Err(format!("Unknown shutdown mode: {}", mode)),
    };

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ShutdownService {
            mode: shutdown_mode,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::Ack => Ok(()),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Quit UI and stop service (explicit user action)
///
/// Phase 6: This now uses IPC ShutdownService instead of directly stopping
/// the service process. Rdesk no longer owns service lifecycle.
#[tauri::command]
async fn shell_quit_ui_and_stop_service(app_handle: tauri::AppHandle) -> Result<(), String> {
    // Notify service that UI is detaching
    let _ = shell_ui_detached("user_quit".to_string()).await;

    // Stop all active sessions via IPC
    use mrd_ipc::{IpcRequest, IpcResponse};
    let mut client = mrd_ipc::client::IpcClient::new();
    if let Ok(IpcResponse::SessionList { sessions }) =
        client.send_request(IpcRequest::ListSessions).await
    {
        for session_info in sessions {
            let _ = client
                .send_request(IpcRequest::StopSession {
                    session_id: session_info.session_id,
                })
                .await;
        }
    }

    // Request service shutdown via IPC (Phase 6: service owns lifecycle)
    let _ = client
        .send_request(IpcRequest::ShutdownService {
            mode: mrd_ipc::ShutdownMode::Graceful,
        })
        .await;

    // Exit the UI application
    request_app_exit(&app_handle, "user_quit");
    Ok(())
}

// ============================================================================
// IPC-based commands (migrated to use mrd-service)
// ============================================================================

/// Register device via IPC (migrated version)
#[tauri::command]
async fn ipc_register_device(device_id: String, device_name: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::DeviceId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::RegisterDevice {
            device_id: DeviceId(device_id),
            device_name,
        })
        .await
        .map_err(|e| e.to_string())?;

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
    let response = client
        .send_request(IpcRequest::ListDevices)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::DeviceList { devices } => Ok(devices),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get LAN peer discovery snapshot via IPC.
#[tauri::command]
async fn ipc_lan_discovery_snapshot() -> Result<mrd_ipc::LanDiscoverySnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::LanDiscoverySnapshot)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::LanDiscoverySnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Trigger a LAN discovery probe via IPC.
#[tauri::command]
async fn ipc_refresh_lan_discovery() -> Result<mrd_ipc::LanDiscoverySnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::RefreshLanDiscovery)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::LanDiscoverySnapshot { snapshot } => Ok(snapshot),
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
    use mrd_proto::{DeviceId, SessionId};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::StartSession {
            session_id: SessionId(session_id),
            target_device_id: DeviceId(target_device_id),
            transport_kind,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start LAN P2P session via IPC and ask the peer to auto-accept it.
#[tauri::command]
async fn ipc_start_lan_remote_session(
    session_id: String,
    target_device_id: String,
    transport_kind: String,
    requested_profile: Option<mrd_ipc::MediaProfile>,
) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::{DeviceId, SessionId};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::StartLanRemoteSession {
            session_id: SessionId(session_id),
            target_device_id: DeviceId(target_device_id),
            transport_kind,
            requested_profile,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Request runtime media profile switch via IPC.
#[tauri::command]
async fn ipc_update_media_profile(
    session_id: String,
    requested_profile: mrd_ipc::MediaProfile,
) -> Result<mrd_ipc::MediaProfileNegotiation, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::UpdateMediaProfile {
            session_id: SessionId(session_id),
            requested_profile,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::MediaProfileUpdated { negotiation, .. } => Ok(negotiation),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// List remote capture sources through mrd-service IPC.
#[tauri::command]
async fn ipc_list_remote_capture_sources(
    session_id: String,
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<mrd_ipc::CaptureSource>, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ListRemoteCaptureSources {
            session_id: SessionId(session_id),
            include_previews,
            limit,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::CaptureSourceList { sources, .. } => Ok(sources),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Select a remote capture source through mrd-service IPC.
#[tauri::command]
async fn ipc_select_remote_capture_source(
    session_id: String,
    source_id: String,
) -> Result<mrd_ipc::CaptureSourceSelection, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::SelectRemoteCaptureSource {
            session_id: SessionId(session_id),
            source_id,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::CaptureSourceSelected { selection, .. } => Ok(selection),
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
    use mrd_proto::{DeviceId, SessionId};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::AcceptSession {
            session_id: SessionId(session_id),
            source_device_id: DeviceId(source_device_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionAccepted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Stop session via IPC (migrated version)
#[tauri::command]
async fn ipc_stop_session(session_id: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::StopSession {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionStopped { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Mark a session failed via IPC.
#[tauri::command]
async fn ipc_fail_session(session_id: String, reason: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::FailSession {
            session_id: SessionId(session_id),
            reason,
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionFailed { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Recover a failed or closed session via IPC.
#[tauri::command]
async fn ipc_recover_session(session_id: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::RecoverSession {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionRecovered { session_id } => Ok(session_id.0),
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
    let response = client
        .send_request(IpcRequest::SessionRuntimeSnapshot {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionSnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start sender via IPC (migrated version)
#[tauri::command]
async fn ipc_start_sender(session_id: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::StartSender {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SenderStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Start receiver via IPC (migrated version)
#[tauri::command]
async fn ipc_start_receiver(session_id: String) -> Result<String, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::StartReceiver {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ReceiverStarted { session_id } => Ok(session_id.0),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// List sessions via IPC (migrated version)
#[tauri::command]
async fn ipc_list_sessions() -> Result<Vec<mrd_ipc::SessionInfo>, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ListSessions)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::SessionList { sessions } => Ok(sessions),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get runtime snapshot via IPC (migrated version)
#[tauri::command]
async fn ipc_runtime_snapshot() -> Result<mrd_ipc::RuntimeSnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::RuntimeSnapshot)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::RuntimeSnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get structured local capability snapshot via IPC.
#[tauri::command]
async fn ipc_capability_snapshot() -> Result<mrd_ipc::CapabilitySnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::CapabilitySnapshot)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::CapabilitySnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Service health check via IPC (migrated version)
#[tauri::command]
async fn ipc_service_health() -> Result<mrd_ipc::ServiceStatus, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ServiceHealth)
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ServiceHealth { status } => Ok(status),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get probe snapshot via IPC (migrated version)
#[tauri::command]
async fn ipc_probe_snapshot(session_id: String) -> Result<mrd_ipc::ProbeSnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ProbeSnapshot {
            session_id: SessionId(session_id),
        })
        .await
        .map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ProbeSnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

// Legacy Tauri commands removed - use rdesk-legacy-harness package for testing

// Example of migrated WebRTC command using IPC
#[tauri::command]
async fn webrtc_session_list_via_ipc() -> Result<Vec<String>, String> {
    use mrd_ipc::IpcRequest;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client
        .send_request(IpcRequest::ListDevices)
        .await
        .map_err(|e| format!("IPC error: {}", e))?;

    match response {
        mrd_ipc::IpcResponse::DeviceList { devices } => {
            Ok(devices.into_iter().map(|d| d.device_id.0).collect())
        }
        mrd_ipc::IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

// Legacy Tauri commands removed - use rdesk-legacy-harness package for testing

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

fn parse_decode_policy(value: &str) -> Result<DecodePolicy, String> {
    match value {
        "auto" => Ok(DecodePolicy::Auto),
        "software" => Ok(DecodePolicy::Software),
        "d3d11va" => Ok(DecodePolicy::D3d11va),
        "nvdec" => Ok(DecodePolicy::Nvdec),
        other => Err(format!("未知 decode policy: {other}")),
    }
}

async fn set_decode_policy_with(
    settings_path: &std::path::Path,
    decode_policy: DecodePolicy,
) -> Result<DecodePolicyResponse, String> {
    // Save policy to settings - actual decode policy application now happens in mrd-service
    save_settings(settings_path, &AppSettings { decode_policy })?;
    Ok(DecodePolicyResponse {
        decode_policy: decode_policy.as_str().to_string(),
    })
}

// ============================================================================
// Test harness commands - end-to-end pipeline visualization
// ============================================================================

#[tauri::command]
fn test_harness_start(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .test_harness
        .lock()
        .unwrap()
        .start()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn test_harness_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .test_harness
        .lock()
        .unwrap()
        .stop()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn test_harness_set_chain(state: tauri::State<'_, AppState>, chain: String) -> Result<(), String> {
    use test_harness::TestChain;
    let parsed = match chain.as_str() {
        "capture_only" => TestChain::CaptureOnly,
        "nvenc_nvdec" => TestChain::NvencNvdec,
        "nvenc_only" => TestChain::NvencOnly,
        "openh264" => TestChain::OpenH264,
        _ => return Err(format!("未知的测试链路: {}", chain)),
    };
    state.test_harness.lock().unwrap().set_chain(parsed);
    Ok(())
}

#[tauri::command]
fn test_harness_set_custom(
    state: tauri::State<'_, AppState>,
    capture: String,
    encoder: String,
    decoder: String,
) -> Result<(), String> {
    use test_harness::{CaptureType, DecoderType, EncoderType, TestChain};

    let capture = match capture.as_str() {
        "dxgi" => CaptureType::Dxgi,
        "winrt" => CaptureType::Winrt,
        "macos" => CaptureType::Macos,
        #[cfg(target_os = "linux")]
        "linux" => CaptureType::Linux,
        "synthetic" => CaptureType::Synthetic,
        _ => return Err(format!("Unsupported capture type: {}", capture)),
    };
    let encoder = match encoder.as_str() {
        "none" => EncoderType::None,
        "nvenc_h264" => EncoderType::NvencH264,
        "nvenc_av1" => EncoderType::NvencAv1,
        "nvenc_hevc" | "hevc" => EncoderType::NvencHevc,
        "nvenc_hevc_main10" | "hevc_main10" | "hevc-main10" => EncoderType::NvencHevcMain10,
        "openh264" | "software_h264" | "h264_software" | "software-h264" | "h264-software"
        | "sw_h264" => EncoderType::OpenH264,
        "videotoolbox_h264" | "videotoolbox" => EncoderType::VideoToolboxH264,
        _ => return Err(format!("Unsupported encoder type: {}", encoder)),
    };
    let decoder = match decoder.as_str() {
        "none" => DecoderType::None,
        "nvdec" => DecoderType::Nvdec,
        "software" | "software_h264" | "h264_software" | "software-h264" | "h264-software"
        | "openh264" => DecoderType::Software,
        "linux_h264" | "gstreamer_h264" | "vaapi_h264" => DecoderType::LinuxH264,
        "linux_hevc" | "gstreamer_hevc" | "vaapi_hevc" => DecoderType::LinuxHevc,
        "linux_hevc_main10" | "gstreamer_hevc_main10" | "vaapi_hevc_main10" => {
            DecoderType::LinuxHevcMain10
        }
        "videotoolbox" => DecoderType::VideoToolbox,
        _ => return Err(format!("Unsupported decoder type: {}", decoder)),
    };

    state
        .test_harness
        .lock()
        .unwrap()
        .set_chain(TestChain::Custom {
            capture,
            encoder,
            decoder,
        });
    Ok(())
}

#[tauri::command]
fn test_harness_get_chain(state: tauri::State<'_, AppState>) -> String {
    use test_harness::TestChain;
    match state.test_harness.lock().unwrap().get_chain() {
        TestChain::CaptureOnly => "capture_only".to_string(),
        TestChain::NvencNvdec => "nvenc_nvdec".to_string(),
        TestChain::NvencOnly => "nvenc_only".to_string(),
        TestChain::OpenH264 => "openh264".to_string(),
        #[cfg(target_os = "linux")]
        TestChain::LinuxOpenh264 => "linux_openh264".to_string(),
        TestChain::Custom { .. } => "custom".to_string(),
    }
}

#[tauri::command]
fn test_harness_get_metrics(state: tauri::State<'_, AppState>) -> test_harness::HarnessMetrics {
    state.test_harness.lock().unwrap().get_metrics()
}

#[tauri::command]
fn test_harness_get_comparison_result(
    state: tauri::State<'_, AppState>,
) -> mrd_observability::PipelineComparisonResult {
    state
        .test_harness
        .lock()
        .unwrap()
        .get_pipeline_comparison_result()
}

#[tauri::command]
async fn test_harness_get_frames(
    state: tauri::State<'_, AppState>,
    include_captured: Option<bool>,
    include_rendered: Option<bool>,
    last_captured_generation: Option<u64>,
    last_rendered_generation: Option<u64>,
) -> Result<
    (
        Option<(String, usize, usize, u64)>,
        Option<(String, usize, usize, u64)>,
    ),
    String,
> {
    let include_captured = include_captured.unwrap_or(true);
    let include_rendered = include_rendered.unwrap_or(true);
    let test_harness = state.test_harness.clone();

    tokio::task::spawn_blocking(move || {
        let (captured, rendered) = test_harness.lock().unwrap().get_latest_frames_since(
            include_captured,
            include_rendered,
            last_captured_generation,
            last_rendered_generation,
        );

        let captured_base64 = if include_captured {
            captured.and_then(|(data, width, height, generation)| {
                encode_bgra_png_base64(&data, width, height)
                    .map(|png| (png, width, height, generation))
            })
        } else {
            None
        };

        let rendered_base64 = if include_rendered {
            rendered.and_then(|(data, width, height, generation)| {
                encode_bgra_png_base64(&data, width, height)
                    .map(|png| (png, width, height, generation))
            })
        } else {
            None
        };

        (captured_base64, rendered_base64)
    })
    .await
    .map_err(|error| error.to_string())
}

fn encode_bgra_png_base64(bgra: &[u8], width: usize, height: usize) -> Option<String> {
    use base64::Engine;

    let rgba = convert_bgra_to_rgba(bgra);
    let image = image::RgbaImage::from_raw(width as u32, height as u32, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut png, image::ImageFormat::Png)
        .ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(png.into_inner()))
}

fn convert_bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    bgra.chunks_exact(4)
        .flat_map(|chunk| [chunk[2], chunk[1], chunk[0], chunk[3]])
        .collect()
}

#[cfg(test)]
mod frame_encoding_tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn bgra_frame_is_encoded_as_png() {
        let bgra = [
            0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 255, 255, 255, 255,
        ];

        let encoded = encode_bgra_png_base64(&bgra, 2, 2).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();

        assert_eq!(&decoded[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn native_probe_frame_is_bgra_and_non_empty() {
        let frame = build_native_probe_frame(64, 48);

        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert_eq!(frame.pixel_format, mrd_render::RenderPixelFormat::Bgra32);
        assert_eq!(frame.as_bgra32().unwrap().len(), 64 * 48 * 4);
    }
}

// ============================================================================
// Test Workbench Commands (New Unified Test API)
// ============================================================================

/// List all available test scenarios
#[tauri::command]
fn test_list_scenarios(state: tauri::State<'_, AppState>) -> Vec<test_orchestrator::TestScenario> {
    state.test_orchestrator.list_scenarios()
}

/// Get environment capabilities
#[tauri::command]
async fn test_get_capabilities(
    state: tauri::State<'_, AppState>,
) -> Result<test_orchestrator::EnvironmentSnapshot, String> {
    let orchestrator = state.test_orchestrator.clone();
    tokio::task::spawn_blocking(move || orchestrator.get_capabilities().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// List visible top-level windows that can be used as platform capture targets.
#[tauri::command]
fn test_list_window_capture_targets() -> Result<Vec<test_orchestrator::WindowCaptureTarget>, String>
{
    test_orchestrator::list_window_capture_targets().map_err(|e| e.to_string())
}

/// List platform window capture targets and attach best-effort screenshot previews.
#[tauri::command]
fn test_list_window_capture_targets_with_previews(
    limit: Option<usize>,
) -> Result<Vec<test_orchestrator::WindowCaptureTarget>, String> {
    test_orchestrator::list_window_capture_targets_with_previews(limit).map_err(|e| e.to_string())
}

/// Start a test run
#[tauri::command]
async fn test_start_run(
    state: tauri::State<'_, AppState>,
    scenario_id: String,
    config: test_orchestrator::TestConfigData,
) -> Result<String, String> {
    let orchestrator = state.test_orchestrator.clone();
    tokio::task::spawn_blocking(move || {
        orchestrator
            .start_run(scenario_id, config)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stop a test run
#[tauri::command]
async fn test_stop_run(state: tauri::State<'_, AppState>, run_id: String) -> Result<(), String> {
    let orchestrator = state.test_orchestrator.clone();
    tokio::task::spawn_blocking(move || orchestrator.stop_run(&run_id).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

/// List test runs
#[tauri::command]
fn test_list_runs(
    state: tauri::State<'_, AppState>,
    scenario_id: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
) -> Vec<test_orchestrator::TestRun> {
    state
        .test_orchestrator
        .list_runs(scenario_id, status, limit)
}

/// Get a test run
#[tauri::command]
fn test_get_run(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Option<test_orchestrator::TestRun> {
    state.test_orchestrator.get_run(&run_id)
}

/// Get run events
#[tauri::command]
fn test_get_run_events(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Vec<test_orchestrator::TestStageEvent> {
    state.test_orchestrator.get_run_events(&run_id)
}

/// Get run metrics
#[tauri::command]
fn test_get_run_metrics(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> std::collections::HashMap<String, test_orchestrator::MetricSeries> {
    state.test_orchestrator.get_run_metrics(&run_id)
}

/// Get run artifacts
#[tauri::command]
fn test_get_run_artifacts(
    state: tauri::State<'_, AppState>,
    run_id: String,
) -> Vec<test_orchestrator::Artifact> {
    state.test_orchestrator.get_run_artifacts(&run_id)
}

/// List test presets
#[tauri::command]
fn test_list_presets(state: tauri::State<'_, AppState>) -> Vec<test_orchestrator::TestPreset> {
    state.test_orchestrator.list_presets()
}

/// Save a test preset
#[tauri::command]
fn test_save_preset(
    state: tauri::State<'_, AppState>,
    name: String,
    description: String,
    scenario_id: String,
    config: test_orchestrator::TestConfigData,
) -> String {
    state
        .test_orchestrator
        .save_preset(name, description, scenario_id, config)
}

/// Delete a test preset
#[tauri::command]
fn test_delete_preset(state: tauri::State<'_, AppState>, preset_id: String) -> Result<(), String> {
    state
        .test_orchestrator
        .delete_preset(&preset_id)
        .map_err(|e| e.to_string())
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())
}

fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = main_window(app)?;
    let _ = window.unminimize();
    window.show().map_err(|err| err.to_string())?;
    window.set_focus().map_err(|err| err.to_string())?;
    Ok(())
}

fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    main_window(app)?.hide().map_err(|err| err.to_string())
}

fn detach_ui_async(reason: &'static str) {
    std::thread::spawn(move || {
        let Ok(rt) = tokio::runtime::Runtime::new() else {
            return;
        };
        rt.block_on(async move {
            let _ = shell_ui_detached(reason.to_string()).await;
        });
    });
}

fn claim_single_instance() -> Option<TcpListener> {
    match TcpListener::bind(SINGLE_INSTANCE_ADDR) {
        Ok(listener) => Some(listener),
        Err(_) => {
            if let Ok(mut stream) = TcpStream::connect(SINGLE_INSTANCE_ADDR) {
                let _ = stream.write_all(b"show\n");
            }
            None
        }
    }
}

fn spawn_single_instance_listener(listener: TcpListener, app: AppHandle) {
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            if incoming.is_err() {
                continue;
            }
            if APP_IS_QUITTING.load(Ordering::SeqCst) {
                break;
            }
            if let Err(error) = show_main_window(&app) {
                eprintln!("failed to show existing Rdesk instance: {error}");
            }
        }
    });
}

fn request_app_exit(app: &AppHandle, reason: &'static str) {
    if APP_IS_QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }

    detach_ui_async(reason);
    for window in app.webview_windows().into_values() {
        let _ = window.close();
    }
    app.exit(0);

    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(750));
        std::process::exit(0);
    });
}

fn setup_system_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, TRAY_MENU_SHOW_ID, "显示主窗口", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, TRAY_MENU_HIDE_ID, "隐藏到托盘", true, None::<&str>)?;
    let center_item = MenuItem::with_id(app, TRAY_MENU_CENTER_ID, "居中窗口", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, TRAY_MENU_QUIT_ID, "退出 Rdesk", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&show_item, &hide_item, &center_item, &separator, &quit_item],
    )?;
    let icon = app.default_window_icon().cloned().unwrap_or_else(|| {
        let image = image::load_from_memory(include_bytes!("../icons/tray-icon.png"))
            .expect("embedded tray icon should be a valid PNG")
            .into_rgba8();
        let (width, height) = image.dimensions();
        tauri::image::Image::new_owned(image.into_raw(), width, height)
    });

    TrayIconBuilder::with_id(TRAY_ICON_ID)
        .icon(icon)
        .tooltip("Rdesk")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .build(app)?;

    Ok(())
}

fn tray_action_from_menu_id(id: &str) -> Option<TrayAction> {
    match id {
        TRAY_MENU_SHOW_ID => Some(TrayAction::ShowWindow),
        TRAY_MENU_HIDE_ID => Some(TrayAction::HideWindow),
        TRAY_MENU_CENTER_ID => Some(TrayAction::CenterWindow),
        TRAY_MENU_QUIT_ID => Some(TrayAction::QuitUi),
        _ => None,
    }
}

fn tray_action_from_icon_event(event: &TrayIconEvent) -> Option<TrayAction> {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
        | TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => Some(TrayAction::ShowWindow),
        _ => None,
    }
}

fn apply_tray_action(app: &AppHandle, action: TrayAction) -> Result<(), String> {
    match action {
        TrayAction::ShowWindow => show_main_window(app),
        TrayAction::HideWindow => hide_main_window(app),
        TrayAction::CenterWindow => main_window(app)?.center().map_err(|err| err.to_string()),
        TrayAction::QuitUi => {
            request_app_exit(app, "user_quit");
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LanE2eAutorunLaunchConfig {
    target_device_id: Option<String>,
    transport: Option<String>,
    timeout_ms: Option<String>,
    min_decoded_frames: Option<String>,
    min_fps: Option<String>,
    stop_on_complete: Option<String>,
}

fn lan_e2e_autorun_config_from_env() -> Option<LanE2eAutorunLaunchConfig> {
    lan_e2e_autorun_config_from_env_lookup(|key| std::env::var(key).ok())
}

fn lan_e2e_autorun_config_from_env_lookup<F>(env: F) -> Option<LanE2eAutorunLaunchConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let enabled = env(LAN_E2E_AUTORUN_ENV)?;
    if !is_truthy_env_value(&enabled) {
        return None;
    }

    Some(LanE2eAutorunLaunchConfig {
        target_device_id: non_empty_env(env(LAN_E2E_TARGET_DEVICE_ID_ENV)),
        transport: non_empty_env(env(LAN_E2E_TRANSPORT_ENV)),
        timeout_ms: non_empty_env(env(LAN_E2E_TIMEOUT_MS_ENV)),
        min_decoded_frames: non_empty_env(env(LAN_E2E_MIN_DECODED_FRAMES_ENV)),
        min_fps: non_empty_env(env(LAN_E2E_MIN_FPS_ENV)),
        stop_on_complete: non_empty_env(env(LAN_E2E_STOP_ON_COMPLETE_ENV)),
    })
}

fn build_lan_e2e_autorun_route(config: LanE2eAutorunLaunchConfig) -> String {
    let mut params = vec![("autorun".to_string(), "lan-e2e".to_string())];

    push_query_param(&mut params, "targetDeviceId", config.target_device_id);
    push_query_param(&mut params, "transport", config.transport);
    push_query_param(&mut params, "timeoutMs", config.timeout_ms);
    push_query_param(&mut params, "minDecodedFrames", config.min_decoded_frames);
    push_query_param(&mut params, "minFps", config.min_fps);
    push_query_param(&mut params, "stopOnComplete", config.stop_on_complete);

    let query = params
        .into_iter()
        .map(|(key, value)| format!("{}={}", key, url_query_escape(&value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/test/e2e?{query}")
}

fn navigate_main_window_to_route(window: &WebviewWindow, route: &str) -> Result<(), String> {
    let route_json = serde_json::to_string(route).map_err(|error| error.to_string())?;
    let script = format!(
        "window.history.replaceState(null, '', {route_json}); window.dispatchEvent(new PopStateEvent('popstate'));"
    );
    window.eval(&script).map_err(|error| error.to_string())
}

fn is_truthy_env_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn non_empty_env(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn push_query_param(params: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.push((key.to_string(), value));
    }
}

fn url_query_escape(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char)
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

#[cfg(test)]
mod tray_tests {
    use super::*;

    #[test]
    fn tray_menu_ids_map_to_actions() {
        assert_eq!(
            tray_action_from_menu_id(TRAY_MENU_SHOW_ID),
            Some(TrayAction::ShowWindow)
        );
        assert_eq!(
            tray_action_from_menu_id(TRAY_MENU_HIDE_ID),
            Some(TrayAction::HideWindow)
        );
        assert_eq!(
            tray_action_from_menu_id(TRAY_MENU_CENTER_ID),
            Some(TrayAction::CenterWindow)
        );
        assert_eq!(
            tray_action_from_menu_id(TRAY_MENU_QUIT_ID),
            Some(TrayAction::QuitUi)
        );
        assert_eq!(tray_action_from_menu_id("unknown"), None);
    }

    #[test]
    fn lan_e2e_autorun_route_uses_env_configuration() {
        let route = build_lan_e2e_autorun_route(LanE2eAutorunLaunchConfig {
            target_device_id: Some("agent device/1".to_string()),
            transport: Some("quic".to_string()),
            timeout_ms: Some("2500".to_string()),
            min_decoded_frames: Some("2".to_string()),
            min_fps: Some("5".to_string()),
            stop_on_complete: Some("false".to_string()),
        });

        assert_eq!(
            route,
            "/test/e2e?autorun=lan-e2e&targetDeviceId=agent%20device%2F1&transport=quic&timeoutMs=2500&minDecodedFrames=2&minFps=5&stopOnComplete=false"
        );
    }

    #[test]
    fn lan_e2e_autorun_requires_enabled_flag() {
        let disabled = lan_e2e_autorun_config_from_env_lookup(|_| None);
        assert!(disabled.is_none());

        let enabled = lan_e2e_autorun_config_from_env_lookup(|key| {
            if key == LAN_E2E_AUTORUN_ENV {
                Some("true".to_string())
            } else {
                None
            }
        });
        assert!(enabled.is_some());
    }
}

fn main() {
    let settings_path = default_settings_path();
    let _settings = load_settings(&settings_path).unwrap_or_else(|error| {
        eprintln!("failed to load app settings: {error}");
        AppSettings::default()
    });

    // Create shared service manager (bootstrap-only in Phase 6)
    let service_manager = std::sync::Arc::new(std::sync::Mutex::new(
        service_manager::ServiceManager::new().expect("failed to create ServiceManager"),
    ));

    // Create test harness for end-to-end pipeline visualization
    let test_harness = std::sync::Arc::new(std::sync::Mutex::new(
        test_harness::TestHarness::new().expect("failed to create TestHarness"),
    ));

    // Create test orchestrator
    let test_orchestrator = std::sync::Arc::new(test_orchestrator::TestOrchestrator::new(
        test_harness.clone(),
    ));
    let resource_monitor = std::sync::Arc::new(std::sync::Mutex::new(ResourceMonitor::new()));
    let render_window_registry =
        std::sync::Arc::new(std::sync::Mutex::new(RenderWindowRegistry::default()));
    let remote_display_surfaces =
        std::sync::Arc::new(std::sync::Mutex::new(RemoteDisplaySurfaceManager::default()));
    let webrtc_host =
        std::sync::Arc::new(tokio::sync::Mutex::new(webrtc_host::WebrtcHost::default()));

    let Some(single_instance_listener) = claim_single_instance() else {
        return;
    };

    // Build the app
    tauri::Builder::default()
        .manage(AppState {
            settings_path,
            service_manager,
            test_harness,
            test_orchestrator,
            resource_monitor,
            render_window_registry,
            remote_display_surfaces,
            webrtc_host,
        })
        .on_menu_event(|app, event| {
            if let Some(action) = tray_action_from_menu_id(event.id().as_ref()) {
                if let Err(error) = apply_tray_action(app, action) {
                    eprintln!("tray menu action failed: {error}");
                }
            }
        })
        .on_tray_icon_event(|app, event| {
            if event.id().as_ref() != TRAY_ICON_ID {
                return;
            }

            if let Some(action) = tray_action_from_icon_event(&event) {
                if let Err(error) = apply_tray_action(app, action) {
                    eprintln!("tray icon action failed: {error}");
                }
            }
        })
        .setup(move |app| {
            spawn_single_instance_listener(single_instance_listener, app.handle().clone());
            setup_system_tray(app.handle())?;

            if let Some(main_window) = app.get_webview_window("main") {
                let backdrop_status = platform::configure_main_window(&main_window);
                if !backdrop_status.applied {
                    eprintln!(
                        "failed to apply native backdrop: {}",
                        backdrop_status.detail
                    );
                }
                if let Some(config) = lan_e2e_autorun_config_from_env() {
                    let route = build_lan_e2e_autorun_route(config);
                    if let Err(error) = navigate_main_window_to_route(&main_window, &route) {
                        eprintln!("failed to navigate to LAN E2E autorun route: {error}");
                    }
                }

                let app_handle_for_close = app.handle().clone();
                main_window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        if APP_IS_QUITTING.load(Ordering::SeqCst) {
                            return;
                        }
                        api.prevent_close();
                        if let Err(error) = hide_main_window(&app_handle_for_close) {
                            eprintln!("failed to hide Rdesk window: {error}");
                        }
                    }
                });
            }

            // Step 1: Bootstrap mrd-service if not already running
            let service_mgr = app.state::<AppState>().service_manager.clone();

            // Spawn a blocking task for service bootstrap
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    // Bootstrap mrd-service only if not reachable via IPC
                    let bootstrapped = match service_mgr.lock().unwrap().bootstrap_if_needed().await
                    {
                        Ok(did_bootstrap) => did_bootstrap,
                        Err(e) => {
                            eprintln!("Failed to bootstrap mrd-service: {}", e);
                            return;
                        }
                    };

                    if bootstrapped {
                        println!("Bootstrapped mrd-service");
                    } else {
                        println!("mrd-service already running");
                    }

                    // Wait for service to be healthy (max 30 seconds)
                    if let Err(e) = service_mgr.lock().unwrap().wait_for_healthy(30).await {
                        eprintln!("mrd-service health check failed: {}", e);
                    } else {
                        println!("mrd-service is ready");

                        // Register UI presence with service
                        if let Err(e) = shell_ui_attached().await {
                            eprintln!("Failed to register UI presence: {}", e);
                        }
                    }
                });
            });

            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // Hardware and decode policy (local-only)
            get_hardware_info,
            get_system_resource_snapshot,
            start_drag_window,
            minimize_window,
            toggle_maximize_window,
            hide_to_tray,
            show_window,
            center_window,
            close_window,
            set_window_decorations,
            apply_native_chrome,
            open_remote_display_window,
            list_remote_display_windows,
            current_remote_display_window_context,
            close_remote_display_window,
            browser_webrtc_preview_start,
            browser_webrtc_preview_stop,
            configure_remote_display_native_surface,
            present_test_harness_frame_on_native_surface,
            get_client_diagnostics,
            open_diagnostics_folder,
            automation_write_report,
            nvdec_runtime_probe,
            decode_policy,
            set_decode_policy,
            // Bootstrap commands (Phase 6: bootstrap-only behavior)
            service_bootstrap_if_needed,
            service_wait_for_healthy,
            service_did_bootstrap,
            // IPC-based commands (all session control goes through mrd-service)
            ipc_register_device,
            ipc_list_devices,
            ipc_lan_discovery_snapshot,
            ipc_refresh_lan_discovery,
            ipc_list_sessions,
            ipc_start_session,
            ipc_start_lan_remote_session,
            ipc_update_media_profile,
            ipc_list_remote_capture_sources,
            ipc_select_remote_capture_source,
            ipc_accept_session,
            ipc_stop_session,
            ipc_fail_session,
            ipc_recover_session,
            ipc_session_snapshot,
            ipc_runtime_snapshot,
            ipc_capability_snapshot,
            ipc_service_health,
            ipc_probe_snapshot,
            ipc_start_sender,
            ipc_start_receiver,
            // Legacy commands
            register_device,
            check_device_registration,
            webrtc_session_list_via_ipc,
            // Shell / Lifecycle commands (Phase 2-6: service owns lifecycle)
            shell_ui_attached,
            shell_ui_detached,
            shell_get_status,
            shell_shutdown_service,
            shell_quit_ui_and_stop_service,
            // Test harness commands
            test_harness_start,
            test_harness_stop,
            test_harness_set_chain,
            test_harness_set_custom,
            test_harness_get_chain,
            test_harness_get_metrics,
            test_harness_get_comparison_result,
            test_harness_get_frames,
            // Test Workbench commands (new unified test API)
            test_list_scenarios,
            test_get_capabilities,
            test_list_window_capture_targets,
            test_list_window_capture_targets_with_previews,
            test_start_run,
            test_stop_run,
            test_list_runs,
            test_get_run,
            test_get_run_events,
            test_get_run_metrics,
            test_get_run_artifacts,
            test_list_presets,
            test_save_preset,
            test_delete_preset,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
