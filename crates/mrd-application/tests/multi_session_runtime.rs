// Multi-session regression tests
//
// These tests verify that multiple sessions can be orchestrated
// correctly without state being collapsed or lost.

use mrd_proto::{DeviceId, SessionId};
use mrd_session::QuicSessionCoordinator;

#[test]
fn multiple_sessions_can_be_tracked_independently() {
    let mut coordinator = QuicSessionCoordinator::default();

    let session1 = SessionId("session-1".to_string());
    let session2 = SessionId("session-2".to_string());

    // Request first session
    coordinator
        .request_session(
            session1.clone(),
            DeviceId("controller-1".to_string()),
            DeviceId("agent-1".to_string()),
            "quic_quinn".to_string(),
            Some("127.0.0.1:5000".to_string()),
            Some("localhost".to_string()),
            Some("cert1".to_string()),
        )
        .expect("request session 1");

    // Request second session
    coordinator
        .request_session(
            session2.clone(),
            DeviceId("controller-2".to_string()),
            DeviceId("agent-2".to_string()),
            "quic_quinn".to_string(),
            Some("127.0.0.1:6000".to_string()),
            Some("localhost".to_string()),
            Some("cert2".to_string()),
        )
        .expect("request session 2");

    // Verify both sessions exist independently
    let snap1 = coordinator.snapshot(&session1).expect("session 1 snapshot");
    let snap2 = coordinator.snapshot(&session2).expect("session 2 snapshot");

    assert_eq!(snap1.local_listen_addr, Some("127.0.0.1:5000".to_string()));
    assert_eq!(snap2.local_listen_addr, Some("127.0.0.1:6000".to_string()));

    // Verify sessions are not collapsed
    assert_ne!(snap1.local_listen_addr, snap2.local_listen_addr);
}

#[test]
fn accept_session_adds_remote_bootstrap_without_affecting_local() {
    let mut coordinator = QuicSessionCoordinator::default();

    let session = SessionId("session-accept".to_string());

    // First, request as controller (sets local bootstrap)
    coordinator
        .request_session(
            session.clone(),
            DeviceId("controller".to_string()),
            DeviceId("agent".to_string()),
            "quic_quinn".to_string(),
            Some("127.0.0.1:5000".to_string()),
            Some("controller-host".to_string()),
            Some("controller-cert".to_string()),
        )
        .expect("request session");

    // Then, accept as agent (adds remote bootstrap)
    coordinator
        .accept_session(
            session.clone(),
            "quic_quinn".to_string(),
            Some("127.0.0.1:6000".to_string()),
            Some("agent-host".to_string()),
            Some("agent-cert".to_string()),
        )
        .expect("accept session");

    let snap = coordinator.snapshot(&session).expect("session snapshot");

    // Verify local bootstrap is preserved
    assert_eq!(snap.local_listen_addr, Some("127.0.0.1:5000".to_string()));
    assert_eq!(snap.local_server_name, Some("controller-host".to_string()));

    // Verify remote bootstrap is added
    assert_eq!(snap.remote_listen_addr, Some("127.0.0.1:6000".to_string()));
    assert_eq!(snap.remote_server_name, Some("agent-host".to_string()));
}

#[test]
fn session_not_found_returns_none() {
    let coordinator = QuicSessionCoordinator::default();
    let session = SessionId("nonexistent".to_string());

    let snap = coordinator.snapshot(&session);
    assert!(snap.is_none(), "Non-existent session should return None");
}

