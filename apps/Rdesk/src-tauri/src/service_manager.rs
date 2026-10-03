// Service bootstrap management for mrd-service
//
// Starts the installed service or a development fallback and confirms its exit.
// Session cleanup and shutdown remain owned by mrd-service through IPC.
//
// For service lifecycle operations (start, stop, restart), use IPC commands:
// - GetShellStatus: check service status
// - ShutdownService: request service shutdown

use anyhow::{Context, Result};
use mrd_ipc::{IpcRequest, IpcResponse, ShutdownMode};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const SERVICE_BOOTSTRAP_DISABLED_ENV: &str = "MRD_SERVICE_BOOTSTRAP_DISABLED";

/// Bootstraps mrd-service and observes the actual process lifetime.
/// Shutdown requests go through IPC so the service owns session cleanup.
pub struct ServiceManager {
    child: Arc<Mutex<Option<Child>>>,
    observed_process: Arc<Mutex<Option<ObservedServiceProcess>>>,
    management_endpoint: mrd_ipc::transport::IpcEndpoint,
    exe_path: PathBuf,
    /// Whether this instance performed bootstrap (started service)
    bootstrapped: Arc<Mutex<bool>>,
}

impl ServiceManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            child: Arc::new(Mutex::new(None)),
            observed_process: Arc::new(Mutex::new(None)),
            management_endpoint: mrd_ipc::transport::IpcEndpoint::management_from_env_or_default(),
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
        if self.probe_service_health().await?.is_some() {
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

        #[cfg(windows)]
        if uses_default_service_endpoint(&self.management_endpoint) {
            if let Some(started) =
                tokio::task::spawn_blocking(start_installed_windows_service).await??
            {
                *self.bootstrapped.lock().await = started;
                return Ok(started);
            }
        }

        if let Some(child) = child_guard.as_mut() {
            if child
                .try_wait()
                .context("Cannot inspect the existing background service process")?
                .is_none()
            {
                return Ok(false);
            }
        }
        *child_guard = None;

        let exe_path = self.ensure_service_executable()?;

        let mut command = Command::new(&exe_path);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
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
    #[allow(dead_code)]
    pub async fn is_reachable_via_ipc(&self) -> bool {
        self.probe_service_health().await.ok().flatten().is_some()
    }

    /// None means no endpoint; Some(false) means a present service is not ready yet.
    async fn probe_service_health(&self) -> Result<Option<bool>> {
        let mut client = one_attempt_ipc_client(&self.management_endpoint);
        match tokio::time::timeout(
            Duration::from_secs(2),
            client.send_request(IpcRequest::ServiceHealth),
        )
        .await
        {
            Ok(Ok(IpcResponse::ServiceHealth { status })) => {
                if let Some(pid) = status.pid {
                    self.observe_service_process(pid).await;
                }
                Ok(Some(health_status_is_ready(&status)))
            }
            Ok(Ok(IpcResponse::Error { code, message })) => anyhow::bail!("{code}: {message}"),
            Ok(Ok(_)) => anyhow::bail!("Unexpected service health response"),
            Ok(Err(error)) if ipc_endpoint_is_unavailable(&error) => Ok(None),
            Ok(Err(error)) if ipc_endpoint_is_busy(&error) => Ok(Some(false)),
            Ok(Err(error)) => Err(error.context("Cannot inspect background service health")),
            Err(_) => anyhow::bail!("Background service health request timed out"),
        }
    }

    /// Wait for service to be healthy (with timeout)
    pub async fn wait_for_healthy(&self, timeout_secs: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            if self.probe_service_health().await? == Some(true) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok(false)
    }

    /// Wait for actual service termination, not just acceptance of a shutdown request.
    pub async fn wait_for_stopped(&self, timeout_secs: u64) -> Result<bool> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        loop {
            if self.is_confirmed_stopped().await? {
                return Ok(true);
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn is_confirmed_stopped(&self) -> Result<bool> {
        #[allow(unused_mut)]
        let mut scm_stopped = false;
        #[cfg(windows)]
        if uses_default_service_endpoint(&self.management_endpoint) {
            if let Some(stopped) =
                tokio::task::spawn_blocking(installed_windows_service_is_stopped).await??
            {
                if !stopped {
                    return Ok(false);
                }
                scm_stopped = true;
            }
        }

        if !ipc_endpoint_is_gone(&self.management_endpoint).await? {
            return Ok(false);
        }
        self.confirm_process_exit(scm_stopped).await
    }

    async fn observe_service_process(&self, pid: u32) {
        let mut observation = self.observed_process.lock().await;
        if let Some(previous) = observation.as_ref() {
            if previous.pid == pid
                && previous
                    .watcher
                    .as_ref()
                    .is_ok_and(|watcher| matches!(watcher.has_exited(), Ok(false)))
            {
                return;
            }
        }
        *observation = Some(ObservedServiceProcess {
            pid,
            watcher: ServiceProcessWatcher::open(pid).map_err(|error| format!("{error:#}")),
        });
    }

    /// A missing endpoint alone does not prove that a console process finished cleanup.
    async fn confirm_process_exit(&self, scm_stopped: bool) -> Result<bool> {
        if let Some(exited) = owned_child_has_exited(&mut *self.child.lock().await)? {
            return Ok(exited);
        }
        match self.observed_process.lock().await.as_ref() {
            Some(ObservedServiceProcess {
                watcher: Ok(watcher),
                ..
            }) => watcher.has_exited(),
            Some(ObservedServiceProcess {
                watcher: Err(_), ..
            }) if scm_stopped => Ok(true),
            Some(ObservedServiceProcess {
                watcher: Err(error),
                ..
            }) => anyhow::bail!("Cannot confirm background process termination: {error}"),
            None if scm_stopped => Ok(true),
            None => anyhow::bail!(
                "Cannot confirm background process termination without its process identity"
            ),
        }
    }

    pub async fn request_shutdown(&self, mode: ShutdownMode) -> Result<()> {
        if self.probe_service_health().await?.is_none() {
            anyhow::bail!("Background service is unavailable");
        }
        let mut client = one_attempt_ipc_client(&self.management_endpoint);
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            client.send_request(IpcRequest::ShutdownService { mode }),
        )
        .await
        .context("Background service shutdown request timed out")??;
        validate_shutdown_response(response)
    }

    pub async fn shutdown_and_wait(&self, mode: ShutdownMode, timeout_secs: u64) -> Result<()> {
        self.request_shutdown(mode).await?;
        if !self.wait_for_stopped(timeout_secs).await? {
            anyhow::bail!("后台服务未在规定时间内停止，窗口保持打开");
        }
        Ok(())
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

fn one_attempt_ipc_client(
    endpoint: &mrd_ipc::transport::IpcEndpoint,
) -> mrd_ipc::client::IpcClient {
    let mut client = mrd_ipc::client::IpcClient::management_with_endpoint(endpoint.clone());
    client.set_reconnect_config(mrd_ipc::client::ReconnectConfig {
        enabled: false,
        ..Default::default()
    });
    client
}

fn owned_child_has_exited(child: &mut Option<Child>) -> Result<Option<bool>> {
    child
        .as_mut()
        .map(|child| {
            child
                .try_wait()
                .map(|status| status.is_some())
                .context("Cannot inspect the background service process")
        })
        .transpose()
}

struct ObservedServiceProcess {
    pid: u32,
    watcher: std::result::Result<ServiceProcessWatcher, String>,
}

struct ServiceProcessWatcher {
    #[cfg(windows)]
    handle: Option<windows::Win32::Foundation::HANDLE>,
    #[cfg(unix)]
    pid: sysinfo::Pid,
    #[cfg(unix)]
    started_at: Option<u64>,
}

impl ServiceProcessWatcher {
    fn open(pid: u32) -> Result<Self> {
        if pid == 0 {
            anyhow::bail!("Background service reported an invalid process identifier");
        }
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            };
            match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
                Ok(handle) => Ok(Self {
                    handle: Some(handle),
                }),
                Err(error) if error.code().0 as u32 == 0x80070057 => Ok(Self { handle: None }),
                Err(error) => Err(anyhow::Error::new(error)
                    .context("Cannot observe background service process exit")),
            }
        }
        #[cfg(unix)]
        {
            let pid = sysinfo::Pid::from_u32(pid);
            let mut system = sysinfo::System::new();
            system.refresh_process(pid);
            Ok(Self {
                pid,
                started_at: system.process(pid).map(|process| process.start_time()),
            })
        }
    }

    fn has_exited(&self) -> Result<bool> {
        #[cfg(windows)]
        {
            use windows::Win32::{Foundation::STILL_ACTIVE, System::Threading::GetExitCodeProcess};
            let Some(handle) = self.handle else {
                return Ok(true);
            };
            let mut exit_code = 0;
            unsafe { GetExitCodeProcess(handle, &mut exit_code) }
                .context("Cannot observe background service process exit")?;
            // The service exits with 0 or 1. Retaining this handle prevents PID reuse races.
            Ok(exit_code != STILL_ACTIVE.0 as u32)
        }
        #[cfg(unix)]
        {
            let Some(started_at) = self.started_at else {
                return Ok(true);
            };
            let mut system = sysinfo::System::new();
            system.refresh_process(self.pid);
            Ok(system.process(self.pid).map(|process| process.start_time()) != Some(started_at))
        }
    }
}

