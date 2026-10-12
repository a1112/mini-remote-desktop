use super::*;
use crate::wan_session::{
    backend::WanSessionBinding,
    coordinator::{
        NoopWanSessionCleanup, SystemWanSessionClock, WanBackendSessionSnapshot,
        WanSessionWorkflowBackend, WanSessionWorkflowSignaling,
    },
    model::{GrantBinding, RelayAccessBinding, WanSessionIdentity},
};
use mrd_application::VerifiedSignalingIdentity;
use mrd_identity::DeviceIdentity;
use mrd_ipc::{ConsentDecision, ConsentResponse, DecimalU64};
use mrd_signal_proto::{
    AuthClaims, SessionGrantV3, SessionGrantV3Payload, SessionIntentV3, SessionIntentV3Payload,
    WanAccessModeV3, WanRoutePolicyV3,
};
use ring::rand::SystemRandom;

struct Backend {
    request: WanSessionRequestV3,
    now: u64,
    lease_ms: u64,
    approval_error: Option<WanSessionPortError>,
}
#[async_trait]
impl WanSessionWorkflowBackend for Backend {
    async fn create(
        &self,
        _: &WanSessionRequestV3,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        unreachable!()
    }
    async fn inspect(
        &self,
        _: &WanSessionBinding,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        Ok(WanBackendSessionSnapshot::requested(
            self.request.clone(),
            self.request.commitment().unwrap(),
        )
        .unwrap())
    }
    async fn approve(
        &self,
        _: &WanSessionBinding,
        approval: &WanSessionApproval,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        if let Some(error) = self.approval_error {
            return Err(error);
        }
        Ok(WanBackendSessionSnapshot::approved(
            self.request.clone(),
            self.request.commitment().unwrap(),
            GrantBinding::new(
                self.request.commitment().unwrap(),
                approval.approved_scopes().to_vec(),
                7,
                self.now + self.lease_ms,
                self.now + self.lease_ms,
                self.request.route_policy,
            )
            .unwrap(),
        )
        .unwrap())
    }
    async fn access_generation_zero(
        &self,
        _: &WanSessionBinding,
        _: u64,
        _: u64,
    ) -> Result<RelayAccessBinding, WanSessionPortError> {
        Ok(RelayAccessBinding::generation_zero(
            7,
            "directory".into(),
            "primary".into(),
            "3".repeat(64),
        )
        .unwrap())
    }
}
struct Signaling {
    identity: Arc<DeviceIdentity>,
    commitment: std::sync::Mutex<Option<String>>,
    revoke: Option<Arc<crate::session_authorization::SessionAuthorizationRegistry>>,
    change_local_target: Option<Arc<crate::AppState>>,
    grant_error: Option<WanSessionPortError>,
}
#[async_trait]
impl WanSessionWorkflowSignaling for Signaling {
    async fn send_intent(
        &self,
        _: &WanSessionIdentity,
        _: &WanSessionRequestV3,
        _: &str,
        _: u64,
    ) -> Result<String, WanSessionPortError> {
        unreachable!()
    }
    async fn send_grant_with_commitment(
        &self,
        identity: &WanSessionIdentity,
        intent: &str,
        grant: &GrantBinding,
        access: &RelayAccessBinding,
        deadline: u64,
    ) -> Result<String, WanSessionPortError> {
        if let Some(error) = self.grant_error {
            return Err(error);
        }
        let signed = SessionGrantV3::sign(
            &self.identity,
            SessionGrantV3Payload {
                claims: AuthClaims {
                    issuer_device_id: identity.target_device_id().clone(),
                    issuer_key_id: self.identity.key_id().into(),
                    intended_peer_device_id: identity.controller_device_id().clone(),
                    issued_at_ms: now_unix_ms(),
                    expires_at_ms: deadline,
                    counter: 2,
                    nonce: [2; 16],
                },
                session_id: identity.session_id().clone(),
                controller_device_id: identity.controller_device_id().clone(),
                target_device_id: identity.target_device_id().clone(),
                intent_commitment: intent.into(),
                approved_scopes: grant.approved_scopes().to_vec(),
                approved_profile: grant.approved_profile().cloned(),
                backend_policy_revision: grant.policy_revision(),
                policy_expires_at_ms: grant.policy_expires_at_ms(),
                relay_generation: 0,
                relay_directory_id: access.directory_id().into(),
                primary_relay_node_id: access.primary_node_id().into(),
                route_policy: grant.route_policy(),
            },
        )
        .unwrap();
        if let Some(auth) = &self.revoke {
            auth.record_failure(
                identity.session_id(),
                RemoteAuthorizationState::Revoked,
                controller_grant_mismatch(),
                now_unix_ms(),
            )
            .await;
        }
        if let Some(app) = &self.change_local_target {
            app.devices.lock().await.register(
                DeviceId("changed-local-target".into()),
                "Changed target".into(),
            );
        }
        let digest = signed.commitment().unwrap();
        *self.commitment.lock().unwrap() = Some(digest.clone());
        Ok(digest)
    }
}

