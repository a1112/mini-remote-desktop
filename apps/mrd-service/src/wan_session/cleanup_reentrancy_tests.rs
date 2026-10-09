//! Actual service cleanup and coordinator-owned sibling regression.
use super::*;
use crate::wan_session::{
    backend::{HttpWanSessionBackend, WanSessionBinding},
    config::WanSessionBackendConfig,
    coordinator::{WanBackendSessionSnapshot, WanSessionCleanup, WanSessionWorkflowBackend},
    model::{
        GrantBinding, RelayAccessBinding, RelayRouteProof, WanSessionEvent, WanSessionIdentity,
    },
};
use mrd_input::{InputError, InputEvent, InputInjector, InputKey};
use mrd_ipc::{ControlInputEvent, ControlInputKey};
use mrd_signal_proto::{WanAccessModeV3, WanRoutePolicyV3};
use std::{collections::BTreeMap, sync::Mutex as StdMutex};
use tokio::sync::{oneshot, watch};

struct RecordingInput(Arc<StdMutex<Vec<InputEvent>>>);
impl InputInjector for RecordingInput {
    fn is_available(&self) -> bool {
        true
    }
    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        self.0.lock().unwrap().push(*event);
        Ok(())
    }
}
struct RevokedBackend;
#[async_trait]
impl WanSessionWorkflowBackend for RevokedBackend {
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
        Err(WanSessionPortError::Rejected)
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
        unreachable!()
    }
}

async fn fixture() -> (
    Arc<crate::AppState>,
    Arc<WanSessionCoordinator>,
    WanSessionState,
    Arc<StdMutex<Vec<InputEvent>>>,
) {
    fixture_with_media(false).await
}

