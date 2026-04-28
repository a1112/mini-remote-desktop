// Network interruption recovery and automatic reconnection
//
// Provides automatic reconnection with exponential backoff,
// connection health monitoring, and state recovery.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;

use crate::{QuinnDatagramEndpoint, QuinnServerBootstrap};

/// Reconnection configuration
#[derive(Debug, Clone)]
pub struct ReconnectConfig {
    /// Enable automatic reconnection
    pub enabled: bool,
    /// Maximum number of reconnection attempts
    pub max_attempts: u32,
    /// Initial backoff duration
    pub initial_backoff: Duration,
    /// Maximum backoff duration
    pub max_backoff: Duration,
    /// Backoff multiplier
    pub backoff_multiplier: f32,
    /// Connection timeout for each attempt
    pub connection_timeout: Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 5,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
            backoff_multiplier: 2.0,
            connection_timeout: Duration::from_secs(5),
        }
    }
}

/// Connection health status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionHealth {
    /// Connection is healthy
    Healthy,
    /// Connection is degraded (high latency, packet loss)
    Degraded,
    /// Connection is down
    Disconnected,
    /// Reconnection in progress
    Reconnecting { attempt: u32 },
}

/// Connection state for recovery
#[derive(Debug)]
struct ConnectionState {
    health: ConnectionHealth,
    last_successful_activity: Option<Instant>,
    reconnect_attempt: u32,
    last_error: Option<String>,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            health: ConnectionHealth::Disconnected,
            last_successful_activity: None,
            reconnect_attempt: 0,
            last_error: None,
        }
    }
}

/// Reconnectable QUIC endpoint with automatic recovery
pub struct ReconnectableEndpoint {
    /// Server bootstrap info for reconnection
    bootstrap: QuinnServerBootstrap,
    /// Current active endpoint (if connected)
    endpoint: Arc<Mutex<Option<QuinnDatagramEndpoint>>>,
    /// Connection state
    state: Arc<Mutex<ConnectionState>>,
    /// Reconnect configuration
    config: ReconnectConfig,
    /// Notify for connection state changes
    notify: Arc<Notify>,
}

impl ReconnectableEndpoint {
    /// Create a new reconnectable endpoint
    pub fn new(bootstrap: QuinnServerBootstrap, config: ReconnectConfig) -> Self {
        Self {
            bootstrap,
            endpoint: Arc::new(Mutex::new(None)),
            state: Arc::new(Mutex::new(ConnectionState::new())),
            config,
            notify: Arc::new(Notify::new()),
        }
    }

    /// Get the reconnect configuration
    pub fn config(&self) -> &ReconnectConfig {
        &self.config
    }

    /// Update the reconnect configuration
    pub fn set_config(&mut self, config: ReconnectConfig) {
        self.config = config;
    }

    /// Connect to the server (with auto-reconnect if enabled)
    pub async fn connect(&self) -> Result<(), ReconnectError> {
        self.connect_internal().await?;

        // Start background reconnection task if enabled
        if self.config.enabled {
            let state = self.state.clone();
            let endpoint = self.endpoint.clone();
            let bootstrap = self.bootstrap.clone();
            let config = self.config.clone();
            let notify = self.notify.clone();

            tokio::spawn(async move {
                Self::reconnection_task(state, endpoint, bootstrap, config, notify).await;
            });
        }

        Ok(())
    }

    /// Internal connection logic
    async fn connect_internal(&self) -> Result<(), ReconnectError> {
        let bind_addr = "127.0.0.1:0";
        let endpoint = tokio::time::timeout(
            self.config.connection_timeout,
            QuinnDatagramEndpoint::connect_client(bind_addr, &self.bootstrap),
        )
        .await
        .map_err(|_| ReconnectError::Timeout)?
        .map_err(|e| ReconnectError::ConnectionFailed(e.to_string()))?;

        {
            let mut ep_guard = self.endpoint.lock().await;
            *ep_guard = Some(endpoint);
        }

        {
            let mut state = self.state.lock().await;
            state.health = ConnectionHealth::Healthy;
            state.last_successful_activity = Some(Instant::now());
            state.reconnect_attempt = 0;
            state.last_error = None;
        }

        self.notify.notify_waiters();
        Ok(())
    }