#[cfg(windows)]
impl Drop for ServiceProcessWatcher {
    fn drop(&mut self) {
        if let Some(handle) = self.handle {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
        }
    }
}

fn validate_shutdown_response(response: IpcResponse) -> Result<()> {
    match response {
        IpcResponse::Ack => Ok(()),
        IpcResponse::Error { code, message } => anyhow::bail!("{code}: {message}"),
        _ => anyhow::bail!("Unexpected service shutdown response"),
    }
}

fn health_status_is_ready(status: &mrd_ipc::ServiceStatus) -> bool {
    status.running && status.healthy
}

fn ipc_endpoint_is_unavailable(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
        .any(|error| {
            matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
        })
}

async fn ipc_endpoint_is_gone(endpoint: &mrd_ipc::transport::IpcEndpoint) -> Result<bool> {
    match tokio::time::timeout(
        Duration::from_secs(2),
        mrd_ipc::transport::IpcClient::connect_management_with_endpoint(endpoint),
    )
    .await
    {
        Ok(Ok(_stream)) => Ok(false),
        Ok(Err(error)) if ipc_endpoint_is_unavailable(&error) => Ok(true),
        Ok(Err(error)) if ipc_endpoint_is_busy(&error) => Ok(false),
        Ok(Err(error)) => Err(error.context("Cannot confirm background service termination")),
        Err(_) => anyhow::bail!("Background service termination probe timed out"),
    }
}

