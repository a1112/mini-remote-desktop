use std::sync::Arc;

/// Shell state - tracks UI presence and service lifecycle
#[derive(Debug, Default)]
pub struct ShellState {
    /// UI process PID if attached
    pub ui_pid: Option<u32>,
    /// UI executable path for relaunch
    pub ui_executable_path: Option<String>,
    /// Tray availability (platform-dependent)
    pub tray_available: bool,
    /// Autostart enabled state (None if not supported)
    pub autostart_enabled: Option<bool>,
    /// Active session count (for tray display)
    pub active_session_count: usize,
    /// Last error message
    pub last_error: Option<String>,
}

/// Tray port - abstracts platform-specific tray implementation
pub type TrayPortRef = Arc<std::sync::Mutex<dyn crate::shell::TrayPort + Send + Sync>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shell_state_starts_detached_and_idle() {
        let state = ShellState::default();

        assert_eq!(state.ui_pid, None);
        assert_eq!(state.ui_executable_path, None);
        assert!(!state.tray_available);
        assert_eq!(state.autostart_enabled, None);
        assert_eq!(state.active_session_count, 0);
        assert_eq!(state.last_error, None);
    }
}
