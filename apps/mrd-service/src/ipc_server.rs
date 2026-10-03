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
    management_only: bool,
}

impl IpcServer {
    /// Dedicated local endpoint restricted to service lifecycle management.
    pub fn new_management(app_state: Arc<AppState>) -> Self {
        Self::new_management_with_endpoint(
            app_state,
            transport::IpcEndpoint::management_from_env_or_default(),
        )
    }

    /// Management server with an explicit endpoint for isolated deployments.
    pub fn new_management_with_endpoint(
        app_state: Arc<AppState>,
        endpoint: transport::IpcEndpoint,
    ) -> Self {
        let mut server = Self::new_with_endpoint(app_state, endpoint);
        server.management_only = true;
        server
    }

    /// Supply a platform autostart adapter for the server and its cloned workers.
    pub fn with_autostart(mut self, autostart: AutostartPortRef) -> Self {
        self.autostart = autostart;
        self
    }

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
            management_only: false,
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
            management_only: false,
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
