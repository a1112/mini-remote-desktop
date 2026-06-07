use mrd_ipc::MediaProfileNegotiation;
use mrd_proto::SessionId;
use std::collections::HashMap;

/// Runtime media profile negotiation state keyed by session.
#[derive(Debug, Default)]
pub struct MediaProfileRegistry {
    profiles: HashMap<SessionId, MediaProfileNegotiation>,
}

impl MediaProfileRegistry {
    pub fn set(&mut self, session_id: SessionId, negotiation: MediaProfileNegotiation) {
        self.profiles.insert(session_id, negotiation);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.get(session_id).cloned()
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<MediaProfileNegotiation> {
        self.profiles.remove(session_id)
    }
}

/// Peer media capabilities observed for each active session.
#[derive(Debug, Default)]
pub struct SessionPeerMediaCapabilityRegistry {
    capabilities: HashMap<SessionId, Vec<String>>,
}

impl SessionPeerMediaCapabilityRegistry {
    pub fn set(&mut self, session_id: SessionId, capabilities: Vec<String>) {
        self.capabilities.insert(session_id, capabilities);
    }

    pub fn get(&self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.get(session_id).cloned()
    }

    pub fn supports(&self, session_id: &SessionId, capability: &str) -> bool {
        self.capabilities
            .get(session_id)
            .map(|capabilities| capabilities.iter().any(|value| value == capability))
            .unwrap_or(false)
    }

    pub fn remove(&mut self, session_id: &SessionId) -> Option<Vec<String>> {
        self.capabilities.remove(session_id)
    }
}
