use mrd_agent_ipc::MediaCodec;
use mrd_proto::SessionId;
use mrd_service::agent_runtime::{AgentRenderRouteError, AgentRenderRouteRegistry};

#[test]
fn render_routes_bind_session_resource_sequence_and_explicit_revocation() {
    let mut routes = AgentRenderRouteRegistry::new(1).expect("bounded registry");
    let session = SessionId("session-1".to_string());
    assert_eq!(
        routes.install(session.clone(), "binding-1", [7; 16]),
        Ok(())
    );
    assert_eq!(
        routes.install(session.clone(), "binding-2", [8; 16]),
        Err(AgentRenderRouteError::DuplicateSession)
    );

    let prepared = routes
        .prepare(&session, 4, 5, MediaCodec::H264, true, vec![1, 2, 3])
        .expect("prepare exact route");
    assert_eq!(prepared.binding(), &"binding-1");
    assert_eq!(prepared.unit().resource_id, [7; 16]);
    assert_eq!(prepared.unit().session_id, "session-1");
    assert_eq!(prepared.unit().sequence, 4);

    assert_eq!(
        routes.prepare(&session, 4, 6, MediaCodec::H264, false, vec![4]),
        Err(AgentRenderRouteError::NonMonotonicSequence)
    );
    assert_eq!(routes.remove(&session), Some("binding-1"));
    assert_eq!(
        routes.prepare(&session, 5, 7, MediaCodec::H264, false, vec![5]),
        Err(AgentRenderRouteError::MissingSession)
    );
}

#[test]
fn render_route_capacity_and_invalid_resources_fail_closed() {
    assert!(AgentRenderRouteRegistry::<u8>::new(0).is_none());
    let mut routes = AgentRenderRouteRegistry::new(1).unwrap();
    assert_eq!(
        routes.install(SessionId("zero".into()), 1, [0; 16]),
        Err(AgentRenderRouteError::InvalidResource)
    );
    routes
        .install(SessionId("first".into()), 1, [1; 16])
        .unwrap();
    assert_eq!(
        routes.install(SessionId("second".into()), 2, [2; 16]),
        Err(AgentRenderRouteError::CapacityExceeded)
    );
}

#[test]
fn pending_render_route_requires_explicit_activation_or_cancellation() {
    let mut routes = AgentRenderRouteRegistry::new(1).unwrap();
    let session = SessionId("pending".into());
    routes
        .reserve(session.clone(), "binding", [3; 16])
        .expect("reserve route before StartRender");
    assert_eq!(
        routes.prepare(&session, 1, 1, MediaCodec::H264, true, vec![1]),
        Err(AgentRenderRouteError::PendingSession)
    );
    assert!(routes.activate(&session));
    assert!(routes
        .prepare(&session, 1, 1, MediaCodec::H264, true, vec![1])
        .is_ok());

    assert_eq!(routes.remove(&session), Some("binding"));
    routes
        .reserve(session.clone(), "replacement", [4; 16])
        .unwrap();
    assert_eq!(routes.cancel(&session), Some("replacement"));
    assert_eq!(
        routes.prepare(&session, 2, 2, MediaCodec::H264, false, vec![2]),
        Err(AgentRenderRouteError::MissingSession)
    );
}