struct Consent {
    inner: ServiceWanSessionConsentPublisher,
    approval_error: Option<WanSessionPortError>,
}

#[async_trait]
impl WanSessionConsentPublisher for Consent {
    async fn publish_attended_request(
        &self,
        identity: &WanSessionIdentity,
        request: &WanSessionRequestV3,
        deadline: u64,
    ) -> Result<(), WanSessionPortError> {
        self.inner
            .publish_attended_request(identity, request, deadline)
            .await
    }

    async fn load_attended_approval(
        &self,
        identity: &WanSessionIdentity,
        deadline: u64,
    ) -> Result<WanSessionApproval, WanSessionPortError> {
        if let Some(error) = self.approval_error {
            return Err(error);
        }
        self.inner.load_attended_approval(identity, deadline).await
    }
}

async fn run_flow(
    case: &str,
) -> (
    Arc<crate::AppState>,
    Arc<WanSessionCoordinator>,
    WanSessionRequestV3,
    DeviceIdentity,
    Arc<Signaling>,
    Result<(), ()>,
) {
    let app = Arc::new(crate::AppState::default());
    let target = DeviceId("physical-target-authority".into());
    app.devices
        .lock()
        .await
        .register(target.clone(), "Target".into());
    let target_key = app.device_identities.machine_identity();
    let controller_key = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let now = now_unix_ms();
    let request = WanSessionRequestV3 {
        session_id: SessionId("target-intent-authority".into()),
        idempotency_key: [7; 16],
        controller_device_id: DeviceId("browser_0123456789abcdef".into()),
        target_device_id: target.clone(),
        access_mode: WanAccessModeV3::Attended,
        requested_scopes: vec![
            WanPermissionScopeV3::InputKeyboard,
            WanPermissionScopeV3::InputPointer,
            WanPermissionScopeV3::ScreenView,
        ],
        requested_profile: None,
        route_policy: WanRoutePolicyV3::DirectFirst,
    };
    let mut claims = AuthClaims {
        issuer_device_id: request.controller_device_id.clone(),
        issuer_key_id: controller_key.key_id().into(),
        intended_peer_device_id: target.clone(),
        issued_at_ms: now,
        expires_at_ms: now + 60_000,
        counter: 1,
        nonce: [1; 16],
    };
    if case == "expired" {
        claims.issued_at_ms = now - 1000;
        claims.expires_at_ms = now - 1;
    }
    let signed = SessionIntentV3::sign(
        &controller_key,
        SessionIntentV3Payload {
            claims: claims.clone(),
            request: request.clone(),
            request_commitment: request.commitment().unwrap(),
        },
    )
    .unwrap();
    let mut event = VerifiedSignalingEvent {
        sender: VerifiedSignalingIdentity {
            device_id: claims.issuer_device_id.clone(),
            key_id: claims.issuer_key_id.clone(),
            public_key: controller_key.public_key().to_vec(),
            counter: claims.counter,
            nonce: claims.nonce,
            issued_at_ms: claims.issued_at_ms,
            expires_at_ms: claims.expires_at_ms,
        },
        signal: AuthenticatedSessionSignal::SessionIntentV3 { message: signed },
    };
    if case == "metadata_key" {
        event.sender.key_id = "f".repeat(64);
    }
    if case == "wrong_target" {
        event.sender.device_id = DeviceId("other-controller".into());
    }
    let signaling = Arc::new(Signaling {
        identity: target_key.clone(),
        commitment: Default::default(),
        revoke: (case == "revoked_during_grant").then(|| app.session_authorizations.clone()),
        change_local_target: (case == "target_changed_during_grant").then(|| app.clone()),
        grant_error: match case {
            "signaling_unavailable" => Some(WanSessionPortError::Unavailable),
            "signaling_rejected" => Some(WanSessionPortError::Rejected),
            _ => None,
        },
    });
    let coordinator = Arc::new(
        WanSessionCoordinator::with_workflow_ports(
            Default::default(),
            Arc::new(NoopWanSessionCleanup),
            WanSessionWorkflowPorts::new(
                Arc::new(Backend {
                    request: request.clone(),
                    now,
                    lease_ms: if case == "long_lease" {
                        600_000
                    } else {
                        50_000
                    },
                    approval_error: match case {
                        "directory_not_yet_valid" => Some(WanSessionPortError::DirectoryNotYetValid),
                        "backend_unavailable" => Some(WanSessionPortError::Unavailable),
                        "backend_rejected" => Some(WanSessionPortError::Rejected),
                        _ => None,
                    },
                }),
                signaling.clone(),
                Arc::new(Consent {
                    inner: ServiceWanSessionConsentPublisher::new(
                        app.session_authorizations.clone(),
                    ),
                    approval_error: match case {
                        "consent_unavailable" => Some(WanSessionPortError::Unavailable),
                        "consent_rejected" => Some(WanSessionPortError::Rejected),
                        _ => None,
                    },
                }),
                Arc::new(SystemWanSessionClock),
            ),
        )
        .unwrap(),
    );
    app.bind_wan_session_coordinator(coordinator.clone())
        .unwrap();
    let denied = case == "denied";
    let approve_app = app.clone();
    let approve_session = request.session_id.clone();
    let approval = if case == "expired" || case == "metadata_key" || case == "wrong_target" {
        None
    } else {
        Some(tokio::spawn(async move {
            loop {
                if approve_app
                    .session_authorizations
                    .snapshot(&approve_session)
                    .await
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            approve_app
                .session_authorizations
                .respond_to_consent(
                    ConsentResponse {
                        session_id: approve_session,
                        decision: if denied {
                            ConsentDecision::Deny
                        } else {
                            ConsentDecision::Approve
                        },
                        approved_scopes: if denied {
                            vec![]
                        } else {
                            vec![
                                RemotePermissionScope::InputKeyboard,
                                RemotePermissionScope::InputPointer,
                                RemotePermissionScope::ScreenView,
                            ]
                        },
                        expected_policy_revision: DecimalU64::new(1),
                    },
                    now_unix_ms(),
                )
                .await
                .unwrap();
        }))
    };
    let result = handle_target_intent(&app, &coordinator, event, &target, target_key.as_ref())
        .await
        .map(|_| ());
    if let Some(approval) = approval {
        approval.await.unwrap();
    }
    (app, coordinator, request, controller_key, signaling, result)
}

async fn assert_approval_diagnostic(case: &str) {
    let (app, coordinator, request, _, signaling, result) = run_flow(case).await;
    assert!(result.is_err(), "{case} must not install authority");
    assert!(coordinator
        .snapshot(&request.session_id)
        .await
        .unwrap()
        .phase()
        .is_terminal());
    let snapshot = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.failure.as_ref().unwrap().message,
        format!("WAN target authorization failed (stage=target_backend_approve, code={case}).")
    );
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
    assert!(signaling.commitment.lock().unwrap().is_none());
    reconcile_wan_session(&app, &request.session_id).await.unwrap();
    assert_eq!(
        app.session_authorizations
            .snapshot(&request.session_id)
            .await
            .unwrap()
            .failure,
        snapshot.failure
    );
}

