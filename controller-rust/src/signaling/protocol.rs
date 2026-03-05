use serde::{Deserialize, Serialize};
use webrtc::{
    ice_transport::ice_candidate::RTCIceCandidateInit,
    peer_connection::sdp::session_description::RTCSessionDescription,
};

/// 信令消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SignalingMessage {
    /// 设备相关消息
    #[serde(rename = "device")]
    Device(DeviceMessage),
    /// WebRTC 相关消息
    #[serde(rename = "webrtc")]
    WebRTC(WebRTCMessage),
    /// 系统消息
    #[serde(rename = "system")]
    System(SystemMessage),
}

impl SignalingMessage {
    pub fn action(&self) -> &'static str {
        match self {
            SignalingMessage::Device(msg) => msg.action(),
            SignalingMessage::WebRTC(msg) => msg.action(),
            SignalingMessage::System(msg) => msg.action(),
        }
    }
}

/// 设备消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceMessage {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<DevicePayload>,
}

impl DeviceMessage {
    pub fn action(&self) -> &'static str {
        match self.action.as_str() {
            "register" => "register",
            "registered" => "registered",
            "deviceList" => "deviceList",
            "offline" => "offline",
            _ => "unknown",
        }
    }
}

/// WebRTC 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRTCMessage {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<WebRTCPayload>,
}

impl WebRTCMessage {
    pub fn action(&self) -> &'static str {
        match self.action.as_str() {
            "offer" => "offer",
            "answer" => "answer",
            "iceCandidate" => "iceCandidate",
            _ => "unknown",
        }
    }
}

/// 系统消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMessage {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<SystemPayload>,
}

impl SystemMessage {
    pub fn action(&self) -> &'static str {
        match self.action.as_str() {
            "connected" => "connected",
            _ => "unknown",
        }
    }
}

/// 设备载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevicePayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(rename = "type")]
    pub device_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_list: Option<Vec<DeviceInfo>>,
}

/// WebRTC 载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebRTCPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controller_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offer: Option<SessionDescriptionJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<SessionDescriptionJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<IceCandidateJson>,
}

/// 系统载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// 会话描述 JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptionJson {
    #[serde(rename = "type")]
    pub sdp_type: String,
    pub sdp: String,
}

impl From<RTCSessionDescription> for SessionDescriptionJson {
    fn from(sd: RTCSessionDescription) -> Self {
        Self {
            sdp_type: sd.sdp_type.to_string(),
            sdp: sd.sdp,
        }
    }
}

impl TryFrom<SessionDescriptionJson> for RTCSessionDescription {
    type Error = anyhow::Error;

    fn try_from(json: SessionDescriptionJson) -> Result<Self, Self::Error> {
        match json.sdp_type.as_str() {
            "offer" => Ok(RTCSessionDescription::offer(json.sdp)?),
            "answer" => Ok(RTCSessionDescription::answer(json.sdp)?),
            "pranswer" => Ok(RTCSessionDescription::pranswer(json.sdp)?),
            _ => Err(anyhow::anyhow!("Unknown SDP type: {}", json.sdp_type)),
        }
    }
}

/// ICE 候选 JSON
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IceCandidateJson {
    #[serde(rename = "candidate")]
    pub candidate: String,
    #[serde(rename = "sdpMid")]
    pub sdp_mid: Option<String>,
    #[serde(rename = "sdpMLineIndex")]
    pub sdp_mline_index: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicTransportInfo {
    pub addr: String,
    #[serde(rename = "serverName")]
    pub server_name: String,
    #[serde(rename = "certDerBase64")]
    pub cert_der_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioQuicTransportInfo {
    pub addr: String,
    #[serde(rename = "serverName")]
    pub server_name: String,
    #[serde(rename = "certDerBase64")]
    pub cert_der_base64: String,
    pub codec: String,
    #[serde(rename = "sampleRate")]
    pub sample_rate: u32,
    pub channels: u16,
}

impl From<RTCIceCandidateInit> for IceCandidateJson {
    fn from(cand: RTCIceCandidateInit) -> Self {
        Self {
            candidate: cand.candidate,
            sdp_mid: cand.sdp_mid,
            sdp_mline_index: cand.sdp_mline_index,
        }
    }
}

impl TryFrom<IceCandidateJson> for RTCIceCandidateInit {
    type Error = anyhow::Error;

    fn try_from(json: IceCandidateJson) -> Result<Self, Self::Error> {
        Ok(RTCIceCandidateInit {
            candidate: json.candidate,
            sdp_mid: json.sdp_mid,
            sdp_mline_index: json.sdp_mline_index,
            ..Default::default()
        })
    }
}

/// 设备信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub online: bool,
}

/// 信令消息载荷枚举（用于事件处理）
#[derive(Debug, Clone)]
pub enum SignalingMessagePayload {
    Connected {
        device_id: String,
    },
    Registered {
        device_id: String,
        device_list: Vec<DeviceInfo>,
    },
    DeviceList {
        device_list: Vec<DeviceInfo>,
    },
    DeviceOffline {
        device_id: String,
    },
    Offer {
        target_device_id: String,
        controller_id: String,
        session_id: String,
        offer: RTCSessionDescription,
    },
    Answer {
        answer: RTCSessionDescription,
        controller_id: String,
        selected_transport: String,
        quic: Option<QuicTransportInfo>,
        audio_quic: Option<AudioQuicTransportInfo>,
    },
    IceCandidate {
        target_device_id: Option<String>,
        controller_id: Option<String>,
        candidate: RTCIceCandidateInit,
    },
}

/// 创建注册消息
pub fn create_register_message(name: &str) -> String {
    serde_json::json!({
        "type": "device",
        "action": "register",
        "payload": {
            "type": "controller",
            "name": name,
            "protocolVersion": 2,
            "transports": ["webrtc", "quic"],
            "capabilities": {
                "protocols": ["webrtc", "quic"],
                "platforms": ["windows", "linux", "macos"],
                "codecs": ["h264"],
                "features": ["multi-end-compat", "capability-negotiation"]
            }
        }
    })
    .to_string()
}

/// 创建 Offer 消息
pub fn create_offer_message(
    target_device_id: &str,
    offer: &RTCSessionDescription,
    session_id: &str,
    controller_id: &str,
) -> String {
    let offer_json = SessionDescriptionJson::from(offer.clone());
    serde_json::json!({
        "type": "webrtc",
        "action": "offer",
        "payload": {
            "targetDeviceId": target_device_id,
            "offer": offer_json,
            "sessionId": session_id,
            "controllerId": controller_id
        }
    })
    .to_string()
}

/// 创建 ICE 候选消息
pub fn create_ice_candidate_message(
    target_device_id: &str,
    candidate: &RTCIceCandidateInit,
    controller_id: &str,
) -> String {
    let cand_json = IceCandidateJson::from(candidate.clone());
    serde_json::json!({
        "type": "webrtc",
        "action": "iceCandidate",
        "payload": {
            "targetDeviceId": target_device_id,
            "candidate": cand_json,
            "controllerId": controller_id
        }
    })
    .to_string()
}
