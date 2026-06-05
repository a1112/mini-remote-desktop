use crate::{
    app_state::AppState,
    shell::{AutostartPortRef, UiLaunchRequest, UiLaunchResult, UiLauncherPortRef},
};
use mrd_application::ports::SessionLifecycleState;
use mrd_ipc::{IpcResponse, OpenUiReason, ShutdownMode, UiDetachReason, UiOpenStatus};
use std::{path::PathBuf, sync::Arc};

/// Launch or focus the UI shell.
pub fn open_ui(ui_launcher: &UiLauncherPortRef, reason: OpenUiReason) -> IpcResponse {
    tracing::info!("OpenUi requested: reason={:?}", reason);
    let launcher = ui_launcher.lock().unwrap();
    let request = UiLaunchRequest {
        reason: format!("{:?}", reason),
    };
    match launcher.launch_or_focus(request) {
        Ok(UiLaunchResult::FocusedExisting { pid }) => {
            tracing::info!("Focused existing UI: pid={}", pid);
            IpcResponse::UiOpenResult {
                status: UiOpenStatus::FocusedExisting,
                pid: Some(pid),
            }
        }
        Ok(UiLaunchResult::SpawnedNew { pid }) => {
            tracing::info!("Spawned new UI: pid={}", pid);
            IpcResponse::UiOpenResult {
                status: UiOpenStatus::SpawnedNew,
                pid: Some(pid),
            }
        }
        Ok(UiLaunchResult::Unavailable) => {
            tracing::warn!("UI launch unavailable - no configured path");
            IpcResponse::UiOpenResult {
                status: UiOpenStatus::Unavailable,
                pid: None,
            }
        }
        Ok(UiLaunchResult::Failed { error }) => {
            tracing::error!("UI launch failed: {}", error);
            IpcResponse::Error {
                code: "E500".to_string(),
                message: error,
            }
        }
        Err(error) => {
            tracing::error!("UI launch error: {}", error);
            IpcResponse::Error {
                code: "E500".to_string(),
                message: error.to_string(),
            }
        }
    }
}

/// Focus the UI shell, launching it if the launcher supports that behavior.
pub fn focus_ui(ui_launcher: &UiLauncherPortRef) -> IpcResponse {
    tracing::info!("FocusUi requested");
    let launcher = ui_launcher.lock().unwrap();
    let request = UiLaunchRequest {
        reason: "focus".to_string(),
    };
    match launcher.launch_or_focus(request) {
        Ok(UiLaunchResult::FocusedExisting { .. }) => IpcResponse::Ack,
        Ok(UiLaunchResult::SpawnedNew { .. }) => IpcResponse::Ack,
        Ok(UiLaunchResult::Unavailable) => IpcResponse::Error {
            code: "E404".to_string(),
            message: "UI not available".to_string(),
        },
        Ok(UiLaunchResult::Failed { error }) => IpcResponse::Error {
            code: "E500".to_string(),
            message: error,
        },
        Err(error) => IpcResponse::Error {
            code: "E500".to_string(),
            message: error.to_string(),
        },
    }
}

/// Record UI process attachment and persist its executable path for future launch.
pub async fn ui_attached(
    app_state: &Arc<AppState>,
    ui_launcher: &UiLauncherPortRef,
    pid: u32,
    executable_path: Option<String>,
) -> IpcResponse {
    tracing::info!("UI attached: pid={} path={:?}", pid, executable_path);
    let mut shell = app_state.shell.lock().await;
    shell.ui_pid = Some(pid);
    shell.ui_executable_path = executable_path.clone();
    shell.last_error = None;
    drop(shell);

    if let Some(path) = executable_path {
        let launcher = ui_launcher.lock().unwrap();
        let _ = launcher.set_ui_path(PathBuf::from(path));
    }

    IpcResponse::Ack
}

/// Record UI process detachment, clearing the tracked PID only when it matches.
pub async fn ui_detached(
    app_state: &Arc<AppState>,
    pid: u32,
    reason: UiDetachReason,
) -> IpcResponse {
    tracing::info!("UI detached: pid={} reason={:?}", pid, reason);
    let mut shell = app_state.shell.lock().await;
    if shell.ui_pid == Some(pid) {
        shell.ui_pid = None;
    }
    IpcResponse::Ack
}

/// Return current service shell status.
pub async fn shell_status(app_state: &Arc<AppState>) -> IpcResponse {
    let shell = app_state.shell.lock().await;
    let sessions = app_state.sessions.lock().await;
    let active_session_count = sessions
        .list_all()
        .into_iter()
        .filter(|session| session.lifecycle_state != SessionLifecycleState::Closed)
        .count();
    IpcResponse::ShellStatus {
        status: mrd_ipc::ShellStatusSnapshot {
            service_pid: std::process::id(),
            ui_pid: shell.ui_pid,
            tray_available: shell.tray_available,
            autostart_enabled: shell.autostart_enabled,
            active_session_count,
            last_error: shell.last_error.clone(),
        },
    }
}

/// Enable or disable service autostart through the configured platform port.
pub async fn set_autostart(
    app_state: &Arc<AppState>,
    autostart: &AutostartPortRef,
    enabled: bool,
) -> IpcResponse {
    tracing::info!("SetAutostart: enabled={}", enabled);
    let result = {
        let autostart = autostart.lock().unwrap();
        let supported = autostart.is_supported();
        let set_result = autostart.set_enabled(enabled);
        (supported, set_result)
    };

    match result {
        (supported, Ok(())) => {
            let mut shell = app_state.shell.lock().await;
            shell.autostart_enabled = if supported { Some(enabled) } else { None };
            IpcResponse::Ack
        }
        (_supported, Err(error)) => {
            tracing::error!("SetAutostart failed: {}", error);
            IpcResponse::Error {
                code: "E500".to_string(),
                message: error.to_string(),
            }
        }
    }
}

/// Return the current autostart state from the configured platform port.
pub fn autostart_status(autostart: &AutostartPortRef) -> IpcResponse {
    let autostart = autostart.lock().unwrap();
    let enabled = autostart.is_enabled().unwrap_or(false);
    let supported = autostart.is_supported();
    IpcResponse::AutostartStatus { enabled, supported }
}

/// Acknowledge shutdown requests with the current not-implemented contract.
pub fn shutdown_service(mode: ShutdownMode) -> IpcResponse {
    tracing::info!("ShutdownService requested: mode={:?}", mode);
    match mode {
        ShutdownMode::Force => IpcResponse::Error {
            code: "E501".to_string(),
            message: "Force shutdown not yet implemented".to_string(),
        },
        ShutdownMode::Graceful | ShutdownMode::AfterSessions => IpcResponse::Error {
            code: "E501".to_string(),
            message: "Service shutdown not yet implemented".to_string(),
        },
    }
}