#[test]
fn multiple_realtime_events_in_one_drain_cycle() {
    use mrd_application::ports::{SessionCoordinatorPort, SessionSnapshot, SignalingPort};
    use mrd_signal_proto::{SessionAccept, SessionRequest, SignalMessage};
    use std::sync::{Arc, Mutex};

    // Mock signaling port that returns multiple events
    struct MockSignalingPort {
        events: Vec<SignalMessage>,
    }

    #[async_trait::async_trait]
    impl SignalingPort for MockSignalingPort {
        async fn drain_events(&self, _handle: u64) -> anyhow::Result<Vec<SignalMessage>> {
            Ok(self.events.clone())
        }

        async fn device_id(&self, _handle: u64) -> anyhow::Result<mrd_proto::DeviceId> {
            Ok(mrd_proto::DeviceId("test-device".to_string()))
        }
    }

    // Mock session coordinator
    struct MockSessionCoordinator {
        sessions: Arc<Mutex<std::collections::HashMap<mrd_proto::SessionId, SessionSnapshot>>>,
    }

    impl SessionCoordinatorPort for MockSessionCoordinator {
        fn request_session(
            &mut self,
            session_id: mrd_proto::SessionId,
            _source_device_id: mrd_proto::DeviceId,
            _target_device_id: mrd_proto::DeviceId,
            transport: String,
            local_listen_addr: Option<String>,
            local_server_name: Option<String>,
            local_cert_der_b64: Option<String>,
        ) -> anyhow::Result<()> {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.insert(
                session_id.clone(),
                SessionSnapshot {
                    session_id,
                    transport,
                    source_device_id: None,
                    target_device_id: None,
                    local_listen_addr,
                    local_server_name,
                    local_cert_der_b64,
                    remote_listen_addr: None,
                    remote_server_name: None,
                    remote_cert_der_b64: None,
                    lifecycle_state: "created".to_string(),
                    last_error: None,
                    sender_active: false,
                    receiver_active: false,
                },
            );
            Ok(())
        }

        fn accept_session(
            &mut self,
            session_id: mrd_proto::SessionId,
            transport: String,
            remote_listen_addr: Option<String>,
            remote_server_name: Option<String>,
            remote_cert_der_b64: Option<String>,
        ) -> anyhow::Result<()> {
            let mut sessions = self.sessions.lock().unwrap();
            sessions.insert(
                session_id.clone(),
                SessionSnapshot {
                    session_id,
                    transport,
                    source_device_id: None,
                    target_device_id: None,
                    local_listen_addr: None,
                    local_server_name: None,
                    local_cert_der_b64: None,
                    remote_listen_addr,
                    remote_server_name,
                    remote_cert_der_b64,
                    lifecycle_state: "listening".to_string(),
                    last_error: None,
                    sender_active: false,
                    receiver_active: false,
                },
            );
            Ok(())
        }

        fn apply_remote_offer(
            &mut self,
            _session_id: mrd_proto::SessionId,
            _sdp: String,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn apply_remote_answer(
            &mut self,
            _session_id: mrd_proto::SessionId,
            _sdp: String,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn apply_remote_ice_candidate(
            &mut self,
            _session_id: mrd_proto::SessionId,
            _candidate: mrd_signal_proto::IceCandidate,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn snapshot(&self, session_id: &mrd_proto::SessionId) -> Option<SessionSnapshot> {
            self.sessions.lock().unwrap().get(session_id).cloned()
        }
    }

    let session1 = SessionId("session-1".to_string());
    let session2 = SessionId("session-2".to_string());

    let signaling = MockSignalingPort {
        events: vec![
            SignalMessage::SessionRequest(SessionRequest {
                session_id: session1.clone(),
                source_device_id: mrd_proto::DeviceId("controller-1".to_string()),
                target_device_id: mrd_proto::DeviceId("agent-1".to_string()),
                transport: "quic_quinn".to_string(),
                quic_listen_addr: Some("127.0.0.1:5000".to_string()),
                quic_server_name: Some("localhost".to_string()),
                quic_cert_der_b64: Some("cert1".to_string()),
            }),
            SignalMessage::SessionAccept(SessionAccept {
                session_id: session2.clone(),
                transport: "quic_quinn".to_string(),
                quic_listen_addr: Some("127.0.0.1:6000".to_string()),
                quic_server_name: Some("localhost".to_string()),
                quic_cert_der_b64: Some("cert2".to_string()),
            }),
        ],
    };

    let mut quic_sessions = MockSessionCoordinator {
        sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
    };

    let mut webrtc_sessions = MockSessionCoordinator {
        sessions: Arc::new(Mutex::new(std::collections::HashMap::new())),
    };

    // Process multiple events
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let result = mrd_application::usecases::apply_realtime_events(
            &signaling,
            &mut webrtc_sessions,
            &mut quic_sessions,
            0,
        )
        .await;

        assert!(result.is_ok());

        // Both sessions should have been processed
        let sessions = quic_sessions.sessions.lock().unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains_key(&session1));
        assert!(sessions.contains_key(&session2));
    });
}
