#[path = "support/browser_fixture.rs"]
mod browser_fixture;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mrd_identity::DeviceIdentity;
use mrd_proto::{BackendRole, DeviceId, SessionId};
use mrd_signal_proto::{
    AuthClaims, AuthenticatedRegister, AuthenticatedSignalMessage, PresenceHeartbeat,
    PresenceHeartbeatPayload, ProtocolReasonCode, RegisterPayload, RelayMigrationOffer,
    RelayMigrationOfferPayload, SessionClose, SessionClosePayload, SessionGrantV3, SessionIntentV3,
    SignalEnvelope, WanPermissionScopeV3, WebRtcCandidateV3, WebRtcOfferV3,
};
use realtime_server::{
    ConnectionId, CoreConfig, JwtBackendTokenVerifier, RealtimeCore, RealtimeError,
};
use ring::hmac;
use serde_json::{json, Value};
use std::{collections::BTreeSet, sync::Arc};

const SECRET: &[u8] = b"test-backend-signing-secret-32-bytes-minimum";
const NOW: u64 = 1_800_000_000_000;

fn signed_jwt(payload: &Value) -> String {
    let encoded = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(payload.to_string())
    );
    let signature = hmac::sign(
        &hmac::Key::new(hmac::HMAC_SHA256, SECRET),
        encoded.as_bytes(),
    );
    format!("{encoded}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
}

struct Fixture {
    core: RealtimeCore,
    data: Value,
    browser: DeviceIdentity,
    target: DeviceIdentity,
    browser_connection: ConnectionId,
    target_connection: ConnectionId,
    guest_authority: bool,
}

impl Fixture {
    fn new() -> Self {
        Self::new_for_authority(false)
    }

