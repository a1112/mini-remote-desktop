use crate::{
    app_state::AppState,
    shell::{AutostartPortRef, UiLaunchRequest, UiLaunchResult, UiLauncherPortRef},
};
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
    let active_session_count = crate::shutdown::active_session_count(app_state).await;
    let shell = app_state.shell.lock().await;
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
    let result = (|| -> anyhow::Result<Option<bool>> {
        let autostart = autostart
            .lock()
            .map_err(|_| anyhow::anyhow!("Autostart configuration lock is unavailable"))?;
        if !autostart.is_supported() {
            return Ok(None);
        }
        autostart.set_enabled(enabled)?;
        Ok(Some(autostart.is_enabled()?))
    })();

    match result {
        Ok(Some(actual)) => {
            app_state.shell.lock().await.autostart_enabled = Some(actual);
            if actual == enabled {
                IpcResponse::Ack
            } else {
                IpcResponse::Error {
                    code: "E500".to_string(),
                    message: "Autostart configuration does not match the requested state"
                        .to_string(),
                }
            }
        }
        Ok(None) => {
            app_state.shell.lock().await.autostart_enabled = None;
            IpcResponse::Error {
                code: "E501".to_string(),
                message: "Autostart is not supported; install the background service first"
                    .to_string(),
            }
        }
        Err(error) => {
            app_state.shell.lock().await.autostart_enabled = None;
            tracing::error!("SetAutostart failed: {error:#}");
            IpcResponse::Error {
                code: "E500".to_string(),
                message: format!("{error:#}"),
            }
        }
    }
}

/// Return the current autostart state from the configured platform port.
pub fn autostart_status(autostart: &AutostartPortRef) -> IpcResponse {
    match read_autostart_state(autostart) {
        Ok(Some(enabled)) => IpcResponse::AutostartStatus {
            enabled,
            supported: true,
        },
        Ok(None) => IpcResponse::AutostartStatus {
            enabled: false,
            supported: false,
        },
        Err(error) => IpcResponse::Error {
            code: "E500".to_string(),
            message: format!("{error:#}"),
        },
    }
}

fn read_autostart_state(autostart: &AutostartPortRef) -> anyhow::Result<Option<bool>> {
    let autostart = autostart
        .lock()
        .map_err(|_| anyhow::anyhow!("Autostart configuration lock is unavailable"))?;
    if autostart.is_supported() {
        autostart.is_enabled().map(Some)
    } else {
        Ok(None)
    }
}

/// Refresh the shell snapshot from platform configuration instead of UI state.
pub async fn refresh_autostart_state(
    app_state: &Arc<AppState>,
    autostart: &AutostartPortRef,
) -> anyhow::Result<Option<bool>> {
    let result = read_autostart_state(autostart);
    app_state.shell.lock().await.autostart_enabled = result.as_ref().ok().copied().flatten();
    result
}

/// Queue a runtime-owned shutdown; the local connection flushes Ack before exit.
pub fn shutdown_service(app_state: &Arc<AppState>, mode: ShutdownMode) -> IpcResponse {
    tracing::info!("ShutdownService requested: mode={:?}", mode);
    match app_state.shutdown.request(mode) {
        Ok(()) => IpcResponse::Ack,
        Err(error) => IpcResponse::Error {
            code: "E_SERVICE_SHUTDOWN_UNAVAILABLE".to_string(),
            message: error.to_string(),
        },
    }
}
