use std::collections::HashMap;

use mrd_proto::SessionId;
use mrd_signal_proto::{IceCandidate, SessionDescription};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebrtcSessionSnapshot {
    pub local_offer: Option<String>,
    pub remote_offer: Option<String>,
    pub remote_answer: Option<String>,
    pub remote_ice_candidates: Vec<IceCandidate>,
}

#[derive(Debug, Default)]
pub struct WebrtcSessionCoordinator {
    sessions: HashMap<SessionId, WebrtcSessionSnapshot>,
}

impl WebrtcSessionCoordinator {
    pub fn create_local_offer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<SessionDescription, String> {
        let snapshot = self.sessions.entry(session_id.clone()).or_default();
        snapshot.local_offer = Some(sdp.clone());

        Ok(SessionDescription { session_id, sdp })
    }

    pub fn apply_remote_offer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let snapshot = self.sessions.entry(session_id).or_default();
        snapshot.remote_offer = Some(sdp);
        Ok(())
    }

    pub fn apply_remote_answer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let snapshot = self.sessions.entry(session_id).or_default();
        snapshot.remote_answer = Some(sdp);
        Ok(())
    }

    pub fn apply_remote_ice_candidate(
        &mut self,
        session_id: SessionId,
        candidate: IceCandidate,
    ) -> Result<(), String> {
        let snapshot = self.sessions.entry(session_id).or_default();
        snapshot.remote_ice_candidates.push(candidate);
        Ok(())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Option<&WebrtcSessionSnapshot> {
        self.sessions.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use mrd_proto::SessionId;
    use mrd_signal_proto::IceCandidate;

    use super::WebrtcSessionCoordinator;

    #[test]
    fn creating_local_offer_records_offer_state() {
        let mut coordinator = WebrtcSessionCoordinator::default();

        let offer = coordinator
            .create_local_offer(SessionId("session-1".into()), "offer-sdp".into())
            .expect("create local offer");

        assert_eq!(offer.session_id.0, "session-1");
        assert_eq!(offer.sdp, "offer-sdp");

        let snapshot = coordinator
            .snapshot(&SessionId("session-1".into()))
            .expect("offer snapshot");
        assert_eq!(snapshot.local_offer.as_deref(), Some("offer-sdp"));
        assert_eq!(snapshot.remote_offer, None);
        assert_eq!(snapshot.remote_answer, None);
    }

    #[test]
    fn applying_remote_answer_and_ice_updates_snapshot() {
        let mut coordinator = WebrtcSessionCoordinator::default();

        coordinator
            .apply_remote_answer(SessionId("session-1".into()), "answer-sdp".into())
            .expect("apply remote answer");
        coordinator
            .apply_remote_ice_candidate(
                SessionId("session-1".into()),
                IceCandidate {
                    session_id: SessionId("session-1".into()),
                    candidate: "candidate:1 1 UDP 123 127.0.0.1 5000 typ host".into(),
                    sdp_mid: Some("0".into()),
                    sdp_mline_index: Some(0),
                },
            )
            .expect("apply remote ice");

        let snapshot = coordinator
            .snapshot(&SessionId("session-1".into()))
            .expect("answer snapshot");
        assert_eq!(snapshot.remote_answer.as_deref(), Some("answer-sdp"));
        assert_eq!(snapshot.remote_ice_candidates.len(), 1);
    }
}
