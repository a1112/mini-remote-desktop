use super::*;
use crate::wan_session::{
    media::WanMediaAuthority,
    model::{
        GrantBinding, RelayAccessBinding, RelayRouteProof, WanSessionEvent, WanSessionIdentity,
        WanSessionRole, WanSessionState,
    },
};
use mrd_application::ports::TransportRouteSnapshot;
use mrd_signal_proto::{WanPermissionScopeV3, WanRoutePolicyV3};

fn authority(policy: WanRoutePolicyV3, local_relay: bool, remote_relay: bool) -> WanMediaAuthority {
    let now = now_ms();
    let access = RelayAccessBinding::generation_zero(
        7,
        "directory".into(),
        "primary".into(),
        "3".repeat(64),
    )
    .unwrap();
    let mut state = WanSessionState::new(
        WanSessionRole::Target,
        WanSessionIdentity::new(
            SessionId("browser-direct".into()),
            DeviceId("browser_controller".into()),
            DeviceId("target".into()),
            "1".repeat(64),
            "2".repeat(64),
            now + 60_000,
        )
        .unwrap(),
    );
    let grant = GrantBinding::new(
        "4".repeat(64),
        vec![
            WanPermissionScopeV3::InputPointer,
            WanPermissionScopeV3::ScreenView,
        ],
        7,
        now + 50_000,
        now + 50_000,
        policy,
    )
    .unwrap()
    .with_grant_commitment("5".repeat(64))
    .unwrap();
    for event in [
        WanSessionEvent::BackendBound {
            request_commitment: "4".repeat(64),
        },
        WanSessionEvent::AwaitingConsent {
            intent_commitment: "6".repeat(64),
        },
        WanSessionEvent::Granted(grant),
        WanSessionEvent::AccessBound(access.clone()),
        WanSessionEvent::Negotiating,
        WanSessionEvent::RelayVerified(
            RelayRouteProof::from_verified_policy(&access, policy, local_relay, remote_relay)
                .unwrap(),
        ),
    ] {
        state.apply(event, now).unwrap();
    }
    WanMediaAuthority::from_relay_verified(&state)
        .expect("a verified DirectFirst selected pair must authorize media")
}
struct Mux(tokio::sync::Mutex<TransportRouteSnapshot>);
#[async_trait]
impl TransportMuxPort for Mux {
    async fn send(&self, _: TransportEnvelope) -> anyhow::Result<TransportSendOutcome> {
        Ok(TransportSendOutcome::Enqueued)
    }
    async fn recv(&self, _: TransportLane) -> anyhow::Result<Option<TransportEnvelope>> {
        Ok(None)
    }
    async fn route_snapshot(&self) -> TransportRouteSnapshot {
        self.0.lock().await.clone()
    }
    async fn close(&self) -> anyhow::Result<()> {
        self.0.lock().await.closed = true;
        Ok(())
    }
}
fn mux(authority: &WanMediaAuthority, kind: TransportRouteKind) -> Arc<Mux> {
    Arc::new(Mux(tokio::sync::Mutex::new(TransportRouteSnapshot::new(
        authority.session_id().clone(),
        kind,
        "local",
        "peer",
    ))))
}

#[tokio::test]
async fn verified_direct_pair_binds_but_an_untrusted_mux_cannot_resolve_for_input() {
    let authority = authority(WanRoutePolicyV3::DirectFirst, false, false);
    let app = Arc::new(AppState::default());
    let mux = mux(&authority, TransportRouteKind::WebRtcDirect);
    bind_verified_mux(&app, authority.clone(), mux.clone())
        .await
        .unwrap();
    let port = ServiceWanControlInputPort::new(&app);
    assert!(
        port.resolve_binding(authority.session_id()).await.is_err(),
        "a route snapshot alone cannot replace the service host generation-zero proof"
    );
    mux.0.lock().await.kind = TransportRouteKind::WebRtcRelay;
    assert!(
        port.resolve_binding(authority.session_id()).await.is_err(),
        "a selected-pair change requires new verified authority"
    );
}
#[tokio::test]
async fn verified_direct_authority_cannot_bind_a_relay_or_another_session() {
    let authority = authority(WanRoutePolicyV3::DirectFirst, false, false);
    let app = Arc::new(AppState::default());
    assert!(bind_verified_mux(
        &app,
        authority.clone(),
        mux(&authority, TransportRouteKind::WebRtcRelay)
    )
    .await
    .is_err());
    let other = mux(&authority, TransportRouteKind::WebRtcDirect);
    other.0.lock().await.session_id = SessionId("another".into());
    assert!(bind_verified_mux(&app, authority, other).await.is_err());
}
#[tokio::test]
async fn relay_only_authority_still_refuses_direct_selected_pairs() {
    let authority = authority(WanRoutePolicyV3::RelayOnly, true, true);
    let app = Arc::new(AppState::default());
    assert!(bind_verified_mux(
        &app,
        authority.clone(),
        mux(&authority, TransportRouteKind::WebRtcDirect)
    )
    .await
    .is_err());
    assert!(RelayRouteProof::from_verified_policy(
        &RelayAccessBinding::generation_zero(
            7,
            "directory".into(),
            "primary".into(),
            "3".repeat(64)
        )
        .unwrap(),
        WanRoutePolicyV3::RelayOnly,
        true,
        false
    )
    .is_err());
}
#[tokio::test]
async fn direct_first_turn_fallback_binds_only_its_verified_relay_route() {
    let authority = authority(WanRoutePolicyV3::DirectFirst, true, true);
    let app = Arc::new(AppState::default());
    assert!(bind_verified_mux(
        &app,
        authority.clone(),
        mux(&authority, TransportRouteKind::WebRtcRelay)
    )
    .await
    .is_ok());
    assert!(bind_verified_mux(
        &Arc::new(AppState::default()),
        authority.clone(),
        mux(&authority, TransportRouteKind::WebRtcDirect)
    )
    .await
    .is_err());
}

#[tokio::test]
async fn direct_first_mixed_pair_matches_the_actual_turn_route_classification() {
    for (local, remote) in [(true, false), (false, true)] {
        let authority = authority(WanRoutePolicyV3::DirectFirst, local, remote);
        let app = Arc::new(AppState::default());
        assert!(bind_verified_mux(
            &app,
            authority.clone(),
            mux(&authority, TransportRouteKind::WebRtcRelay)
        )
        .await
        .is_ok());
        assert!(bind_verified_mux(
            &Arc::new(AppState::default()),
            authority.clone(),
            mux(&authority, TransportRouteKind::WebRtcDirect)
        )
        .await
        .is_err());
    }
}
