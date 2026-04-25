// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_settings;
mod ipc_client;
mod service_manager;
mod device_info;
mod render_window_registry;
mod test_harness;
mod test_orchestrator;

use app_settings::{
    default_settings_path, load_settings, save_settings, AppSettings, DecodePolicy,
};
use device_info::HardwareInfo;
use mrd_ipc;
use mrd_proto::DeviceId;
use render_window_registry::RenderWindowContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::Manager;

#[derive(Clone)]
struct AppState {
    settings_path: std::path::PathBuf,
    // Service lifecycle manager - controls mrd-service
    service_manager: std::sync::Arc<std::sync::Mutex<service_manager::ServiceManager>>,
    // Test harness for end-to-end pipeline visualization
    test_harness: std::sync::Arc<std::sync::Mutex<test_harness::TestHarness>>,
    // Test orchestrator - unified test execution and management
    test_orchestrator: std::sync::Arc<test_orchestrator::TestOrchestrator>,
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

/// Tauri 命令：获取硬件信息
#[tauri::command]
fn get_hardware_info() -> Result<HardwareInfo, String> {
    Ok(device_info::get_hardware_info())
}

// nvdec_runtime_probe - moved to rdesk-legacy-harness package
#[tauri::command]
fn nvdec_runtime_probe() -> Result<serde_json::Value, String> {
    Err("nvdec_runtime_probe moved to mrd-service - use rdesk-legacy-harness for testing".to_string())
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
    set_decode_policy_with(
        &state.settings_path,
        decode_policy,
    )
    .await
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
            manager.lock().unwrap().bootstrap_if_needed().await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())?
}

/// Wait for service to be healthy (with timeout)
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

/// Check if this instance bootstrapped the service
#[tauri::command]
async fn service_did_bootstrap(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            Ok(manager.lock().unwrap().did_bootstrap().await)
        })
    }).await.map_err(|e| e.to_string())?
}

// ============================================================================
// Shell / Lifecycle Commands (Phase 2)
// ============================================================================

/// Register UI presence with mrd-service
#[tauri::command]
async fn shell_ui_attached() -> Result<(), String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::UiAttached {
        pid: std::process::id(),
        executable_path: std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(String::from)),
    }).await.map_err(|e| e.to_string())?;

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
    let response = client.send_request(IpcRequest::UiDetached {
        pid: std::process::id(),
        reason: detach_reason,
    }).await.map_err(|e| e.to_string())?;

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
    let response = client.send_request(IpcRequest::GetShellStatus).await.map_err(|e| e.to_string())?;

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
    let response = client.send_request(IpcRequest::ShutdownService { mode: shutdown_mode }).await.map_err(|e| e.to_string())?;

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
async fn shell_quit_ui_and_stop_service(
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    // Notify service that UI is detaching
    let _ = shell_ui_detached("user_quit".to_string()).await;

    // Stop all active sessions via IPC
    use mrd_ipc::{IpcRequest, IpcResponse};
    let mut client = mrd_ipc::client::IpcClient::new();
    if let Ok(IpcResponse::SessionList { sessions }) =
        client.send_request(IpcRequest::ListSessions).await
    {
        for session_info in sessions {
            let _ = client.send_request(IpcRequest::StopSession {
                session_id: session_info.session_id,
            }).await;
        }
    }

    // Request service shutdown via IPC (Phase 6: service owns lifecycle)
    let _ = client.send_request(IpcRequest::ShutdownService {
        mode: mrd_ipc::ShutdownMode::Graceful,
    }).await;

    // Exit the UI application
    app_handle.exit(0);
    Ok(())
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

/// List sessions via IPC (migrated version)
#[tauri::command]
async fn ipc_list_sessions() -> Result<Vec<mrd_ipc::SessionInfo>, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::ListSessions).await.map_err(|e| e.to_string())?;

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
    let response = client.send_request(IpcRequest::RuntimeSnapshot).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::RuntimeSnapshot { snapshot } => Ok(snapshot),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Service health check via IPC (migrated version)