#[tokio::test]
async fn approval_diagnostic_directory_not_yet_valid() {
    assert_approval_diagnostic("directory_not_yet_valid").await;
}

#[tokio::test]
async fn approval_diagnostic_backend_unavailable() {
    assert_approval_diagnostic("backend_unavailable").await;
}

#[tokio::test]
async fn approval_diagnostic_backend_rejected() {
    assert_approval_diagnostic("backend_rejected").await;
}

#[tokio::test]
async fn approval_diagnostic_signaling_unavailable() {
    assert_approval_diagnostic("signaling_unavailable").await;
}

#[tokio::test]
async fn approval_diagnostic_signaling_rejected() {
    assert_approval_diagnostic("signaling_rejected").await;
}

#[tokio::test]
async fn approval_diagnostic_consent_unavailable() {
    assert_approval_diagnostic("consent_unavailable").await;
}

#[tokio::test]
async fn approval_diagnostic_consent_rejected() {
    assert_approval_diagnostic("consent_rejected").await;
}

#[test]
fn approval_diagnostic_binding_deadline_and_unknown_categories_are_finite() {
    for (reason, expected) in [
        (
            WanSessionCoordinatorError::BackendBindingMismatch,
            "backend_binding_mismatch",
        ),
        (WanSessionCoordinatorError::DeadlineExceeded, "expired"),
        (
            WanSessionCoordinatorError::Backend(WanSessionPortError::DeadlineExceeded),
            "expired",
        ),
        (
            WanSessionCoordinatorError::Signaling(WanSessionPortError::DeadlineExceeded),
            "expired",
        ),
        (
            WanSessionCoordinatorError::Consent(WanSessionPortError::DeadlineExceeded),
            "expired",
        ),
        (
            WanSessionCoordinatorError::CapacityExceeded,
            "capacity_exhausted",
        ),
        (
            WanSessionCoordinatorError::TaskCapacityExceeded,
            "capacity_exhausted",
        ),
        (
            WanSessionCoordinatorError::BufferCapacityExceeded,
            "capacity_exhausted",
        ),
        (
            WanSessionCoordinatorError::InvalidConfiguration,
            "guard_rejected",
        ),
        (WanSessionCoordinatorError::CleanupFailed, "guard_rejected"),
        (
            WanSessionCoordinatorError::Signaling(WanSessionPortError::DirectoryNotYetValid),
            "guard_rejected",
        ),
        (
            WanSessionCoordinatorError::Consent(WanSessionPortError::DirectoryNotYetValid),
            "guard_rejected",
        ),
    ] {
        assert_eq!(
            target_approval_diagnostic(&reason).remote_failure().message,
            format!("WAN target authorization failed (stage=target_backend_approve, code={expected}).")
        );
    }
}

