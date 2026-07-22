use crate::app_state::AuthenticatedPeerTrust;
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
        self.peers.insert(peer.registry_key(), peer);
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
        self.controllable_peer(device_id)
            .map(|peer| peer.control_addr())
    }

    pub(super) fn transports(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.controllable_peer(device_id)
            .map(|peer| peer.transports.clone())
    }

    pub(super) fn media_capabilities(&self, device_id: &DeviceId) -> Option<Vec<String>> {
        self.controllable_peer(device_id)
            .map(|peer| peer.media_capabilities_with_transports())
    }

    pub(super) fn controllable_peer(&self, device_id: &DeviceId) -> Option<LanPeerRecord> {
        let mut matches = self
            .peers
            .values()
            .filter(|peer| peer.device_id == device_id.0 && peer.is_controllable());
        let peer = matches.next()?.clone();
        if matches.next().is_some() {
            return None;
        }
        Some(peer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LanPeerAuthentication {
    Signed(AuthenticatedPeerTrust),
    LegacyDiagnostic,
}

impl LanPeerAuthentication {
    fn is_controllable(self) -> bool {
        matches!(self, Self::Signed(trust) if trust.is_controllable())
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
    pub(super) mac_address: Option<String>,
    pub(super) peer_key_id: Option<String>,
    pub(super) public_key: Option<Vec<u8>>,
    pub(super) key_epoch: Option<u64>,
    pub(super) authentication: LanPeerAuthentication,
    pub(super) last_seen_ms: u64,
}

impl LanPeerRecord {
    fn registry_key(&self) -> String {
        self.peer_key_id
            .clone()
            .map(|key_id| format!("signed:{key_id}"))
            .unwrap_or_else(|| format!("legacy:{}", self.device_id))
    }

    pub(super) fn is_controllable(&self) -> bool {
        self.authentication.is_controllable()
            && self.peer_key_id.is_some()
            && self.public_key.as_ref().is_some_and(|key| key.len() == 32)
            && self.key_epoch.is_some_and(|epoch| epoch > 0)
    }

    pub(super) fn control_addr(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.discovery_port)
    }

    pub(super) fn to_peer_info(&self, now_ms: u64) -> LanPeerInfo {
        let controllable = self.is_controllable();
        LanPeerInfo {
            device_id: DeviceId(self.device_id.clone()),
            device_name: self.device_name.clone(),
            device_type: self.device_type.clone(),
            ip: self.ip.to_string(),
            discovery_port: self.discovery_port,
            p2p_control_addr: if controllable {
                self.control_addr().to_string()
            } else {
                String::new()
            },
            transports: self.transports.clone(),
            protocol_version: self.protocol_version,
            service_build_id: self.service_build_id.clone(),
            media_protocol_version: self.media_protocol_version,
            media_capabilities: self.media_capabilities.clone(),
            mac_address: self.mac_address.clone(),
            age_ms: now_ms.saturating_sub(self.last_seen_ms),
            p2p_available: controllable,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_peer(
        key_id: &str,
        public_key_byte: u8,
        trust: AuthenticatedPeerTrust,
        ip: IpAddr,
    ) -> LanPeerRecord {
        LanPeerRecord {
            device_id: "shared-device-id".to_string(),
            device_name: format!("Peer {key_id}"),
            device_type: "rdesk".to_string(),
            ip,
            discovery_port: 21_116,
            transports: vec!["quic".to_string()],
            protocol_version: 2,
            service_build_id: None,
            media_protocol_version: Some(3),
            media_capabilities: vec!["quic_stream_media_v2".to_string()],
            mac_address: None,
            peer_key_id: Some(key_id.to_string()),
            public_key: Some(vec![public_key_byte; 32]),
            key_epoch: Some(1),
            authentication: LanPeerAuthentication::Signed(trust),
            last_seen_ms: 1_000,
        }
    }

    #[test]
    fn untrusted_key_with_same_device_id_cannot_replace_trusted_peer() {
        let mut registry = LanPeerRegistry::default();
        registry.upsert(signed_peer(
            "trusted-key",
            1,
            AuthenticatedPeerTrust::Trusted,
            "192.168.1.10".parse().unwrap(),
        ));
        registry.upsert(signed_peer(
            "attacker-key",
            2,
            AuthenticatedPeerTrust::Untrusted,
            "192.168.1.99".parse().unwrap(),
        ));

        let device_id = DeviceId("shared-device-id".to_string());
        assert_eq!(
            registry.control_addr(&device_id),
            Some("192.168.1.10:21116".parse().unwrap())
        );

        let snapshot = registry.snapshot(1_001);
        assert_eq!(snapshot.len(), 2);
        let controllable = snapshot
            .iter()
            .filter(|peer| peer.p2p_available)
            .collect::<Vec<_>>();
        assert_eq!(controllable.len(), 1);
        assert_eq!(controllable[0].ip, "192.168.1.10");
        assert_eq!(controllable[0].p2p_control_addr, "192.168.1.10:21116");
        assert!(snapshot
            .iter()
            .find(|peer| peer.ip == "192.168.1.99")
            .is_some_and(|peer| !peer.p2p_available && peer.p2p_control_addr.is_empty()));
    }
}
