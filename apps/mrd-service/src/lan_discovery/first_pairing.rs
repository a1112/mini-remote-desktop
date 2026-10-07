//! Short-lived public candidates for explicit first pairing. Discovery must
//! already have accepted the signature, UDP source and replay admission. A
//! candidate ID is a UI correlation value and never grants session authority.

use super::SignedLanAnnouncement;
use crate::app_state::AuthenticatedPeerTrust;
use anyhow::Result;
use mrd_ipc::{DecimalU64, LanPairingApproval, LanPairingCandidate, RemotePermissionScope};
use mrd_proto::DeviceId;
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

const MAX_PAIRING_CANDIDATES: usize = 128;

#[derive(Debug, Default)]
pub(super) struct LanPairingCandidates {
    peers: BTreeMap<String, StoredCandidate>,
}

#[derive(Debug)]
struct StoredCandidate {
    peer: VerifiedLanPairingPeer,
    signed: SignedLanAnnouncement,
}

#[derive(Debug, Clone)]
pub(super) struct VerifiedLanPairingPeer {
    pub(super) candidate_id: String,
    pub(super) device_id: DeviceId,
    pub(super) device_name: String,
    pub(super) peer_key_id: String,
    pub(super) public_key: Vec<u8>,
    pub(super) key_epoch: u64,
    pub(super) discovery_endpoint: SocketAddr,
    pub(super) expires_at_ms: u64,
    pub(super) valid_until: Instant,
}

