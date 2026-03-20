// IPC client for Rdesk shell
//
// Provides a client for communicating with mrd-service over local IPC.

use anyhow::Result;
use crate::{IpcRequest, IpcResponse};

/// IPC client - communicates with mrd-service
///
/// This is a synchronous in-process implementation for development.
/// The production version will use named pipes or Unix sockets.
#[derive(Debug, Clone)]
pub struct IpcClient {
    // TODO: Replace with actual named pipe / socket connection
    _marker: std::marker::PhantomData<()>,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new() -> Self {
        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Send a request and return the response
    pub async fn send_request(&self, request: IpcRequest) -> Result<IpcResponse> {
        // TODO: Implement actual IPC transport
        // For now, return a placeholder error
        Ok(IpcResponse::Error {
            code: "E501".to_string(),
            message: format!("IPC not implemented yet: {:?}", std::mem::discriminant(&request)),
        })
    }
}

impl Default for IpcClient {
    fn default() -> Self {
        Self::new()
    }
}
