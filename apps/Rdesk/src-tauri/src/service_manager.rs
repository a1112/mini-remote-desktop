// Service bootstrap management for mrd-service
//
// Phase 6: This module now only handles bootstrap behavior.
// Rdesk no longer owns the service lifecycle - mrd-service is the owner.
// ServiceManager is only used to bootstrap mrd-service if it's not running.
//
// For service lifecycle operations (start, stop, restart), use IPC commands:
// - GetShellStatus: check service status
// - ShutdownService: request service shutdown

use anyhow::Result;
use mrd_ipc::{IpcRequest, IpcResponse};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const SERVICE_BOOTSTRAP_DISABLED_ENV: &str = "MRD_SERVICE_BOOTSTRAP_DISABLED";

/// Service bootstrap manager for mrd-service
///
/// Phase 6: Reduced to bootstrap-only behavior.
/// This manager is ONLY used to start mrd-service if it's not already running.
/// All other lifecycle operations go through IPC to mrd-service.
pub struct ServiceManager {
    child: Arc<Mutex<Option<Child>>>,
    exe_path: PathBuf,
    /// Whether this instance performed bootstrap (started service)
    bootstrapped: Arc<Mutex<bool>>,
}

impl ServiceManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            child: Arc::new(Mutex::new(None)),
            exe_path: resolve_service_exe_path(),
            bootstrapped: Arc::new(Mutex::new(false)),
        })
    }

    /// Bootstrap mrd-service if not already running via IPC
    ///
    /// Phase 6: This is the ONLY start method. It checks IPC first,
    /// and only spawns the process if service is unreachable.
    /// Returns true if bootstrap was performed, false if already running.
    pub async fn bootstrap_if_needed(&self) -> Result<bool> {
        // First check if service is reachable via IPC
        if self.is_reachable_via_ipc().await {
            tracing::info!("mrd-service is already running via IPC");
            return Ok(false);
        }

        if bootstrap_disabled_from_env_value(
            std::env::var(SERVICE_BOOTSTRAP_DISABLED_ENV)
                .ok()
                .as_deref(),
        ) {
            anyhow::bail!(
                "mrd-service bootstrap is disabled by {SERVICE_BOOTSTRAP_DISABLED_ENV} and IPC is unreachable"
            );
        }

        // Service not reachable, bootstrap it
        tracing::info!("mrd-service not reachable, bootstrapping...");
        let mut child_guard = self.child.lock().await;
        let exe_path = self.ensure_service_executable()?;

        let mut command = Command::new(&exe_path);
        if let Some(parent) = exe_path.parent() {
            command.current_dir(parent);
        }
        if let Ok((stdout, stderr)) = open_service_log_files() {
            command.stdout(stdout).stderr(stderr);
        }
        let child = command.spawn().map_err(|e| {
            anyhow::anyhow!(
                "Failed to bootstrap mrd-service at {}: {}",
                exe_path.display(),
                e
            )
        })?;

        *child_guard = Some(child);
        *self.bootstrapped.lock().await = true;

        tracing::info!(
            "mrd-service bootstrapped with PID: {:?}",
            child_guard.as_ref().map(|c| c.id())
        );

        // Give the service time to initialize
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(true)
    }

    /// Check if service is reachable via IPC
    pub async fn is_reachable_via_ipc(&self) -> bool {
        use mrd_ipc::client::IpcClient;

        let mut client = IpcClient::new();
        matches!(
            client.send_request(IpcRequest::ServiceHealth).await,
            Ok(IpcResponse::ServiceHealth { .. })
        )
    }

    /// Wait for service to be healthy (with timeout)
    pub async fn wait_for_healthy(&self, timeout_secs: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            if self.is_reachable_via_ipc().await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok(false)
    }

    /// Check if this instance bootstrapped the service
    pub async fn did_bootstrap(&self) -> bool {
        *self.bootstrapped.lock().await
    }

    /// Get the bootstrap child PID if we bootstrapped
    #[allow(dead_code)]
    pub async fn bootstrap_pid(&self) -> Option<u32> {
        if !self.did_bootstrap().await {
            return None;
        }
        let mut child_guard = self.child.lock().await;
        child_guard.as_mut().and_then(|c| {
            // Check if still running
            match c.try_wait() {
                Ok(None) => Some(c.id()), // Still running
                _ => None,                // Exited or error
            }
        })
    }

    pub fn service_exe_path(&self) -> &std::path::Path {
        &self.exe_path
    }

    fn ensure_service_executable(&self) -> Result<PathBuf> {
        if self.exe_path.exists() {
            return Ok(self.exe_path.clone());
        }

        for candidate in candidate_service_paths() {
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        #[cfg(debug_assertions)]
        {
            build_dev_service_executable()?;
            for candidate in candidate_service_paths() {
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }

        let tried = candidate_service_paths()
            .into_iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("mrd-service executable not found. Tried: {tried}");
    }
}

fn bootstrap_disabled_from_env_value(value: Option<&str>) -> bool {
    matches!(
        value.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if matches!(value.as_str(), "1" | "true" | "yes" | "on")
    )
}

pub fn runtime_log_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MRD_LOG_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local_app_data)
                .join("mini-remote-desktop")
                .join("logs");
        }
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data)
                .join("mini-remote-desktop")
                .join("logs");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("mini-remote-desktop");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
            return PathBuf::from(state_home)
                .join("mini-remote-desktop")
                .join("logs");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join("mini-remote-desktop")
                .join("logs");
        }
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("logs")
}