fn ipc_endpoint_is_busy(error: &anyhow::Error) -> bool {
    #[cfg(windows)]
    {
        error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
            .any(|error| error.raw_os_error() == Some(231)) // ERROR_PIPE_BUSY: endpoint still exists.
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

#[cfg(windows)]
fn uses_default_service_endpoint(endpoint: &mrd_ipc::transport::IpcEndpoint) -> bool {
    *endpoint
        == mrd_ipc::transport::IpcEndpoint::management_for_service(
            mrd_ipc::transport::IpcEndpoint::default_service(),
        )
}

#[cfg(windows)]
const WINDOWS_SERVICE_NAME: &str = "MiniRemoteDesktop";

#[cfg(windows)]
fn is_scm_service_absent(error: &windows_service::Error) -> bool {
    matches!(error, windows_service::Error::Winapi(error) if error.raw_os_error() == Some(1060))
}

#[cfg(windows)]
fn installed_windows_service_is_stopped() -> Result<Option<bool>> {
    use windows_service::{
        service::{ServiceAccess, ServiceState},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("Cannot open Windows service manager")?;
    let service = match manager.open_service(WINDOWS_SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Ok(service) => service,
        Err(error) if is_scm_service_absent(&error) => return Ok(None),
        Err(error) => {
            return Err(
                anyhow::Error::new(error).context("Cannot query installed background service")
            )
        }
    };
    Ok(Some(
        service
            .query_status()
            .context("Cannot query installed background service status")?
            .current_state
            == ServiceState::Stopped,
    ))
}

#[cfg(windows)]
fn start_installed_windows_service() -> Result<Option<bool>> {
    use windows_service::{
        service::{ServiceAccess, ServiceState},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context("Cannot open Windows service manager")?;
    let service =
        match manager.open_service(WINDOWS_SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Ok(service) => service,
            Err(error) if is_scm_service_absent(&error) => return Ok(None),
            Err(error) => return Err(anyhow::Error::new(error).context(
                "Cannot inspect installed background service; console bootstrap was not attempted",
            )),
        };
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut started = false;
    loop {
        match service
            .query_status()
            .context("Cannot query installed background service status")?
            .current_state
        {
            ServiceState::Running => return Ok(Some(started)),
            ServiceState::Stopped if !started => {
                let start_service = manager.open_service(WINDOWS_SERVICE_NAME, ServiceAccess::START)
                    .context("Cannot start installed background service; run the service installer as administrator to update its permissions")?;
                match start_service.start::<&str>(&[]) {
                    Ok(()) => started = true,
                    Err(windows_service::Error::Winapi(error))
                        if error.raw_os_error() == Some(1056) => {}
                    Err(error) => {
                        return Err(anyhow::Error::new(error)
                            .context("Installed background service failed to start"))
                    }
                }
            }
            ServiceState::Stopped => {
                anyhow::bail!("Installed background service stopped during startup")
            }
            ServiceState::StartPending | ServiceState::StopPending => {}
            state => anyhow::bail!(
                "Installed background service cannot be started while in state {state:?}"
            ),
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("Installed background service startup timed out");
        }
        std::thread::sleep(Duration::from_millis(200));
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
            #[cfg(target_os = "macos")]
            {
                if let Some(contents_dir) = dir.parent() {
                    candidates.push(
                        contents_dir
                            .join("Resources")
                            .join("MrdService.app")
                            .join("Contents")
                            .join("MacOS")
                            .join(service_exe_name()),
                    );
                }
                candidates.push(
                    dir.join("MrdService.app")
                        .join("Contents")
                        .join("MacOS")
                        .join(service_exe_name()),
                );
            }
            candidates.push(dir.join(service_exe_name()));
        }
    }

    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        if !target_dir.trim().is_empty() {
            #[cfg(target_os = "macos")]
            candidates.push(
                PathBuf::from(&target_dir)
                    .join(cargo_profile_dir())
                    .join("MrdService.app")
                    .join("Contents")
                    .join("MacOS")
                    .join(service_exe_name()),
            );
            candidates.push(
                PathBuf::from(target_dir)
                    .join(cargo_profile_dir())
                    .join(service_exe_name()),
            );
        }
    }

    #[cfg(target_os = "macos")]
    candidates.push(
        workspace_root()
            .join("target")
            .join(cargo_profile_dir())
            .join("MrdService.app")
            .join("Contents")
            .join("MacOS")
            .join(service_exe_name()),
    );
    candidates.push(
        workspace_root()
            .join("target")
            .join(cargo_profile_dir())
            .join(service_exe_name()),
    );

    if let Ok(current_dir) = std::env::current_dir() {
        #[cfg(target_os = "macos")]
        candidates.push(
            current_dir
                .join("..")
                .join("..")
                .join("target")
                .join(cargo_profile_dir())
                .join("MrdService.app")
                .join("Contents")
                .join("MacOS")
                .join(service_exe_name()),
        );
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

    struct TestChildGuard(Arc<Mutex<Option<Child>>>);

    impl Drop for TestChildGuard {
        fn drop(&mut self) {
            if let Ok(mut guard) = self.0.try_lock() {
                if let Some(child) = guard.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }

    fn sleeping_test_child() -> Child {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 10",
                ])
                .creation_flags(0x0800_0000)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        }
        #[cfg(unix)]
        {
            Command::new("sleep")
                .arg("10")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        }
    }

    fn isolated_absent_endpoint() -> mrd_ipc::transport::IpcEndpoint {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        #[cfg(windows)]
        {
            mrd_ipc::transport::IpcEndpoint::named_pipe(format!(
                r"\\.\pipe\mrd-owned-exit-test-{suffix}"
            ))
        }
        #[cfg(unix)]
        {
            mrd_ipc::transport::IpcEndpoint::unix_socket(
                std::env::temp_dir()
                    .join(format!("mrd-owned-exit-test-{suffix}.sock"))
                    .to_string_lossy(),
            )
        }
    }

    #[tokio::test]
    async fn a_missing_pipe_does_not_confirm_exit_of_an_owned_service_process() {
        let mut manager = ServiceManager::new().unwrap();
        manager.management_endpoint = isolated_absent_endpoint();
        let _cleanup = TestChildGuard(manager.child.clone());
        *manager.child.lock().await = Some(sleeping_test_child());
        assert!(!manager.wait_for_stopped(0).await.unwrap());
        {
            let mut guard = manager.child.lock().await;
            let child = guard.as_mut().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(manager.wait_for_stopped(0).await.unwrap());
    }

    #[tokio::test]
    async fn a_missing_pipe_does_not_confirm_exit_of_a_preexisting_console_process() {
        let mut manager = ServiceManager::new().unwrap();
        manager.management_endpoint = isolated_absent_endpoint();
        let fixture = Arc::new(Mutex::new(Some(sleeping_test_child())));
        let _cleanup = TestChildGuard(fixture.clone());
        let pid = fixture.lock().await.as_ref().unwrap().id();
        manager.observe_service_process(pid).await;
        assert!(!manager.wait_for_stopped(0).await.unwrap());
        {
            let mut guard = fixture.lock().await;
            let child = guard.as_mut().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(manager.wait_for_stopped(0).await.unwrap());
    }

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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_service_bundle_candidates_precede_raw_binaries() {
        let candidates = candidate_service_paths();
        assert!(candidates.windows(2).any(|pair| {
            pair[0]
                .components()
                .any(|component| component.as_os_str() == "MrdService.app")
                && !pair[1]
                    .components()
                    .any(|component| component.as_os_str() == "MrdService.app")
                && pair[0].file_name() == pair[1].file_name()
        }));
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

    #[test]
    fn only_missing_or_refused_endpoints_confirm_service_absence() {
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::ConnectionRefused,
        ] {
            let error = anyhow::Error::from(std::io::Error::from(kind));
            assert!(ipc_endpoint_is_unavailable(&error));
        }
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::InvalidData,
        ] {
            let error = anyhow::Error::from(std::io::Error::from(kind));
            assert!(!ipc_endpoint_is_unavailable(&error));
        }
        assert!(!ipc_endpoint_is_unavailable(&anyhow::anyhow!(
            "invalid IPC response"
        )));
    }

    #[test]
    fn shutdown_rejection_and_unexpected_responses_remain_errors() {
        assert!(validate_shutdown_response(IpcResponse::Ack).is_ok());
        let rejected = validate_shutdown_response(IpcResponse::Error {
            code: "E503".to_string(),
            message: "Shutdown runtime is unavailable".to_string(),
        })
        .unwrap_err();
        assert!(rejected.to_string().contains("503"));
        assert!(rejected
            .to_string()
            .contains("Shutdown runtime is unavailable"));
        assert!(validate_shutdown_response(IpcResponse::DeviceList { devices: vec![] }).is_err());
    }

    #[test]
    fn a_health_response_does_not_claim_readiness_for_an_unhealthy_service() {
        for (running, healthy, ready) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
            (false, false, false),
        ] {
            assert_eq!(
                health_status_is_ready(&mrd_ipc::ServiceStatus {
                    running,
                    healthy,
                    pid: Some(123)
                }),
                ready
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_an_uninstalled_scm_service_allows_console_fallback() {
        assert!(is_scm_service_absent(&windows_service::Error::Winapi(
            std::io::Error::from_raw_os_error(1060)
        )));
        for code in [5, 1053, 1072] {
            assert!(!is_scm_service_absent(&windows_service::Error::Winapi(
                std::io::Error::from_raw_os_error(code)
            )));
        }
    }

    #[tokio::test]
    async fn termination_probe_waits_for_the_actual_endpoint_to_disappear() {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        #[cfg(windows)]
        let endpoint = mrd_ipc::transport::IpcEndpoint::named_pipe(format!(
            r"\\.\pipe\mrd-ui-stop-test-{suffix}"
        ));
        #[cfg(unix)]
        let endpoint = mrd_ipc::transport::IpcEndpoint::unix_socket(
            std::env::temp_dir()
                .join(format!("mrd-ui-stop-test-{suffix}.sock"))
                .to_string_lossy(),
        );
        assert!(ipc_endpoint_is_gone(&endpoint).await.unwrap());
        let server = mrd_ipc::transport::IpcServer::bind_with_endpoint(endpoint.clone())
            .await
            .unwrap();
        let server_task = tokio::spawn(async move {
            loop {
                let _stream = server.accept().await.unwrap();
            }
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while ipc_endpoint_is_gone(&endpoint).await.unwrap() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "fixture endpoint did not start"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        server_task.abort();
        let _ = server_task.await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while !ipc_endpoint_is_gone(&endpoint).await.unwrap() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "fixture endpoint remained after shutdown"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        #[cfg(unix)]
        if let mrd_ipc::transport::IpcEndpoint::UnixSocket(path) = endpoint {
            std::fs::remove_file(path).unwrap();
        }
    }
}
