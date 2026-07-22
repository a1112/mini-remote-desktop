use mrd_proto::SessionId;
use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;

    #[test]
    fn capabilities_are_scoped_by_session_and_removed_with_session() {
        let h264_session = SessionId("h264-capability-session".to_string());
        let hevc_session = SessionId("hevc-capability-session".to_string());
        let mut registry = SessionPeerMediaCapabilityRegistry::default();

        registry.set(
            h264_session.clone(),
            vec![
                "media.codec.h264".to_string(),
                "media.color_mode_v1".to_string(),
            ],
        );
        registry.set(
            hevc_session.clone(),
            vec![
                "media.codec.hevc".to_string(),
                "media.profile.main10".to_string(),
            ],
        );

        assert!(registry.supports(&h264_session, "media.codec.h264"));
        assert!(!registry.supports(&h264_session, "media.codec.hevc"));
        assert!(registry.supports(&hevc_session, "media.profile.main10"));
        assert_eq!(
            registry.remove(&h264_session),
            Some(vec![
                "media.codec.h264".to_string(),
                "media.color_mode_v1".to_string()
            ])
        );
        assert!(!registry.supports(&h264_session, "media.codec.h264"));
        assert!(registry.supports(&hevc_session, "media.codec.hevc"));
    }
}
