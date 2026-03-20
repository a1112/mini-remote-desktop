// IPC client integration for Rdesk shell
//
// This module will eventually replace direct calls to QuicHost, WebrtcHost,
// and RealtimeRuntime with IPC calls to mrd-service.

use mrd_ipc::client::IpcClient;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Global IPC client for communicating with mrd-service
pub struct ServiceClient {
    client: Arc<Mutex<IpcClient>>,
}

impl ServiceClient {
    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(IpcClient::new())),
        }
    }

    pub fn client(&self) -> &Arc<Mutex<IpcClient>> {
        &self.client
    }
}

impl Default for ServiceClient {
    fn default() -> Self {
        Self::new()
    }
}
