// Shell abstraction for mrd-service
//
// Provides platform-agnostic ports for:
// - UI launch/focus (UiLauncherPort)
// - Tray management (TrayPort) - Phase 4
// - Autostart management (AutostartPort) - Phase 5

use std::path::PathBuf;

// Platform-specific modules
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod windows;

// ============================================================================
// UI Launcher (Phase 3)
// ============================================================================

/// Result of UI launch operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiLaunchResult {
    /// Successfully focused existing UI
    FocusedExisting { pid: u32 },
    /// Successfully spawned new UI
    SpawnedNew { pid: u32 },
    /// UI unavailable (no configured path)
    Unavailable,
    /// Launch failed
    Failed { error: String },
}

/// Request to open/focus the UI
#[derive(Debug, Clone)]
pub struct UiLaunchRequest {
    pub reason: String, // "tray_open", "session_incoming", "user_request", "diagnostics"
}

/// Port for UI launcher operations
///
/// Abstracts platform-specific UI launch and focus mechanisms.
/// - Windows: CreateProcessW / focus window
/// - macOS: open -a / Apple Events
/// - Linux: .desktop entry / D-Bus focus
pub trait UiLauncherPort: Send + Sync {
    /// Check if UI is currently running
    fn is_ui_running(&self) -> anyhow::Result<bool>;

    /// Get the PID of the running UI, if any
    fn get_ui_pid(&self) -> anyhow::Result<Option<u32>>;

    /// Launch or focus the UI
    fn launch_or_focus(&self, request: UiLaunchRequest) -> anyhow::Result<UiLaunchResult>;

    /// Set the UI executable path
    fn set_ui_path(&self, path: PathBuf) -> anyhow::Result<()>;

    /// Get the UI executable path
    fn get_ui_path(&self) -> anyhow::Result<Option<PathBuf>>;
}

/// In-memory UI launcher (for development/testing)
///
/// Tracks UI state in memory without actual process launching.
/// Used for Phase 2/3 development before platform-specific implementation.
pub struct InMemoryUiLauncher {
    ui_pid: std::sync::Arc<std::sync::Mutex<Option<u32>>>,
    ui_path: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>>,
}

