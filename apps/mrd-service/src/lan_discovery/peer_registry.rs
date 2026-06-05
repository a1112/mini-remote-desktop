use mrd_ipc::LanPeerInfo;
use mrd_proto::DeviceId;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

#[derive(Debug, Default)]
pub(super) struct LanPeerRegistry {
    peers: HashMap<String, LanPeerRecord>,
}

impl LanPeerRegistry {
    pub(super) fn upsert(&mut self, peer: LanPeerRecord) {
        self.peers.insert(peer.device_id.clone(), peer);
    }

    pub(super) fn prune_stale(&mut self, now_ms: u64, ttl_ms: u64) {
        self.peers
            .retain(|_, peer| now_ms.saturating_sub(peer.last_seen_ms) <= ttl_ms);
    }

    pub(super) fn snapshot(&self, now_ms: u64) -> Vec<LanPeerInfo> {
        self.peers
            .values()
            .map(|peer| peer.to_peer_info(now_ms))
            .collect()
    }

    pub(super) fn control_addr(&self, device_id: &DeviceId) -> Option<SocketAddr> {
        self.peers
            .get(&device_id.0)
            .map(LanPeerRecord::control_addr)
    }

    pub(super) fn transports(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.peers
            .get(&device_id.0)
            .map(|peer| peer.transports.clone())
    }

    pub(super) fn media_capabilities(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.peers
            .get(&device_id.0)
            .map(LanPeerRecord::media_capabilities_with_transports)
    }
}

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