    fn new_for_authority(guest_authority: bool) -> Self {
        let data = browser_fixture::fixture();
        let browser = browser_fixture::identity(data["browser"]["seed_hex"].as_str().unwrap());
        let target = browser_fixture::identity(data["target"]["seed_hex"].as_str().unwrap());
        let core = RealtimeCore::new(
            CoreConfig {
                server_device_id: DeviceId("signal-server".into()),
                challenge_ttl_ms: 10_000,
                presence_ttl_ms: 30_000,
                route_ttl_ms: 60_000,
                max_connections: 32,
                max_messages_per_window: 64,
                rate_window_ms: 1_000,
            },
            Arc::new(
                JwtBackendTokenVerifier::new(
                    SECRET,
                    "rdesk-backend".into(),
                    "rdesk-signaling".into(),
                )
                .unwrap(),
            ),
        )
        .unwrap();
        let mut result = Self {
            core,
            data,
            browser,
            target,
            browser_connection: ConnectionId::from_bytes([1; 16]).unwrap(),
            target_connection: ConnectionId::from_bytes([2; 16]).unwrap(),
            guest_authority,
        };
        result.register(false, result.target_connection, 1).unwrap();
        result.register(true, result.browser_connection, 1).unwrap();
        result
    }
    fn device_id(&self, browser: bool) -> DeviceId {
        DeviceId(
            self.data[if browser { "browser" } else { "target" }]["device_id"]
                .as_str()
                .unwrap()
                .into(),
        )
    }
    fn claims(&self, browser: bool, counter: u64) -> AuthClaims {
        AuthClaims {
            issuer_device_id: self.device_id(browser),
            issuer_key_id: if browser {
                self.browser.key_id()
            } else {
                self.target.key_id()
            }
            .into(),
            intended_peer_device_id: self.device_id(!browser),
            issued_at_ms: NOW,
            expires_at_ms: NOW + 10_000,
            counter,
            nonce: [counter as u8; 16],
        }
    }
    fn register(
        &mut self,
        browser: bool,
        connection: ConnectionId,
        counter: u64,
    ) -> Result<(), RealtimeError> {
        let identity = if browser { &self.browser } else { &self.target };
        let id = self.device_id(browser);
        let role = if browser {
            BackendRole::Controller
        } else {
            BackendRole::Agent
        };
        let mut payload = json!({"sub":id.0,"device_id":id.0,"device_key_id":identity.key_id(),"role":role,"token_type":if browser{"browser_signaling"}else{"signaling"},"iss":"rdesk-backend","aud":"rdesk-signaling","iat":NOW/1000,"exp":NOW/1000+300});
        if browser {
            if self.guest_authority {
                payload["token_type"] = json!("guest_browser_signaling");
                payload["authority_kind"] = json!("temporary_password");
                payload["temporary_access_generation"] = json!(1);
                payload["target_auth_version"] = json!(2);
                payload["user_id"] = json!(null);
            } else {
                payload["user_id"] = json!("user-1");
            }
            payload["tenant_id"] = json!("tenant-1");
            payload["session_id"] = self.data["request"]["session_id"].clone();
            payload["target_device_id"] = self.data["target"]["device_id"].clone();
            payload["allowed_scopes"] = self.data["request"]["requested_scopes"].clone();
        }
        let challenge = self
            .core
            .open_connection(connection, NOW + counter)
            .unwrap();
        let message = AuthenticatedRegister::sign(
            identity,
            RegisterPayload {
                claims: AuthClaims {
                    issuer_device_id: id,
                    issuer_key_id: identity.key_id().into(),
                    intended_peer_device_id: DeviceId("signal-server".into()),
                    issued_at_ms: NOW + counter,
                    expires_at_ms: NOW + 10_000,
                    counter,
                    nonce: [counter as u8; 16],
                },
                role,
                device_name: "Fixture".into(),
                backend_device_token: signed_jwt(&payload),
                challenge_id: challenge.challenge_id,
                challenge_nonce: challenge.challenge_nonce,
            },
        )
        .unwrap();
        self.core
            .handle(
                connection,
                SignalEnvelope::new(AuthenticatedSignalMessage::Register(message)),
                NOW + counter,
            )
            .map(|_| ())
    }
    fn intent(&self) -> SessionIntentV3 {
        serde_json::from_value(self.data["intent"].clone()).unwrap()
    }
    fn open_route(&mut self) {
        let intent = self.intent();
        self.core
            .handle(
                self.browser_connection,
                SignalEnvelope::new(AuthenticatedSignalMessage::SessionIntentV3(intent)),
                NOW + 2,
            )
            .unwrap();
        let grant: SessionGrantV3 = serde_json::from_value(self.data["grant"].clone()).unwrap();
        self.core
            .handle(
                self.target_connection,
                SignalEnvelope::new(AuthenticatedSignalMessage::SessionGrantV3(grant)),
                NOW + 3,
            )
            .unwrap();
    }
    fn close(&mut self, browser: bool) {
        let message = SessionClose::sign(
            if browser { &self.browser } else { &self.target },
            SessionClosePayload {
                claims: self.claims(browser, 8),
                session_id: SessionId(self.data["request"]["session_id"].as_str().unwrap().into()),
                reason: ProtocolReasonCode::Expired,
            },
        )
        .unwrap();
        self.core
            .handle(
                if browser {
                    self.browser_connection
                } else {
                    self.target_connection
                },
                SignalEnvelope::new(AuthenticatedSignalMessage::SessionClose(message)),
                NOW + 8,
            )
            .unwrap();
    }
}

#[test]
fn browser_controller_routes_real_v3_intent_offer_and_candidate_without_wire_changes() {
    let mut fixture = Fixture::new();
    fixture.open_route();
    let offer: WebRtcOfferV3 = serde_json::from_value(fixture.data["offer"].clone()).unwrap();
    let candidate: WebRtcCandidateV3 =
        serde_json::from_value(fixture.data["candidate"].clone()).unwrap();
    for signal in [
        AuthenticatedSignalMessage::WebrtcOfferV3(offer),
        AuthenticatedSignalMessage::WebrtcCandidateV3(candidate),
    ] {
        let deliveries = fixture
            .core
            .handle(
                fixture.browser_connection,
                SignalEnvelope::new(signal),
                NOW + 4,
            )
            .unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(
            deliveries[0].target,
            realtime_server::DeliveryTarget::Connection(fixture.target_connection)
        );
    }
    assert_eq!(fixture.core.route_count(), 1);
}