async fn fixture_with_media(
    media_verified: bool,
) -> (
    Arc<crate::AppState>,
    Arc<WanSessionCoordinator>,
    WanSessionState,
    Arc<StdMutex<Vec<InputEvent>>>,
) {
    let app = Arc::new(crate::AppState::default());
    let config = WanSessionBackendConfig::new(
        "https://unused.invalid/",
        "test-token",
        BTreeMap::from([("test-directory".into(), vec![1; 32])]),
        Duration::from_secs(1),
        4096,
        1,
    )
    .unwrap();
    // There is no retained HTTP binding in this fixture; the real cleanup
    // adapter still executes all input/media/transport/failover/signaling ports.
    let backend = Arc::new(ServiceWanSessionWorkflowBackend::new(Arc::new(
        HttpWanSessionBackend::new(config).unwrap(),
    )));
    let consent = Arc::new(ServiceWanSessionConsentPublisher::new(
        app.session_authorizations.clone(),
    ));
    let cleanup: Arc<dyn WanSessionCleanup> =
        Arc::new(ServiceWanSessionCleanup::new(&app, backend, consent));
    let coordinator = Arc::new(
        WanSessionCoordinator::new(Default::default(), cleanup, Arc::new(SystemWanSessionClock))
            .unwrap(),
    );
    app.bind_wan_session_coordinator(coordinator.clone())
        .unwrap();
    let now = now_unix_ms();
    let controller_key =
        mrd_identity::DeviceIdentity::generate(&ring::rand::SystemRandom::new()).unwrap();
    let request = WanSessionRequestV3 {
        session_id: SessionId("browser-cleanup-sibling".into()),
        idempotency_key: [5; 16],
        controller_device_id: DeviceId("browser_0123456789abcdef".into()),
        target_device_id: DeviceId("physical-target".into()),
        access_mode: WanAccessModeV3::Attended,
        requested_scopes: vec![
            WanPermissionScopeV3::InputKeyboard,
            WanPermissionScopeV3::ScreenView,
        ],
        requested_profile: None,
        route_policy: WanRoutePolicyV3::DirectFirst,
    };
    let commitment = request.commitment().unwrap();
    let identity = WanSessionIdentity::new(
        request.session_id.clone(),
        request.controller_device_id.clone(),
        request.target_device_id.clone(),
        controller_key.key_id().to_owned(),
        "2".repeat(64),
        now + 60_000,
    )
    .unwrap();
    let grant = GrantBinding::new(
        commitment.clone(),
        request.requested_scopes.clone(),
        7,
        now + 60_000,
        now + 60_000,
        request.route_policy,
    )
    .unwrap()
    .with_grant_commitment("3".repeat(64))
    .unwrap();
    let access = RelayAccessBinding::generation_zero(
        7,
        "directory".into(),
        "primary".into(),
        "4".repeat(64),
    )
    .unwrap();
    let route_proof =
        RelayRouteProof::from_verified_policy(&access, request.route_policy, false, false).unwrap();
    let mut state = WanSessionState::new(WanSessionRole::Target, identity);
    for event in [
        WanSessionEvent::BackendBound {
            request_commitment: commitment,
        },
        WanSessionEvent::AwaitingConsent {
            intent_commitment: "5".repeat(64),
        },
        WanSessionEvent::Granted(grant),
        WanSessionEvent::AccessBound(access),
        WanSessionEvent::Negotiating,
    ] {
        state.apply(event, now).unwrap();
    }
    if media_verified {
        state
            .apply(WanSessionEvent::RelayVerified(route_proof), now)
            .unwrap();
    }
    coordinator.begin(state.clone()).await.unwrap();
    if media_verified {
        let scopes = vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputKeyboard,
        ];
        app.session_authorizations
            .begin_verified_incoming(
                crate::session_authorization::VerifiedIncomingAuthorizationRequest {
                    session_id: request.session_id.clone(),
                    peer_device_id: request.controller_device_id.clone(),
                    peer_key_id: controller_key.key_id().to_owned(),
                    peer_key_epoch: 1,
                    access_mode: mrd_ipc::RemoteAccessMode::Attended,
                    requested_scopes: scopes.clone(),
                    peer_permission_ceiling: scopes.clone(),
                    machine_permission_ceiling: scopes.clone(),
                    runtime_capabilities: scopes.clone(),
                    transport_kind: "webrtc_relay".into(),
                    request_nonce: request.idempotency_key,
                    created_at_ms: now,
                    expires_at_ms: now + 60_000,
                },
            )
            .await
            .unwrap();
        app.session_authorizations
            .respond_to_consent(
                mrd_ipc::ConsentResponse {
                    session_id: request.session_id.clone(),
                    decision: mrd_ipc::ConsentDecision::Approve,
                    approved_scopes: scopes.clone(),
                    expected_policy_revision: mrd_ipc::DecimalU64::new(1),
                },
                now,
            )
            .await
            .unwrap();
        app.session_authorizations
            .bind_authenticated_peer_key(&request.session_id, controller_key.public_key(), now)
            .await
            .unwrap();
        app.session_authorizations
            .install_verified_wan_target_grant(
                crate::session_authorization::VerifiedSessionGrant {
                    grant_id: format!("sha256:{}", "3".repeat(64)),
                    session_id: request.session_id.clone(),
                    granted_scopes: scopes,
                    issued_at_ms: now,
                    expires_at_ms: now + 60_000,
                    policy_revision: 7,
                    route_constraint: "webrtc_relay".into(),
                    transport_fingerprint_sha256: [0x44; 32],
                },
                1,
                now,
            )
            .await
            .unwrap();
        reconcile_wan_session(&app, &request.session_id)
            .await
            .unwrap();
    }
    let input = Arc::new(StdMutex::new(Vec::new()));
    app.replace_control_input_for_test(RecordingInput(input.clone()))
        .await;
    app.control_input
        .lock()
        .await
        .handle_session_event(
            state.identity().session_id(),
            &ControlInputEvent::Key {
                key: ControlInputKey::VirtualKey { code: 65 },
                pressed: true,
            },
        )
        .unwrap();
    (app, coordinator, state, input)
}

