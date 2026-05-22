// mrd-service library
//
// This library is used by tests to access the service's internal modules.

pub mod app_state;
pub mod browser_webrtc_preview;
pub mod capabilities;
pub mod capture_source;
pub mod display_mode;
pub mod handlers;
pub mod ipc_server;
pub mod lan_discovery;
pub mod media_adaptation;
pub mod shell;
pub mod web_bridge;

pub use app_state::{AppState, DeviceRegistry, SessionRegistry};
#[cfg(target_os = "macos")]
pub use shell::macos::{MacosAutostart, MacosTray, MacosUiLauncher};
pub use shell::{
    build_tray_model, default_autostart, default_tray, default_ui_launcher, AutostartPort,
    AutostartPortRef, InMemoryUiLauncher, NoOpAutostart, NoOpTray, TrayAction, TrayMenuItem,
    TrayModel, TrayPort, UiLaunchRequest, UiLaunchResult, UiLauncherPort, UiLauncherPortRef,
};
#[cfg(windows)]
pub use shell::{windows::WindowsTray, WindowsAutostart};
