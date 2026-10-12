use super::*;
use mrd_application::VerifiedSignalingIdentity;
use mrd_proto::DeviceId;
use mrd_signal_proto::{
    AuthClaims, SessionGrantV3, SessionGrantV3Payload, WanAccessModeV3, WanPermissionScopeV3,
};
use ring::rand::SystemRandom;
use std::sync::atomic::{AtomicU64, Ordering};

struct Clock(AtomicU64);
impl WanSessionClock for Clock {
    fn now_unix_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
struct Backend {
    request: WanSessionRequestV3,
    grant: GrantBinding,
    access: RelayAccessBinding,
}
#[async_trait]
impl WanSessionWorkflowBackend for Backend {
    async fn create(
        &self,
        _: &WanSessionRequestV3,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        Ok(WanBackendSessionSnapshot::requested(
            self.request.clone(),
            self.request.commitment().unwrap(),
        )
        .unwrap())
    }
    async fn inspect(
        &self,
        _: &WanSessionBinding,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        Ok(WanBackendSessionSnapshot::approved(
            self.request.clone(),
            self.request.commitment().unwrap(),
            self.grant.clone(),
        )
        .unwrap())
    }
    async fn approve(
        &self,
        _: &WanSessionBinding,
        _: &WanSessionApproval,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        unreachable!()
    }
    async fn access_generation_zero(
        &self,
        _: &WanSessionBinding,
        _: u64,
        _: u64,
    ) -> Result<RelayAccessBinding, WanSessionPortError> {
        Ok(self.access.clone())
    }
}
struct Signaling;
#[async_trait]
impl WanSessionWorkflowSignaling for Signaling {
    async fn send_intent(
        &self,
        _: &WanSessionIdentity,
        _: &WanSessionRequestV3,
        _: &str,
        _: u64,
    ) -> Result<String, WanSessionPortError> {
        Ok("1".repeat(64))
    }
    async fn send_grant_with_commitment(
        &self,
        _: &WanSessionIdentity,
        _: &str,
        _: &GrantBinding,
        _: &RelayAccessBinding,
        _: u64,
    ) -> Result<String, WanSessionPortError> {
        unreachable!()
    }
}
struct Consent;
#[async_trait]
impl WanSessionConsentPublisher for Consent {
    async fn publish_attended_request(
        &self,
        _: &WanSessionIdentity,
        _: &WanSessionRequestV3,
        _: u64,
    ) -> Result<(), WanSessionPortError> {
        unreachable!()
    }
    async fn load_attended_approval(
        &self,
        _: &WanSessionIdentity,
        _: u64,
    ) -> Result<WanSessionApproval, WanSessionPortError> {
        unreachable!()
    }
}
struct Fixture {
    coordinator: Arc<WanSessionCoordinator>,
    clock: Arc<Clock>,
    identity: WanSessionIdentity,
    controller: DeviceIdentity,
    target: DeviceIdentity,
    backend: Arc<Backend>,
}
impl Fixture {
    async fn new(grant_expiry: u64, policy_expiry: u64) -> Self {
        let controller = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let target = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let identity = WanSessionIdentity::new_controller_pending_target(
            SessionId("authority-deadlines".into()),
            DeviceId("controller".into()),
            DeviceId("target".into()),
            controller.key_id().into(),
            61_000,
        )
        .unwrap();
        let request = WanSessionRequestV3 {
            session_id: identity.session_id().clone(),
            idempotency_key: [1; 16],
            controller_device_id: identity.controller_device_id().clone(),
            target_device_id: identity.target_device_id().clone(),
            access_mode: WanAccessModeV3::Attended,
            requested_scopes: vec![WanPermissionScopeV3::ScreenView],
            requested_profile: None,
            route_policy: WanRoutePolicyV3::DirectFirst,
        };
        let backend = Arc::new(Backend {
            grant: GrantBinding::new(
                request.commitment().unwrap(),
                request.requested_scopes.clone(),
                7,
                policy_expiry,
                grant_expiry,
                request.route_policy,
            )
            .unwrap(),
            access: RelayAccessBinding::generation_zero(
                7,
                "directory".into(),
                "primary".into(),
                "4".repeat(64),
            )
            .unwrap(),
            request,
        });
        let clock = Arc::new(Clock(AtomicU64::new(1_000)));
        let coordinator = Arc::new(
            WanSessionCoordinator::with_workflow_ports(
                Default::default(),
                Arc::new(NoopWanSessionCleanup),
                WanSessionWorkflowPorts::new(
                    backend.clone(),
                    Arc::new(Signaling),
                    Arc::new(Consent),
                    clock.clone(),
                ),
            )
            .unwrap(),
        );
        coordinator
            .start_controller(identity.clone(), backend.request.clone())
            .await
            .unwrap();
        Self {
            coordinator,
            clock,
            identity,
            controller,
            target,
            backend,
        }
    }
    fn signed_grant(&self, issued: u64, expires: u64, mutate: &str) -> VerifiedWanSessionGrant {
        let signer = if mutate == "key" {
            &self.controller
        } else {
            &self.target
        };
        let claims = AuthClaims {
            issuer_device_id: self.identity.target_device_id().clone(),
            issuer_key_id: signer.key_id().into(),
            intended_peer_device_id: self.identity.controller_device_id().clone(),
            issued_at_ms: issued,
            expires_at_ms: expires,
            counter: 1,
            nonce: [2; 16],
        };
        let message = SessionGrantV3::sign(
            signer,
            SessionGrantV3Payload {
                claims: claims.clone(),
                session_id: if mutate == "session" {
                    SessionId("other-session".into())
                } else {
                    self.identity.session_id().clone()
                },
                controller_device_id: self.identity.controller_device_id().clone(),
                target_device_id: self.identity.target_device_id().clone(),
                intent_commitment: if mutate == "intent" {
                    "9".repeat(64)
                } else {
                    "1".repeat(64)
                },
                approved_scopes: if mutate == "scopes" {
                    vec![WanPermissionScopeV3::InputKeyboard]
                } else {
                    self.backend.grant.approved_scopes().to_vec()
                },
                approved_profile: None,
                backend_policy_revision: if mutate == "revision" { 8 } else { 7 },
                policy_expires_at_ms: self.backend.grant.policy_expires_at_ms()
                    + u64::from(mutate == "policy"),
                relay_generation: 0,
                relay_directory_id: "directory".into(),
                primary_relay_node_id: "primary".into(),
                route_policy: WanRoutePolicyV3::DirectFirst,
            },
        )
        .unwrap();
        VerifiedWanSessionGrant::verify_event(
            VerifiedSignalingEvent {
                sender: VerifiedSignalingIdentity {
                    device_id: claims.issuer_device_id,
                    key_id: claims.issuer_key_id,
                    public_key: signer.public_key().to_vec(),
                    counter: claims.counter,
                    nonce: claims.nonce,
                    issued_at_ms: issued,
                    expires_at_ms: expires,
                },
                signal: AuthenticatedSessionSignal::SessionGrantV3 { message },
            },
            self.identity.controller_device_id(),
            &self.controller,
            issued,
        )
        .unwrap()
    }
}

#[tokio::test]
async fn fresh_delivery_can_outlive_setup_and_active_authority_outlives_both_messages() {
    let f = Fixture::new(601_000, 601_000).await;
    f.clock.0.store(50_000, Ordering::SeqCst);
    let verified = f.signed_grant(50_000, 80_000, "valid");
    f.coordinator
        .install_controller_grant(f.identity.session_id(), verified.clone())
        .await
        .unwrap();
    assert_eq!(
        f.coordinator
            .install_controller_grant(f.identity.session_id(), verified)
            .await
            .unwrap(),
        TransitionResult::Duplicate
    );
    let state = f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap();
    assert_eq!(state.identity().deadline_unix_ms(), 61_000);
    assert_eq!(state.effective_deadline_unix_ms(), 601_000);
    f.clock.0.store(90_000, Ordering::SeqCst);
    assert_eq!(f.coordinator.expire_due_sessions().await, 0);
    let context = super::super::webrtc::GenerationZeroNegotiationContext::from_state(
        &state,
        state.grant().unwrap().grant_commitment().unwrap().into(),
        90_000,
    )
    .unwrap();
    use super::super::webrtc::GenerationZeroNegotiationAuthority;
    super::super::webrtc::ImmutableGenerationZeroAuthority
        .revalidate_generation_zero(&context, 90_000)
        .await
        .unwrap();
    assert_eq!(
        f.coordinator
            .start_controller(f.identity.clone(), f.backend.request.clone())
            .await
            .unwrap(),
        TransitionResult::Duplicate
    );
    f.coordinator
        .begin_negotiation(f.identity.session_id())
        .await
        .unwrap();
    f.clock.0.store(601_000, Ordering::SeqCst);
    assert_eq!(f.coordinator.expire_due_sessions().await, 1);
    assert!(f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap()
        .phase()
        .is_terminal());
}

#[tokio::test]
async fn fresh_delivery_never_resurrects_an_expired_setup_request() {
    let f = Fixture::new(601_000, 601_000).await;
    let verified = f.signed_grant(50_000, 80_000, "valid");
    f.clock.0.store(61_000, Ordering::SeqCst);
    assert!(f
        .coordinator
        .install_controller_grant(f.identity.session_id(), verified)
        .await
        .is_err());
    assert!(f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap()
        .active_authority()
        .is_none());
}

#[tokio::test]
async fn expired_or_unbounded_backend_authority_cannot_activate() {
    for (grant, policy) in [
        (1_000, 601_000),
        (601_000, 3_602_000),
        (3_602_000, 3_602_000),
    ] {
        let f = Fixture::new(grant, policy).await;
        let verified = f.signed_grant(1_000, 31_000, "valid");
        assert!(f
            .coordinator
            .install_controller_grant(f.identity.session_id(), verified)
            .await
            .is_err());
        assert!(f
            .coordinator
            .snapshot(f.identity.session_id())
            .await
            .unwrap()
            .active_authority()
            .is_none());
    }
}

#[tokio::test]
async fn wrong_keys_request_commitments_scopes_policy_and_revision_cannot_activate() {
    for mutation in ["key", "session", "intent", "scopes", "policy", "revision"] {
        let f = Fixture::new(601_000, 601_000).await;
        let verified = f.signed_grant(1_000, 31_000, mutation);
        assert!(
            f.coordinator
                .install_controller_grant(f.identity.session_id(), verified)
                .await
                .is_err(),
            "{mutation}"
        );
        assert!(f
            .coordinator
            .snapshot(f.identity.session_id())
            .await
            .unwrap()
            .active_authority()
            .is_none());
    }
}

#[tokio::test]
async fn raw_grants_and_changed_duplicate_messages_cannot_extend_authority() {
    let f = Fixture::new(601_000, 601_000).await;
    let mut raw = f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap();
    assert!(raw
        .apply(
            WanSessionEvent::Granted(
                f.backend
                    .grant
                    .clone()
                    .with_grant_commitment("8".repeat(64))
                    .unwrap()
            ),
            1_000
        )
        .is_err());
    assert!(raw.active_authority().is_none());
    f.coordinator
        .install_controller_grant(
            f.identity.session_id(),
            f.signed_grant(1_000, 31_000, "valid"),
        )
        .await
        .unwrap();
    let before = f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap();
    f.clock.0.store(1_001, Ordering::SeqCst);
    assert!(f
        .coordinator
        .install_controller_grant(
            f.identity.session_id(),
            f.signed_grant(1_001, 31_001, "policy")
        )
        .await
        .is_err());
    let after = f
        .coordinator
        .snapshot(f.identity.session_id())
        .await
        .unwrap();
    assert_eq!(before.grant(), after.grant());
    assert_eq!(
        before.effective_deadline_unix_ms(),
        after.effective_deadline_unix_ms()
    );
}

#[tokio::test]
async fn expired_target_duplicate_never_reports_a_live_grant() {
    let f = Fixture::new(601_000, 601_000).await;
    let (proof, _) = WanActiveAuthorityProof::registry_fixture_for_test(
        WanSessionRole::Target,
        1_000,
        61_000,
        31_000,
    );
    let session_id = proof.identity().session_id().clone();
    let mut state = WanSessionState::new(WanSessionRole::Target, proof.identity().clone());
    state
        .apply(
            WanSessionEvent::BackendBound {
                request_commitment: proof.grant().request_commitment().into(),
            },
            1_000,
        )
        .unwrap();
    state
        .apply(
            WanSessionEvent::AwaitingConsent {
                intent_commitment: "1".repeat(64),
            },
            1_000,
        )
        .unwrap();
    state.install_active_authority(proof, 1_000).unwrap();
    f.coordinator.begin(state).await.unwrap();
    f.clock.0.store(601_000, Ordering::SeqCst);
    assert!(f.coordinator.approve_target(&session_id).await.is_err());
    assert!(f
        .coordinator
        .snapshot(&session_id)
        .await
        .unwrap()
        .phase()
        .is_terminal());
}

#[tokio::test]
async fn authority_time_window_overflow_fails_closed() {
    let f = Fixture::new(601_000, 601_000).await;
    let now = u64::MAX - 1_000;
    let identity = WanSessionIdentity::new(
        f.identity.session_id().clone(),
        f.identity.controller_device_id().clone(),
        f.identity.target_device_id().clone(),
        f.controller.key_id().into(),
        f.target.key_id().into(),
        u64::MAX,
    )
    .unwrap();
    let mut state = WanSessionState::new(WanSessionRole::Controller, identity);
    state
        .apply(
            WanSessionEvent::BackendBound {
                request_commitment: f.backend.request.commitment().unwrap(),
            },
            now,
        )
        .unwrap();
    state
        .apply(
            WanSessionEvent::AwaitingConsent {
                intent_commitment: "1".repeat(64),
            },
            now,
        )
        .unwrap();
    let grant = GrantBinding::new(
        f.backend.request.commitment().unwrap(),
        vec![WanPermissionScopeV3::ScreenView],
        7,
        u64::MAX - 1,
        u64::MAX - 2,
        WanRoutePolicyV3::DirectFirst,
    )
    .unwrap();
    let backend = WanBackendSessionSnapshot::approved(
        f.backend.request.clone(),
        f.backend.request.commitment().unwrap(),
        grant.clone(),
    )
    .unwrap();
    assert!(WanActiveAuthorityProof::combine(
        &state,
        &backend,
        grant.with_grant_commitment("8".repeat(64)).unwrap(),
        f.backend.access.clone(),
        now
    )
    .is_err());
}

#[tokio::test]
async fn activation_lock_wait_crossing_setup_finalizes_once() {
    for cutoff in [61_000, 31_000] {
        let f = Fixture::new(601_000, 601_000).await;
        let entry = f.coordinator.entry(f.identity.session_id()).await.unwrap();
        let cancellation = entry.cancellation.subscribe();
        let mut locked = entry.state.lock().await;
        locked
            .bind_controller_target_key(f.target.key_id())
            .unwrap();
        let verified = f.signed_grant(1_000, 31_000, "valid");
        let grant = f
            .backend
            .grant
            .clone()
            .with_grant_commitment(verified.grant_commitment().into())
            .unwrap();
        let backend = f
            .backend
            .inspect(f.identity.binding(), 61_000)
            .await
            .unwrap();
        let mut proof = WanActiveAuthorityProof::combine(
            &locked,
            &backend,
            grant,
            f.backend.access.clone(),
            1_000,
        )
        .unwrap();
        proof.delivery_expires_at_ms = Some(31_000);
        let coordinator = f.coordinator.clone();
        let waiting_entry = entry.clone();
        let pending =
            tokio::spawn(
                async move { coordinator.activate_authority(&waiting_entry, proof).await },
            );
        tokio::task::yield_now().await;
        f.clock.0.store(cutoff, Ordering::SeqCst);
        drop(locked);
        assert!(pending.await.unwrap().is_err());
        assert!(entry.state.lock().await.phase().is_terminal());
        assert_eq!(
            f.coordinator.registry.lock().await.active_sessions,
            0,
            "terminal activation must release capacity"
        );
        assert!(
            *cancellation.borrow(),
            "terminal activation must cancel every owned resource"
        );
        assert!(
            entry.cleanup_receipt.get().is_some(),
            "terminal activation must run backend/resource cleanup"
        );
        let _ = f
            .coordinator
            .fail(f.identity.session_id(), WanSessionFailure::Internal)
            .await;
        assert_eq!(
            f.coordinator.registry.lock().await.terminal_order.len(),
            1,
            "cleanup must execute only once"
        );
    }
}