#[tokio::test]
async fn fresh_target_grant_establishes_a_ten_minute_lease_after_one_minute_setup() {
    let (app, coordinator, request, _, _, result) = run_flow("long_lease").await;
    assert!(
        result.is_ok(),
        "a signed grant and exact backend lease must establish authority: {:?}",
        coordinator.snapshot(&request.session_id).await
    );
    let state = coordinator.snapshot(&request.session_id).await.unwrap();
    assert_eq!(state.phase(), WanSessionPhase::AccessBound);
    let grant = app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .unwrap();
    assert!(grant.expires_at_ms > state.identity().deadline_unix_ms());
}

#[test]
fn fresh_grant_message_verifies_a_longer_signed_policy_lease() {
    let controller = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let target = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let now = now_unix_ms();
    let claims = AuthClaims {
        issuer_device_id: DeviceId("lease-target".into()),
        issuer_key_id: target.key_id().into(),
        intended_peer_device_id: DeviceId("lease-controller".into()),
        issued_at_ms: now,
        expires_at_ms: now + 30_000,
        counter: 1,
        nonce: [4; 16],
    };
    let signed = SessionGrantV3::sign(
        &target,
        SessionGrantV3Payload {
            claims: claims.clone(),
            session_id: SessionId("lease-message".into()),
            controller_device_id: claims.intended_peer_device_id.clone(),
            target_device_id: claims.issuer_device_id.clone(),
            intent_commitment: "1".repeat(64),
            approved_scopes: vec![WanPermissionScopeV3::ScreenView],
            approved_profile: None,
            backend_policy_revision: 7,
            policy_expires_at_ms: now + 600_000,
            relay_generation: 0,
            relay_directory_id: "directory".into(),
            primary_relay_node_id: "primary".into(),
            route_policy: WanRoutePolicyV3::DirectFirst,
        },
    )
    .unwrap();
    let event = VerifiedSignalingEvent {
        sender: VerifiedSignalingIdentity {
            device_id: claims.issuer_device_id,
            key_id: claims.issuer_key_id,
            public_key: target.public_key().to_vec(),
            counter: claims.counter,
            nonce: claims.nonce,
            issued_at_ms: claims.issued_at_ms,
            expires_at_ms: claims.expires_at_ms,
        },
        signal: AuthenticatedSessionSignal::SessionGrantV3 { message: signed },
    };
    assert!(VerifiedWanSessionGrant::verify_event(
        event,
        &claims.intended_peer_device_id,
        &controller,
        now
    )
    .is_ok());
}

