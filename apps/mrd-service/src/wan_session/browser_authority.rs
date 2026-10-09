//! Continuously re-check browser authority while its retained WebRTC peer is live.
use super::{
    coordinator::{SystemWanSessionClock, WanSessionClock, WanSessionWorkflowBackend},
    model::{WanSessionFailure, WanSessionRole, WanSessionState},
};
use std::time::Duration;
use tokio::sync::watch;

// The backend reserves browser_ for ephemeral browser principals and rejects
// physical enrollment with this prefix. The service only calls this after the
// signed intent, backend principal, local consent, and signed grant are bound.
pub(crate) fn requires_browser_authority_watch(state: &WanSessionState) -> bool {
    state.role() == WanSessionRole::Target
        && state
            .identity()
            .controller_device_id()
            .0
            .starts_with("browser_")
}

pub(crate) async fn monitor_browser_authority(
    backend: &dyn WanSessionWorkflowBackend,
    state: &WanSessionState,
    mut cancellation: watch::Receiver<bool>,
    poll_interval: Duration,
    inspect_timeout: Duration,
    runtime: Option<&crate::transports::webrtc::ServiceWebRtcTransportHost>,
) -> Option<WanSessionFailure> {
    if !requires_browser_authority_watch(state) {
        return None;
    }
    loop {
        if *cancellation.borrow() {
            return None;
        }
        if let Some(runtime) = runtime {
            let generation = match tokio::time::timeout(
                inspect_timeout,
                runtime.active_wan_generation(state.identity().session_id()),
            )
            .await
            {
                Ok(generation) => generation,
                Err(_) => return Some(WanSessionFailure::RouteMismatch),
            };
            if !browser_generation_is_authorized(state, generation) {
                return Some(WanSessionFailure::RouteMismatch);
            }
        }
        let now = SystemWanSessionClock.now_unix_ms();
        let grant = match state.grant() {
            Some(grant) => grant,
            None => return Some(WanSessionFailure::PolicyMismatch),
        };
        let deadline = state
            .identity()
            .deadline_unix_ms()
            .min(grant.grant_expires_at_ms())
            .min(grant.policy_expires_at_ms());
        if state.phase().is_terminal() || now >= deadline {
            return Some(WanSessionFailure::PolicyMismatch);
        }
        let bounded_timeout = inspect_timeout.min(Duration::from_millis(deadline - now));
        let snapshot = tokio::select! {
         biased;
         _=cancellation.changed()=>return None,
         result=tokio::time::timeout(bounded_timeout,backend.inspect(state.identity().binding(),now.saturating_add(bounded_timeout.as_millis().min(u64::MAX as u128) as u64)))=>match result { Ok(Ok(snapshot))=>snapshot,_=>return Some(WanSessionFailure::PolicyMismatch) },
        };
        let current = snapshot.grant();
        let valid = snapshot.status() == super::backend::WanSessionStatus::Approved
            && snapshot.request().session_id == *state.identity().session_id()
            && snapshot.request().controller_device_id == *state.identity().controller_device_id()
            && snapshot.request().target_device_id == *state.identity().target_device_id()
            && state.request_commitment() == Some(snapshot.request_commitment())
            && current.is_some_and(|current| {
                current.request_commitment() == grant.request_commitment()
                    && current.approved_scopes() == grant.approved_scopes()
                    && current.approved_profile() == grant.approved_profile()
                    && current.policy_revision() == grant.policy_revision()
                    && current.policy_expires_at_ms() == grant.policy_expires_at_ms()
                    && current.grant_expires_at_ms() == grant.grant_expires_at_ms()
                    && current.route_policy() == grant.route_policy()
            })
            && SystemWanSessionClock.now_unix_ms() < deadline;
        if !valid {
            return Some(WanSessionFailure::PolicyMismatch);
        }
        tokio::select! { biased; _=cancellation.changed()=>return None, _=tokio::time::sleep(poll_interval.min(Duration::from_millis(deadline-SystemWanSessionClock.now_unix_ms().min(deadline))))=>{} }
    }
}

