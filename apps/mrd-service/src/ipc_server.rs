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
#[cfg(windows)]
mod product;

/// IPC server - handles requests from Rdesk shell
#[derive(Clone)]
pub struct IpcServer {
    app_state: Arc<AppState>,
    endpoint: transport::IpcEndpoint,
    ui_launcher: UiLauncherPortRef,
    autostart: AutostartPortRef,
    management_only: bool,
    #[cfg(target_os = "macos")]
    peer_pid: Option<u32>,
    #[cfg(target_os = "macos")]
    peer_executable_path: Option<std::path::PathBuf>,
    #[cfg(windows)]
    product_only: bool,
    #[cfg(windows)]
    product_caller: Option<crate::agent_runtime::ObservedAgentIdentity>,
}

impl IpcServer {
    /// Installed UI channel with kernel caller verification and a strict command boundary.
    #[cfg(windows)]
    pub fn new_product(app_state: Arc<AppState>) -> Self {
        Self::new_product_with_endpoint(
            app_state,
            transport::IpcEndpoint::product_from_env_or_default(),
        )
    }

    /// Explicit endpoint retains the same caller verification, including in tests.
    #[cfg(windows)]
    pub fn new_product_with_endpoint(
        app_state: Arc<AppState>,
        endpoint: transport::IpcEndpoint,
    ) -> Self {
        let mut server = Self::new_with_endpoint(app_state, endpoint);
        server.product_only = true;
        server
    }

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
            #[cfg(target_os = "macos")]
            peer_pid: None,
            #[cfg(target_os = "macos")]
            peer_executable_path: None,
            #[cfg(windows)]
            product_only: false,
            #[cfg(windows)]
            product_caller: None,
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
            #[cfg(target_os = "macos")]
            peer_pid: None,
            #[cfg(target_os = "macos")]
            peer_executable_path: None,
            #[cfg(windows)]
            product_only: false,
            #[cfg(windows)]
            product_caller: None,
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

    #[cfg(target_os = "macos")]
    pub(crate) fn with_peer_identity(
        &self,
        peer_pid: u32,
        peer_executable_path: std::path::PathBuf,
    ) -> Self {
        let mut bound = self.clone();
        bound.peer_pid = Some(peer_pid);
        bound.peer_executable_path = Some(peer_executable_path);
        bound
    }
}

#[cfg(test)]
mod tests;