#[tokio::test]
async fn browser_revoke_during_negotiation_joins_owned_sibling_without_reentering_security_gate() {
    let (app, coordinator, state, input) = fixture().await;
    let session = state.identity().session_id().clone();
    let sibling_app = app.clone();
    let sibling_coordinator = coordinator.clone();
    let sibling_session = session.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let mut cancellation = cancellation.into_receiver();
            ready_tx.send(()).unwrap();
            cancellation.changed().await.unwrap();
            finish_negotiation_failure(
                &sibling_app,
                &sibling_coordinator,
                &sibling_session,
                &cancellation,
                GenerationZeroNegotiationError::Cancelled,
            )
            .await;
        })
        .await
        .unwrap();
    ready_rx.await.unwrap();
    let watcher_app = app.clone();
    let watcher_session = session.clone();
    let (finished_tx, finished_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let failure = super::super::browser_authority::monitor_browser_authority(
                &RevokedBackend,
                &state,
                cancellation.into_receiver(),
                Duration::from_millis(2),
                Duration::from_millis(10),
                None,
            )
            .await
            .unwrap();
            let result = fail_wan_session(&watcher_app, &watcher_session, failure).await;
            let _ = finished_tx.send(result);
        })
        .await
        .unwrap();
    // Wait for terminal state so the watcher, rather than the assertion below,
    // owns the security gate while cancelling and joining its real sibling.
    while !coordinator
        .snapshot(&session)
        .await
        .unwrap()
        .phase()
        .is_terminal()
    {
        tokio::task::yield_now().await;
    }
    let gate_available = tokio::time::timeout(
        Duration::from_secs(1),
        app.authorization_security_gate.lock(),
    )
    .await
    .is_ok();
    let result = tokio::time::timeout(Duration::from_secs(7), finished_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_ok(),
        "owned cancellation must complete cleanup, not report CleanupTimeout: {result:?}"
    );
    assert!(
        gate_available,
        "cleanup must release the global gate promptly for other sessions"
    );
    assert_eq!(
        *input.lock().unwrap(),
        vec![
            InputEvent::Key {
                key: InputKey::VirtualKey(65),
                pressed: true
            },
            InputEvent::Key {
                key: InputKey::VirtualKey(65),
                pressed: false
            }
        ]
    );
    assert!(
        app.control_input
            .lock()
            .await
            .handle_session_event(
                &session,
                &ControlInputEvent::Key {
                    key: ControlInputKey::VirtualKey { code: 65 },
                    pressed: true
                }
            )
            .is_err(),
        "cleanup must keep the session input fence closed"
    );
}

