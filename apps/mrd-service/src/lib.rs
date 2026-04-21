// mrd-service library
//
// This library is used by tests to access the service's internal modules.

pub mod app_state;
pub mod handlers;
pub mod ipc_server;
pub mod shell;

pub use app_state::{AppState, SessionRegistry, DeviceRegistry};
pub use shell::{
    UiLauncherPort, UiLaunchResult, UiLaunchRequest, InMemoryUiLauncher,
    TrayPort, TrayModel, TrayMenuItem, TrayAction, NoOpTray,
    build_tray_model,
    AutostartPort, NoOpAutostart,
};
#[cfg(windows)]
pub use shell::{windows::WindowsTray, WindowsAutostart};
