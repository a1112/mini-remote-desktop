// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_settings;
mod ipc_client;
mod service_manager;
mod device_info;
mod render_window_registry;

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

// mrd-service lifecycle commands
#[tauri::command]
async fn service_start(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().start().await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())??;

    Ok(true)
}

#[tauri::command]
async fn service_stop(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let manager = state.service_manager.clone();

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            manager.lock().unwrap().stop().await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())??;

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
    }).await.map_err(|e| e.to_string())??;

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
            manager.lock().unwrap().restart().await.map_err(|e| e.to_string())
        })
    }).await.map_err(|e| e.to_string())??;

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

fn main() {
    let settings_path = default_settings_path();
    let settings = load_settings(&settings_path).unwrap_or_else(|error| {
        eprintln!("failed to load app settings: {error}");
        AppSettings::default()
    });

    // Create shared service manager
    let service_manager = std::sync::Arc::new(std::sync::Mutex::new(
        service_manager::ServiceManager::new()
            .expect("failed to create ServiceManager")
    ));

    tauri::Builder::default()
        .manage(AppState {
            settings_path,
            service_manager,
        })
        .invoke_handler(tauri::generate_handler![
            // Hardware and decode policy (local-only)
            get_hardware_info,
            nvdec_runtime_probe,
            decode_policy,
            set_decode_policy,
            // Service lifecycle commands
            service_start,
            service_stop,
            service_status,
            service_health_check,
            service_wait_for_healthy,
            service_restart_with_backoff,
            service_pid,
            service_start_guard,
            // IPC-based commands (all session control goes through mrd-service)
            ipc_register_device,
            ipc_list_devices,
            ipc_start_session,
            ipc_accept_session,
            ipc_stop_session,
            ipc_session_snapshot,
            ipc_start_sender,
            ipc_start_receiver,
            // Legacy commands
            register_device,
            check_device_registration,
            webrtc_session_list_via_ipc,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
