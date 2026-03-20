// mrd-service library
//
// This library is used by tests to access the service's internal modules.

pub mod app_state;
pub mod handlers;
pub mod ipc_server;

pub use app_state::{AppState, SessionRegistry, DeviceRegistry};