impl InMemoryUiLauncher {
    pub fn new() -> Self {
        Self {
            ui_pid: std::sync::Arc::new(std::sync::Mutex::new(None)),
            ui_path: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Simulate UI attaching (for testing)
    pub fn simulate_attach(&self, pid: u32) {
        *self.ui_pid.lock().unwrap() = Some(pid);
    }

    /// Simulate UI detaching (for testing)
    pub fn simulate_detach(&self) {
        *self.ui_pid.lock().unwrap() = None;
    }
}

impl Default for InMemoryUiLauncher {
    fn default() -> Self {
        Self::new()
    }
}

impl UiLauncherPort for InMemoryUiLauncher {
    fn is_ui_running(&self) -> anyhow::Result<bool> {
        Ok(self.ui_pid.lock().unwrap().is_some())
    }

    fn get_ui_pid(&self) -> anyhow::Result<Option<u32>> {
        Ok(*self.ui_pid.lock().unwrap())
    }

    fn launch_or_focus(&self, _request: UiLaunchRequest) -> anyhow::Result<UiLaunchResult> {
        let pid_guard = self.ui_pid.lock().unwrap();

        if let Some(pid) = *pid_guard {
            // Simulate focusing existing UI
            Ok(UiLaunchResult::FocusedExisting { pid })
        } else {
            // Simulate spawning new UI
            drop(pid_guard);
            let new_pid = std::process::id(); // Use current PID as placeholder
            *self.ui_pid.lock().unwrap() = Some(new_pid);
            Ok(UiLaunchResult::SpawnedNew { pid: new_pid })
        }
    }

    fn set_ui_path(&self, path: PathBuf) -> anyhow::Result<()> {
        *self.ui_path.lock().unwrap() = Some(path);
        Ok(())
    }

    fn get_ui_path(&self) -> anyhow::Result<Option<PathBuf>> {
        Ok(self.ui_path.lock().unwrap().clone())
    }
}

pub type UiLauncherPortRef = std::sync::Arc<std::sync::Mutex<dyn UiLauncherPort + Send + Sync>>;

pub fn default_ui_launcher() -> UiLauncherPortRef {
    #[cfg(target_os = "macos")]
    {
        return std::sync::Arc::new(std::sync::Mutex::new(macos::MacosUiLauncher::new("Rdesk")));
    }

    #[cfg(not(target_os = "macos"))]
    {
        std::sync::Arc::new(std::sync::Mutex::new(InMemoryUiLauncher::new()))
    }
}

// ============================================================================
// Tray Management (Phase 4)
// ============================================================================

/// Tray menu item
#[derive(Debug, Clone)]
pub struct TrayMenuItem {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
}

/// Tray action events
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// Open/focus Rdesk UI
    OpenUi,
    /// Show service status
    ShowStatus,
    /// Show active sessions
    ShowSessions,
    /// Stop all active sessions
    StopSessions,
    /// Restart service
    RestartService,
    /// Quit service
    QuitService,
    /// Open diagnostics
    OpenDiagnostics,
    /// Custom action with ID
    Custom(String),
}

/// Tray model - data-driven tray state
#[derive(Debug, Clone)]
pub struct TrayModel {
    /// Service status text
    pub status_text: String,
    /// Active session count
    pub session_count: usize,
    /// Is service healthy
    pub is_healthy: bool,
    /// Menu items
    pub menu_items: Vec<TrayMenuItem>,
    /// Tooltip text
    pub tooltip: String,
}

impl Default for TrayModel {
    fn default() -> Self {
        Self {
            status_text: "Ready".to_string(),
            session_count: 0,
            is_healthy: true,
            menu_items: vec![
                TrayMenuItem {
                    id: "open".to_string(),
                    label: "Open Rdesk".to_string(),
                    enabled: true,
                    separator: false,
                },
                TrayMenuItem {
                    id: "status".to_string(),
                    label: "Status: Ready".to_string(),
                    enabled: false,
                    separator: false,
                },
                TrayMenuItem {
                    id: "sessions".to_string(),
                    label: "Sessions (0)".to_string(),
                    enabled: false,
                    separator: false,
                },
                TrayMenuItem {
                    id: "separator1".to_string(),
                    label: "".to_string(),
                    enabled: false,
                    separator: true,
                },
                TrayMenuItem {
                    id: "restart".to_string(),
                    label: "Restart Service".to_string(),
                    enabled: true,
                    separator: false,
                },
                TrayMenuItem {
                    id: "quit".to_string(),
                    label: "Quit Service".to_string(),
                    enabled: true,
                    separator: false,
                },
            ],
            tooltip: "mrd-service".to_string(),
        }
    }
}

/// Port for tray operations
///
/// Abstracts platform-specific tray implementations.
/// - Windows: Win32 Shell_NotifyIcon
/// - macOS: NSStatusItem / NSMenu
/// - Linux: StatusNotifierItem / AppIndicator (where available)
pub trait TrayPort: Send + Sync {
    /// Install the tray with initial model
    fn install(&self, model: TrayModel) -> anyhow::Result<()>;

    /// Update the tray model
    fn update(&self, model: TrayModel) -> anyhow::Result<()>;

    /// Show a notification/balloon tooltip
    fn show_notification(&self, title: &str, message: &str) -> anyhow::Result<()>;

    /// Shutdown and remove the tray
    fn shutdown(&self) -> anyhow::Result<()>;

    /// Check if tray is available on this platform
    fn is_available(&self) -> bool;
}

/// In-memory/no-op tray implementation
///
/// Used for platforms without tray support or for testing.
pub struct NoOpTray {
    available: bool,
}

impl NoOpTray {
    pub fn new() -> Self {
        Self { available: false }
    }

