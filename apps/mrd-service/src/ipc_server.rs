#![allow(dead_code)]

// IPC server for mrd-service
//
// Handles incoming IPC requests from Rdesk shell and dispatches
// to application layer use cases.

use crate::{
    app_state::AppState,
    shell::{AutostartPortRef, UiLauncherPortRef},
};
use mrd_ipc::{transport, IpcRequest, IpcResponse};
use std::sync::Arc;

mod accept_loop;
mod audit;
mod connection;
mod dispatch;

/// IPC server - handles requests from Rdesk shell
#[derive(Clone)]
pub struct IpcServer {
    app_state: Arc<AppState>,
    endpoint: transport::IpcEndpoint,
    ui_launcher: UiLauncherPortRef,
    autostart: AutostartPortRef,
}

impl IpcServer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self::new_with_endpoint(
            app_state,
            transport::IpcEndpoint::service_from_env_or_default(),
        )
    }

    pub fn new_with_endpoint(app_state: Arc<AppState>, endpoint: transport::IpcEndpoint) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher: crate::shell::default_ui_launcher(),
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    pub fn new_with_launcher(
        app_state: Arc<AppState>,
        endpoint: transport::IpcEndpoint,
        ui_launcher: UiLauncherPortRef,
    ) -> Self {
        Self {
            app_state,
            endpoint,
            ui_launcher,
            autostart: crate::shell::default_autostart("mrd-service"),
        }
    }

    /// Handle an IPC request and return a response
    pub async fn handle_request(&self, request: IpcRequest) -> IpcResponse {
        dispatch::dispatch_request(self, request).await
    }

    /// Get access to the app state (for testing/integration)
    pub fn app_state(&self) -> &Arc<AppState> {
        &self.app_state
    }
}

#[cfg(test)]
mod tests;
