use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_session::{
    AuthorizationState, CapabilitySet, MediaState, RemoteSessionAggregate, RouteKind,
    RouteState, SessionPlan, SessionTransitionError,
};

fn fixture_plan() -> SessionPlan {
    SessionPlan {
        session_id: SessionId("session-1".into()),
        initiator: DeviceId("controller".into()),
        target: DeviceId("target".into()),
        role: BackendRole::Controller,
        capabilities: CapabilitySet { supports_webrtc: true, supports_quic: true },
    }
}

#[test]
fn media_cannot_start_before_authorization() {
    let mut session = RemoteSessionAggregate::new(fixture_plan());
    assert_eq!(
        session.start_media(),
        Err(SessionTransitionError::AuthorizationRequired)
    );
}

#[test]
fn route_migration_preserves_granted_scopes() {
    let mut session = RemoteSessionAggregate::new(fixture_plan());
    session.authorize(vec!["screen_view".into(), "input_pointer".into()], 3).unwrap();
    session.begin_route_migration(RouteKind::LanQuic).unwrap();
    session.complete_route_migration(RouteKind::LanQuic).unwrap();
    let scopes = session.granted_scopes().to_vec();
    session.begin_route_migration(RouteKind::WebRtcRelay).unwrap();
    session.complete_route_migration(RouteKind::WebRtcRelay).unwrap();
    assert_eq!(session.granted_scopes(), scopes.as_slice());
    assert_eq!(session.route_state(), &RouteState::Active(RouteKind::WebRtcRelay));
}

#[test]
fn denied_authorization_is_terminal_for_media() {
    let mut session = RemoteSessionAggregate::new(fixture_plan());
    session.deny_authorization("local consent denied");
    assert!(matches!(session.authorization_state(), AuthorizationState::Denied { .. }));
    assert_eq!(session.start_media(), Err(SessionTransitionError::AuthorizationDenied));
    assert_eq!(session.media_state(), &MediaState::Idle);
}
