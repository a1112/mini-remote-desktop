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
    case: String,
    calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
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
        .unwrap()
        .with_temporary_guest(self.case == "guest"))
    }
    async fn approve(
        &self,
        _: &WanSessionBinding,
        approval: &WanSessionApproval,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        self.calls.lock().unwrap().push("approve");
        self.approved(approval)
    }
    async fn same_account(
        &self,
        _: &WanSessionIdentity,
        _: u64,
    ) -> Result<bool, WanSessionPortError> {
        self.calls.lock().unwrap().push("same-account");
        if self.case == "trust_unavailable" {
            return Err(WanSessionPortError::Unavailable);
        }
        Ok(matches!(
            self.case.as_str(),
            "same_account"
                | "same_account_changed"
                | "same_account_extra_scope"
                | "same_account_revoked"
                | "guest"
        ))
    }
    async fn approve_same_account(
        &self,
        _: &WanSessionIdentity,
        approval: &WanSessionApproval,
        _: u64,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        self.calls.lock().unwrap().push("approve-same-account");
        if self.case == "same_account_changed" {
            return Err(WanSessionPortError::Rejected);
        }
        self.approved(approval)
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
impl Backend {
    fn approved(
        &self,
        approval: &WanSessionApproval,
    ) -> Result<WanBackendSessionSnapshot, WanSessionPortError> {
        Ok(WanBackendSessionSnapshot::approved(
            self.request.clone(),
            self.request.commitment().unwrap(),
            GrantBinding::new(
                self.request.commitment().unwrap(),
                approval.approved_scopes().to_vec(),
                7,
                // Backend authorization is a session lease, intentionally
                // longer than the signed intent's 60-second consent window.
                self.now + 600_000,
                self.now + 600_000,
                self.request.route_policy,
            )
            .unwrap(),
        )
        .unwrap())
    }
}
struct Signaling {
    identity: Arc<DeviceIdentity>,
    commitment: std::sync::Mutex<Option<String>>,
    revoke: Option<Arc<crate::session_authorization::SessionAuthorizationRegistry>>,
    calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
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
        let digest = signed.commitment().unwrap();
        *self.commitment.lock().unwrap() = Some(digest.clone());
        Ok(digest)
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
    VerifiedSignalingEvent,
) {
    let app = Arc::new(crate::AppState::default());
    let target = DeviceId("physical-target-authority".into());
    app.devices
        .lock()
        .await
        .register(target.clone(), "Target".into());
    let target_key = app.device_identities.machine_identity();
    let controller_key = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    if case == "same_account_revoked" {
        app.device_identities.trust_authenticated_peer_for_test(
            &controller_key,
            1,
            mrd_store_sqlite::TrustState::Revoked,
        );
    }
    let now = now_unix_ms();
    let request = WanSessionRequestV3 {
        session_id: SessionId("target-intent-authority".into()),
        idempotency_key: [7; 16],
        controller_device_id: DeviceId(
            if case.starts_with("same_account") {
                "physical-controller"
            } else {
                "browser_0123456789abcdef"
            }
            .into(),
        ),
        target_device_id: target.clone(),
        access_mode: WanAccessModeV3::Attended,
        requested_scopes: if case == "same_account_extra_scope" {
            vec![
                WanPermissionScopeV3::FileRead,
                WanPermissionScopeV3::InputKeyboard,
                WanPermissionScopeV3::InputPointer,
                WanPermissionScopeV3::ScreenView,
            ]
        } else {
            vec![
                WanPermissionScopeV3::InputKeyboard,
                WanPermissionScopeV3::InputPointer,
                WanPermissionScopeV3::ScreenView,
            ]
        },
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
        calls: Default::default(),
    });
    let coordinator = Arc::new(
        WanSessionCoordinator::with_workflow_ports(
            Default::default(),
            Arc::new(NoopWanSessionCleanup),
            WanSessionWorkflowPorts::new(
                Arc::new(Backend {
                    request: request.clone(),
                    now,
                    case: case.to_owned(),
                    calls: signaling.calls.clone(),
                }),
                signaling.clone(),
                Arc::new(
                    ServiceWanSessionConsentPublisher::new(app.session_authorizations.clone())
                        .with_app_state(&app),
                ),
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
    let approval = if matches!(
            case,
            "same_account"
                | "same_account_changed"
                | "same_account_extra_scope"
                | "same_account_revoked"
                | "trust_unavailable"
        )
        || case == "expired"
        || case == "metadata_key"
        || case == "wrong_target"
    {
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
    let replay_event = event.clone();
    let result = handle_target_intent(
        &app,
        &coordinator,
        event,
        &target,
        target_key.as_ref(),
    )
    .await;
    if let Some(approval) = approval {
        approval.await.unwrap();
    }
    (
        app,
        coordinator,
        request,
        controller_key,
        signaling,
        result,
        replay_event,
    )
}

#[tokio::test]
async fn approved_target_intent_installs_the_exact_signed_grant_and_peer_key() {
    let (app, coordinator, request, controller_key, signaling, result, _) =
        run_flow("approved").await;
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
async fn same_account_uses_guarded_backend_approval_without_consent_prompt() {
    let (app, coordinator, request, _, signaling, result, _) = run_flow("same_account").await;
    assert!(result.is_ok());
    assert_eq!(
        *signaling.calls.lock().unwrap(),
        ["same-account", "approve-same-account"]
    );
    assert!(signaling.commitment.lock().unwrap().is_some());
    let snapshot = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert_eq!(
        snapshot.authorization_state,
        RemoteAuthorizationState::Granted
    );
    let events = app
        .session_authorizations
        .subscribe(mrd_ipc::SessionEventSubscriptionQuery {
            session_id: Some(request.session_id.clone()),
            after_sequence: None,
            limit: 32,
            wait_timeout_ms: 0,
        })
        .await
        .unwrap();
    assert!(!events.events.iter().any(|event| matches!(
        event.event,
        mrd_ipc::RemoteSessionEvent::ConsentRequested { .. }
    )));
    assert!(!events
        .pending_sessions
        .iter()
        .any(|pending| pending.session_id == request.session_id));
    coordinator.close(&request.session_id).await.unwrap();
}

#[tokio::test]
async fn changed_account_cannot_fall_back_to_ordinary_approval() {
    let (app, coordinator, request, _, signaling, result, _) =
        run_flow("same_account_changed").await;
    assert!(result.is_err());
    assert_eq!(
        *signaling.calls.lock().unwrap(),
        ["same-account", "approve-same-account"]
    );
    assert!(signaling.commitment.lock().unwrap().is_none());
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
    assert!(coordinator
        .snapshot(&request.session_id)
        .await
        .unwrap()
        .phase()
        .is_terminal());
}

#[tokio::test]
async fn same_account_respects_current_capabilities_and_existing_revocation() {
    let (app, coordinator, request, _, _, result, _) = run_flow("same_account_extra_scope").await;
    assert!(result.is_ok());
    let authorization = app
        .session_authorizations
        .snapshot(&request.session_id)
        .await
        .unwrap();
    assert!(!authorization
        .granted_scopes
        .contains(&RemotePermissionScope::FileRead));
    assert!(authorization
        .granted_scopes
        .contains(&RemotePermissionScope::ScreenView));
    coordinator.close(&request.session_id).await.unwrap();

    let (app, coordinator, request, _, signaling, result, _) =
        run_flow("same_account_revoked").await;
    assert!(result.is_err());
    assert_eq!(*signaling.calls.lock().unwrap(), ["same-account"]);
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
    assert!(coordinator
        .snapshot(&request.session_id)
        .await
        .unwrap()
        .phase()
        .is_terminal());
}

#[tokio::test]
async fn guest_and_other_accounts_use_normal_consent() {
    for case in ["guest", "other_account"] {
        let (app, coordinator, request, _, signaling, result, _) = run_flow(case).await;
        assert!(result.is_ok(), "{case}");
        let calls = signaling.calls.lock().unwrap().clone();
        assert!(calls.contains(&"approve"));
        assert!(!calls.contains(&"approve-same-account"));
        if case == "guest" {
            assert!(!calls.contains(&"same-account"));
        }
        let events = app
            .session_authorizations
            .subscribe(mrd_ipc::SessionEventSubscriptionQuery {
                session_id: Some(request.session_id.clone()),
                after_sequence: None,
                limit: 32,
                wait_timeout_ms: 0,
            })
            .await
            .unwrap();
        assert!(events.events.iter().any(|event| matches!(
            event.event,
            mrd_ipc::RemoteSessionEvent::ConsentRequested { .. }
        )));
        coordinator.close(&request.session_id).await.unwrap();
    }
}

#[tokio::test]
async fn unverifiable_account_fails_without_prompt_or_approval() {
    let (app, coordinator, request, _, signaling, result, _) = run_flow("trust_unavailable").await;
    assert!(result.is_err());
    assert_eq!(*signaling.calls.lock().unwrap(), ["same-account"]);
    assert!(app
        .session_authorizations
        .active_grant(&request.session_id)
        .await
        .is_none());
    assert!(coordinator
        .snapshot(&request.session_id)
        .await
        .unwrap()
        .phase()
        .is_terminal());
}

#[tokio::test]
async fn rejected_expired_and_mismatched_intents_never_install_authority() {
    for case in ["denied", "expired", "metadata_key", "wrong_target"] {
        let (app, coordinator, request, _, _, result, _) = run_flow(case).await;
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
    let (app, coordinator, request, _, signaling, result, _) =
        run_flow("revoked_during_grant").await;
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