impl LanPairingCandidates {
    pub(super) fn observe_verified(
        &mut self,
        signed: &SignedLanAnnouncement,
        trust: AuthenticatedPeerTrust,
        observed_at_ms: u64,
        received_at: Instant,
        peer_ttl: Duration,
    ) -> Result<()> {
        self.prune_expired(observed_at_ms, received_at);
        // Reverify instead of trusting an accidental direct call that skipped
        // the UDP receiver's authenticated admission. Source/replay admission
        // still belongs to that receiver, under its security gate.
        signed.verify(observed_at_ms)?;
        let key_id = &signed.payload.signer_key_id;
        if trust != AuthenticatedPeerTrust::Untrusted {
            self.remove_peer(key_id);
            return Ok(());
        }
        let remaining_ms = signed
            .payload
            .expires_at_ms
            .checked_sub(observed_at_ms)
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| anyhow::anyhow!("first pairing signed announcement expired"))?;
        let freshness = peer_ttl.min(Duration::from_millis(remaining_ms));
        let freshness_ms = u64::try_from(freshness.as_millis())?;
        if freshness_ms == 0 {
            anyhow::bail!("first pairing candidate freshness is empty");
        }
        let expires_at_ms = observed_at_ms
            .checked_add(freshness_ms)
            .ok_or_else(|| anyhow::anyhow!("first pairing expiry is invalid"))?;
        let valid_until = received_at
            .checked_add(freshness)
            .ok_or_else(|| anyhow::anyhow!("first pairing monotonic deadline is invalid"))?;
        let announcement = &signed.payload.announcement;
        if announcement.device_id.len() > 128
            || announcement.device_name.len() > 256
            || announcement.device_id.chars().any(char::is_control)
            || announcement.device_name.chars().any(char::is_control)
        {
            anyhow::bail!("first pairing public identity is outside its bounded contract");
        }
        let candidate_id = if let Some(previous) = self.peers.get(key_id) {
            if previous.signed.payload.nonce == signed.payload.nonce {
                anyhow::bail!("first pairing announcement nonce was already observed");
            }
            if same_binding(&previous.peer, signed) {
                previous.peer.candidate_id.clone()
            } else {
                self.new_candidate_id()?
            }
        } else {
            if self.peers.len() >= MAX_PAIRING_CANDIDATES {
                anyhow::bail!("first pairing candidate cache is full");
            }
            self.new_candidate_id()?
        };
        self.peers.insert(
            key_id.clone(),
            StoredCandidate {
                peer: VerifiedLanPairingPeer {
                    candidate_id,
                    device_id: DeviceId(announcement.device_id.clone()),
                    device_name: announcement.device_name.clone(),
                    peer_key_id: key_id.clone(),
                    public_key: signed.public_key.clone(),
                    key_epoch: signed.payload.signer_key_epoch,
                    discovery_endpoint: signed.payload.discovery_endpoint,
                    expires_at_ms,
                    valid_until,
                },
                signed: signed.clone(),
            },
        );
        Ok(())
    }

    pub(super) fn snapshot(&mut self, now_ms: u64, now: Instant) -> Vec<LanPairingCandidate> {
        self.prune_expired(now_ms, now);
        self.peers
            .values()
            .filter(|entry| !self.is_ambiguous(&entry.peer))
            .map(|entry| LanPairingCandidate {
                candidate_id: entry.peer.candidate_id.clone(),
                device_id: entry.peer.device_id.clone(),
                device_name: entry.peer.device_name.clone(),
                peer_key_id: entry.peer.peer_key_id.clone(),
                key_epoch: DecimalU64::new(entry.peer.key_epoch),
                discovery_endpoint: entry.peer.discovery_endpoint.to_string(),
                expires_at_ms: entry.peer.expires_at_ms,
                permission_ceiling: vec![RemotePermissionScope::ScreenView],
            })
            .collect()
    }

    pub(super) fn resolve(
        &mut self,
        approval: &LanPairingApproval,
        now_ms: u64,
        now: Instant,
    ) -> Result<VerifiedLanPairingPeer> {
        self.prune_expired(now_ms, now);
        if approval.permission_ceiling != [RemotePermissionScope::ScreenView] {
            anyhow::bail!("first pairing requires exactly screen.view");
        }
        let entry = self
            .peers
            .get(&approval.peer_key_id)
            .ok_or_else(|| anyhow::anyhow!("first pairing candidate is absent or expired"))?;
        let peer = &entry.peer;
        if peer.candidate_id != approval.candidate_id
            || peer.device_id != approval.device_id
            || peer.peer_key_id != approval.peer_key_id
            || peer.key_epoch != approval.key_epoch.get()
            || peer.discovery_endpoint.to_string() != approval.discovery_endpoint
            || self.is_ambiguous(peer)
        {
            anyhow::bail!("first pairing candidate binding changed or is ambiguous");
        }
        entry.signed.verify(now_ms)?;
        Ok(peer.clone())
    }

    pub(super) fn remove_peer(&mut self, key_id: &str) {
        self.peers.remove(key_id);
    }

    fn prune_expired(&mut self, now_ms: u64, now: Instant) {
        self.peers
            .retain(|_, entry| now_ms < entry.peer.expires_at_ms && now < entry.peer.valid_until);
    }

    fn is_ambiguous(&self, peer: &VerifiedLanPairingPeer) -> bool {
        self.peers.values().any(|other| {
            other.peer.peer_key_id != peer.peer_key_id
                && (other.peer.device_id == peer.device_id
                    || other.peer.discovery_endpoint == peer.discovery_endpoint)
        })
    }

    fn new_candidate_id(&self) -> Result<String> {
        for _ in 0..4 {
            let mut bytes = [0_u8; 16];
            SystemRandom::new().fill(&mut bytes).map_err(|_| {
                anyhow::anyhow!("first pairing candidate identifier generation failed")
            })?;
            let candidate_id = bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if self
                .peers
                .values()
                .all(|entry| entry.peer.candidate_id != candidate_id)
            {
                return Ok(candidate_id);
            }
        }
        anyhow::bail!("first pairing candidate identifier collision")
    }
}

