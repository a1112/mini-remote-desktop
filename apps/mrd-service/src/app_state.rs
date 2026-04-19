// mrd-service application state
//
// This module defines the shared state owned by mrd-service.
// After the hard-cut migration, this becomes the single source
// of truth for all session orchestration, transport runtime,
// and media control.

use std::sync::Arc;
use tokio::sync::Mutex;
use mrd_proto::{SessionId, DeviceId};
use mrd_application::ports::SessionSnapshot;

/// Session registry tracking all active sessions
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: std::collections::HashMap<SessionId, SessionSnapshot>,
}

impl SessionRegistry {
    pub fn insert(&mut self, session_id: SessionId, snapshot: SessionSnapshot) {
        self.sessions.insert(session_id, snapshot);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<&SessionSnapshot> {
        self.sessions.get(session_id)
    }

    pub fn get_mut(&mut self, session_id: &SessionId) -> Option<&mut SessionSnapshot> {
        self.sessions.get_mut(session_id)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<SessionSnapshot> {
        self.sessions.remove(session_id)
    }

    pub fn list_all(&self) -> Vec<SessionSnapshot> {
        self.sessions.values().cloned().collect()
    }
}

/// Device registry
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    local_device: Option<(DeviceId, String)>, // (id, name)
}

impl DeviceRegistry {
    pub fn register(&mut self, device_id: DeviceId, device_name: String) {
        self.local_device = Some((device_id, device_name));
    }

    pub fn get_local_device(&self) -> Option<&(DeviceId, String)> {
        self.local_device.as_ref()
    }

    pub fn is_registered(&self) -> bool {
        self.local_device.is_some()
    }
}

/// Application state for mrd-service
///
/// This is the shared state that will be injected into IPC handlers.
/// After migration, it will own:
/// - RealtimeRuntime / signaling client
/// - WebrtcHost / WebrtcSessionCoordinator
/// - QuicHost / QuicSessionCoordinator
/// - Media senders/receivers
/// - Probe/telemetry state
pub struct AppState {
    /// Session registry - single source of truth for all sessions
    pub sessions: Arc<Mutex<SessionRegistry>>,
    /// Device registry
    pub devices: Arc<Mutex<DeviceRegistry>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(SessionRegistry::default())),
            devices: Arc::new(Mutex::new(DeviceRegistry::default())),
        }
    }

    /// Get a clone of the sessions Arc for injection into handlers
    pub fn sessions(&self) -> Arc<Mutex<SessionRegistry>> {
        self.sessions.clone()
    }

    /// Get a clone of the devices Arc for injection into handlers
    pub fn devices(&self) -> Arc<Mutex<DeviceRegistry>> {
        self.devices.clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_registry_tracks_sessions() {
        let mut registry = SessionRegistry::default();

        let session_id = SessionId("test-session".to_string());
        let snapshot = SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId("agent".to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: "created".to_string(),
            last_error: None,
            sender_active: false,
            receiver_active: false,
        };

        registry.insert(session_id.clone(), snapshot);

        let retrieved = registry.get(&session_id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().transport, "quic");
    }

    #[test]
    fn device_registry_tracks_local_device() {
        let mut registry = DeviceRegistry::default();

        let device_id = DeviceId("test-device".to_string());
        registry.register(device_id.clone(), "Test Device".to_string());

        assert!(registry.is_registered());

        let retrieved = registry.get_local_device();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().0, device_id);
    }
}
