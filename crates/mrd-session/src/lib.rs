use mrd_proto::{BackendRole, DeviceId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilitySet {
    pub supports_webrtc: bool,
    pub supports_quic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionPlan {
    pub session_id: SessionId,
    pub initiator: DeviceId,
    pub target: DeviceId,
    pub role: BackendRole,
    pub capabilities: CapabilitySet,
}