    pub fn with_availability(available: bool) -> Self {
        Self { available }
    }
}

impl Default for NoOpTray {
    fn default() -> Self {
        Self::new()
    }
}

impl TrayPort for NoOpTray {
    fn install(&self, _model: TrayModel) -> anyhow::Result<()> {
        if self.available {
            tracing::warn!("Tray install called but NoOpTray has no implementation");
        }
        Ok(())
    }

    fn update(&self, _model: TrayModel) -> anyhow::Result<()> {
        Ok(())
    }

    fn show_notification(&self, _title: &str, _message: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        self.available
    }
}

pub fn default_tray() -> std::sync::Arc<std::sync::Mutex<dyn TrayPort + Send + Sync>> {
    #[cfg(windows)]
    {
        return std::sync::Arc::new(std::sync::Mutex::new(windows::WindowsTray::new()));
    }

    #[cfg(target_os = "macos")]
    {
        return std::sync::Arc::new(std::sync::Mutex::new(macos::MacosTray::new()));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::sync::Arc::new(std::sync::Mutex::new(NoOpTray::with_availability(false)))
    }
}

/// Build a tray model from current state
pub fn build_tray_model(
    is_healthy: bool,
    session_count: usize,
    last_error: Option<&str>,
) -> TrayModel {
    let status_text = if let Some(error) = last_error {
        format!("Error: {}", error)
    } else if is_healthy {
        "Ready".to_string()
    } else {
        "Starting".to_string()
    };

    let tooltip = if session_count > 0 {
        format!("mrd-service - {} session(s)", session_count)
    } else {
        "mrd-service".to_string()
    };

    TrayModel {
        status_text: status_text.clone(),
        session_count,
        is_healthy,
        menu_items: vec![
            TrayMenuItem {
                id: "open".to_string(),
                label: "Open Rdesk".to_string(),
                enabled: true,
                separator: false,
            },
            TrayMenuItem {
                id: "status".to_string(),
                label: format!("Status: {}", status_text),
                enabled: false,
                separator: false,
            },
            TrayMenuItem {
                id: "sessions".to_string(),
                label: format!("Sessions ({})", session_count),
                enabled: false,
                separator: false,
            },
            TrayMenuItem {
                id: "separator1".to_string(),
                label: "".to_string(),
                enabled: false,
                separator: true,
            },
            TrayMenuItem {
                id: "restart".to_string(),
                label: "Restart Service".to_string(),
                enabled: true,
                separator: false,
            },
            TrayMenuItem {
                id: "quit".to_string(),
                label: "Quit Service".to_string(),
                enabled: true,
                separator: false,
            },
        ],
        tooltip,
    }
}

// ============================================================================
// Autostart Management (Phase 5)
// ============================================================================

/// Port for autostart operations
///
/// Abstracts platform-specific autostart mechanisms.
/// - Windows: Registry Run key (HKCU\Software\Microsoft\Windows\CurrentVersion\Run)
/// - macOS: LaunchAgent or login item
/// - Linux: XDG autostart desktop entry or systemd user service
pub trait AutostartPort: Send + Sync {
    /// Check if autostart is enabled
    fn is_enabled(&self) -> anyhow::Result<bool>;

    /// Set autostart enabled state
    fn set_enabled(&self, enabled: bool) -> anyhow::Result<()>;

    /// Check if autostart is supported on this platform
    fn is_supported(&self) -> bool;

    /// Get the autostart entry name/identifier
    fn get_entry_name(&self) -> &str;
}

/// Windows autostart implementation using Registry
#[cfg(windows)]
pub struct WindowsAutostart {
    entry_name: String,
    executable_path: Option<PathBuf>,
}

#[cfg(windows)]
impl WindowsAutostart {
    pub fn new(entry_name: impl Into<String>) -> Self {
        Self {
            entry_name: entry_name.into(),
            executable_path: None,
        }
    }