#[tokio::test]
async fn approved_target_intent_installs_the_exact_signed_grant_and_peer_key() {
    let (app, coordinator, request, controller_key, signaling, result) = run_flow("approved").await;
    assert!(result.is_ok(), "target workflow must reach AccessBound before usable authorization: state={:?}, authorization={:?}",coordinator.snapshot(&request.session_id).await,app.session_authorizations.snapshot(&request.session_id).await);
    let snapshot = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.authorization_state,
        RemoteAuthorizationState::Granted,
        "formal target approval must install usable service authorization"
    );
    assert_eq!(snapshot.peer_device_id, request.controller_device_id);
    assert_eq!(snapshot.peer_key_id, controller_key.key_id());
    assert_eq!(
        snapshot.policy_revision.get(),
        7,
        "backend policy is bound separately from local consent revision 1"
    );
    let grant = app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .unwrap();
    assert_eq!(
        grant.grant_id,
        format!(
            "sha256:{}",
            signaling.commitment.lock().unwrap().as_ref().unwrap()
        )
    );
    assert_eq!(grant.granted_scopes, snapshot.granted_scopes);
    // Installing without an authenticated peer key cannot yield active input even after route/media become ready.
    app.session_authorizations
        .mark_streaming(&request.session_id, now_unix_ms())
        .await
        .unwrap();
    assert!(app
        .session_authorizations
        .active_control_authorization(&request.session_id, now_unix_ms())
        .await
        .is_ok());
    coordinator.close(&request.session_id).await.unwrap();
}

#[tokio::test]
async fn rejected_expired_and_mismatched_intents_never_install_authority() {
    for case in ["denied", "expired", "metadata_key", "wrong_target"] {
        let (app, coordinator, request, _, _, result) = run_flow(case).await;
        assert!(result.is_err(), "{case} must fail");
        assert!(
            app.session_authorizations
                .active_grant(&request.session_id)
                .await
                .is_none(),
            "{case} installed authority"
        );
        if let Ok(state) = coordinator.snapshot(&request.session_id).await {
            assert!(state.phase().is_terminal());
        }
    }
}

#[tokio::test]
async fn authorization_install_failure_terminalizes_an_approved_coordinator() {
    let (app, coordinator, request, _, signaling, result) = run_flow("revoked_during_grant").await;
    assert!(
        signaling.commitment.lock().unwrap().is_some(),
        "the signed grant was produced before local revocation"
    );
    assert!(result.is_err(), "local revocation must prevent activation");
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
    assert!(
        coordinator
            .snapshot(&request.session_id)
            .await
            .unwrap()
            .phase()
            .is_terminal(),
        "an authorization install failure must clean up the signed backend grant"
    );
}