    /// Background reconnection task
    async fn reconnection_task(
        state: Arc<Mutex<ConnectionState>>,
        endpoint: Arc<Mutex<Option<QuinnDatagramEndpoint>>>,
        bootstrap: QuinnServerBootstrap,
        config: ReconnectConfig,
        notify: Arc<Notify>,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            interval.tick().await;

            let should_reconnect = {
                let st = state.lock().await;
                matches!(st.health, ConnectionHealth::Disconnected)
            };

            if !should_reconnect {
                continue;
            }

            // Check max attempts
            {
                let mut st = state.lock().await;
                if st.reconnect_attempt >= config.max_attempts {
                    // Give up, wait for manual reconnect
                    continue;
                }
                st.reconnect_attempt += 1;
                st.health = ConnectionHealth::Reconnecting {
                    attempt: st.reconnect_attempt,
                };
            }

            notify.notify_waiters();

            // Calculate backoff
            let attempt = state.lock().await.reconnect_attempt;
            let backoff = Self::calculate_backoff(&config, attempt);

            // Wait for backoff
            tokio::time::sleep(backoff).await;

            // Try to reconnect
            let result = Self::try_reconnect(&endpoint, &bootstrap, &config).await;

            match result {
                Ok(_) => {
                    let mut st = state.lock().await;
                    st.health = ConnectionHealth::Healthy;
                    st.last_successful_activity = Some(Instant::now());
                    st.reconnect_attempt = 0;
                    st.last_error = None;
                    notify.notify_waiters();
                }
                Err(e) => {
                    let mut st = state.lock().await;
                    st.last_error = Some(e.to_string());
                    st.health = ConnectionHealth::Disconnected;
                    notify.notify_waiters();
                }
            }
        }
    }

    /// Attempt a single reconnection
    async fn try_reconnect(
        endpoint: &Arc<Mutex<Option<QuinnDatagramEndpoint>>>,
        bootstrap: &QuinnServerBootstrap,
        config: &ReconnectConfig,
    ) -> Result<(), ReconnectError> {
        let bind_addr = "127.0.0.1:0";
        let new_endpoint = tokio::time::timeout(
            config.connection_timeout,
            QuinnDatagramEndpoint::connect_client(bind_addr, bootstrap),
        )
        .await
        .map_err(|_| ReconnectError::Timeout)?
        .map_err(|e| ReconnectError::ConnectionFailed(e.to_string()))?;

        {
            let mut ep_guard = endpoint.lock().await;
            *ep_guard = Some(new_endpoint);
        }

        Ok(())
    }

    /// Calculate exponential backoff duration
    fn calculate_backoff(config: &ReconnectConfig, attempt: u32) -> Duration {
        let base_ms = config.initial_backoff.as_millis() as f64;
        let multiplier = f64::from(
            config
                .backoff_multiplier
                .powi(attempt.saturating_sub(1) as i32),
        );
        let backoff_ms = (base_ms * multiplier).min(config.max_backoff.as_millis() as f64);
        Duration::from_millis(backoff_ms as u64)
    }

    /// Get current connection health
    pub async fn health(&self) -> ConnectionHealth {
        let state = self.state.lock().await;
        state.health.clone()
    }

    /// Check if endpoint is connected
    pub async fn is_connected(&self) -> bool {
        matches!(self.health().await, ConnectionHealth::Healthy)
    }

    /// Get last successful activity time
    pub async fn last_activity(&self) -> Option<Instant> {
        let state = self.state.lock().await;
        state.last_successful_activity
    }

    /// Get current reconnection attempt count
    pub async fn reconnect_attempt(&self) -> u32 {
        let state = self.state.lock().await;
        state.reconnect_attempt
    }

    /// Get last error message
    pub async fn last_error(&self) -> Option<String> {
        let state = self.state.lock().await;
        state.last_error.clone()
    }

    /// Send a datagram (with automatic reconnection if enabled)
    pub async fn send_datagram(&self, payload: bytes::Bytes) -> Result<(), ReconnectError> {
        let endpoint = {
            let ep_guard = self.endpoint.lock().await;
            ep_guard.as_ref().cloned()
        };

        match endpoint {
            Some(ep) => {
                ep.send_datagram(payload)
                    .map_err(|e| ReconnectError::SendFailed(e.to_string()))?;

                // Update activity timestamp
                let mut state = self.state.lock().await;
                state.last_successful_activity = Some(Instant::now());
                Ok(())
            }
            None => {
                if self.config.enabled {
                    Err(ReconnectError::NotConnected("Reconnecting...".to_string()))
                } else {
                    Err(ReconnectError::NotConnected("Disconnected".to_string()))
                }
            }
        }
    }

    /// Read a datagram
    pub async fn read_datagram(&self) -> Result<bytes::Bytes, ReconnectError> {
        loop {
            let endpoint = {
                let ep_guard = self.endpoint.lock().await;
                ep_guard.as_ref().cloned()
            };

            match endpoint {
                Some(ep) => {
                    let result = ep.read_datagram().await;
                    match result {
                        Ok(data) => {
                            // Update activity timestamp
                            let mut state = self.state.lock().await;
                            state.last_successful_activity = Some(Instant::now());
                            return Ok(data);
                        }
                        Err(e) => {
                            // Connection error, mark as disconnected
                            let mut state = self.state.lock().await;
                            state.health = ConnectionHealth::Disconnected;
                            state.last_error = Some(e.to_string());
                            self.notify.notify_waiters();

                            if self.config.enabled {
                                // Wait for reconnection and retry
                                drop(state);
                                self.wait_for_reconnection().await?;
                                continue;
                            } else {
                                return Err(ReconnectError::ReceiveFailed(e.to_string()));
                            }
                        }
                    }
                }
                None => {
                    if self.config.enabled {
                        self.wait_for_reconnection().await?;
                    } else {
                        return Err(ReconnectError::NotConnected("Disconnected".to_string()));
                    }
                }
            }
        }
    }

    /// Wait for reconnection to complete
    async fn wait_for_reconnection(&self) -> Result<(), ReconnectError> {
        loop {
            let health = self.health().await;
            if matches!(health, ConnectionHealth::Healthy) {
                return Ok(());
            }

            // Check if we've given up
            let attempt = self.reconnect_attempt().await;
            if attempt >= self.config.max_attempts {
                return Err(ReconnectError::ReconnectFailed);
            }

            // Wait for state change notification
            self.notify.notified().await;
        }
    }

    /// Manually trigger reconnection
    pub async fn reconnect(&self) -> Result<(), ReconnectError> {
        // Reset reconnect counter
        {
            let mut state = self.state.lock().await;
            state.reconnect_attempt = 0;
            state.health = ConnectionHealth::Disconnected;
        }

        self.connect_internal().await
    }

    /// Close the connection
    pub async fn close(&self) {
        let mut ep_guard = self.endpoint.lock().await;
        *ep_guard = None;

        let mut state = self.state.lock().await;
        state.health = ConnectionHealth::Disconnected;
        state.last_error = Some("Connection closed".to_string());
    }
}