pub(crate) fn browser_generation_is_authorized(
    state: &WanSessionState,
    generation: Option<u64>,
) -> bool {
    !requires_browser_authority_watch(state) || generation.is_none_or(|generation| generation == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wan_session::{
        backend::{WanSessionApproval, WanSessionBinding},
        coordinator::{
            SystemWanSessionClock, WanBackendSessionSnapshot, WanSessionCleanup, WanSessionClock,
            WanSessionCoordinator, WanSessionCoordinatorError, WanSessionPortError,
            WanSessionWorkflowBackend,
        },
        model::{
            GrantBinding, RelayAccessBinding, RelayRouteProof, WanSessionEvent, WanSessionFailure,
            WanSessionIdentity, WanSessionRole, WanSessionState,
        },
    };
    use async_trait::async_trait;
    use mrd_proto::{DeviceId, SessionId};
    use mrd_signal_proto::{
        WanAccessModeV3, WanPermissionScopeV3, WanRoutePolicyV3, WanSessionRequestV3,
    };
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };
    use tokio::sync::watch;

    struct Backend {
        replies: Mutex<VecDeque<Result<WanBackendSessionSnapshot, WanSessionPortError>>>,
        calls: AtomicUsize,
        stall: bool,
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
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.stall {
                std::future::pending::<()>().await;
            }
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(WanSessionPortError::Rejected))
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

    fn fixture(controller: &str) -> (WanSessionState, WanSessionRequestV3, GrantBinding) {
        let now = SystemWanSessionClock.now_unix_ms();
        let request = WanSessionRequestV3 {
            session_id: SessionId("browser-authority-fixture".into()),
            idempotency_key: [5; 16],
            controller_device_id: DeviceId(controller.into()),
            target_device_id: DeviceId("physical-target".into()),
            access_mode: WanAccessModeV3::Attended,
            requested_scopes: vec![
                WanPermissionScopeV3::InputKeyboard,
                WanPermissionScopeV3::InputPointer,
                WanPermissionScopeV3::ScreenView,
            ],
            requested_profile: None,
            route_policy: WanRoutePolicyV3::RelayOnly,
        };
        let commitment = request.commitment().unwrap();
        let grant = GrantBinding::new(
            commitment.clone(),
            request.requested_scopes.clone(),
            7,
            now + 60_000,
            now + 60_000,
            request.route_policy,
        )
        .unwrap();
        let identity = WanSessionIdentity::new(
            request.session_id.clone(),
            request.controller_device_id.clone(),
            request.target_device_id.clone(),
            "1".repeat(64),
            "2".repeat(64),
            now + 60_000,
        )
        .unwrap();
        let access = RelayAccessBinding::generation_zero(
            7,
            "directory".into(),
            "primary".into(),
            "3".repeat(64),
        )
        .unwrap();
        let mut state = WanSessionState::new(WanSessionRole::Target, identity);
        for event in [
            WanSessionEvent::BackendBound {
                request_commitment: commitment,
            },
            WanSessionEvent::AwaitingConsent {
                intent_commitment: "4".repeat(64),
            },
            WanSessionEvent::Granted(grant.clone().with_grant_commitment("6".repeat(64)).unwrap()),
            WanSessionEvent::AccessBound(access.clone()),
            WanSessionEvent::Negotiating,
            WanSessionEvent::RelayVerified(RelayRouteProof::for_test(&access, true, true).unwrap()),
            WanSessionEvent::Streaming,
        ] {
            state.apply(event, now).unwrap();
        }
        (state, request, grant)
    }

    fn backend(replies: Vec<Result<WanBackendSessionSnapshot, WanSessionPortError>>) -> Backend {
        Backend {
            replies: Mutex::new(replies.into()),
            calls: AtomicUsize::new(0),
            stall: false,
        }
    }

    async fn monitor(backend: &Backend, state: &WanSessionState) -> Option<WanSessionFailure> {
        let (_sender, cancel) = watch::channel(false);
        tokio::time::timeout(
            Duration::from_millis(100),
            monitor_browser_authority(
                backend,
                state,
                cancel,
                Duration::from_millis(1),
                Duration::from_millis(5),
                None,
            ),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn retained_browser_peer_is_revoked_after_an_approved_inspection() {
        let (state, request, grant) = fixture("browser_0123456789abcdef");
        let snapshot = WanBackendSessionSnapshot::approved(
            request.clone(),
            request.commitment().unwrap(),
            grant,
        )
        .unwrap();
        let backend = backend(vec![Ok(snapshot), Err(WanSessionPortError::Rejected)]);
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
        assert_eq!(backend.calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn backend_unavailable_fails_closed() {
        let (state, _, _) = fixture("browser_0123456789abcdef");
        let backend = backend(vec![Err(WanSessionPortError::Unavailable)]);
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
    }

    #[tokio::test]
    async fn stalled_inspection_is_bounded() {
        let (state, _, _) = fixture("browser_0123456789abcdef");
        let mut backend = backend(vec![]);
        backend.stall = true;
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
    }

    #[tokio::test]
    async fn changed_policy_is_not_allowed_by_the_retained_grant() {
        let (state, request, _) = fixture("browser_0123456789abcdef");
        let now = SystemWanSessionClock.now_unix_ms();
        let changed = GrantBinding::new(
            request.commitment().unwrap(),
            vec![WanPermissionScopeV3::ScreenView],
            8,
            now + 60_000,
            now + 60_000,
            request.route_policy,
        )
        .unwrap();
        let snapshot = WanBackendSessionSnapshot::approved(
            request.clone(),
            request.commitment().unwrap(),
            changed,
        )
        .unwrap();
        let backend = backend(vec![Ok(snapshot)]);
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
    }

    #[tokio::test]
    async fn requested_record_cannot_preserve_streaming_authority() {
        let (state, request, _) = fixture("browser_0123456789abcdef");
        let snapshot =
            WanBackendSessionSnapshot::requested(request.clone(), request.commitment().unwrap())
                .unwrap();
        let backend = backend(vec![Ok(snapshot)]);
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
    }

    #[tokio::test]
    async fn another_target_record_cannot_preserve_streaming_authority() {
        let (state, mut request, _) = fixture("browser_0123456789abcdef");
        request.target_device_id = DeviceId("another-physical-target".into());
        let now = SystemWanSessionClock.now_unix_ms();
        let grant = GrantBinding::new(
            request.commitment().unwrap(),
            request.requested_scopes.clone(),
            7,
            now + 60_000,
            now + 60_000,
            request.route_policy,
        )
        .unwrap();
        let snapshot = WanBackendSessionSnapshot::approved(
            request.clone(),
            request.commitment().unwrap(),
            grant,
        )
        .unwrap();
        let backend = backend(vec![Ok(snapshot)]);
        assert_eq!(
            monitor(&backend, &state).await,
            Some(WanSessionFailure::PolicyMismatch)
        );
    }

    #[tokio::test]
    async fn physical_controller_does_not_add_backend_polling() {
        let (state, _, _) = fixture("physical-controller");
        let backend = backend(vec![]);
        assert_eq!(monitor(&backend, &state).await, None);
        assert_eq!(backend.calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn cancelled_browser_watch_does_not_inspect_or_fail() {
        let (state, _, _) = fixture("browser_0123456789abcdef");
        let backend = backend(vec![]);
        let (_sender, cancel) = watch::channel(true);
        assert_eq!(
            monitor_browser_authority(
                &backend,
                &state,
                cancel,
                Duration::from_millis(1),
                Duration::from_millis(5),
                None,
            )
            .await,
            None
        );
        assert_eq!(backend.calls.load(Ordering::Relaxed), 0);
    }

    #[derive(Default)]
    struct Cleanup(Mutex<Vec<&'static str>>);
    #[async_trait]
    impl WanSessionCleanup for Cleanup {
        async fn freeze_input(&self, _: &SessionId) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("freeze_input");
            Ok(())
        }
        async fn stop_media(&self, _: &SessionId) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("stop_media");
            Ok(())
        }
        async fn close_transport(&self, _: &SessionId) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("close_transport");
            Ok(())
        }
        async fn remove_failover(&self, _: &SessionId) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("remove_failover");
            Ok(())
        }
        async fn clear_signaling(&self, _: &SessionId) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("clear_signaling");
            Ok(())
        }
        async fn close_backend(
            &self,
            _: &SessionId,
            _: bool,
        ) -> Result<(), WanSessionCoordinatorError> {
            self.0.lock().unwrap().push("close_backend");
            Ok(())
        }
    }

    #[tokio::test]
    async fn browser_authority_loss_runs_coordinator_cleanup_without_a_signed_close() {
        let (state, _, _) = fixture("browser_0123456789abcdef");
        let cleanup = Arc::new(Cleanup::default());
        let coordinator = Arc::new(
            WanSessionCoordinator::new(
                Default::default(),
                cleanup.clone(),
                Arc::new(SystemWanSessionClock),
            )
            .unwrap(),
        );
        coordinator.begin(state.clone()).await.unwrap();
        let backend = backend(vec![Err(WanSessionPortError::Rejected)]);
        let loss = monitor(&backend, &state).await;
        if let Some(failure) = loss {
            coordinator
                .fail(state.identity().session_id(), failure)
                .await
                .unwrap();
        }
        assert!(coordinator
            .snapshot(state.identity().session_id())
            .await
            .unwrap()
            .phase()
            .is_terminal());
        assert_eq!(
            *cleanup.0.lock().unwrap(),
            [
                "freeze_input",
                "stop_media",
                "close_transport",
                "remove_failover",
                "clear_signaling",
                "close_backend"
            ]
        );
    }
    #[test]
    fn browser_sessions_cannot_keep_authority_after_a_native_ice_migration() {
        let (browser, _, _) = fixture("browser_0123456789abcdef");
        assert!(browser_generation_is_authorized(&browser, None));
        assert!(browser_generation_is_authorized(&browser, Some(0)));
        assert!(!browser_generation_is_authorized(&browser, Some(1)));
        let (native, _, _) = fixture("physical-controller");
        assert!(
            browser_generation_is_authorized(&native, Some(1)),
            "native verified atomic migration remains supported"
        );
    }
}
