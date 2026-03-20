// IPC client for Rdesk shell
//
// Provides a client for communicating with mrd-service over local IPC.

use anyhow::Result;
use crate::{IpcRequest, IpcResponse};

/// IPC client - communicates with mrd-service
pub struct IpcClient {
    // Stream will be created on first use
    #[cfg(unix)]
    stream: Option<crate::transport::IpcStream>,

    #[cfg(windows)]
    stream: Option<crate::transport::IpcStream>,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new() -> Self {
        Self { stream: None }
    }

    /// Ensure the stream is connected
    async fn ensure_connected(&mut self) -> Result<()> {
        if self.stream.is_none() {
            self.stream = Some(crate::transport::IpcClient::connect().await?);
        }
        Ok(())
    }

    /// Send a request and return the response
    pub async fn send_request(&mut self, request: IpcRequest) -> Result<IpcResponse> {
        self.ensure_connected().await?;
        let stream = self.stream.as_mut().unwrap();
        stream.send_request(&request).await?;
        let response = stream.recv_response().await?;
        Ok(response)
    }
}

impl Default for IpcClient {
    fn default() -> Self {
        Self::new()
    }
}
