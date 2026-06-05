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

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_ipc::{MediaProfile, MediaProfileNegotiation};
    use mrd_proto::SessionId;

    fn negotiation(codec: &str, width: u32, height: u32) -> MediaProfileNegotiation {
        let profile = MediaProfile {
            width,
            height,
            fps: 144,
            bitrate_mbps: 64,
            codec: codec.to_string(),
            ..MediaProfile::default()
        };
        MediaProfileNegotiation {
            requested: profile.clone(),
            selected: profile,
            status: "accepted".to_string(),
            reason: None,
            selected_source_id: None,
            selected_width: Some(width),
            selected_height: Some(height),
            downgrade_reason: None,
        }
    }

    #[test]
    fn profile_entries_are_scoped_by_session() {
        let first_session = SessionId("first-profile-session".to_string());
        let second_session = SessionId("second-profile-session".to_string());
        let first_profile = negotiation("hevc", 2560, 1440);
        let second_profile = negotiation("av1", 1920, 1080);
        let mut registry = MediaProfileRegistry::default();

        registry.set(first_session.clone(), first_profile.clone());
        registry.set(second_session.clone(), second_profile.clone());

        assert_eq!(registry.get(&first_session), Some(first_profile.clone()));
        assert_eq!(registry.get(&second_session), Some(second_profile));
        assert_eq!(registry.remove(&first_session), Some(first_profile));
        assert!(registry.get(&first_session).is_none());
        assert!(registry.get(&second_session).is_some());
    }
}