#[tokio::test]
async fn target_install_diagnostic_survives_terminalization_and_query_reconciliation() {
    let (app, coordinator, request, _, signaling, result) =
        run_flow("target_changed_during_grant").await;
    assert!(signaling.commitment.lock().unwrap().is_some());
    assert!(result.is_err());
    assert_eq!(
        coordinator
            .snapshot(&request.session_id)
            .await
            .unwrap()
            .phase(),
        WanSessionPhase::Failed
    );
    let expected =
        "WAN target authorization failed (stage=target_expected_grant, code=guard_rejected).";
    let snapshot = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert_eq!(snapshot.failure.as_ref().unwrap().message, expected);
    reconcile_wan_session(&app, &request.session_id)
        .await
        .unwrap();
    let after = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert_eq!(after.failure.as_ref().unwrap().message, expected);
    let events = app
        .session_authorizations
        .subscribe(mrd_ipc::SessionEventSubscriptionQuery {
            session_id: Some(request.session_id.clone()),
            after_sequence: Some(DecimalU64::new(0)),
            limit: 64,
            wait_timeout_ms: 0,
        })
        .await
        .unwrap();
    assert!(events
        .events
        .iter()
        .any(|envelope| matches!(&envelope.event,
        mrd_ipc::RemoteSessionEvent::AuthorizationChanged { failure: Some(failure), .. }
            if failure.message == expected)));
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
}

#[tokio::test]
async fn target_install_diagnostics_do_not_terminalize_unverified_intents() {
    for case in ["expired", "metadata_key", "wrong_target"] {
        let (app, coordinator, request, _, _, result) = run_flow(case).await;
        assert!(result.is_err());
        assert!(matches!(
            coordinator.snapshot(&request.session_id).await,
            Err(WanSessionCoordinatorError::SessionNotFound)
        ));
        assert!(app
            .session_authorizations
            .snapshot(&request.session_id)
            .await
            .is_none());
    }
}

struct ActivePairedIntent {
    app: Arc<crate::AppState>,
    coordinator: Arc<WanSessionCoordinator>,
    request: WanSessionRequestV3,
    controller: DeviceIdentity,
    event: VerifiedSignalingEvent,
    verified: VerifiedWanSessionIntent,
}

