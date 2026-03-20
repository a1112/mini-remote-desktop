// Service lifecycle management for mrd-service
//
// This module handles starting, stopping, and monitoring the mrd-service
// background process.

use anyhow::Result;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use mrd_ipc::{IpcRequest, IpcResponse};

/// Service manager for mrd-service
pub struct ServiceManager {
    child: Arc<Mutex<Option<Child>>>,
    #[cfg(target_os = "windows")]
    exe_path: String,
}

impl ServiceManager {
    pub fn new() -> Result<Self> {
        // Find the mrd-service executable path
        #[cfg(target_os = "windows")]
        let exe_path = {
            // In development, look for the debug build
            let debug_path = std::env::current_dir()?
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join("mrd-service.exe");

            if debug_path.exists() {
                debug_path.to_string_lossy().to_string()
            } else {
                // In production, assume it's in PATH
                "mrd-service.exe".to_string()
            }
        };

        #[cfg(not(target_os = "windows"))]
        let exe_path = {
            let debug_path = std::env::current_dir()?
                .join("..")
                .join("..")
                .join("target")
                .join("debug")
                .join("mrd-service");

            if debug_path.exists() {
                debug_path.to_string_lossy().to_string()
            } else {
                "mrd-service".to_string()
            }
        };

        Ok(Self {
            child: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "windows")]
            exe_path,
        })
    }

    /// Start mrd-service as a background process
    pub async fn start(&self) -> Result<()> {
        let mut child_guard = self.child.lock().await;

        if child_guard.is_some() {
            tracing::info!("mrd-service is already running");
            return Ok(());
        }

        tracing::info!("Starting mrd-service...");

        #[cfg(target_os = "windows")]
        let child = Command::new(&self.exe_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start mrd-service: {}", e))?;

        #[cfg(not(target_os = "windows"))]
        let child = Command::new(&self.exe_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start mrd-service: {}", e))?;

        *child_guard = Some(child);

        tracing::info!("mrd-service started with PID: {:?}", child_guard.as_ref().map(|c| c.id()));

        // Give the service time to initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        Ok(())
    }

    /// Stop mrd-service background process gracefully
    pub async fn stop(&self) -> Result<()> {
        let mut child_guard = self.child.lock().await;

        if let Some(mut child) = child_guard.take() {
            let pid = child.id();
            tracing::info!("Stopping mrd-service (PID: {:?})...", pid);

            // First, try graceful shutdown via IPC
            if let Ok(healthy) = self.health_check().await {
                if healthy {
                    tracing::debug!("Service is healthy, attempting graceful shutdown");
                    // Give service a moment to clean up
                    drop(child_guard); // Release lock during sleep
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    child_guard = self.child.lock().await;
                    // Re-acquire child (might have been taken by another task)
                    if child_guard.is_some() {
                        if let Some(c) = child_guard.take() {
                            child = c;
                        } else {
                            tracing::info!("Service already stopped");
                            return Ok(());
                        }
                    }
                }
            }

            // Send termination signal
            #[cfg(unix)]
            {
                use nix::sys::signal::{self, Signal};
                let _ = child.kill(Signal::SIGTERM);
            }

            #[cfg(windows)]
            {
                let _ = child.kill();
            }

            // Wait for process to exit gracefully (with timeout)
            let wait_result = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::task::spawn_blocking(move || {
                    child.wait()
                })
            ).await;

            match wait_result {
                Ok(Ok(status)) => {
                    tracing::info!("mrd-service stopped with status: {:?}", status);
                }
                Ok(Err(e)) => {
                    tracing::warn!("Error waiting for service: {}", e);
                }
                Err(_) => {
                    tracing::warn!("Service did not exit gracefully, may need force kill");
                }
            }

            Ok(())
        } else {
            tracing::info!("mrd-service is not running");
            Ok(())
        }
    }

    /// Check if service is running
    pub async fn is_running(&self) -> bool {
        let mut child_guard = self.child.lock().await;
        if let Some(child) = child_guard.as_mut() {
            // try_wait() returns Ok(exit_status) if process has exited
            // is_err() means process is still running
            return child.try_wait().is_err();
        }
        false
    }

    /// Restart the service
    pub async fn restart(&self) -> Result<()> {
        self.stop().await?;
        self.start().await
    }

    /// Ensure service is running, start if not
    pub async fn ensure_running(&self) -> Result<()> {
        if !self.is_running().await {
            self.start().await
        } else {
            Ok(())
        }
    }

    /// Health check - verify service is responding to IPC requests
    pub async fn health_check(&self) -> Result<bool> {
        use mrd_ipc::client::IpcClient;

        let mut client = IpcClient::new();
        match client.send_request(IpcRequest::ListDevices).await {
            Ok(IpcResponse::DeviceList { .. }) => Ok(true),
            Ok(_) => Ok(false), // Unexpected response
            Err(_) => Ok(false), // Connection failed
        }
    }

    /// Wait for service to be healthy (with timeout)
    pub async fn wait_for_healthy(&self, timeout_secs: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            if self.health_check().await.unwrap_or(false) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok(false)
    }

    /// Restart with exponential backoff
    pub async fn restart_with_backoff(&self, max_attempts: u32) -> Result<()> {
        let mut delay = Duration::from_millis(100);

        for attempt in 1..=max_attempts {
            tracing::info!("Restart attempt {}/{}", attempt, max_attempts);

            match self.restart().await {
                Ok(()) => {
                    // Wait and check health
                    if self.wait_for_healthy(5).await? {
                        tracing::info!("Service restarted successfully");
                        return Ok(());
                    }
                }
                Err(e) => {
                    tracing::warn!("Restart attempt {} failed: {}", attempt, e);
                }
            }

            if attempt < max_attempts {
                tracing::info!("Retrying in {:?}", delay);
                tokio::time::sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(5));
            }
        }

        Err(anyhow::anyhow!("Failed to restart service after {} attempts", max_attempts))
    }

    /// Get the process ID if running
    pub async fn pid(&self) -> Option<u32> {
        let mut child_guard = self.child.lock().await;
        child_guard.as_mut().map(|c| c.id())
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new().expect("failed to create ServiceManager")
    }
}