#[test]
fn browser_intent_cannot_escape_credential_session_target_or_scope() {
    for change in ["session", "target", "scope"] {
        let mut fixture = Fixture::new();
        let mut intent = fixture.intent();
        match change {
            "session" => intent.payload.request.session_id = SessionId("other-session".into()),
            "target" => {
                intent.payload.request.target_device_id = DeviceId("other-target".into());
                intent.payload.claims.intended_peer_device_id = DeviceId("other-target".into());
            }
            "scope" => intent
                .payload
                .request
                .requested_scopes
                .insert(0, WanPermissionScopeV3::FileWrite),
            _ => unreachable!(),
        }
        intent.payload.request_commitment = intent.payload.request.commitment().unwrap();
        let intent = SessionIntentV3::sign(&fixture.browser, intent.payload).unwrap();
        let error = fixture
            .core
            .handle(
                fixture.browser_connection,
                SignalEnvelope::new(AuthenticatedSignalMessage::SessionIntentV3(intent)),
                NOW + 2,
            )
            .unwrap_err();
        assert_eq!(
            error.reason_code(),
            ProtocolReasonCode::UnauthorizedRoute,
            "{change}: {error}"
        );
        assert_eq!(fixture.core.route_count(), 0);
    }
}

#[test]
fn browser_controller_migration_is_explicitly_rejected() {
    let mut fixture = Fixture::new();
    fixture.open_route();
    let offer = RelayMigrationOffer::sign(
        &fixture.browser,
        RelayMigrationOfferPayload {
            claims: fixture.claims(true, 7),
            session_id: SessionId(
                fixture.data["request"]["session_id"]
                    .as_str()
                    .unwrap()
                    .into(),
            ),
            migration_generation: 1,
            directory_id: "directory-fixture".into(),
            node_id: "relay-fixture".into(),
            sdp: "v=0\r\n".into(),
            restart_route_token: "a".repeat(64),
            candidate_fingerprints: BTreeSet::from(["b".repeat(64)]),
        },
    )
    .unwrap();
    let error = fixture
        .core
        .handle(
            fixture.browser_connection,
            SignalEnvelope::new(AuthenticatedSignalMessage::RelayMigrationOffer(offer)),
            NOW + 7,
        )
        .unwrap_err();
    assert_eq!(error.reason_code(), ProtocolReasonCode::UnauthorizedRoute);
    assert_eq!(fixture.core.route_count(), 1);
}

#[test]
fn signed_close_revokes_browser_presence_and_prevents_same_jwt_reregistration() {
    for browser_closes in [true, false] {
        let mut fixture = Fixture::new();
        fixture.open_route();
        fixture.close(browser_closes);
        assert!(!fixture.core.is_present(&fixture.device_id(true)));
        assert!(fixture.core.is_present(&fixture.device_id(false)));
        assert_eq!(fixture.core.route_count(), 0);
        let another = ConnectionId::from_bytes([9; 16]).unwrap();
        assert_eq!(
            fixture
                .register(true, another, 9)
                .unwrap_err()
                .reason_code(),
            ProtocolReasonCode::UnauthorizedRoute
        );
        assert!(!fixture.core.is_present(&fixture.device_id(true)));
    }
}