/// Seed a previously approved session through the real signed coordinator and
/// authorization registry. An installed Windows Agent is deliberately outside
/// this unit fixture; duplicate handling must never try to install another one.
async fn active_paired_intent(install: bool) -> ActivePairedIntent {
    let app = Arc::new(crate::AppState::default());
    let target = DeviceId("paired-duplicate-target".into());
    app.devices
        .lock()
        .await
        .register(target.clone(), "Target".into());
    let target_key = app.device_identities.machine_identity();
    let controller = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let now = now_unix_ms();
    let request = WanSessionRequestV3 {
        session_id: SessionId("paired-duplicate-active".into()),
        idempotency_key: [7; 16],
        controller_device_id: DeviceId("paired-duplicate-controller".into()),
        target_device_id: target.clone(),
        access_mode: WanAccessModeV3::Paired,
        requested_scopes: vec![WanPermissionScopeV3::ScreenView],
        requested_profile: None,
        route_policy: WanRoutePolicyV3::DirectFirst,
    };
    let claims = AuthClaims {
        issuer_device_id: request.controller_device_id.clone(),
        issuer_key_id: controller.key_id().into(),
        intended_peer_device_id: target.clone(),
        issued_at_ms: now,
        expires_at_ms: now + 60_000,
        counter: 1,
        nonce: [1; 16],
    };
    let signed = SessionIntentV3::sign(
        &controller,
        SessionIntentV3Payload {
            claims: claims.clone(),
            request: request.clone(),
            request_commitment: request.commitment().unwrap(),
        },
    )
    .unwrap();
    let event = VerifiedSignalingEvent {
        sender: VerifiedSignalingIdentity {
            device_id: claims.issuer_device_id,
            key_id: claims.issuer_key_id,
            public_key: controller.public_key().to_vec(),
            counter: claims.counter,
            nonce: claims.nonce,
            issued_at_ms: claims.issued_at_ms,
            expires_at_ms: claims.expires_at_ms,
        },
        signal: AuthenticatedSessionSignal::SessionIntentV3 { message: signed },
    };
    let verified =
        VerifiedWanSessionIntent::verify_event(event.clone(), &target, &target_key, now).unwrap();
    let signaling = Arc::new(Signaling {
        identity: target_key,
        commitment: Default::default(),
        revoke: None,
        change_local_target: None,
        grant_error: None,
    });
    let coordinator = Arc::new(
        WanSessionCoordinator::with_workflow_ports(
            Default::default(),
            Arc::new(NoopWanSessionCleanup),
            WanSessionWorkflowPorts::new(
                Arc::new(Backend {
                    request: request.clone(),
                    now,
                    lease_ms: 50_000,
                    approval_error: None,
                }),
                signaling,
                Arc::new(ServiceWanSessionConsentPublisher::new(
                    app.session_authorizations.clone(),
                )),
                Arc::new(SystemWanSessionClock),
            ),
        )
        .unwrap(),
    );
    app.bind_wan_session_coordinator(coordinator.clone())
        .unwrap();
    coordinator
        .accept_verified_target_intent(verified.clone())
        .await
        .unwrap();
    app.session_authorizations
        .bind_authenticated_peer_key(&request.session_id, controller.public_key(), now_unix_ms())
        .await
        .unwrap();
    if install {
        app.session_authorizations
            .authorize_paired_policy_at(
                &request.session_id,
                1,
                &[RemotePermissionScope::ScreenView],
                now_unix_ms(),
            )
            .await
            .unwrap();
        coordinator
            .approve_target(&request.session_id)
            .await
            .unwrap();
        let state = coordinator.snapshot(&request.session_id).await.unwrap();
        let grant = state.grant().unwrap();
        app.session_authorizations
            .install_verified_wan_target_grant(
                crate::session_authorization::VerifiedSessionGrant {
                    grant_id: format!("sha256:{}", grant.grant_commitment().unwrap()),
                    session_id: request.session_id.clone(),
                    granted_scopes: vec![RemotePermissionScope::ScreenView],
                    issued_at_ms: now,
                    expires_at_ms: grant.grant_expires_at_ms(),
                    policy_revision: grant.policy_revision(),
                    route_constraint: "webrtc_relay".into(),
                    transport_fingerprint_sha256: decode_sha256(
                        state.access().unwrap().relay_url_digest(),
                    )
                    .unwrap(),
                },
                1,
                now,
            )
            .await
            .unwrap();
    }
    // Refresh user intent without supplying a new credential. Existing grants
    // must remain valid while every new automatic authorization fails closed.
    let epoch = app.public_connection.temporary_epoch.begin(false).unwrap();
    let mut temporary = app.public_connection.temporary.lock().await;
    temporary.apply_operation_epoch(epoch).unwrap();
    temporary.invalidate_publication();
    drop(temporary);
    ActivePairedIntent {
        app,
        coordinator,
        request,
        controller,
        event,
        verified,
    }
}

#[tokio::test]
async fn paired_active_exact_duplicate_after_password_refresh_preserves_the_existing_grant() {
    let f = active_paired_intent(true).await;
    let before = f
        .app
        .session_authorizations
        .active_grant(&f.request.session_id)
        .await
        .unwrap();
    let target = f.app.device_identities.machine_identity();
    assert!(handle_target_intent(
        &f.app,
        &f.coordinator,
        f.event.clone(),
        &f.request.target_device_id,
        &target
    )
    .await
    .is_ok());
    assert_eq!(
        f.app
            .session_authorizations
            .active_grant(&f.request.session_id)
            .await,
        Some(before)
    );
    assert_eq!(
        f.coordinator
            .snapshot(&f.request.session_id)
            .await
            .unwrap()
            .phase(),
        WanSessionPhase::AccessBound
    );
}

