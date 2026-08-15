use anyhow::Result;
use mrd_application::{
    apply_authenticated_realtime_events, AuthenticatedSessionSignal,
    AuthenticatedSessionSignalPort, AuthenticatedSignalingPort, VerifiedSignalingEvent,
    VerifiedSignalingIdentity,
};
use mrd_proto::{DeviceId, SessionId};
use std::sync::Mutex;

struct Inbox(Mutex<Vec<VerifiedSignalingEvent>>);

#[async_trait::async_trait]
impl AuthenticatedSignalingPort for Inbox {
    async fn drain_authenticated_events(&self) -> Result<Vec<VerifiedSignalingEvent>> {
        Ok(std::mem::take(&mut *self.0.lock().unwrap()))
    }
}

#[derive(Default)]
struct Sessions(Mutex<Vec<VerifiedSignalingEvent>>);

#[async_trait::async_trait]
impl AuthenticatedSessionSignalPort for Sessions {
    async fn apply_authenticated_signal(&self, event: VerifiedSignalingEvent) -> Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[tokio::test]
async fn authenticated_usecase_drains_and_applies_verified_events_once() {
    let event = VerifiedSignalingEvent {
        sender: VerifiedSignalingIdentity {
            device_id: DeviceId("controller-1".into()),
            key_id: "controller-key".into(),
            public_key: vec![7; 32],
            counter: 4,
            nonce: [9; 16],
            issued_at_ms: 1_000,
            expires_at_ms: 31_000,
        },
        signal: AuthenticatedSessionSignal::AuthorizationRequested {
            session_id: SessionId("session-1".into()),
            idempotency_key: [3; 16],
            requested_transport: "webrtc".into(),
        },
    };
    let inbox = Inbox(Mutex::new(vec![event.clone()]));
    let sessions = Sessions::default();

    assert_eq!(
        apply_authenticated_realtime_events(&inbox, &sessions)
            .await
            .unwrap(),
        1
    );
    assert_eq!(sessions.0.lock().unwrap().as_slice(), &[event]);
    assert_eq!(
        apply_authenticated_realtime_events(&inbox, &sessions)
            .await
            .unwrap(),
        0
    );
}