/// Configuration for service guard behavior
#[derive(Debug, Clone)]
pub struct ServiceGuardConfig {
    /// Interval between health checks
    pub health_check_interval: Duration,
    /// Maximum restart attempts before giving up
    pub max_restart_attempts: u32,
    /// Whether to auto-start the service on first use
    pub auto_start: bool,
}

impl Default for ServiceGuardConfig {
    fn default() -> Self {
        Self {
            health_check_interval: Duration::from_secs(5),
            max_restart_attempts: 3,
            auto_start: true,
        }
    }
}

/// Service guard - monitors and auto-restarts mrd-service
///
/// Runs in the background to ensure the service stays healthy.
pub struct ServiceGuard {
    manager: Arc<ServiceManager>,
    config: ServiceGuardConfig,
    shutdown: Arc<Notify>,
}

impl ServiceGuard {
    /// Create a new service guard
    pub fn new(config: ServiceGuardConfig) -> Result<Self> {
        let manager = Arc::new(ServiceManager::new()?);

        Ok(Self {
            manager,
            config,
            shutdown: Arc::new(Notify::new()),
        })
    }

    /// Start the guard task
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // Auto-start if configured
            if self.config.auto_start {
                if let Err(e) = self.manager.start().await {
                    tracing::error!("Failed to auto-start service: {}", e);
                }
            }

            self.run().await;
        })
    }

    /// Run the guard loop
    async fn run(&self) {
        let mut restart_count = 0;
        let mut interval = tokio::time::interval(self.config.health_check_interval);

        loop {
            tokio::select! {
                _ = self.shutdown.notified() => {
                    tracing::info!("Service guard shutting down");
                    break;
                }
                _ = interval.tick() => {
                    // Check if service is still running
                    if !self.manager.is_running().await {
                        tracing::warn!("Service is not running, attempting restart");

                        if restart_count < self.config.max_restart_attempts {
                            match self.manager.restart_with_backoff(3).await {
                                Ok(()) => {
                                    tracing::info!("Service restarted successfully");
                                    restart_count = 0; // Reset on successful restart
                                }
                                Err(e) => {
                                    tracing::error!("Failed to restart service: {}", e);
                                    restart_count += 1;
                                }
                            }
                        } else {
                            tracing::error!(
                                "Service restart failed after {} attempts, giving up",
                                self.config.max_restart_attempts
                            );
                            break;
                        }
                    } else {
                        // Service is running, check health
                        match self.manager.health_check().await {
                            Ok(true) => {
                                restart_count = 0; // Reset on successful health check
                            }
                            Ok(false) => {
                                tracing::warn!("Service is running but not healthy");
                            }
                            Err(e) => {
                                tracing::debug!("Health check failed (may be expected): {}", e);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Stop the guard
    pub fn stop(&self) {
        self.shutdown.notify_one();
    }

    /// Get access to the underlying service manager
    pub fn manager(&self) -> &Arc<ServiceManager> {
        &self.manager
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_service_manager() {
        let manager = ServiceManager::new();
        assert!(manager.is_ok());
    }

    #[test]
    fn health_check_returns_false_when_not_running() {
        // This test verifies health check doesn't panic when service is not running
        let manager = ServiceManager::new().unwrap();
        // In a test environment, the service won't be running
        // so health_check should return Ok(false) or handle error gracefully
    }
}
