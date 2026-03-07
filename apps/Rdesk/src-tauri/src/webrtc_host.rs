use std::{collections::HashMap, sync::Arc};

use mrd_proto::SessionId;
use mrd_signal_proto::{IceCandidate, SessionDescription};
use webrtc::{
    api::{media_engine::MediaEngine, setting_engine::SettingEngine, APIBuilder},
    data_channel::data_channel_init::RTCDataChannelInit,
    ice_transport::{ice_candidate::RTCIceCandidateInit, ice_server::RTCIceServer},
    peer_connection::{
        configuration::RTCConfiguration,
        sdp::session_description::RTCSessionDescription,
        RTCPeerConnection,
    },
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebrtcHostSnapshot {
    pub local_offer: Option<String>,
    pub remote_offer: Option<String>,
    pub local_answer: Option<String>,
    pub remote_answer: Option<String>,
    pub remote_ice_count: usize,
}

#[derive(Debug)]
struct HostedPeer {
    pc: Arc<RTCPeerConnection>,
    snapshot: WebrtcHostSnapshot,
}

#[derive(Debug, Default)]
pub struct WebrtcHost {
    sessions: HashMap<SessionId, HostedPeer>,
}

impl WebrtcHost {
    pub async fn create_offer(
        &mut self,
        session_id: SessionId,
    ) -> Result<SessionDescription, String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let offer = pc
            .create_offer(None)
            .await
            .map_err(|e| format!("创建 WebRTC offer 失败: {}", e))?;
        pc.set_local_description(offer.clone())
            .await
            .map_err(|e| format!("设置本地 offer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.snapshot.local_offer = Some(offer.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: offer.sdp,
        })
    }

    pub async fn apply_remote_offer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let description = RTCSessionDescription::offer(sdp.clone())
            .map_err(|e| format!("构造远端 offer 失败: {}", e))?;
        pc.set_remote_description(description)
            .await
            .map_err(|e| format!("设置远端 offer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.snapshot.remote_offer = Some(sdp);
        Ok(())
    }

    pub async fn create_answer(
        &mut self,
        session_id: SessionId,
    ) -> Result<SessionDescription, String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let answer = pc
            .create_answer(None)
            .await
            .map_err(|e| format!("创建 WebRTC answer 失败: {}", e))?;
        pc.set_local_description(answer.clone())
            .await
            .map_err(|e| format!("设置本地 answer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.snapshot.local_answer = Some(answer.sdp.clone());

        Ok(SessionDescription {
            session_id,
            sdp: answer.sdp,
        })
    }

    pub async fn apply_remote_answer(
        &mut self,
        session_id: SessionId,
        sdp: String,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let description = RTCSessionDescription::answer(sdp.clone())
            .map_err(|e| format!("构造远端 answer 失败: {}", e))?;
        pc.set_remote_description(description)
            .await
            .map_err(|e| format!("设置远端 answer 失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.snapshot.remote_answer = Some(sdp);
        Ok(())
    }

    pub async fn apply_remote_ice_candidate(
        &mut self,
        session_id: SessionId,
        candidate: IceCandidate,
    ) -> Result<(), String> {
        let pc = self.get_or_create_peer(&session_id).await?;
        let init = RTCIceCandidateInit {
            candidate: candidate.candidate.clone(),
            sdp_mid: candidate.sdp_mid.clone(),
            sdp_mline_index: candidate.sdp_mline_index,
            username_fragment: None,
        };
        pc.add_ice_candidate(init)
            .await
            .map_err(|e| format!("添加远端 ICE 候选失败: {}", e))?;

        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| format!("未找到 webrtc host 会话: {}", session_id.0))?;
        session.snapshot.remote_ice_count += 1;
        Ok(())
    }

    pub fn snapshot(&self, session_id: &SessionId) -> Option<&WebrtcHostSnapshot> {
        self.sessions.get(session_id).map(|peer| &peer.snapshot)
    }

    async fn get_or_create_peer(
        &mut self,
        session_id: &SessionId,
    ) -> Result<Arc<RTCPeerConnection>, String> {
        if let Some(peer) = self.sessions.get(session_id) {
            return Ok(peer.pc.clone());
        }

        let pc = build_peer_connection().await?;
        self.sessions.insert(
            session_id.clone(),
            HostedPeer {
                pc: pc.clone(),
                snapshot: WebrtcHostSnapshot::default(),
            },
        );
        Ok(pc)
    }
}

async fn build_peer_connection() -> Result<Arc<RTCPeerConnection>, String> {
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|e| format!("注册默认编解码器失败: {}", e))?;

    let mut setting_engine = SettingEngine::default();
    setting_engine.set_include_loopback_candidate(true);

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_setting_engine(setting_engine)
        .build();

    let pc = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .map_err(|e| format!("创建 PeerConnection 失败: {}", e))?,
    );

    pc.create_data_channel(
        "control",
        Some(RTCDataChannelInit {
            ordered: Some(false),
            max_retransmits: Some(0),
            ..Default::default()
        }),
    )
    .await
    .map_err(|e| format!("创建 control data channel 失败: {}", e))?;

    Ok(pc)
}

#[cfg(test)]
mod tests {
    use mrd_proto::SessionId;

    use super::WebrtcHost;

    #[tokio::test]
    async fn creating_offer_records_local_offer() {
        let mut host = WebrtcHost::default();

        let offer = host
            .create_offer(SessionId("session-1".into()))
            .await
            .expect("create offer");

        assert!(offer.sdp.contains("m=application"));
        let snapshot = host
            .snapshot(&SessionId("session-1".into()))
            .expect("host snapshot");
        assert!(snapshot.local_offer.as_deref().unwrap_or_default().contains("m=application"));
    }

    #[tokio::test]
    async fn offer_answer_roundtrip_between_two_hosts() {
        let mut controller = WebrtcHost::default();
        let mut agent = WebrtcHost::default();
        let session_id = SessionId("session-2".into());

        let offer = controller
            .create_offer(session_id.clone())
            .await
            .expect("controller offer");
        agent
            .apply_remote_offer(session_id.clone(), offer.sdp)
            .await
            .expect("agent apply offer");

        let answer = agent
            .create_answer(session_id.clone())
            .await
            .expect("agent answer");
        controller
            .apply_remote_answer(session_id.clone(), answer.sdp)
            .await
            .expect("controller apply answer");

        let controller_snapshot = controller
            .snapshot(&session_id)
            .expect("controller snapshot");
        let agent_snapshot = agent.snapshot(&session_id).expect("agent snapshot");

        assert!(controller_snapshot.local_offer.is_some());
        assert!(controller_snapshot.remote_answer.is_some());
        assert!(agent_snapshot.remote_offer.is_some());
        assert!(agent_snapshot.local_answer.is_some());
    }
}
