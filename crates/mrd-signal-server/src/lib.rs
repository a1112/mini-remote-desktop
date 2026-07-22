use mrd_proto::{DeviceId, SessionId};
use mrd_signal_proto::{
    SignalEnvelope, SignalProtocolError, SignalReplayGuard, VerifiedSignalMetadata,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRoute {
    pub controller: DeviceId,
    pub agent: DeviceId,
}

#[derive(Debug, Default)]
pub struct SessionRouter {
    routes: HashMap<SessionId, SessionRoute>,
}

impl SessionRouter {
    pub fn register(&mut self, session_id: SessionId, route: SessionRoute) {
        self.routes.insert(session_id, route);
    }

    pub fn resolve_peer(
        &self,
        session_id: &SessionId,
        sender: &DeviceId,
    ) -> Result<DeviceId, SessionRouteError> {
        let route = self
            .routes
            .get(session_id)
            .ok_or(SessionRouteError::UnknownSession)?;

        if &route.controller == sender {
            return Ok(route.agent.clone());
        }

        if &route.agent == sender {
            return Ok(route.controller.clone());
        }

        Err(SessionRouteError::UnknownSender)
    }
}

#[derive(Debug, Error)]
pub enum SessionRouteError {
    #[error("unknown session")]
    UnknownSession,
    #[error("unknown sender")]
    UnknownSender,
}

/// Stateful signature, peer-binding, expiry, counter, and nonce verifier.
#[derive(Debug)]
pub struct AuthenticatedMessageVerifier {
    local_device_id: DeviceId,
    replay: SignalReplayGuard,
}

impl AuthenticatedMessageVerifier {
    pub fn new(local_device_id: DeviceId, max_signers: usize, nonce_capacity: usize) -> Self {
        Self {
            local_device_id,
            replay: SignalReplayGuard::new(max_signers, nonce_capacity),
        }
    }

    pub fn verify(
        &mut self,
        envelope: &SignalEnvelope,
        now_ms: u64,
    ) -> Result<VerifiedSignalMetadata, AuthenticatedMessageError> {
        envelope.validate_version()?;
        envelope
            .message
            .verify_for(&self.local_device_id, now_ms, &mut self.replay)
            .map_err(Into::into)
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthenticatedMessageError {
    #[error(transparent)]
    Protocol(#[from] SignalProtocolError),
}

#[cfg(test)]
mod tests {
    use super::{AuthenticatedMessageVerifier, SessionRoute, SessionRouter};
    use mrd_proto::{DeviceId, SessionId};

    #[test]
    fn resolves_peer_from_session_route() {
        let mut router = SessionRouter::default();
        let session_id = SessionId("session-1".into());
        let controller = DeviceId("controller-1".into());
        let agent = DeviceId("agent-1".into());

        router.register(
            session_id.clone(),
            SessionRoute {
                controller: controller.clone(),
                agent: agent.clone(),
            },
        );

        let peer = router
            .resolve_peer(&session_id, &controller)
            .expect("resolve controller peer");

        assert_eq!(peer, agent);
    }

    #[test]
    fn verifier_rejects_unsigned_server_to_client_message_on_authenticated_ingress() {
        use mrd_signal_proto::{
            AuthenticatedSignalMessage, ProtocolReasonCode, SignalEnvelope, SignalErrorMessage,
            SignalProtocolError,
        };
        let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::ProtocolError(
            SignalErrorMessage {
                reason: ProtocolReasonCode::Malformed,
                correlation_id: None,
                detail: "invalid".into(),
            },
        ));
        let mut verifier =
            AuthenticatedMessageVerifier::new(DeviceId("signal-server".into()), 8, 64);
        assert_eq!(
            verifier.verify(&envelope, 1_000),
            Err(SignalProtocolError::UnsignedMessage.into())
        );
    }

    #[test]
    fn verifier_accepts_one_signed_register_then_rejects_replay() {
        use mrd_identity::DeviceIdentity;
        use mrd_signal_proto::{
            AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, RegisterPayload,
            SignalEnvelope, SignalProtocolError,
        };
        use ring::rand::SystemRandom;

        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let register = AuthenticatedRegister::sign(
            &identity,
            RegisterPayload {
                claims: AuthClaims {
                    issuer_device_id: DeviceId("controller-1".into()),
                    issuer_key_id: identity.key_id().into(),
                    intended_peer_device_id: DeviceId("signal-server".into()),
                    issued_at_ms: 1_000,
                    expires_at_ms: 2_000,
                    counter: 1,
                    nonce: [9; 16],
                },
                role: mrd_proto::BackendRole::Controller,
                device_name: "Rdesk".into(),
                backend_device_token: "backend-token".into(),
                challenge_id: [7; 16],
                challenge_nonce: [8; 32],
            },
        )
        .unwrap();
        let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::Register(register));
        let mut verifier =
            AuthenticatedMessageVerifier::new(DeviceId("signal-server".into()), 8, 64);
        let metadata = verifier.verify(&envelope, 1_500).unwrap();
        assert_eq!(metadata.issuer_device_id, DeviceId("controller-1".into()));
        assert_eq!(
            verifier.verify(&envelope, 1_500),
            Err(SignalProtocolError::RepeatedNonce.into())
        );
    }
}
