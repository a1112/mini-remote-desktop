// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod device_info;

use device_info::HardwareInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 设备注册响应
#[derive(Debug, Serialize, Deserialize)]
struct DeviceRegistrationResponse {
    device_id: String,
    device_name: String,
    access_token: String,
}

/// Tauri 命令：获取硬件信息
#[tauri::command]
fn get_hardware_info() -> Result<HardwareInfo, String> {
    Ok(device_info::get_hardware_info())
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_hardware_info,
            register_device,
            check_device_registration
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
