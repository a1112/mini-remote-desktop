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
    /// No active IPC connection.
    Disconnected,
    /// A connection attempt is in progress.
    Connecting,
    /// Connected to the service.
    Connected,
    /// Reconnect loop is retrying after a failed connection.
    Reconnecting {
        /// Current reconnect attempt number.
        attempt: u32,
    },
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
    management: bool,
    #[cfg(windows)]
    authenticated_product: bool,
}

impl IpcClient {
    /// Create a new IPC client
    pub fn new() -> Self {
        #[cfg(windows)]
        if std::env::var(crate::transport::SERVICE_ENDPOINT_ENV)
            .ok()
            .and_then(|value| IpcEndpoint::from_env_value(&value))
            .is_none()
        {
            return Self::product();
        }
        Self::with_config_and_endpoint(
            ReconnectConfig::default(),
            IpcEndpoint::service_from_env_or_default(),
        )
    }

    /// Authenticated installed Windows UI endpoint. Explicit core constructors
    /// keep their previous semantics for isolated tests and administrator tools.
    #[cfg(windows)]
    pub fn product() -> Self {
        let mut client = Self::with_endpoint(IpcEndpoint::product_from_env_or_default());
        client.authenticated_product = true;
        client
    }

    /// Management containing secrets must authenticate the kernel server process.
    /// No endpoint or environment setting bypasses this Windows identity check.
    pub fn trusted_management() -> Self {
        let client = Self::management();
        #[cfg(windows)]
        let client = {
            let mut client = client;
            client.authenticated_product = true;
            client
        };
        client
    }

    /// Create a client for the narrow service management endpoint.
    pub fn management() -> Self {
        Self::management_with_config(ReconnectConfig::default())
    }

    /// Create a management client with bounded/custom reconnection behavior.
    pub fn management_with_config(config: ReconnectConfig) -> Self {
        Self::management_with_config_and_endpoint(
            config,
            IpcEndpoint::management_from_env_or_default(),
        )
    }

    /// Create a management client for an explicit endpoint.
    pub fn management_with_endpoint(endpoint: IpcEndpoint) -> Self {
        Self::management_with_config_and_endpoint(ReconnectConfig::default(), endpoint)
    }

    /// Create a management client with an explicit endpoint and reconnection policy.
    pub fn management_with_config_and_endpoint(
        config: ReconnectConfig,
        endpoint: IpcEndpoint,
    ) -> Self {
        let mut client = Self::with_config_and_endpoint(config, endpoint);
        client.management = true;
        client
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
            management: false,
            #[cfg(windows)]
            authenticated_product: false,
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

        match self.connect_transport().await {
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

    async fn connect_transport(&self) -> Result<crate::transport::IpcStream> {
        #[cfg(windows)]
        if self.authenticated_product {
            return crate::transport::IpcClient::connect_product_with_endpoint(&self.endpoint)
                .await;
        }
        if self.management {
            crate::transport::IpcClient::connect_management_with_endpoint(&self.endpoint).await
        } else {
            crate::transport::IpcClient::connect_with_endpoint(&self.endpoint).await
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
            match self.connect_transport().await {
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

#[cfg(all(test, windows))]
mod product_constructor_tests {
    use super::*;

    #[test]
    fn authenticated_management_and_product_have_no_untrusted_fallback() {
        let management = IpcClient::trusted_management();
        assert!(management.authenticated_product);
        assert_eq!(
            management.endpoint,
            IpcEndpoint::management_from_env_or_default()
        );
        let product = IpcClient::product();
        assert!(product.authenticated_product);
        assert_eq!(product.endpoint, IpcEndpoint::product_from_env_or_default());
    }

    #[test]
    fn explicit_core_endpoints_preserve_administrator_and_isolated_test_semantics() {
        let endpoint = IpcEndpoint::named_pipe(r"\\.\pipe\explicit-core-test");
        let client = IpcClient::with_endpoint(endpoint.clone());
        assert!(!client.authenticated_product);
        assert_eq!(client.endpoint, endpoint);
        assert!(!IpcClient::with_config(ReconnectConfig::default()).authenticated_product);
    }
}
