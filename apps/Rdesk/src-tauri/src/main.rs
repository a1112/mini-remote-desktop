// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod device_info;
mod realtime_client;
mod realtime_management;
mod realtime_runtime;

use device_info::HardwareInfo;
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_client::encode_message;
use mrd_signal_proto::SignalMessage;
use realtime_management::{RealtimeManagementClient, RealtimeStatus};
use realtime_runtime::{RealtimeRegistration, RealtimeRuntime};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone)]
struct AppState {
    realtime_runtime: RealtimeRuntime,
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

fn main() {
    tauri::Builder::default()
        .manage(AppState {
            realtime_runtime: RealtimeRuntime::from_env(),
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
            realtime_drain_events
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        drain_realtime_events_with, realtime_accept_session_with, realtime_register_with,
        realtime_request_session_with,
    };
    use crate::realtime_runtime::RealtimeRuntime;
    use axum::{
        extract::ws::{Message, WebSocket, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use futures_util::StreamExt;
    use mrd_signal_client::{decode_message, encode_message};
    use mrd_proto::DeviceId;
    use mrd_signal_proto::SignalMessage;
    use tokio::net::TcpListener;

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
}
