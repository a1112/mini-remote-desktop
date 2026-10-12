//! Registration-local paired authority, never an attended consent binding.
use mrd_agent_ipc::{
    validate_paired_session_authority, DesktopKind, ExecuteGrantVerifier, PairedSessionAuthority,
    PairedSessionAuthorityContext, RevokePairedSessionAuthority,
};
use mrd_proto::SessionId;
use std::{collections::HashMap, time::Duration};
use tokio::time::Instant;

#[derive(Clone)]
pub(crate) struct PairedSessionBinding {
    pub(crate) authority: PairedSessionAuthority,
    pub(crate) deadline: Instant,
}

#[derive(Default)]
pub(crate) struct PairedAuthorityRegistry {
    active: HashMap<SessionId, PairedSessionBinding>,
    // Registration-lifetime bounded tombstones prevent replay even after a
    // desktop transition, explicit revoke, or backwards wall-clock adjustment.
    seen: HashMap<[u8; 32], PairedSessionBinding>,
}
impl PairedAuthorityRegistry {
    pub(crate) fn install(
        &mut self,
        authority: &PairedSessionAuthority,
        context: &PairedSessionAuthorityContext,
        verifier: &dyn ExecuteGrantVerifier,
    ) -> bool {
        if validate_paired_session_authority(authority, context, verifier).is_err()
            || self.seen.contains_key(&authority.claims.authority_id)
            || self.active.contains_key(&authority.claims.session_id)
            || self.active.len() >= 64
            || self.seen.len() >= 4096
        {
            return false;
        }
        let binding = PairedSessionBinding {
            authority: authority.clone(),
            deadline: Instant::now()
                + Duration::from_millis(authority.claims.expires_at_ms - context.now_ms),
        };
        self.seen
            .insert(authority.claims.authority_id, binding.clone());
        self.active
            .insert(authority.claims.session_id.clone(), binding);
        true
    }
    pub(crate) fn resolve(&self, session: &SessionId, now_ms: u64) -> Option<PairedSessionBinding> {
        self.active
            .get(session)
            .filter(|binding| {
                now_ms >= binding.authority.claims.not_before_ms
                    && now_ms < binding.authority.claims.expires_at_ms
                    && Instant::now() < binding.deadline
            })
            .cloned()
    }
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.active.values().map(|binding| binding.deadline).min()
    }
    pub(crate) fn take_due(&mut self, now_ms: u64) -> Vec<PairedSessionBinding> {
        self.take_matching(|binding| {
            now_ms < binding.authority.claims.not_before_ms
                || now_ms >= binding.authority.claims.expires_at_ms
                || Instant::now() >= binding.deadline
        })
    }
    pub(crate) fn take_desktop_mismatch(
        &mut self,
        epoch: u64,
        kind: DesktopKind,
    ) -> Vec<PairedSessionBinding> {
        self.take_matching(|binding| {
            binding.authority.claims.desktop_epoch != epoch || kind != DesktopKind::Default
        })
    }
    fn take_matching(
        &mut self,
        predicate: impl Fn(&PairedSessionBinding) -> bool,
    ) -> Vec<PairedSessionBinding> {
        let sessions: Vec<_> = self
            .active
            .iter()
            .filter(|(_, binding)| predicate(binding))
            .map(|(session, _)| session.clone())
            .collect();
        sessions
            .into_iter()
            .filter_map(|session| self.active.remove(&session))
            .collect()
    }
    pub(crate) fn revoke(
        &mut self,
        request: &RevokePairedSessionAuthority,
    ) -> Option<(PairedSessionBinding, bool)> {
        let binding = self.seen.get(&request.authority_id)?;
        let claims = &binding.authority.claims;
        if claims.registration_id != request.registration_id
            || claims.registration_epoch != request.registration_epoch
            || claims.session_id != request.session_id
        {
            return None;
        }
        let binding = binding.clone();
        // A later authority for this session must never be removed by an old
        // tombstone's idempotent revoke.
        let was_active = self
            .active
            .get(&request.session_id)
            .is_some_and(|active| active.authority.claims.authority_id == request.authority_id);
        if was_active {
            self.active.remove(&request.session_id);
        }
        Some((binding, was_active))
    }
    pub(crate) fn drain(&mut self) -> Vec<PairedSessionBinding> {
        let bindings = self.active.drain().map(|(_, binding)| binding).collect();
        self.seen.clear();
        bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_agent_ipc::{DesktopKind, GrantAudience, PairedSessionAuthorityClaims, PeerBinding};
    use mrd_proto::{DeviceId, SessionId};
    use mrd_session::PermissionScope;
    struct Verified;
    impl ExecuteGrantVerifier for Verified {
        fn verify(&self, _: &[u8; 32], _: &[u8], _: &[u8; 64]) -> bool {
            true
        }
    }
    fn fixture() -> (PairedSessionAuthority, PairedSessionAuthorityContext) {
        (
            PairedSessionAuthority {
                claims: PairedSessionAuthorityClaims {
                    authority_id: [1; 32],
                    registration_id: [2; 16],
                    registration_epoch: 3,
                    session_id: SessionId("paired".into()),
                    peer: PeerBinding {
                        device_id: DeviceId("peer".into()),
                        key_id: [4; 32],
                    },
                    scopes: [PermissionScope::ScreenView].into_iter().collect(),
                    policy_revision: 5,
                    windows_session_id: 1,
                    desktop_epoch: 6,
                    desktop_kind: DesktopKind::Default,
                    issued_at_ms: 1000,
                    not_before_ms: 1000,
                    expires_at_ms: 2000,
                    audience: GrantAudience::SessionAgent,
                },
                issuer_key_id: [7; 32],
                signature: [8; 64],
            },
            PairedSessionAuthorityContext {
                registration_id: [2; 16],
                registration_epoch: 3,
                windows_session_id: 1,
                desktop_epoch: 6,
                desktop_kind: DesktopKind::Default,
                now_ms: 1500,
                expected_issuer_key_id: [7; 32],
            },
        )
    }
    #[test]
    fn fresh_verified_paired_authority_installs_once() {
        let (authority, context) = fixture();
        let mut registry = PairedAuthorityRegistry::default();
        assert!(registry.install(&authority, &context, &Verified));
        assert!(!registry.install(&authority, &context, &Verified));
    }
    #[test]
    fn expiry_and_desktop_change_retire_authority_without_forgetting_replay() {
        let (authority, context) = fixture();
        let mut registry = PairedAuthorityRegistry::default();
        assert!(registry.install(&authority, &context, &Verified));
        registry
            .active
            .get_mut(&authority.claims.session_id)
            .unwrap()
            .deadline = Instant::now();
        assert!(registry
            .resolve(&authority.claims.session_id, 1500)
            .is_none());
        assert_eq!(registry.take_due(1500).len(), 1);
        assert!(!registry.install(&authority, &context, &Verified));
        let mut fresh = authority.clone();
        fresh.claims.authority_id = [9; 32];
        assert!(registry.install(&fresh, &context, &Verified));
        assert_eq!(
            registry
                .take_desktop_mismatch(6, DesktopKind::Unknown)
                .len(),
            1
        );
        assert!(!registry.install(&fresh, &context, &Verified));
    }
    #[tokio::test]
    async fn exact_revoke_is_idempotent_but_cannot_remove_new_or_other_session_authority() {
        let (authority, context) = fixture();
        let mut registry = PairedAuthorityRegistry::default();
        assert!(registry.install(&authority, &context, &Verified));
        let mut request = RevokePairedSessionAuthority {
            request_token: 1,
            authority_id: authority.claims.authority_id,
            registration_id: [2; 16],
            registration_epoch: 3,
            session_id: authority.claims.session_id.clone(),
        };
        request.session_id.0.push('x');
        assert!(registry.revoke(&request).is_none());
        request.session_id = authority.claims.session_id.clone();
        request.registration_epoch = 4;
        assert!(registry.revoke(&request).is_none());
        request.registration_epoch = 3;
        assert!(registry.revoke(&request).is_some());
        assert!(registry.resolve(&request.session_id, 1500).is_none());
        assert!(registry.revoke(&request).is_some());
        let mut newer = authority.clone();
        newer.claims.authority_id = [9; 32];
        assert!(registry.install(&newer, &context, &Verified));
        assert!(registry.revoke(&request).is_some());
        assert_eq!(
            registry
                .resolve(&request.session_id, 1500)
                .unwrap()
                .authority
                .claims
                .authority_id,
            [9; 32]
        );
    }
}