#[tauri::command]
async fn ipc_service_health() -> Result<mrd_ipc::ServiceStatus, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::ServiceHealth).await.map_err(|e| e.to_string())?;

    match response {
        IpcResponse::ServiceHealth { status } => Ok(status),
        IpcResponse::Error { code, message } => Err(format!("{}: {}", code, message)),
        _ => Err("Unexpected response".to_string()),
    }
}

/// Get probe snapshot via IPC (migrated version)
#[tauri::command]
async fn ipc_probe_snapshot(
    session_id: String,
) -> Result<mrd_ipc::ProbeSnapshot, String> {
    use mrd_ipc::{IpcRequest, IpcResponse};
    use mrd_proto::SessionId;

    let mut client = mrd_ipc::client::IpcClient::new();
    let response = client.send_request(IpcRequest::ProbeSnapshot {
        session_id: SessionId(session_id),
    }).await.map_err(|e| e.to_string())?;

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
    state.test_harness.lock().unwrap().start().map_err(|e| e.to_string())
}

#[tauri::command]
fn test_harness_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.test_harness.lock().unwrap().stop().map_err(|e| e.to_string())
}

#[tauri::command]
fn test_harness_set_chain(state: tauri::State<'_, AppState>, chain: String) -> Result<(), String> {
    use test_harness::TestChain;
    let parsed = match chain.as_str() {
        "nvenc_nvdec" => TestChain::NvencNvdec,
        "nvenc_only" => TestChain::NvencOnly,
        "openh264" => TestChain::OpenH264,
        _ => return Err(format!("未知的测试链路: {}", chain)),
    };
    state.test_harness.lock().unwrap().set_chain(parsed);
    Ok(())
}

// TODO: Add test_harness_set_matrix command for custom configurations
// #[tauri::command]
// fn test_harness_set_matrix(
//     state: tauri::State<'_, AppState>,
//     config: test_harness::MatrixConfig,
// ) -> Result<(), String> {
//     let chain = test_harness::TestChain::Custom {
//         capture: config.capture,
//         encoder: config.encoder,
//         decoder: config.decoder,
//     };
//     state.test_harness.lock().unwrap().set_chain(chain);
//     Ok(())
// }

#[tauri::command]
fn test_harness_get_chain(state: tauri::State<'_, AppState>) -> String {
    use test_harness::TestChain;
    match state.test_harness.lock().unwrap().get_chain() {
        TestChain::NvencNvdec => "nvenc_nvdec".to_string(),
        TestChain::NvencOnly => "nvenc_only".to_string(),
        TestChain::OpenH264 => "openh264".to_string(),
        TestChain::Custom { .. } => "custom".to_string(),
    }
}

#[tauri::command]
fn test_harness_get_metrics(state: tauri::State<'_, AppState>) -> test_harness::HarnessMetrics {
    state.test_harness.lock().unwrap().get_metrics()
}

#[tauri::command]
fn test_harness_get_frames(state: tauri::State<'_, AppState>) -> (
    Option<(String, usize, usize)>,
    Option<(String, usize, usize)>,
) {
    let (captured, rendered) = state.test_harness.lock().unwrap().get_latest_frames();

    let captured_base64 = captured.and_then(|(data, width, height)| {
        encode_bgra_png_base64(&data, width, height).map(|png| (png, width, height))
    });

    let rendered_base64 = rendered.and_then(|(data, width, height)| {
        encode_bgra_png_base64(&data, width, height).map(|png| (png, width, height))
    });

    (captured_base64, rendered_base64)
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
            0, 0, 255, 255,
            0, 255, 0, 255,
            255, 0, 0, 255,
            255, 255, 255, 255,
        ];

        let encoded = encode_bgra_png_base64(&bgra, 2, 2).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();

        assert_eq!(&decoded[..8], b"\x89PNG\r\n\x1a\n");
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
fn test_get_capabilities(state: tauri::State<'_, AppState>) -> Result<test_orchestrator::EnvironmentSnapshot, String> {
    state.test_orchestrator.get_capabilities().map_err(|e| e.to_string())
}

