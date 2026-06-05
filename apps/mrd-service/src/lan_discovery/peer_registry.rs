use mrd_ipc::LanPeerInfo;
use mrd_proto::DeviceId;
use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Clone)]
pub(super) struct LanPeerRecord {
    pub(super) device_id: String,
    pub(super) device_name: String,
    pub(super) device_type: String,
    pub(super) ip: IpAddr,
    pub(super) discovery_port: u16,
    pub(super) transports: Vec<String>,
    pub(super) protocol_version: u32,
    pub(super) service_build_id: Option<String>,
    pub(super) media_protocol_version: Option<u32>,
    pub(super) media_capabilities: Vec<String>,
    pub(super) last_seen_ms: u64,
}

impl LanPeerRecord {
    pub(super) fn control_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.discovery_port)
    }

    pub(super) fn to_peer_info(&self, now_ms: u64) -> LanPeerInfo {
        LanPeerInfo {
            device_id: DeviceId(self.device_id.clone()),
            device_name: self.device_name.clone(),
            device_type: self.device_type.clone(),
            ip: self.ip.to_string(),
            discovery_port: self.discovery_port,
            p2p_control_addr: self.control_addr().to_string(),
            transports: self.transports.clone(),
            protocol_version: self.protocol_version,
            service_build_id: self.service_build_id.clone(),
            media_protocol_version: self.media_protocol_version,
            media_capabilities: self.media_capabilities.clone(),
            age_ms: now_ms.saturating_sub(self.last_seen_ms),
            p2p_available: true,
        }
    }

    pub(super) fn media_capabilities_with_transports(&self) -> Vec<String> {
        let mut capabilities = self.media_capabilities.clone();
        for transport in &self.transports {
            if !capabilities
                .iter()
                .any(|capability| capability == transport)
            {
                capabilities.push(transport.clone());
            }
        }
        capabilities
    }
}