#[tokio::test]
async fn unexpected_live_negotiation_cancel_still_terminalizes_and_releases_inputs() {
    let (app, coordinator, state, input) = fixture().await;
    let (_sender, cancellation) = watch::channel(false);
    finish_negotiation_failure(
        &app,
        &coordinator,
        state.identity().session_id(),
        &cancellation,
        GenerationZeroNegotiationError::Cancelled,
    )
    .await;
    let terminal = coordinator
        .snapshot(state.identity().session_id())
        .await
        .unwrap();
    assert_eq!(terminal.phase(), WanSessionPhase::Failed);
    assert_eq!(terminal.failure(), Some(WanSessionFailure::Cancelled));
    assert_eq!(input.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn owned_cancel_interrupts_a_negotiation_error_already_waiting_for_the_security_gate() {
    let (app, coordinator, state, _) = fixture().await;
    let session = state.identity().session_id().clone();
    let held_gate = app.authorization_security_gate.lock().await;
    let sibling_app = app.clone();
    let sibling_coordinator = coordinator.clone();
    let sibling_session = session.clone();
    let (ready_tx, ready_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let cancellation = cancellation.into_receiver();
            ready_tx.send(()).unwrap();
            finish_negotiation_failure(
                &sibling_app,
                &sibling_coordinator,
                &sibling_session,
                &cancellation,
                GenerationZeroNegotiationError::TransportUnavailable,
            )
            .await;
        })
        .await
        .unwrap();
    ready_rx.await.unwrap();
    tokio::task::yield_now().await;
    // Error handling began with cancellation=false, but cleanup must still be
    // able to cancel and join it while another owner holds the global gate.
    let completed = tokio::time::timeout(
        Duration::from_secs(1),
        coordinator.fail(&session, WanSessionFailure::PolicyMismatch),
    )
    .await;
    drop(held_gate);
    assert!(
        matches!(completed, Ok(Ok(_))),
        "owned cancellation must interrupt a queued gate lock: {completed:?}"
    );
}

use crate::wan_session::media::{
    start_verified_media_owned, WanMediaReadyEvidence, WanMediaReadySender,
};

struct ReadyOnAbort {
    sender: Option<WanMediaReadySender>,
    authority: WanMediaAuthority,
    ready_on_abort: bool,
}
impl Drop for ReadyOnAbort {
    fn drop(&mut self) {
        if self.ready_on_abort {
            if let Some(sender) = self.sender.take() {
                let _ = sender.send(Ok(WanMediaReadyEvidence::from_authority(
                    &self.authority,
                    1,
                )));
            }
        }
    }
}
struct RegisteredPendingMedia {
    app: Arc<crate::AppState>,
    ready_on_abort: bool,
    fail_on_start: bool,
    spawned: StdMutex<Option<oneshot::Sender<()>>>,
    registered: StdMutex<Option<oneshot::Sender<()>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
#[async_trait]
impl WanMediaActivationPort for RegisteredPendingMedia {
    async fn start_target_capture_send(
        &self,
        authority: &WanMediaAuthority,
    ) -> Result<WanMediaActivationReceipt, WanMediaActivationError> {
        let (receipt, sender) = WanMediaActivationReceipt::pending();
        let ready = ReadyOnAbort {
            sender: Some(sender),
            authority: authority.clone(),
            ready_on_abort: self.ready_on_abort,
        };
        let fail_on_start = self.fail_on_start;
        let task = tokio::spawn(async move {
            let mut ready = ready;
            if fail_on_start {
                let _ = ready
                    .sender
                    .take()
                    .unwrap()
                    .send(Err(WanMediaActivationError::StartupFailed));
            }
            let _ready = ready;
            std::future::pending::<()>().await;
        });
        if let Some(spawned) = self.spawned.lock().unwrap().take() {
            let _ = spawned.send(());
        }
        self.app
            .media_tasks
            .lock()
            .await
            .register(authority.session_id().clone(), task.abort_handle());
        *self.task.lock().await = Some(task);
        if let Some(registered) = self.registered.lock().unwrap().take() {
            let _ = registered.send(());
        }
        Ok(receipt)
    }
    async fn start_controller_receive_render(
        &self,
        _: &WanMediaAuthority,
    ) -> Result<WanMediaActivationReceipt, WanMediaActivationError> {
        unreachable!()
    }
}

async fn registered_media_revoke_race(ready_on_abort: bool) {
    let (app, coordinator, state, input) = fixture_with_media(true).await;
    let session = state.identity().session_id().clone();
    let (registered_tx, registered_rx) = oneshot::channel();
    let media = Arc::new(RegisteredPendingMedia {
        app: app.clone(),
        ready_on_abort,
        fail_on_start: false,
        spawned: StdMutex::new(None),
        registered: StdMutex::new(Some(registered_tx)),
        task: Mutex::new(None),
    });
    let activation_app = app.clone();
    let activation_coordinator = coordinator.clone();
    let activation_state = state.clone();
    let activation_media = media.clone();
    let (activation_tx, activation_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let cancellation = cancellation.into_receiver();
            let result = start_verified_media_owned(
                &activation_coordinator,
                &activation_state,
                activation_media.as_ref(),
                &cancellation,
                &activation_app.authorization_security_gate,
            )
            .await;
            if result.is_err() {
                let _ = fail_owned_wan_session(
                    &activation_app,
                    &activation_coordinator,
                    activation_state.identity().session_id(),
                    &cancellation,
                    WanSessionFailure::Transport,
                )
                .await;
            }
            let _ = activation_tx.send(result);
        })
        .await
        .unwrap();
    registered_rx.await.unwrap();
    assert_eq!(app.media_tasks.lock().await.active_count(&session), 1);
    let watcher_app = app.clone();
    let watcher_session = session.clone();
    let (finished_tx, finished_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |_cancellation| async move {
            let result = fail_wan_session(
                &watcher_app,
                &watcher_session,
                WanSessionFailure::PolicyMismatch,
            )
            .await;
            let _ = finished_tx.send(result);
        })
        .await
        .unwrap();
    while !coordinator
        .snapshot(&session)
        .await
        .unwrap()
        .phase()
        .is_terminal()
    {
        tokio::task::yield_now().await;
    }
    let gate_available = tokio::time::timeout(
        Duration::from_secs(1),
        app.authorization_security_gate.lock(),
    )
    .await
    .is_ok();
    let result = tokio::time::timeout(Duration::from_secs(7), finished_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_ok(),"registered media receipt cancellation must not reenter coordinator cleanup (ready={ready_on_abort}): {result:?}");
    assert!(
        gate_available,
        "media receipt cancellation must promptly release the global gate"
    );
    assert!(activation_rx.await.unwrap().is_err());
    assert_eq!(
        coordinator.snapshot(&session).await.unwrap().phase(),
        WanSessionPhase::Failed
    );
    assert_eq!(app.media_tasks.lock().await.active_count(&session), 0);
    let task = media.task.lock().await.take().unwrap();
    assert!(
        task.await.unwrap_err().is_cancelled(),
        "real cleanup must own and abort the registered task"
    );
    assert_eq!(
        input.lock().unwrap().len(),
        2,
        "held inputs must be released"
    );
}

#[tokio::test]
async fn browser_revoke_aborts_registered_pending_receipt_without_cleanup_reentry() {
    registered_media_revoke_race(false).await;
}
#[tokio::test]
async fn browser_revoke_racing_ready_success_does_not_commit_streaming_or_reenter_cleanup() {
    registered_media_revoke_race(true).await;
}

#[tokio::test]
async fn media_startup_registration_is_owned_before_revoke_can_cleanup() {
    let (app, coordinator, state, _) = fixture_with_media(true).await;
    let session = state.identity().session_id().clone();
    let registry_guard = app.media_tasks.lock().await;
    let (spawned_tx, spawned_rx) = oneshot::channel();
    let (registered_tx, registered_rx) = oneshot::channel();
    let media = Arc::new(RegisteredPendingMedia {
        app: app.clone(),
        ready_on_abort: false,
        fail_on_start: false,
        spawned: StdMutex::new(Some(spawned_tx)),
        registered: StdMutex::new(Some(registered_tx)),
        task: Mutex::new(None),
    });
    let activation_app = app.clone();
    let activation_coordinator = coordinator.clone();
    let activation_state = state.clone();
    let activation_media = media.clone();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let cancellation = cancellation.into_receiver();
            let _ = start_verified_media_owned(
                &activation_coordinator,
                &activation_state,
                activation_media.as_ref(),
                &cancellation,
                &activation_app.authorization_security_gate,
            )
            .await;
        })
        .await
        .unwrap();
    spawned_rx.await.unwrap();
    let watcher_app = app.clone();
    let watcher_session = session.clone();
    let watcher = tokio::spawn(async move {
        fail_wan_session(
            &watcher_app,
            &watcher_session,
            WanSessionFailure::PolicyMismatch,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let cleanup_waited_for_registration =
        coordinator.snapshot(&session).await.unwrap().phase() == WanSessionPhase::RelayVerified;
    drop(registry_guard);
    registered_rx.await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(7), watcher)
        .await
        .unwrap()
        .unwrap();
    assert!(
        cleanup_waited_for_registration,
        "startup spawn/register must be atomic with service cleanup"
    );
    assert!(
        result.is_ok(),
        "startup revoke must not leave an unowned task or cleanup timeout: {result:?}"
    );
    assert_eq!(app.media_tasks.lock().await.active_count(&session), 0);
    assert!(media
        .task
        .lock()
        .await
        .take()
        .unwrap()
        .await
        .unwrap_err()
        .is_cancelled());
}

#[tokio::test]
async fn live_registered_media_startup_error_still_terminalizes_and_releases_input() {
    let (app, coordinator, state, input) = fixture_with_media(true).await;
    let session = state.identity().session_id().clone();
    let media = Arc::new(RegisteredPendingMedia {
        app: app.clone(),
        ready_on_abort: false,
        fail_on_start: true,
        spawned: StdMutex::new(None),
        registered: StdMutex::new(None),
        task: Mutex::new(None),
    });
    let activation_app = app.clone();
    let activation_coordinator = coordinator.clone();
    let activation_media = media.clone();
    let activation_state = state.clone();
    let (finished_tx, finished_rx) = oneshot::channel();
    coordinator
        .spawn_owned_task(&session, move |cancellation| async move {
            let cancellation = cancellation.into_receiver();
            let result = start_verified_media_owned(
                &activation_coordinator,
                &activation_state,
                activation_media.as_ref(),
                &cancellation,
                &activation_app.authorization_security_gate,
            )
            .await;
            if result.is_err() {
                let _ = fail_owned_wan_session(
                    &activation_app,
                    &activation_coordinator,
                    activation_state.identity().session_id(),
                    &cancellation,
                    WanSessionFailure::Transport,
                )
                .await;
            }
            let _ = finished_tx.send(result);
        })
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(1), finished_rx)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    // The service failure wrapper owns gated cleanup and publishes the
    // authorization and IPC terminal state before reporting completion.
    assert!(coordinator
        .fail(&session, WanSessionFailure::Transport)
        .await
        .is_ok());
    assert_eq!(
        coordinator.snapshot(&session).await.unwrap().failure(),
        Some(WanSessionFailure::Transport)
    );
    assert_eq!(app.media_tasks.lock().await.active_count(&session), 0);
    assert!(media
        .task
        .lock()
        .await
        .take()
        .unwrap()
        .await
        .unwrap_err()
        .is_cancelled());
    assert_eq!(input.lock().unwrap().len(), 2);
    assert_eq!(
        app.session_authorizations
            .snapshot(&session)
            .await
            .unwrap()
            .authorization_state,
        RemoteAuthorizationState::Revoked,
        "live media error must revoke service authorization before returning"
    );
    assert!(app
        .session_authorizations
        .active_grant(&session)
        .await
        .is_none());
    assert_eq!(
        app.sessions
            .lock()
            .await
            .get(&session)
            .unwrap()
            .lifecycle_state
            .as_str(),
        "failed",
        "live media error must immediately publish failed IPC state"
    );
}
