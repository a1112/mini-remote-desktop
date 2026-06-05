use mrd_application::ports::SessionSnapshot;
use mrd_proto::SessionId;
use std::collections::HashMap;

/// Session registry tracking all active sessions.
#[derive(Debug, Default)]
pub struct SessionRegistry {
    sessions: HashMap<SessionId, SessionSnapshot>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_application::ports::{SessionLifecycleState, SessionSnapshot};
    use mrd_proto::{DeviceId, SessionId};

    fn snapshot(session_id: &SessionId, target: &str) -> SessionSnapshot {
        SessionSnapshot {
            session_id: session_id.clone(),
            transport: "quic".to_string(),
            source_device_id: Some(DeviceId("controller".to_string())),
            target_device_id: Some(DeviceId(target.to_string())),
            local_listen_addr: None,
            local_server_name: None,
            local_cert_der_b64: None,
            remote_listen_addr: None,
            remote_server_name: None,
            remote_cert_der_b64: None,
            lifecycle_state: SessionLifecycleState::Streaming,
            last_error: None,
            sender_active: true,
            receiver_active: false,
        }
    }

    #[test]
    fn insert_replaces_existing_snapshot_and_list_reflects_latest_state() {
        let session_id = SessionId("replace-session".to_string());
        let mut registry = SessionRegistry::default();

        registry.insert(session_id.clone(), snapshot(&session_id, "first-target"));
        registry.insert(session_id.clone(), snapshot(&session_id, "second-target"));

        assert_eq!(
            registry
                .get(&session_id)
                .and_then(|snapshot| snapshot.target_device_id.as_ref())
                .map(|device_id| device_id.0.as_str()),
            Some("second-target")
        );
        assert_eq!(registry.list_all().len(), 1);
        assert_eq!(
            registry
                .remove(&session_id)
                .and_then(|snapshot| snapshot.target_device_id)
                .map(|device_id| device_id.0),
            Some("second-target".to_string())
        );
        assert!(registry.list_all().is_empty());
    }
}
