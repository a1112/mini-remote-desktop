use mrd_signal_proto::{AuthenticatedSignalMessage, SignalEnvelope};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

pub const MAX_FUTURE_MESSAGE_WAIT_MS: u64 = 2_000;

/// Delay a slightly early message until its unchanged issue time can be checked
/// against the real clock. This never authenticates a message. Callers must
/// reread SystemTime afterward and run the complete strict protocol verifier.
///
/// Untrusted future timestamps cannot request more than two seconds of total
/// waiting. A timestamp beyond that bound is left to the verifier immediately;
/// a backward clock step cannot restart or extend the monotonic wait budget.
pub async fn wait_until_message_issued(envelope: &SignalEnvelope) {
    let issued_at_ms = match &envelope.message {
        AuthenticatedSignalMessage::ServerChallenge(challenge) => challenge.issued_at_ms,
        _ => match envelope.unverified_claims() {
            Some(claims) => claims.issued_at_ms,
            None => return,
        },
    };
    let deadline = Instant::now() + Duration::from_millis(MAX_FUTURE_MESSAGE_WAIT_MS);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Some(delay) = bounded_wait(issued_at_ms, now_ms(), remaining) else {
            return;
        };
        tokio::time::sleep(delay).await;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn bounded_wait(issued_at_ms: u64, observed_now_ms: u64, remaining: Duration) -> Option<Duration> {
    let future_ms = issued_at_ms.checked_sub(observed_now_ms)?;
    if future_ms == 0 || future_ms > MAX_FUTURE_MESSAGE_WAIT_MS || remaining.is_zero() {
        return None;
    }
    Some(Duration::from_millis(future_ms).min(remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_identity::DeviceIdentity;
    use mrd_proto::DeviceId;
    use mrd_signal_proto::{
        AuthClaims, PresenceHeartbeat, PresenceHeartbeatPayload, ProtocolReasonCode,
        ServerChallenge, SignalErrorMessage, SignalProtocolError, SignalReplayGuard,
    };
    use ring::rand::SystemRandom;

    fn heartbeat(issued_at_ms: u64, expires_at_ms: u64) -> SignalEnvelope {
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        SignalEnvelope::new(AuthenticatedSignalMessage::PresenceHeartbeat(
            PresenceHeartbeat::sign(
                &identity,
                PresenceHeartbeatPayload {
                    claims: AuthClaims {
                        issuer_device_id: DeviceId("device".into()),
                        issuer_key_id: identity.key_id().into(),
                        intended_peer_device_id: DeviceId("server".into()),
                        issued_at_ms,
                        expires_at_ms,
                        counter: 1,
                        nonce: [1; 16],
                    },
                    connection_id: [1; 16],
                    observed_at_ms: issued_at_ms,
                },
            )
            .unwrap(),
        ))
    }

    #[test]
    fn deterministic_future_and_total_budget_bounds() {
        let full = Duration::from_millis(MAX_FUTURE_MESSAGE_WAIT_MS);
        assert_eq!(bounded_wait(10_000, 10_001, full), None);
        assert_eq!(bounded_wait(10_000, 10_000, full), None);
        assert_eq!(
            bounded_wait(10_001, 10_000, full),
            Some(Duration::from_millis(1))
        );
        assert_eq!(bounded_wait(12_000, 10_000, full), Some(full));
        assert_eq!(bounded_wait(12_001, 10_000, full), None);
        assert_eq!(bounded_wait(u64::MAX, 0, full), None);
        assert_eq!(bounded_wait(u64::MAX, u64::MAX - 2_000, full), Some(full));
        // A backward wall-clock step still consumes the same monotonic budget.
        assert_eq!(
            bounded_wait(12_000, 10_100, Duration::from_millis(1)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(bounded_wait(12_000, 10_100, Duration::ZERO), None);
    }

    #[tokio::test]
    async fn waits_for_real_future_challenge_without_changing_it() {
        let issued_at_ms = now_ms() + 40;
        let envelope = SignalEnvelope::new(AuthenticatedSignalMessage::ServerChallenge(
            ServerChallenge {
                challenge_id: [1; 16],
                challenge_nonce: [2; 32],
                issued_at_ms,
                expires_at_ms: issued_at_ms + 10_000,
            },
        ));
        let original = envelope.clone();
        wait_until_message_issued(&envelope).await;
        assert!(now_ms() >= issued_at_ms);
        assert_eq!(envelope, original);
    }

    #[tokio::test]
    async fn waits_for_signed_message_then_keeps_strict_signature_and_replay_checks() {
        let issued_at_ms = now_ms() + 60;
        let envelope = heartbeat(issued_at_ms, issued_at_ms + 10_000);
        let original = envelope.clone();
        let peer = DeviceId("server".into());
        let mut replay = SignalReplayGuard::new(16, 32);
        assert!(matches!(
            envelope
                .message
                .verify_for(&peer, issued_at_ms - 1, &mut replay),
            Err(SignalProtocolError::NotYetValid)
        ));
        wait_until_message_issued(&envelope).await;
        envelope
            .message
            .verify_for(&peer, now_ms(), &mut replay)
            .unwrap();
        assert!(matches!(
            envelope.message.verify_for(&peer, now_ms(), &mut replay),
            Err(SignalProtocolError::CounterRollback | SignalProtocolError::RepeatedNonce)
        ));
        assert_eq!(envelope, original);
        let mut tampered = envelope.clone();
        if let AuthenticatedSignalMessage::PresenceHeartbeat(message) = &mut tampered.message {
            message.payload.observed_at_ms += 1;
        }
        assert!(tampered
            .message
            .verify_for(&peer, now_ms(), &mut SignalReplayGuard::new(16, 32))
            .is_err());
    }

    #[tokio::test]
    async fn no_drift_expired_unsigned_and_excessively_future_messages_do_not_sleep() {
        let now = now_ms();
        let expired = heartbeat(now - 100, now - 1);
        let current = heartbeat(now, now + 10_000);
        let future = heartbeat(now + 60_000, now + 70_000);
        let unsigned = SignalEnvelope::new(AuthenticatedSignalMessage::ProtocolError(
            SignalErrorMessage {
                reason: ProtocolReasonCode::Malformed,
                correlation_id: None,
                detail: "invalid".into(),
            },
        ));
        for envelope in [&expired, &current, &future, &unsigned] {
            tokio::time::timeout(
                Duration::from_millis(100),
                wait_until_message_issued(envelope),
            )
            .await
            .unwrap();
        }
        assert!(matches!(
            expired.message.verify_for(
                &DeviceId("server".into()),
                now_ms(),
                &mut SignalReplayGuard::new(16, 32)
            ),
            Err(SignalProtocolError::Expired)
        ));
        assert!(matches!(
            future.message.verify_for(
                &DeviceId("server".into()),
                now_ms(),
                &mut SignalReplayGuard::new(16, 32)
            ),
            Err(SignalProtocolError::NotYetValid)
        ));
    }

    #[tokio::test]
    async fn deferred_message_is_still_rejected_when_its_actual_expiry_passes() {
        let issued_at_ms = now_ms() + 40;
        let envelope = heartbeat(issued_at_ms, issued_at_ms + 1);
        wait_until_message_issued(&envelope).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(matches!(
            envelope.message.verify_for(
                &DeviceId("server".into()),
                now_ms(),
                &mut SignalReplayGuard::new(16, 32)
            ),
            Err(SignalProtocolError::Expired)
        ));
    }
}