#[test]
fn browser_expiry_is_checked_for_every_message_and_prune_cleans_its_route() {
    let mut fixture = Fixture::new();
    fixture.open_route();
    let mut claims = fixture.claims(true, 6);
    claims.issued_at_ms = NOW + 300_000;
    claims.expires_at_ms = NOW + 310_000;
    claims.intended_peer_device_id = DeviceId("signal-server".into());
    let heartbeat = PresenceHeartbeat::sign(
        &fixture.browser,
        PresenceHeartbeatPayload {
            claims,
            connection_id: *fixture.browser_connection.as_bytes(),
            observed_at_ms: NOW + 300_000,
        },
    )
    .unwrap();
    assert_eq!(
        fixture
            .core
            .handle(
                fixture.browser_connection,
                SignalEnvelope::new(AuthenticatedSignalMessage::PresenceHeartbeat(heartbeat)),
                NOW + 300_000
            )
            .unwrap_err()
            .reason_code(),
        ProtocolReasonCode::Expired
    );
    assert!(!fixture.core.is_present(&fixture.device_id(true)));
    assert_eq!(fixture.core.route_count(), 0);
    let mut fixture = Fixture::new();
    fixture.open_route();
    assert!(fixture
        .core
        .prune(NOW + 300_000)
        .contains(&fixture.browser_connection));
    assert!(!fixture.core.is_present(&fixture.device_id(true)));
    assert_eq!(fixture.core.route_count(), 0);
}

#[test]
fn guest_temporary_authority_routes_signed_v3_without_account_or_wire_changes() {
    let mut fixture = Fixture::new_for_authority(true);
    fixture.open_route();
    let offer: WebRtcOfferV3 = serde_json::from_value(fixture.data["offer"].clone()).unwrap();
    let deliveries = fixture
        .core
        .handle(
            fixture.browser_connection,
            SignalEnvelope::new(AuthenticatedSignalMessage::WebrtcOfferV3(offer)),
            NOW + 4,
        )
        .unwrap();
    assert_eq!(
        deliveries[0].target,
        realtime_server::DeliveryTarget::Connection(fixture.target_connection)
    );
}

#[test]
fn guest_temporary_authority_cannot_escape_session_target_or_approved_scopes() {
    for change in ["session", "target", "scope"] {
        let mut fixture = Fixture::new_for_authority(true);
        let mut intent = fixture.intent();
        match change {
            "session" => intent.payload.request.session_id = SessionId("other-session".into()),
            "target" => {
                intent.payload.request.target_device_id = DeviceId("other-target".into());
                intent.payload.claims.intended_peer_device_id = DeviceId("other-target".into());
            }
            "scope" => intent
                .payload
                .request
                .requested_scopes
                .insert(0, WanPermissionScopeV3::FileWrite),
            _ => unreachable!(),
        }
        intent.payload.request_commitment = intent.payload.request.commitment().unwrap();
        let intent = SessionIntentV3::sign(&fixture.browser, intent.payload).unwrap();
        assert_eq!(
            fixture
                .core
                .handle(
                    fixture.browser_connection,
                    SignalEnvelope::new(AuthenticatedSignalMessage::SessionIntentV3(intent)),
                    NOW + 2
                )
                .unwrap_err()
                .reason_code(),
            ProtocolReasonCode::UnauthorizedRoute,
            "{change}"
        );
        assert_eq!(fixture.core.route_count(), 0);
    }
}

#[test]
fn guest_temporary_authority_close_removes_browser_and_tombstones_its_credential() {
    for browser_closes in [true, false] {
        let mut fixture = Fixture::new_for_authority(true);
        fixture.open_route();
        fixture.close(browser_closes);
        assert!(!fixture.core.is_present(&fixture.device_id(true)));
        assert!(fixture.core.is_present(&fixture.device_id(false)));
        assert_eq!(fixture.core.route_count(), 0);
        let another = ConnectionId::from_bytes([9; 16]).unwrap();
        assert_eq!(
            fixture
                .register(true, another, 9)
                .unwrap_err()
                .reason_code(),
            ProtocolReasonCode::UnauthorizedRoute
        );
    }
}
