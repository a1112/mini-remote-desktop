use std::{collections::HashMap, fmt, sync::Arc};

use bytes::Bytes;
use mrd_pipeline_core::EncodedAccessUnit;
use mrd_proto::SessionId;
use mrd_transport_webrtc::{
    ControlLane, IceCandidate, IceServerConfig, IceTransportPolicy, PeerConnectionConfig,
    SelectedCandidatePairStats, SessionDescription, WebRtcPeerConnection,
};
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum ServiceWebRtcTransportError {
    #[error("WebRTC session {0:?} already exists")]
    DuplicateSession(SessionId),
    #[error("WebRTC session {0:?} was not found")]
    SessionNotFound(SessionId),
    #[error("WebRTC transport failed: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayUrlClass {
    TurnUdp,
    TurnTcp,
    TurnsTcp,
    Unknown,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ServiceTurnRelayCredentials {
    pub urls: Vec<String>,
    pub username: String,
    pub credential: String,
    pub expires_at_unix_seconds: u64,
}

impl fmt::Debug for ServiceTurnRelayCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceTurnRelayCredentials")
            .field("url_classes", &self.url_classes())
            .field("username", &"[REDACTED]")
            .field("credential", &"[REDACTED]")
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

impl ServiceTurnRelayCredentials {
    pub fn apply_relay_only(&self, mut config: PeerConnectionConfig) -> PeerConnectionConfig {
        config.ice_servers = vec![IceServerConfig::new(
            self.urls.clone(),
            self.username.clone(),
            self.credential.clone(),
        )];
        config.ice_transport_policy = IceTransportPolicy::Relay;
        config
    }

    pub fn url_classes(&self) -> Vec<RelayUrlClass> {
        self.urls
            .iter()
            .map(|url| {
                if url.starts_with("turns:") {
                    RelayUrlClass::TurnsTcp
                } else if url.starts_with("turn:") && url.contains("transport=tcp") {
                    RelayUrlClass::TurnTcp
                } else if url.starts_with("turn:") && url.contains("transport=udp") {
                    RelayUrlClass::TurnUdp
                } else {
                    RelayUrlClass::Unknown
                }
            })
            .collect()
    }
}

#[derive(Debug, Default)]
pub struct ServiceWebRtcTransportHost {
    sessions: RwLock<HashMap<SessionId, Arc<WebRtcPeerConnection>>>,
}

impl ServiceWebRtcTransportHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn open_session(
        &self,
        session_id: SessionId,
        config: PeerConnectionConfig,
    ) -> Result<(), ServiceWebRtcTransportError> {
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Err(ServiceWebRtcTransportError::DuplicateSession(session_id));
            }
        }
        let peer = Arc::new(
            WebRtcPeerConnection::new(config)
                .await
                .map_err(transport_error)?,
        );
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            drop(sessions);
            let _ = peer.close().await;
            return Err(ServiceWebRtcTransportError::DuplicateSession(session_id));
        }
        sessions.insert(session_id, peer);
        Ok(())
    }

    pub async fn create_offer(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionDescription, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .create_offer()
            .await
            .map_err(transport_error)
    }

    pub async fn accept_offer(
        &self,
        session_id: &SessionId,
        offer: SessionDescription,
    ) -> Result<SessionDescription, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .accept_offer(offer)
            .await
            .map_err(transport_error)
    }

    pub async fn accept_answer(
        &self,
        session_id: &SessionId,
        answer: SessionDescription,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .accept_answer(answer)
            .await
            .map_err(transport_error)
    }

    pub async fn next_local_candidate(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<IceCandidate>, ServiceWebRtcTransportError> {
        Ok(self.session(session_id).await?.next_local_candidate().await)
    }

    pub async fn add_ice_candidate(
        &self,
        session_id: &SessionId,
        candidate: IceCandidate,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .add_ice_candidate(candidate)
            .await
            .map_err(transport_error)
    }

    pub async fn wait_connected(
        &self,
        session_id: &SessionId,
    ) -> Result<(), ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .wait_connected()
            .await
            .map_err(transport_error)
    }

    pub async fn send_video(
        &self,
        session_id: &SessionId,
        access_unit: &EncodedAccessUnit,
    ) -> Result<usize, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .send_h264_access_unit(access_unit)
            .await
            .map_err(transport_error)
    }

    pub async fn next_video(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<EncodedAccessUnit>, ServiceWebRtcTransportError> {
        Ok(self
            .session(session_id)
            .await?
            .next_h264_access_unit()
            .await)
    }

    pub async fn send_control(
        &self,
        session_id: &SessionId,
        lane: ControlLane,
        payload: &[u8],
    ) -> Result<usize, ServiceWebRtcTransportError> {
        self.session(session_id)
            .await?
            .send_control(lane, payload)
            .await
            .map_err(transport_error)
    }

    pub async fn next_control(
        &self,
        session_id: &SessionId,
        lane: ControlLane,
    ) -> Result<Option<Bytes>, ServiceWebRtcTransportError> {
        Ok(self.session(session_id).await?.next_control(lane).await)
    }

    pub async fn selected_candidate_pair_stats(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SelectedCandidatePairStats>, ServiceWebRtcTransportError> {
        Ok(self
            .session(session_id)
            .await?
            .selected_candidate_pair_stats()
            .await)
    }

    pub async fn close_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), ServiceWebRtcTransportError> {
        let peer = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .ok_or_else(|| ServiceWebRtcTransportError::SessionNotFound(session_id.clone()))?;
        peer.close().await.map_err(transport_error)
    }

    pub async fn shutdown(&self) -> Result<(), ServiceWebRtcTransportError> {
        let sessions = std::mem::take(&mut *self.sessions.write().await);
        let mut first_error = None;
        for peer in sessions.into_values() {
            if let Err(error) = peer.close().await {
                first_error.get_or_insert_with(|| transport_error(error));
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }

    async fn session(
        &self,
        session_id: &SessionId,
    ) -> Result<Arc<WebRtcPeerConnection>, ServiceWebRtcTransportError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| ServiceWebRtcTransportError::SessionNotFound(session_id.clone()))
    }
}

fn transport_error(error: mrd_transport_webrtc::TransportError) -> ServiceWebRtcTransportError {
    ServiceWebRtcTransportError::Transport(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_transport_webrtc::PeerConnectionRole;

    fn credentials() -> ServiceTurnRelayCredentials {
        ServiceTurnRelayCredentials {
            urls: vec!["turn:relay.example.test:3478?transport=udp".into()],
            username: "temporary-user".into(),
            credential: "temporary-password".into(),
            expires_at_unix_seconds: 1_800_000_000,
        }
    }

    #[test]
    fn relay_credentials_force_relay_policy_without_debug_secret_leakage() {
        let credentials = credentials();
        let config = credentials.apply_relay_only(PeerConnectionConfig {
            role: PeerConnectionRole::Offerer,
            ..PeerConnectionConfig::default()
        });
        assert_eq!(config.ice_transport_policy, IceTransportPolicy::Relay);
        assert_eq!(config.ice_servers.len(), 1);
        assert_eq!(credentials.url_classes(), vec![RelayUrlClass::TurnUdp]);

        let debug = format!("{credentials:?}");
        assert!(!debug.contains("temporary-user"));
        assert!(!debug.contains("temporary-password"));
        assert!(debug.contains("TurnUdp"));
    }
}
