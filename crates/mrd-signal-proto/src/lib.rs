use mrd_proto::{BackendRole, DeviceId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload")]
pub enum SignalMessage {
    Register(RegisterRequest),
    SessionRequest(SessionRequest),
    SessionAccept(SessionAccept),
    WebrtcOffer(SessionDescription),
    WebrtcAnswer(SessionDescription),
    IceCandidate(IceCandidate),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterRequest {
    pub role: BackendRole,
    pub device_id: Option<DeviceId>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRequest {
    pub session_id: SessionId,
    pub source_device_id: DeviceId,
    pub target_device_id: DeviceId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionAccept {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDescription {
    pub session_id: SessionId,
    pub sdp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IceCandidate {
    pub session_id: SessionId,
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_mline_index: Option<u16>,
}