fn same_binding(peer: &VerifiedLanPairingPeer, signed: &SignedLanAnnouncement) -> bool {
    peer.device_id.0 == signed.payload.announcement.device_id
        // Renaming an identity that was displayed requires a new confirmation,
        // even though the approval DTO deliberately does not carry its name.
        && peer.device_name == signed.payload.announcement.device_name
        && peer.peer_key_id == signed.payload.signer_key_id
        && peer.public_key == signed.public_key
        && peer.key_epoch == signed.payload.signer_key_epoch
        && peer.discovery_endpoint == signed.payload.discovery_endpoint
}

#[cfg(test)]
mod tests {
    use super::super::{
        discovery_identity::{DISCOVERY_APP_ID, DISCOVERY_MAGIC},
        LanAnnouncement, SIGNED_LAN_PROTOCOL_VERSION,
    };
    use super::*;
    use mrd_identity::DeviceIdentity;
    use mrd_ipc::{DecimalU64, RemotePermissionScope};
    use ring::rand::SystemRandom;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::generate(&SystemRandom::new()).unwrap()
    }

    fn signed(
        identity: &DeviceIdentity,
        device: &str,
        endpoint: &str,
        epoch: u64,
        issued_at_ms: u64,
        expires_at_ms: u64,
        nonce: u8,
    ) -> SignedLanAnnouncement {
        let endpoint: SocketAddr = endpoint.parse().unwrap();
        SignedLanAnnouncement::sign(
            identity,
            epoch,
            LanAnnouncement {
                magic: DISCOVERY_MAGIC.to_owned(),
                app_id: DISCOVERY_APP_ID.to_owned(),
                instance_id: format!("instance-{device}"),
                device_id: device.to_owned(),
                device_name: format!("Device {device}"),
                device_type: "desktop".to_owned(),
                protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
                discovery_port: endpoint.port(),
                transports: vec!["quic".to_owned()],
                service_build_id: Some("pairing-test".to_owned()),
                media_protocol_version: Some(3),
                media_capabilities: Vec::new(),
                mac_address: None,
                timestamp_ms: issued_at_ms,
            },
            endpoint,
            expires_at_ms,
            [nonce; 16],
        )
        .unwrap()
    }

    fn approval(candidate: &LanPairingCandidate) -> LanPairingApproval {
        LanPairingApproval {
            candidate_id: candidate.candidate_id.clone(),
            device_id: candidate.device_id.clone(),
            peer_key_id: candidate.peer_key_id.clone(),
            key_epoch: candidate.key_epoch,
            discovery_endpoint: candidate.discovery_endpoint.clone(),
            permission_ceiling: vec![RemotePermissionScope::ScreenView],
        }
    }

    fn observe(
        cache: &mut LanPairingCandidates,
        signed: &SignedLanAnnouncement,
        wall: u64,
        mono: Instant,
    ) {
        cache
            .observe_verified(
                signed,
                AuthenticatedPeerTrust::Untrusted,
                wall,
                mono,
                Duration::from_secs(5),
            )
            .unwrap();
    }

    #[test]
    fn signed_first_discovery_exposes_only_screen_view_and_resolves_internal_public_key() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let message = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
        observe(&mut cache, &message, 1000, now);
        let candidates = cache.snapshot(1000, now);
        assert_eq!(
            candidates.len(),
            1,
            "verified untrusted discovery needs a pairing candidate"
        );
        let candidate = &candidates[0];
        assert_eq!(
            candidate.permission_ceiling,
            [RemotePermissionScope::ScreenView]
        );
        assert_eq!(candidate.key_epoch, DecimalU64::new(1));
        let peer = cache.resolve(&approval(candidate), 1000, now).unwrap();
        assert_eq!(peer.public_key, key.public_key());
        assert_eq!(peer.peer_key_id, key.key_id());
        assert_eq!(peer.device_id, DeviceId("device-a".to_owned()));
        assert_eq!(peer.device_name, "Device device-a");
        assert_eq!(peer.key_epoch, 1);
        assert_eq!(peer.discovery_endpoint.to_string(), "192.168.1.2:21116");
        assert_eq!(peer.expires_at_ms, 6000);
        assert_eq!(peer.candidate_id, candidate.candidate_id);
    }

    #[test]
    fn continuous_fresh_announcements_renew_one_candidate_and_both_deadlines() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
            1000,
            now,
        );
        let first = cache.snapshot(1000, now).remove(0);
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 5000, 10000, 2),
            5000,
            now + Duration::from_secs(4),
        );
        let renewed = cache.snapshot(6000, now + Duration::from_secs(5)).remove(0);
        assert_eq!(first.candidate_id, renewed.candidate_id);
        assert_eq!(renewed.expires_at_ms, 10000);
        cache
            .resolve(&approval(&first), 6000, now + Duration::from_secs(5))
            .unwrap();
        assert!(cache
            .snapshot(10000, now + Duration::from_secs(9))
            .is_empty());
    }

    #[test]
    fn expired_gap_changes_candidate_id_and_rejects_previous_approval() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
            1000,
            now,
        );
        let first = cache.snapshot(1000, now).remove(0);
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 7000, 12000, 2),
            7000,
            now + Duration::from_secs(6),
        );
        let replacement = cache.snapshot(7000, now + Duration::from_secs(6)).remove(0);
        assert_ne!(first.candidate_id, replacement.candidate_id);
        assert!(cache
            .resolve(&approval(&first), 7000, now + Duration::from_secs(6))
            .is_err());
    }

    #[test]
    fn wall_expiry_and_monotonic_freshness_each_independently_expire_candidates() {
        for (wall, elapsed) in [(6000, 1), (1000, 5), (500, 5)] {
            let key = identity();
            let now = Instant::now();
            let mut cache = LanPairingCandidates::default();
            observe(
                &mut cache,
                &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
                1000,
                now,
            );
            let first = cache.snapshot(1000, now).remove(0);
            assert!(cache
                .resolve(&approval(&first), wall, now + Duration::from_secs(elapsed))
                .is_err());
            assert!(cache
                .snapshot(wall, now + Duration::from_secs(elapsed))
                .is_empty());
        }
    }

    #[test]
    fn binding_changes_replace_the_candidate_even_before_the_old_lease_expires() {
        for (device, endpoint, epoch) in [
            ("device-b", "192.168.1.2:21116", 1),
            ("device-a", "192.168.1.3:21116", 1),
            ("device-a", "192.168.1.2:21116", 2),
        ] {
            let key = identity();
            let now = Instant::now();
            let mut cache = LanPairingCandidates::default();
            observe(
                &mut cache,
                &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
                1000,
                now,
            );
            let first = cache.snapshot(1000, now).remove(0);
            observe(
                &mut cache,
                &signed(&key, device, endpoint, epoch, 2000, 7000, 2),
                2000,
                now + Duration::from_secs(1),
            );
            let replacement = cache.snapshot(2000, now + Duration::from_secs(1)).remove(0);
            assert_ne!(first.candidate_id, replacement.candidate_id);
            assert!(cache
                .resolve(&approval(&first), 2000, now + Duration::from_secs(1))
                .is_err());
        }
    }

    #[test]
    fn approval_requires_every_displayed_binding_and_exact_screen_view_scope() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
            1000,
            now,
        );
        let candidate = cache.snapshot(1000, now).remove(0);
        for field in 0..7 {
            let mut request = approval(&candidate);
            match field {
                0 => request.candidate_id = "unknown".to_owned(),
                1 => request.device_id = DeviceId("other-device".to_owned()),
                2 => request.peer_key_id = "unknown-key".to_owned(),
                3 => request.key_epoch = DecimalU64::new(2),
                4 => request.discovery_endpoint = "192.168.1.3:21116".to_owned(),
                5 => request
                    .permission_ceiling
                    .push(RemotePermissionScope::InputPointer),
                _ => request.permission_ceiling.clear(),
            }
            assert!(cache.resolve(&request, 1000, now).is_err());
        }
        cache.resolve(&approval(&candidate), 1000, now).unwrap();
    }

    #[test]
    fn non_untrusted_classifications_remove_the_previous_candidate() {
        for trust in [
            AuthenticatedPeerTrust::Trusted,
            AuthenticatedPeerTrust::Suspended,
            AuthenticatedPeerTrust::Revoked,
            AuthenticatedPeerTrust::EpochMismatch,
        ] {
            let key = identity();
            let now = Instant::now();
            let mut cache = LanPairingCandidates::default();
            let first = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
            observe(&mut cache, &first, 1000, now);
            let candidate = cache.snapshot(1000, now).remove(0);
            cache
                .observe_verified(
                    &signed(&key, "device-a", "192.168.1.2:21116", 1, 2000, 7000, 2),
                    trust,
                    2000,
                    now + Duration::from_secs(1),
                    Duration::from_secs(5),
                )
                .unwrap();
            assert!(cache
                .snapshot(2000, now + Duration::from_secs(1))
                .is_empty());
            assert!(cache
                .resolve(&approval(&candidate), 2000, now + Duration::from_secs(1))
                .is_err());
        }
    }

    #[test]
    fn invalid_signature_and_expired_signed_proof_never_create_or_refresh_candidates() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let mut forged = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
        forged.signature[0] ^= 1;
        assert!(cache
            .observe_verified(
                &forged,
                AuthenticatedPeerTrust::Untrusted,
                1000,
                now,
                Duration::from_secs(5)
            )
            .is_err());
        let expired = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 2);
        assert!(cache
            .observe_verified(
                &expired,
                AuthenticatedPeerTrust::Untrusted,
                6000,
                now,
                Duration::from_secs(5)
            )
            .is_err());
        assert!(cache.snapshot(1000, now).is_empty());
    }

    #[test]
    fn ambiguous_device_or_endpoint_bindings_cannot_be_displayed_or_approved() {
        for (second_device, second_endpoint) in [
            ("device-a", "192.168.1.3:21116"),
            ("device-b", "192.168.1.2:21116"),
        ] {
            let key = identity();
            let other_key = identity();
            let now = Instant::now();
            let mut cache = LanPairingCandidates::default();
            observe(
                &mut cache,
                &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
                1000,
                now,
            );
            let first = cache.snapshot(1000, now).remove(0);
            observe(
                &mut cache,
                &signed(&other_key, second_device, second_endpoint, 1, 2000, 7000, 2),
                2000,
                now + Duration::from_secs(1),
            );
            assert!(cache
                .snapshot(2000, now + Duration::from_secs(1))
                .is_empty());
            assert!(cache
                .resolve(&approval(&first), 2000, now + Duration::from_secs(1))
                .is_err());
        }
    }

    #[test]
    fn explicit_removal_invalidates_the_candidate() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        observe(
            &mut cache,
            &signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1),
            1000,
            now,
        );
        let candidate = cache.snapshot(1000, now).remove(0);
        cache.remove_peer(key.key_id());
        assert!(cache.snapshot(1000, now).is_empty());
        assert!(cache.resolve(&approval(&candidate), 1000, now).is_err());
    }

    #[test]
    fn cache_is_bounded_and_full_cache_can_still_refresh_an_existing_candidate() {
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let key = identity();
        observe(
            &mut cache,
            &signed(&key, "device-0", "192.168.1.1:21116", 1, 1000, 6000, 1),
            1000,
            now,
        );
        let first = cache.snapshot(1000, now).remove(0);
        for n in 1..128 {
            let key = identity();
            let endpoint = format!("192.168.1.{}:21116", n + 1);
            observe(
                &mut cache,
                &signed(&key, &format!("device-{n}"), &endpoint, 1, 1000, 6000, 1),
                1000,
                now,
            );
        }
        assert_eq!(cache.snapshot(1000, now).len(), 128);
        let extra = identity();
        assert!(cache
            .observe_verified(
                &signed(&extra, "extra", "192.168.1.200:21116", 1, 1000, 6000, 1),
                AuthenticatedPeerTrust::Untrusted,
                1000,
                now,
                Duration::from_secs(5)
            )
            .is_err());
        observe(
            &mut cache,
            &signed(&key, "device-0", "192.168.1.1:21116", 1, 2000, 7000, 2),
            2000,
            now + Duration::from_secs(1),
        );
        assert_eq!(
            cache.snapshot(2000, now + Duration::from_secs(1)).len(),
            128
        );
        cache
            .resolve(&approval(&first), 2000, now + Duration::from_secs(1))
            .unwrap();
    }

    #[test]
    fn short_local_ttl_caps_the_displayed_expiry_and_late_transaction_deadline() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let proof = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
        cache
            .observe_verified(
                &proof,
                AuthenticatedPeerTrust::Untrusted,
                1000,
                now,
                Duration::from_secs(2),
            )
            .unwrap();
        let candidate = cache.snapshot(1000, now).remove(0);
        let peer = cache.resolve(&approval(&candidate), 1000, now).unwrap();
        assert_eq!(candidate.expires_at_ms, 3000);
        assert_eq!(peer.valid_until, now + Duration::from_secs(2));
        assert!(cache
            .resolve(&approval(&candidate), 1000, now + Duration::from_secs(2))
            .is_err());
        assert!(cache
            .observe_verified(
                &proof,
                AuthenticatedPeerTrust::Untrusted,
                1000,
                now,
                Duration::ZERO
            )
            .is_err());
    }

    #[test]
    fn changing_the_displayed_device_name_requires_a_new_confirmation() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let original = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
        observe(&mut cache, &original, 1000, now);
        let first = cache.snapshot(1000, now).remove(0);
        let mut renamed = original.payload.announcement.clone();
        renamed.device_name = "Different displayed identity".to_owned();
        renamed.timestamp_ms = 2000;
        let renewed = SignedLanAnnouncement::sign(
            &key,
            1,
            renamed,
            original.payload.discovery_endpoint,
            7000,
            [2; 16],
        )
        .unwrap();
        observe(&mut cache, &renewed, 2000, now + Duration::from_secs(1));
        let changed = cache.snapshot(2000, now + Duration::from_secs(1)).remove(0);
        assert_ne!(first.candidate_id, changed.candidate_id);
        assert!(cache
            .resolve(&approval(&first), 2000, now + Duration::from_secs(1))
            .is_err());
        assert_eq!(changed.device_name, "Different displayed identity");
    }

    #[test]
    fn cached_proof_cannot_be_refreshed_by_a_forgery_or_repeated_nonce() {
        let key = identity();
        let now = Instant::now();
        let mut cache = LanPairingCandidates::default();
        let original = signed(&key, "device-a", "192.168.1.2:21116", 1, 1000, 6000, 1);
        observe(&mut cache, &original, 1000, now);
        let first = cache.snapshot(1000, now).remove(0);
        let mut forged = signed(&key, "device-a", "192.168.1.2:21116", 1, 4000, 9000, 2);
        forged.signature[0] ^= 1;
        assert!(cache
            .observe_verified(
                &forged,
                AuthenticatedPeerTrust::Untrusted,
                4000,
                now + Duration::from_secs(3),
                Duration::from_secs(5)
            )
            .is_err());
        assert!(cache
            .observe_verified(
                &original,
                AuthenticatedPeerTrust::Untrusted,
                4000,
                now + Duration::from_secs(3),
                Duration::from_secs(5)
            )
            .is_err());
        let untouched = cache.snapshot(4000, now + Duration::from_secs(3)).remove(0);
        assert_eq!(untouched.candidate_id, first.candidate_id);
        assert_eq!(untouched.expires_at_ms, first.expires_at_ms);
        assert_eq!(
            cache
                .resolve(&approval(&untouched), 4000, now + Duration::from_secs(3))
                .unwrap()
                .valid_until,
            now + Duration::from_secs(5)
        );
        assert!(cache
            .snapshot(5000, now + Duration::from_secs(5))
            .is_empty());
    }
}
