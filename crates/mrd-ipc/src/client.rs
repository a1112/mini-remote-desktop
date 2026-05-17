// IPC client for Rdesk shell
//
// Provides a client for communicating with mrd-service over local IPC.

use crate::{transport::IpcEndpoint, IpcRequest, IpcResponse};
use anyhow::Result;
use std::time::Duration;

/// Reconnection configuration
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Maximum number of reconnection attempts
    pub max_attempts: u32,
    /// Initial backoff duration
    pub initial_backoff: Duration,
    /// Maximum backoff duration
    pub max_backoff: Duration,
    /// Whether to enable auto-reconnect
    pub enabled: bool,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            enabled: true,
        }
    }
}

/// Connection state of the IPC client
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32 },
}

/// IPC client - communicates with mrd-service
pub struct IpcClient {
    // Stream will be created on first use
    #[cfg(unix)]
    stream: Option<crate::transport::IpcStream>,

    #[cfg(windows)]
    stream: Option<crate::transport::IpcStream>,

    /// Current connection state
    state: ConnectionState,

    /// Reconnection configuration
    reconnect_config: ReconnectConfig,

    /// Target endpoint for the service connection.
    endpoint: IpcEndpoint,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new() -> Self {
        Self::with_config_and_endpoint(
            ReconnectConfig::default(),
            IpcEndpoint::service_from_env_or_default(),
        )
    }

    /// Create a new IPC client that connects to a custom endpoint.
    pub fn with_endpoint(endpoint: IpcEndpoint) -> Self {
        Self::with_config_and_endpoint(ReconnectConfig::default(), endpoint)
    }

    /// Create a new IPC client with custom reconnection config
    pub fn with_config(config: ReconnectConfig) -> Self {
        Self::with_config_and_endpoint(config, IpcEndpoint::default_service())
    }

    /// Create a new IPC client with custom config and endpoint.
    pub fn with_config_and_endpoint(config: ReconnectConfig, endpoint: IpcEndpoint) -> Self {
        Self {
            stream: None,
            state: ConnectionState::Disconnected,
            reconnect_config: config,
            endpoint,
        }
    }

    /// Get the current connection state
    pub fn state(&self) -> &ConnectionState {
        &self.state
    }

    /// Check if currently connected
    pub fn is_connected(&self) -> bool {
        matches!(self.state, ConnectionState::Connected)
    }

    /// Set the reconnection configuration
    pub fn set_reconnect_config(&mut self, config: ReconnectConfig) {
        self.reconnect_config = config;
    }

    async fn connect_once(&mut self) -> Result<()> {
        self.state = ConnectionState::Connecting;

        match crate::transport::IpcClient::connect_with_endpoint(&self.endpoint).await {
            Ok(stream) => {
                self.stream = Some(stream);
                self.state = ConnectionState::Connected;
                Ok(())
            }
            Err(e) => {
                self.state = ConnectionState::Disconnected;
                Err(e)
            }
        }
    }

    /// Ensure the stream is connected with auto-reconnect
    async fn ensure_connected(&mut self) -> Result<()> {
        // If we have a stream, assume it is still usable until I/O says otherwise.
        if self.stream.is_some() {
            return Ok(());
        }

        if !self.reconnect_config.enabled {
            return self.connect_once().await;
        }

        let mut attempt = 0;
        let mut delay = self.reconnect_config.initial_backoff;

        loop {
            match crate::transport::IpcClient::connect_with_endpoint(&self.endpoint).await {
                Ok(stream) => {
                    self.stream = Some(stream);
                    self.state = ConnectionState::Connected;
                    return Ok(());
                }
                Err(e) if attempt < self.reconnect_config.max_attempts => {
                    attempt += 1;
                    self.state = ConnectionState::Reconnecting { attempt };

                    if self.reconnect_config.enabled {
                        tracing::warn!(
                            "IPC connection failed (attempt {}/{}): {}, retrying in {:?}",
                            attempt,
                            self.reconnect_config.max_attempts,
                            e,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, self.reconnect_config.max_backoff);
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => {
                    self.state = ConnectionState::Disconnected;
                    return Err(e);
                }
            }
        }
    }

    /// Send a request and return the response with auto-reconnect
    pub async fn send_request(&mut self, request: IpcRequest) -> Result<IpcResponse> {
        self.ensure_connected().await?;
        let stream = self.stream.as_mut().unwrap();

        // Try to send the request
        match stream.send_request(&request).await {
            Ok(()) => {
                // Try to receive response
                match stream.recv_response().await {
                    Ok(response) => Ok(response),
                    Err(e) => {
                        // Connection likely lost during receive
                        tracing::warn!("IPC receive error: {}, marking as disconnected", e);
                        self.stream = None;
                        self.state = ConnectionState::Disconnected;
                        Err(e)
                    }
                }
            }
            Err(e) => {
                // Connection likely lost during send
                tracing::warn!("IPC send error: {}, marking as disconnected", e);
                self.stream = None;
                self.state = ConnectionState::Disconnected;
                Err(e)
            }
        }
    }

    /// Send a request with a single attempt (no auto-reconnect on failure)
    pub async fn send_request_no_reconnect(&mut self, request: IpcRequest) -> Result<IpcResponse> {
        if self.stream.is_none() {
            self.connect_once().await?;
        }
        let stream = self.stream.as_mut().unwrap();

        match stream.send_request(&request).await {
            Ok(()) => match stream.recv_response().await {
                Ok(response) => Ok(response),
                Err(e) => {
                    self.stream = None;
                    self.state = ConnectionState::Disconnected;
                    Err(e)
                }
            },
            Err(e) => {
                self.stream = None;
                self.state = ConnectionState::Disconnected;
                Err(e)
            }
        }
    }

    /// Explicitly disconnect from the service
    pub fn disconnect(&mut self) {
        self.stream = None;
        self.state = ConnectionState::Disconnected;
    }
}

impl Default for IpcClient {
    fn default() -> Self {
        Self::new()
    }
}