#[tokio::test]
async fn paired_active_exact_duplicate_dispatch_does_not_start_generation_zero_again() {
    use super::super::{backend::HttpWanSessionBackend, config::WanSessionBackendConfig};
    let f = active_paired_intent(true).await;
    let before = f
        .app
        .session_authorizations
        .active_grant(&f.request.session_id)
        .await
        .unwrap();
    let config = WanSessionBackendConfig::new(
        "https://unused.invalid/",
        "synthetic-test-token",
        std::collections::BTreeMap::from([("directory".into(), vec![1; 32])]),
        Duration::from_secs(1),
        4096,
        1,
    )
    .unwrap();
    let backend = Arc::new(ServiceWanSessionWorkflowBackend::new(Arc::new(
        HttpWanSessionBackend::new(config).unwrap(),
    )));
    // This adapter deliberately has no relay access. Any attempted generation
    // restart would synchronously fail and terminalize the established session.
    handle_initial_event(
        f.app.clone(),
        f.coordinator.clone(),
        backend,
        f.request.target_device_id.clone(),
        f.app.device_identities.machine_identity(),
        f.event,
    )
    .await;
    assert_eq!(
        f.app
            .session_authorizations
            .active_grant(&f.request.session_id)
            .await,
        Some(before)
    );
    assert_eq!(
        f.coordinator
            .snapshot(&f.request.session_id)
            .await
            .unwrap()
            .phase(),
        WanSessionPhase::AccessBound
    );
}

#[tokio::test]
async fn paired_active_exact_install_retry_after_password_refresh_is_a_no_op() {
    let f = active_paired_intent(true).await;
    let before = f
        .app
        .session_authorizations
        .active_grant(&f.request.session_id)
        .await
        .unwrap();
    let state = f.coordinator.snapshot(&f.request.session_id).await.unwrap();
    assert!(install_target_authorization(&f.app, &state, &f.verified, 1)
        .await
        .is_ok());
    assert_eq!(
        f.app
            .session_authorizations
            .active_grant(&f.request.session_id)
            .await,
        Some(before)
    );
}

#[tokio::test]
async fn paired_pending_and_revoked_or_expired_duplicates_never_restore_authority() {
    for case in ["pending", "revoked", "expired", "closed"] {
        let f = active_paired_intent(case != "pending").await;
        match case {
            "revoked" => {
                f.app
                    .session_authorizations
                    .record_failure(
                        &f.request.session_id,
                        RemoteAuthorizationState::Revoked,
                        controller_grant_mismatch(),
                        now_unix_ms(),
                    )
                    .await;
            }
            "expired" => {
                f.app
                    .session_authorizations
                    .snapshot_at(&f.request.session_id, now_unix_ms() + 60_001)
                    .await;
            }
            "closed" => {
                f.coordinator.close(&f.request.session_id).await.unwrap();
            }
            _ => {}
        }
        let target = f.app.device_identities.machine_identity();
        assert!(
            handle_target_intent(
                &f.app,
                &f.coordinator,
                f.event,
                &f.request.target_device_id,
                &target
            )
            .await
            .is_err(),
            "{case}"
        );
        assert!(
            f.app
                .session_authorizations
                .active_grant(&f.request.session_id)
                .await
                .is_none(),
            "{case}"
        );
    }
}

#[tokio::test]
async fn paired_changed_signed_intent_is_not_an_exact_existing_grant_retry() {
    let f = active_paired_intent(true).await;
    let mut event = f.event.clone();
    if let AuthenticatedSessionSignal::SessionIntentV3 { message } = &mut event.signal {
        let mut payload = message.payload.clone();
        payload.claims.counter += 1;
        payload.claims.nonce = [9; 16];
        *message = SessionIntentV3::sign(&f.controller, payload.clone()).unwrap();
        event.sender.counter = payload.claims.counter;
        event.sender.nonce = payload.claims.nonce;
    }
    let target = f.app.device_identities.machine_identity();
    assert!(handle_target_intent(
        &f.app,
        &f.coordinator,
        event,
        &f.request.target_device_id,
        &target
    )
    .await
    .is_err());
}