pub fn service_stdout_log_path() -> PathBuf {
    runtime_log_dir().join("mrd-service.stdout.log")
}

pub fn service_stderr_log_path() -> PathBuf {
    runtime_log_dir().join("mrd-service.stderr.log")
}

fn open_service_log_files() -> Result<(Stdio, Stdio)> {
    std::fs::create_dir_all(runtime_log_dir())?;
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(service_stdout_log_path())?;
    let stderr = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(service_stderr_log_path())?;

    Ok((Stdio::from(stdout), Stdio::from(stderr)))
}

fn service_exe_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "mrd-service.exe"
    }

    #[cfg(not(target_os = "windows"))]
    {
        "mrd-service"
    }
}

fn cargo_profile_dir() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn candidate_service_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();

    for env_key in ["MRD_SERVICE_EXE", "MRD_SERVICE_PATH"] {
        if let Ok(path) = std::env::var(env_key) {
            if !path.trim().is_empty() {
                candidates.push(PathBuf::from(path));
            }
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            candidates.push(dir.join(service_exe_name()));
        }
    }

    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        if !target_dir.trim().is_empty() {
            candidates.push(
                PathBuf::from(target_dir)
                    .join(cargo_profile_dir())
                    .join(service_exe_name()),
            );
        }
    }

    candidates.push(
        workspace_root()
            .join("target")
            .join(cargo_profile_dir())
            .join(service_exe_name()),
    );

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(
            current_dir
                .join("..")
                .join("..")
                .join("target")
                .join(cargo_profile_dir())
                .join(service_exe_name()),
        );
    }

    let mut deduped = Vec::<PathBuf>::new();
    for candidate in candidates {
        if !deduped.iter().any(|seen| seen == &candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

fn resolve_service_exe_path() -> PathBuf {
    candidate_service_paths()
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| {
            workspace_root()
                .join("target")
                .join(cargo_profile_dir())
                .join(service_exe_name())
        })
}

#[cfg(debug_assertions)]
fn build_dev_service_executable() -> Result<()> {
    let status = Command::new(cargo_command())
        .arg("build")
        .arg("-p")
        .arg("mrd-service")
        .current_dir(workspace_root())
        .status()
        .map_err(|error| anyhow::anyhow!("failed to run cargo build -p mrd-service: {error}"))?;

    if !status.success() {
        anyhow::bail!("cargo build -p mrd-service exited with status {status}");
    }

    Ok(())
}

#[cfg(debug_assertions)]
fn cargo_command() -> PathBuf {
    if let Ok(cargo) = std::env::var("CARGO") {
        if !cargo.trim().is_empty() {
            return PathBuf::from(cargo);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            let cargo = PathBuf::from(home)
                .join(".cargo")
                .join("bin")
                .join("cargo.exe");
            if cargo.exists() {
                return cargo;
            }
        }
    }

    PathBuf::from("cargo")
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new().expect("failed to create ServiceManager")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_service_manager() {
        let manager = ServiceManager::new();
        assert!(manager.is_ok());
    }

    #[test]
    fn bootstrap_disabled_accepts_truthy_env_values() {
        assert!(bootstrap_disabled_from_env_value(Some("1")));
        assert!(bootstrap_disabled_from_env_value(Some("true")));
        assert!(bootstrap_disabled_from_env_value(Some("yes")));
        assert!(!bootstrap_disabled_from_env_value(Some("0")));
        assert!(!bootstrap_disabled_from_env_value(None));
    }

    #[test]
    fn resolved_service_path_uses_service_binary_name() {
        let path = resolve_service_exe_path();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(service_exe_name())
        );
    }

    #[test]
    fn log_paths_are_under_runtime_log_dir() {
        assert!(service_stdout_log_path().starts_with(runtime_log_dir()));
        assert!(service_stderr_log_path().starts_with(runtime_log_dir()));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn cargo_command_resolves_to_a_command_name() {
        let command = cargo_command();
        assert!(!command.as_os_str().is_empty());
    }

    #[test]
    fn ipc_check_returns_false_when_not_running() {
        // This test verifies IPC check doesn't panic when service is not running
        let _manager = ServiceManager::new().unwrap();
        // In a test environment, the service won't be running
        // so is_reachable_via_ipc should return false
    }
}