/// Start a test run
#[tauri::command]
fn test_start_run(
    state: tauri::State<'_, AppState>,
    scenario_id: String,
    config: test_orchestrator::TestConfigData,
) -> Result<String, String> {
    state.test_orchestrator.start_run(scenario_id, config)
        .map_err(|e| e.to_string())
}

/// Stop a test run
#[tauri::command]
fn test_stop_run(state: tauri::State<'_, AppState>, run_id: String) -> Result<(), String> {
    state.test_orchestrator.stop_run(&run_id)
        .map_err(|e| e.to_string())
}

/// List test runs
#[tauri::command]
fn test_list_runs(
    state: tauri::State<'_, AppState>,
    scenario_id: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
) -> Vec<test_orchestrator::TestRun> {
    state.test_orchestrator.list_runs(scenario_id, status, limit)
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
    state.test_orchestrator.save_preset(name, description, scenario_id, config)
}

/// Delete a test preset
#[tauri::command]
fn test_delete_preset(state: tauri::State<'_, AppState>, preset_id: String) -> Result<(), String> {
    state.test_orchestrator.delete_preset(&preset_id)
        .map_err(|e| e.to_string())
}

fn main() {
    let settings_path = default_settings_path();
    let _settings = load_settings(&settings_path).unwrap_or_else(|error| {
        eprintln!("failed to load app settings: {error}");
        AppSettings::default()
    });

    // Create shared service manager (bootstrap-only in Phase 6)
    let service_manager = std::sync::Arc::new(std::sync::Mutex::new(
        service_manager::ServiceManager::new()
            .expect("failed to create ServiceManager")
    ));

    // Create test harness for end-to-end pipeline visualization
    let test_harness = std::sync::Arc::new(std::sync::Mutex::new(
        test_harness::TestHarness::new()
            .expect("failed to create TestHarness")
    ));

    // Create test orchestrator
    let test_orchestrator = std::sync::Arc::new(test_orchestrator::TestOrchestrator::new(test_harness.clone()));

    // Build the app
    tauri::Builder::default()
        .manage(AppState {
            settings_path,
            service_manager,
            test_harness,
            test_orchestrator,
        })
        .setup(|app| {
            // Phase 6: Tray is now owned by mrd-service, not Rdesk
            // Step 1: Bootstrap mrd-service if not already running
            let service_mgr = app.state::<AppState>().service_manager.clone();

            // Spawn a blocking task for service bootstrap
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async move {
                    // Bootstrap mrd-service only if not reachable via IPC
                    let bootstrapped = match service_mgr.lock().unwrap().bootstrap_if_needed().await {
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

            // Step 2: Set up window close event listener
            // Phase 6: Normal close does NOT stop service - service continues running
            // Use shell_shutdown_service IPC command for service shutdown
            let main_window = app.get_webview_window("main").unwrap();
            let app_handle_for_close = app.handle().clone();

            main_window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { .. } = event {
                    // Notify service of UI detachment, but do NOT stop service
                    println!("Window close requested - detaching UI (service stays running)...");

                    let handle = app_handle_for_close.clone();

                    // Spawn task to notify service and exit UI only
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async move {
                            // Notify service that UI is detaching
                            let _ = shell_ui_detached("user_close".to_string()).await;
                        });

                        // Exit the UI only (service continues running)
                        handle.exit(0);
                    });
                }
            });

            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // Hardware and decode policy (local-only)
            get_hardware_info,
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
            ipc_list_sessions,
            ipc_start_session,
            ipc_accept_session,
            ipc_stop_session,
            ipc_session_snapshot,
            ipc_runtime_snapshot,
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
            test_harness_get_chain,
            test_harness_get_metrics,
            test_harness_get_frames,
            // Test Workbench commands (new unified test API)
            test_list_scenarios,
            test_get_capabilities,
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
