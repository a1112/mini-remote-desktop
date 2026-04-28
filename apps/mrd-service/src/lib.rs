// mrd-service library
//
// This library is used by tests to access the service's internal modules.

pub mod app_state;
pub mod handlers;
pub mod ipc_server;
pub mod lan_discovery;
pub mod shell;

pub use app_state::{AppState, DeviceRegistry, SessionRegistry};
pub use shell::{
    build_tray_model, AutostartPort, InMemoryUiLauncher, NoOpAutostart, NoOpTray, TrayAction,
    TrayMenuItem, TrayModel, TrayPort, UiLaunchRequest, UiLaunchResult, UiLauncherPort,
};
#[cfg(windows)]
pub use shell::{windows::WindowsTray, WindowsAutostart};
