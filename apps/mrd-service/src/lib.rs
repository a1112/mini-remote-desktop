// mrd-service library
//
// This library is used by tests to access the service's internal modules.

pub mod app_state;
pub mod capture_source;
pub mod handlers;
pub mod ipc_server;
pub mod lan_discovery;
pub mod shell;

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
