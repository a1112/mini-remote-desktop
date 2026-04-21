// Windows tray implementation (placeholder)
//
// This module provides a Windows-specific tray implementation.
// Currently a placeholder that logs - full implementation would use
// Win32 Shell_NotifyIcon directly via FFI.

use super::{TrayPort, TrayModel};

/// Windows tray implementation (placeholder)
///
/// Full implementation would use Win32 API:
/// - Shell_NotifyIconW for icon management
/// - CreateWindowExW for message handling
/// - TrackPopupMenu for menu display
pub struct WindowsTray {
    _private: (),
}

impl WindowsTray {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for WindowsTray {
    fn default() -> Self {
        Self::new()
    }
}

impl TrayPort for WindowsTray {
    fn install(&self, model: TrayModel) -> anyhow::Result<()> {
        // Phase 4: Placeholder for Windows tray implementation
        tracing::info!("WindowsTray::install called - placeholder implementation");
        tracing::info!("Model: status={}, sessions={}", model.status_text, model.session_count);
        tracing::info!("Menu items: {}", model.menu_items.len());
        Ok(())
    }

    fn update(&self, model: TrayModel) -> anyhow::Result<()> {
        tracing::info!("WindowsTray::update called - placeholder");
        tracing::info!("Model: status={}, sessions={}", model.status_text, model.session_count);
        Ok(())
    }

    fn show_notification(&self, title: &str, message: &str) -> anyhow::Result<()> {
        tracing::info!("WindowsTray::show_notification: {} - {}", title, message);
        Ok(())
    }

    fn shutdown(&self) -> anyhow::Result<()> {
        tracing::info!("WindowsTray::shutdown called");
        Ok(())
    }

    fn is_available(&self) -> bool {
        cfg!(windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_tray_can_be_created() {
        let tray = WindowsTray::new();
        assert_eq!(tray.is_available(), cfg!(windows));
    }

    #[test]
    fn windows_tray_install_works() {
        let tray = WindowsTray::new();
        let model = TrayModel::default();
        assert!(tray.install(model).is_ok());
    }
}
