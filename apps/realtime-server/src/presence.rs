use crate::ConnectionId;
use mrd_proto::{BackendRole, DeviceId};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceEntry {
    pub connection_id: ConnectionId,
    pub device_id: DeviceId,
    pub device_key_id: String,
    pub role: BackendRole,
    pub last_seen_ms: u64,
    pub token_expires_at_ms: u64,
}

#[derive(Debug, Default)]
pub struct PresenceRegistry {
    by_device: HashMap<DeviceId, PresenceEntry>,
    by_connection: HashMap<ConnectionId, DeviceId>,
}

impl PresenceRegistry {
    pub fn register(&mut self, entry: PresenceEntry) -> Result<(), PresenceError> {
        if self.by_connection.contains_key(&entry.connection_id)
            || self.by_device.contains_key(&entry.device_id)
        {
            return Err(PresenceError::AlreadyRegistered);
        }
        self.by_connection
            .insert(entry.connection_id, entry.device_id.clone());
        self.by_device.insert(entry.device_id.clone(), entry);
        Ok(())
    }

    pub fn by_connection(&self, connection_id: ConnectionId) -> Option<&PresenceEntry> {
        self.by_connection
            .get(&connection_id)
            .and_then(|device| self.by_device.get(device))
    }

    pub fn by_device(&self, device_id: &DeviceId) -> Option<&PresenceEntry> {
        self.by_device.get(device_id)
    }

    pub fn heartbeat(
        &mut self,
        connection_id: ConnectionId,
        now_ms: u64,
    ) -> Result<(), PresenceError> {
        let device = self
            .by_connection
            .get(&connection_id)
            .cloned()
            .ok_or(PresenceError::NotRegistered)?;
        let entry = self
            .by_device
            .get_mut(&device)
            .ok_or(PresenceError::NotRegistered)?;
        entry.last_seen_ms = now_ms;
        Ok(())
    }

    pub fn remove_connection(&mut self, connection_id: ConnectionId) -> Option<PresenceEntry> {
        let device = self.by_connection.remove(&connection_id)?;
        self.by_device.remove(&device)
    }

    pub fn prune(&mut self, now_ms: u64, ttl_ms: u64) -> Vec<PresenceEntry> {
        let expired: Vec<ConnectionId> = self
            .by_device
            .values()
            .filter(|entry| {
                now_ms >= entry.token_expires_at_ms
                    || now_ms >= entry.last_seen_ms.saturating_add(ttl_ms)
            })
            .map(|entry| entry.connection_id)
            .collect();
        expired
            .into_iter()
            .filter_map(|connection| self.remove_connection(connection))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.by_device.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_device.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum PresenceError {
    #[error("connection or device is already registered")]
    AlreadyRegistered,
    #[error("connection is not registered")]
    NotRegistered,
}
