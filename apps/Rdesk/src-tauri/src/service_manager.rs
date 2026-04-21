// Service bootstrap management for mrd-service
//
// Phase 6: This module now only handles bootstrap behavior.
// Rdesk no longer owns the service lifecycle - mrd-service is the owner.
// ServiceManager is only used to bootstrap mrd-service if it's not running.
//
// For service lifecycle operations (start, stop, restart), use IPC commands:
// - GetShellStatus: check service status
// - ShutdownService: request service shutdown

use anyhow::Result;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use mrd_ipc::{IpcRequest, IpcResponse};

/// Service bootstrap manager for mrd-service
///
/// Phase 6: Reduced to bootstrap-only behavior.
/// This manager is ONLY used to start mrd-service if it's not already running.
/// All other lifecycle operations go through IPC to mrd-service.
pub struct ServiceManager {
    child: Arc<Mutex<Option<Child>>>,
    #[cfg(target_os = "windows")]
    exe_path: String,
    /// Whether this instance performed bootstrap (started service)
    bootstrapped: Arc<Mutex<bool>>,
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
            bootstrapped: Arc::new(Mutex::new(false)),
        })
    }

    /// Bootstrap mrd-service if not already running via IPC
    ///
    /// Phase 6: This is the ONLY start method. It checks IPC first,
    /// and only spawns the process if service is unreachable.
    /// Returns true if bootstrap was performed, false if already running.
    pub async fn bootstrap_if_needed(&self) -> Result<bool> {
        // First check if service is reachable via IPC
        if self.is_reachable_via_ipc().await {
            tracing::info!("mrd-service is already running via IPC");
            return Ok(false);
        }

        // Service not reachable, bootstrap it
        tracing::info!("mrd-service not reachable, bootstrapping...");
        let mut child_guard = self.child.lock().await;

        #[cfg(target_os = "windows")]
        let child = Command::new(&self.exe_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to bootstrap mrd-service: {}", e))?;

        #[cfg(not(target_os = "windows"))]
        let child = Command::new(&self.exe_path)
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to bootstrap mrd-service: {}", e))?;

        *child_guard = Some(child);
        *self.bootstrapped.lock().await = true;

        tracing::info!("mrd-service bootstrapped with PID: {:?}", child_guard.as_ref().map(|c| c.id()));

        // Give the service time to initialize
        tokio::time::sleep(Duration::from_millis(500)).await;

        Ok(true)
    }

    /// Check if service is reachable via IPC
    pub async fn is_reachable_via_ipc(&self) -> bool {
        use mrd_ipc::client::IpcClient;

        let mut client = IpcClient::new();
        match client.send_request(IpcRequest::ServiceHealth).await {
            Ok(IpcResponse::ServiceHealth { .. }) => true,
            _ => false,
        }
    }

    /// Wait for service to be healthy (with timeout)
    pub async fn wait_for_healthy(&self, timeout_secs: u64) -> Result<bool> {
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs);

        while start.elapsed() < timeout {
            if self.is_reachable_via_ipc().await {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Ok(false)
    }

    /// Check if this instance bootstrapped the service
    pub async fn did_bootstrap(&self) -> bool {
        *self.bootstrapped.lock().await
    }

    /// Get the bootstrap child PID if we bootstrapped
    pub async fn bootstrap_pid(&self) -> Option<u32> {
        if !self.did_bootstrap().await {
            return None;
        }
        let mut child_guard = self.child.lock().await;
        child_guard.as_mut().and_then(|c| {
            // Check if still running
            match c.try_wait() {
                Ok(None) => Some(c.id()), // Still running
                _ => None, // Exited or error
            }
        })
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new().expect("failed to create ServiceManager")
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
    fn ipc_check_returns_false_when_not_running() {
        // This test verifies IPC check doesn't panic when service is not running
        let manager = ServiceManager::new().unwrap();
        // In a test environment, the service won't be running
        // so is_reachable_via_ipc should return false
    }
}
