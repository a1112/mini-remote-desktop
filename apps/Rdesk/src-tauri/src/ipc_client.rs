// IPC client integration for Rdesk shell
//
// This module will eventually replace direct calls to QuicHost, WebrtcHost,
// and RealtimeRuntime with IPC calls to mrd-service.

use mrd_ipc::client::IpcClient;

/// Global IPC client for communicating with mrd-service
///
/// TODO: Initialize this properly once mrd-service is running
pub struct ServiceClient {
    client: IpcClient,
}

impl ServiceClient {
    pub fn new() -> Self {
        Self {
            client: IpcClient::new(),
        }
    }

    pub fn client(&self) -> &IpcClient {
        &self.client
    }
}

impl Default for ServiceClient {
    fn default() -> Self {
        Self::new()
    }
}