/// Reconnection errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconnectError {
    ConnectionFailed(String),
    Timeout,
    SendFailed(String),
    ReceiveFailed(String),
    NotConnected(String),
    ReconnectFailed,
}

impl std::fmt::Display for ReconnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            Self::Timeout => write!(f, "Connection timeout"),
            Self::SendFailed(msg) => write!(f, "Send failed: {}", msg),
            Self::ReceiveFailed(msg) => write!(f, "Receive failed: {}", msg),
            Self::NotConnected(msg) => write!(f, "Not connected: {}", msg),
            Self::ReconnectFailed => write!(f, "Reconnect failed"),
        }
    }
}

impl std::error::Error for ReconnectError {}

/// Connection health monitor with periodic checks
pub struct HealthMonitor {
    endpoint: Arc<ReconnectableEndpoint>,
    check_interval: Duration,
    idle_timeout: Duration,
}

impl HealthMonitor {
    /// Create a new health monitor
    pub fn new(
        endpoint: Arc<ReconnectableEndpoint>,
        check_interval: Duration,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            endpoint,
            check_interval,
            idle_timeout,
        }
    }

    /// Start the health monitor in the background
    pub async fn start(&self) {
        let endpoint = self.endpoint.clone();
        let check_interval = self.check_interval;
        let idle_timeout = self.idle_timeout;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                interval.tick().await;

                let last_activity = endpoint.last_activity().await;
                let is_connected = endpoint.is_connected().await;

                if is_connected {
                    if let Some(activity) = last_activity {
                        if activity.elapsed() > idle_timeout {
                            // Connection is idle, mark as degraded
                            let mut state = endpoint.state.lock().await;
                            if state.health == ConnectionHealth::Healthy {
                                state.health = ConnectionHealth::Degraded;
                            }
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QuinnServerListener;

    #[tokio::test]
    async fn reconnectable_endpoint_connects_on_first_attempt() {
        let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind server");

        // Start server in background
        tokio::spawn(async move {
            listener.accept().await.ok();
        });

        let config = ReconnectConfig {
            enabled: false, // Disable auto-reconnect for this test
            ..Default::default()
        };

        let endpoint = ReconnectableEndpoint::new(bootstrap, config);
        endpoint.connect().await.unwrap();

        assert!(endpoint.is_connected().await);
        assert_eq!(endpoint.health().await, ConnectionHealth::Healthy);
    }

    #[test]
    fn reconnect_config_calculates_exponential_backoff() {
        let config = ReconnectConfig {
            initial_backoff: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
            ..Default::default()
        };

        // Test backoff calculation (via private method simulation)
        let backoff_1 = ReconnectableEndpoint::calculate_backoff(&config, 1);
        assert_eq!(backoff_1, Duration::from_millis(100));

        let backoff_2 = ReconnectableEndpoint::calculate_backoff(&config, 2);
        assert_eq!(backoff_2, Duration::from_millis(200));

        let backoff_3 = ReconnectableEndpoint::calculate_backoff(&config, 3);
        assert_eq!(backoff_3, Duration::from_millis(400));

        // Should cap at max_backoff
        let backoff_10 = ReconnectableEndpoint::calculate_backoff(&config, 10);
        assert_eq!(backoff_10, Duration::from_secs(1));
    }

    #[test]
    fn connection_health_variants_exist() {
        let _ = ConnectionHealth::Healthy;
        let _ = ConnectionHealth::Degraded;
        let _ = ConnectionHealth::Disconnected;
        let _ = ConnectionHealth::Reconnecting { attempt: 1 };
    }

    #[test]
    fn reconnect_error_implements_display() {
        let errors = vec![
            ReconnectError::ConnectionFailed("test".to_string()),
            ReconnectError::Timeout,
            ReconnectError::SendFailed("test".to_string()),
            ReconnectError::ReceiveFailed("test".to_string()),
            ReconnectError::NotConnected("test".to_string()),
            ReconnectError::ReconnectFailed,
        ];

        for error in errors {
            let _ = format!("{}", error);
        }
    }

    #[tokio::test]
    async fn reconnectable_endpoint_tracks_activity() {
        let (listener, bootstrap) = QuinnServerListener::bind("127.0.0.1:0")
            .await
            .expect("Failed to bind server");

        tokio::spawn(async move {
            listener.accept().await.ok();
        });

        let config = ReconnectConfig {
            enabled: false,
            ..Default::default()
        };

        let endpoint = ReconnectableEndpoint::new(bootstrap, config);
        endpoint.connect().await.unwrap();

        // Initially should have recent activity
        let activity = endpoint.last_activity().await;
        assert!(activity.is_some());

        // After connection, activity should be recent
        let elapsed = activity.unwrap().elapsed();
        assert!(elapsed < Duration::from_secs(1));
    }
}