    pub fn with_path(entry_name: impl Into<String>, executable_path: PathBuf) -> Self {
        Self {
            entry_name: entry_name.into(),
            executable_path: Some(executable_path),
        }
    }

    /// Get the Run key path for the current user
    fn get_run_key_path() -> String {
        r"Software\Microsoft\Windows\CurrentVersion\Run".to_string()
    }
}

#[cfg(windows)]
impl AutostartPort for WindowsAutostart {
    fn is_enabled(&self) -> anyhow::Result<bool> {
        // Phase 5: Placeholder - would read from Windows Registry
        // using winreg crate or win32 API
        tracing::info!("WindowsAutostart::is_enabled - placeholder");
        Ok(false)
    }

    fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        tracing::info!("WindowsAutostart::set_enabled: {} - placeholder", enabled);
        // Phase 5: Placeholder - would write to Windows Registry
        // HKCU\Software\Microsoft\Windows\CurrentVersion\Run
        Ok(())
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn get_entry_name(&self) -> &str {
        &self.entry_name
    }
}

/// No-op autostart implementation for platforms without support
pub struct NoOpAutostart {
    entry_name: String,
}

impl NoOpAutostart {
    pub fn new(entry_name: impl Into<String>) -> Self {
        Self {
            entry_name: entry_name.into(),
        }
    }
}

impl AutostartPort for NoOpAutostart {
    fn is_enabled(&self) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        tracing::warn!("Autostart not supported, attempted to set: {}", enabled);
        Err(anyhow::anyhow!("Autostart not supported on this platform"))
    }

    fn is_supported(&self) -> bool {
        false
    }

    fn get_entry_name(&self) -> &str {
        &self.entry_name
    }
}

pub type AutostartPortRef = std::sync::Arc<std::sync::Mutex<dyn AutostartPort + Send + Sync>>;

pub fn default_autostart(entry_name: impl Into<String>) -> AutostartPortRef {
    let entry_name = entry_name.into();

    #[cfg(windows)]
    {
        return std::sync::Arc::new(std::sync::Mutex::new(WindowsAutostart::new(entry_name)));
    }

    #[cfg(target_os = "macos")]
    {
        return std::sync::Arc::new(std::sync::Mutex::new(
            macos::MacosAutostart::for_current_exe(entry_name),
        ));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::sync::Arc::new(std::sync::Mutex::new(NoOpAutostart::new(entry_name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_launcher_tracks_ui_state() {
        let launcher = InMemoryUiLauncher::new();

        assert!(!launcher.is_ui_running().unwrap());
        assert_eq!(launcher.get_ui_pid().unwrap(), None);

        launcher.simulate_attach(12345);
        assert!(launcher.is_ui_running().unwrap());
        assert_eq!(launcher.get_ui_pid().unwrap(), Some(12345));

        launcher.simulate_detach();
        assert!(!launcher.is_ui_running().unwrap());
    }

    #[test]
    fn in_memory_launcher_persists_path() {
        let launcher = InMemoryUiLauncher::new();
        let path = PathBuf::from("/path/to/Rdesk");

        launcher.set_ui_path(path.clone()).unwrap();
        assert_eq!(launcher.get_ui_path().unwrap(), Some(path));
    }

    #[test]
    fn in_memory_launcher_launch_or_focus() {
        let launcher = InMemoryUiLauncher::new();

        // First launch should spawn new
        let result = launcher
            .launch_or_focus(UiLaunchRequest {
                reason: "test".to_string(),
            })
            .unwrap();
        assert!(matches!(result, UiLaunchResult::SpawnedNew { .. }));

        // Second launch should focus existing
        let result = launcher
            .launch_or_focus(UiLaunchRequest {
                reason: "test".to_string(),
            })
            .unwrap();
        assert!(matches!(result, UiLaunchResult::FocusedExisting { .. }));
    }
}
