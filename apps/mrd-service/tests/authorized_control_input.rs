use common_control_proto::{encode_authenticated_input_event, ControlEvent};
use mrd_identity::DeviceIdentity;
use mrd_input::{InputError, InputEvent, InputInjector};
use mrd_ipc::{
    AuditLogQuery, ConsentDecision, ConsentResponse, DecimalU64, RemoteAccessMode,
    RemoteAuthorizationState, RemoteFailure, RemotePermissionScope, RemoteReasonCode,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_service::{
    control_input::ControlInputRegistry,
    lan_discovery::{process_lan_discovery_packet, AUTHENTICATED_CONTROL_INPUT_DATAGRAM_PREFIX},
    session_authorization::{VerifiedIncomingAuthorizationRequest, VerifiedSessionGrant},
    AppState,
};
use mrd_session::{
    ControlEnvelopeV2, PermissionScope, SignedControlEnvelopeV2,
    CONTROL_ENVELOPE_SIGNATURE_CONTEXT, CONTROL_ENVELOPE_VERSION,
};
use ring::rand::SystemRandom;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::UdpSocket;

const CONTROL_AUDIT_ACTION: &str = "session.control_input_decision";

#[derive(Clone)]
struct SharedRecordingInputInjector {
    events: Arc<StdMutex<Vec<InputEvent>>>,
    fail_next_release: Arc<AtomicBool>,
}

impl InputInjector for SharedRecordingInputInjector {
    fn is_available(&self) -> bool {
        true
    }

    fn inject(&mut self, event: &InputEvent) -> Result<(), InputError> {
        if matches!(
            event,
            InputEvent::Key { pressed: false, .. } | InputEvent::MouseButton { pressed: false, .. }
        ) && self.fail_next_release.swap(false, Ordering::AcqRel)
        {
            return Err(InputError::Platform("injected release failure".to_string()));
        }
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(*event);
        Ok(())
    }
}

struct AuthorizedControlFixture {
    state: Arc<AppState>,
    controller: DeviceIdentity,
    session_id: SessionId,
    source_device_id: DeviceId,
    target_device_id: DeviceId,
    target_key_id: String,
    grant_id: [u8; 32],
    policy_revision: u64,
    expires_at_ms: u64,
    injected: Arc<StdMutex<Vec<InputEvent>>>,
    fail_next_release: Arc<AtomicBool>,
    service_socket: UdpSocket,
    controller_socket: UdpSocket,
}

impl AuthorizedControlFixture {
    async fn new(granted_scopes: Vec<RemotePermissionScope>) -> Self {
        let state = Arc::new(AppState::new());
        let controller = DeviceIdentity::generate(&SystemRandom::new()).expect("controller key");
        let session_id = SessionId(format!("authorized-control-{}", now_ms()));
        let source_device_id = DeviceId("controller-device".to_string());
        let target_device_id = DeviceId("target-device".to_string());
        state
            .devices
            .lock()
            .await
            .register(target_device_id.clone(), "Target".to_string());
        let target_key_id = state
            .device_identities
            .machine_key_id()
            .expect("target key id")
            .to_string();
        let created_at_ms = now_ms();
        let expires_at_ms = created_at_ms.saturating_add(60_000);
        let requested_scopes = vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ];

        state
            .session_authorizations
            .begin_verified_incoming(VerifiedIncomingAuthorizationRequest {
                session_id: session_id.clone(),
                peer_device_id: source_device_id.clone(),
                peer_key_id: controller.key_id().to_string(),
                peer_key_epoch: 1,
                access_mode: RemoteAccessMode::Attended,
                requested_scopes: requested_scopes.clone(),
                peer_permission_ceiling: requested_scopes.clone(),
                machine_permission_ceiling: requested_scopes.clone(),
                runtime_capabilities: requested_scopes,
                transport_kind: "quic".to_string(),
                request_nonce: [0x44; 16],
                created_at_ms,
                expires_at_ms,
            })
            .await
            .expect("begin incoming authorization");
        state
            .session_authorizations
            .bind_authenticated_peer_key(
                &session_id,
                controller.public_key(),
                created_at_ms.saturating_add(1),
            )
            .await
            .expect("bind controller public key");
        let approved = state
            .session_authorizations
            .respond_to_consent(
                ConsentResponse {
                    session_id: session_id.clone(),
                    decision: ConsentDecision::Approve,
                    approved_scopes: granted_scopes.clone(),
                    expected_policy_revision: DecimalU64::new(1),
                },
                created_at_ms.saturating_add(2),
            )
            .await
            .expect("approve control scopes");
        let policy_revision = approved.policy_revision.get();
        let grant_id = [0x31; 32];
        state
            .session_authorizations
            .install_verified_grant(
                VerifiedSessionGrant {
                    grant_id: format!("sha256:{}", hex_bytes(&grant_id)),
                    session_id: session_id.clone(),
                    granted_scopes,
                    issued_at_ms: created_at_ms.saturating_add(3),
                    expires_at_ms,
                    policy_revision,
                    route_constraint: "quic".to_string(),
                    transport_fingerprint_sha256: [0x55; 32],
                },
                created_at_ms.saturating_add(3),
            )
            .await
            .expect("install verified grant");
        state
            .session_authorizations
            .mark_streaming(&session_id, created_at_ms.saturating_add(4))
            .await
            .expect("mark authorization streaming");

        let injected = Arc::new(StdMutex::new(Vec::new()));
        let fail_next_release = Arc::new(AtomicBool::new(false));
        *state.control_input().lock().await =
            ControlInputRegistry::with_injector(SharedRecordingInputInjector {
                events: injected.clone(),
                fail_next_release: Arc::clone(&fail_next_release),
            });

        Self {
            state,
            controller,
            session_id,
            source_device_id,
            target_device_id,
            target_key_id,
            grant_id,
            policy_revision,
            expires_at_ms,
            injected,
            fail_next_release,
            service_socket: UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("service socket"),
            controller_socket: UdpSocket::bind("127.0.0.1:0")
                .await
                .expect("controller socket"),
        }
    }

    fn signed(
        &self,
        scope: PermissionScope,
        sequence: u64,
        event_id: u64,
        event: ControlEvent,
    ) -> SignedControlEnvelopeV2 {
        let issued_at_ms = now_ms();
        sign_envelope(
            &self.controller,
            ControlEnvelopeV2 {
                protocol_version: CONTROL_ENVELOPE_VERSION,
                session_id: self.session_id.clone(),
                grant_id: self.grant_id,
                source_device_id: self.source_device_id.clone(),
                target_device_id: self.target_device_id.clone(),
                source_key_id: self.controller.key_id().to_string(),
                target_key_id: self.target_key_id.clone(),
                scope,
                sequence,
                event_id,
                issued_at_ms,
                expires_at_ms: issued_at_ms
                    .saturating_add(mrd_session::CONTROL_ENVELOPE_MAX_LIFETIME_MS)
                    .min(self.expires_at_ms),
                policy_revision: self.policy_revision,
                authenticated_event_bytes: encode_authenticated_input_event(&event)
                    .expect("encode authenticated input"),
            },
        )
    }

    async fn deliver(&self, envelope: &SignedControlEnvelopeV2) {
        let mut datagram = AUTHENTICATED_CONTROL_INPUT_DATAGRAM_PREFIX.to_vec();
        datagram.extend_from_slice(&serde_json::to_vec(envelope).expect("serialize envelope"));
        process_lan_discovery_packet(
            &self.service_socket,
            &self.state,
            &datagram,
            self.controller_socket
                .local_addr()
                .expect("controller socket address"),
        )
        .await
        .expect("process authenticated control input");
    }

    fn injected_count(&self) -> usize {
        self.injected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    fn control_audits_for(&self, session_id: &SessionId) -> usize {
        self.state
            .audit_log
            .query(&AuditLogQuery {
                session_id: Some(session_id.clone()),
                action: Some(CONTROL_AUDIT_ACTION.to_string()),
                limit: Some(16),
            })
            .expect("query control-input audit")
            .len()
    }

    async fn assert_rejected_without_new_injection(
        &self,
        envelope: &SignedControlEnvelopeV2,
        baseline_injections: usize,
    ) {
        self.deliver(envelope).await;
        assert_eq!(self.injected_count(), baseline_injections);
        assert_eq!(self.control_audits_for(&envelope.payload.session_id), 1);
    }
}

fn sign_envelope(identity: &DeviceIdentity, payload: ControlEnvelopeV2) -> SignedControlEnvelopeV2 {
    let signature = identity
        .sign_context_bytes(
            CONTROL_ENVELOPE_SIGNATURE_CONTEXT,
            &payload.signing_bytes().expect("control signing bytes"),
        )
        .expect("sign control envelope")
        .try_into()
        .expect("Ed25519 signature length");
    SignedControlEnvelopeV2 {
        payload,
        public_key: identity
            .public_key()
            .try_into()
            .expect("Ed25519 public key length"),
        signature,
    }
}

fn resign(identity: &DeviceIdentity, envelope: &mut SignedControlEnvelopeV2) {
    *envelope = sign_envelope(identity, envelope.payload.clone());
}

fn full_input_scopes() -> Vec<RemotePermissionScope> {
    vec![
        RemotePermissionScope::ScreenView,
        RemotePermissionScope::InputPointer,
        RemotePermissionScope::InputKeyboard,
    ]
}

fn pointer_event(x: i32) -> ControlEvent {
    ControlEvent::MouseMove { x, y: 20 }
}

#[tokio::test]
async fn authorized_control_input_rejects_forged_source_device_without_injection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let mut envelope = fixture.signed(PermissionScope::InputPointer, 1, 101, pointer_event(10));
    envelope.payload.source_device_id = DeviceId("forged-controller".to_string());
    resign(&fixture.controller, &mut envelope);

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_wrong_session_without_injection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let mut envelope = fixture.signed(PermissionScope::InputPointer, 1, 102, pointer_event(11));
    envelope.payload.session_id = SessionId("other-session".to_string());
    resign(&fixture.controller, &mut envelope);

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_missing_keyboard_scope_without_injection() {
    let fixture = AuthorizedControlFixture::new(vec![
        RemotePermissionScope::ScreenView,
        RemotePermissionScope::InputPointer,
    ])
    .await;
    let envelope = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        103,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_stale_policy_revision_without_injection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let mut envelope = fixture.signed(PermissionScope::InputPointer, 1, 104, pointer_event(12));
    envelope.payload.policy_revision = envelope.payload.policy_revision.saturating_add(1);
    resign(&fixture.controller, &mut envelope);

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_duplicate_sequence_without_reinjection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let first = fixture.signed(PermissionScope::InputPointer, 20, 120, pointer_event(20));
    fixture.deliver(&first).await;
    assert_eq!(fixture.injected_count(), 1);

    let conflict = fixture.signed(PermissionScope::InputPointer, 20, 121, pointer_event(21));
    fixture
        .assert_rejected_without_new_injection(&conflict, 1)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_out_of_window_sequence_without_reinjection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let high = fixture.signed(PermissionScope::InputPointer, 200, 200, pointer_event(30));
    fixture.deliver(&high).await;
    assert_eq!(fixture.injected_count(), 1);

    let stale = fixture.signed(PermissionScope::InputPointer, 1, 201, pointer_event(31));
    fixture
        .assert_rejected_without_new_injection(&stale, 1)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_revoked_grant_without_injection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    fixture
        .state
        .session_authorizations
        .record_failure(
            &fixture.session_id,
            RemoteAuthorizationState::Revoked,
            RemoteFailure {
                code: RemoteReasonCode::GrantRevoked,
                message: "test revocation".to_string(),
                suggested_action: None,
            },
            now_ms(),
        )
        .await
        .expect("revoke grant");
    let envelope = fixture.signed(PermissionScope::InputPointer, 1, 105, pointer_event(13));

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_rejects_tampered_payload_without_injection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let mut envelope = fixture.signed(PermissionScope::InputPointer, 1, 106, pointer_event(14));
    *envelope
        .payload
        .authenticated_event_bytes
        .last_mut()
        .expect("event byte") ^= 1;

    fixture
        .assert_rejected_without_new_injection(&envelope, 0)
        .await;
}

#[tokio::test]
async fn authorized_control_input_exact_reliable_retry_replays_ack_without_reinjection() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let envelope = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        130,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );

    fixture.deliver(&envelope).await;
    fixture.deliver(&envelope).await;

    assert_eq!(fixture.injected_count(), 1);
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 0);
}

#[tokio::test]
async fn reliable_input_requires_the_next_per_grant_sequence() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let key_up = fixture.signed(
        PermissionScope::InputKeyboard,
        2,
        202,
        ControlEvent::Key {
            key: 0x41,
            pressed: false,
        },
    );
    fixture.deliver(&key_up).await;
    assert_eq!(fixture.injected_count(), 0);

    let key_down = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        201,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    fixture.deliver(&key_down).await;
    fixture.deliver(&key_up).await;

    assert_eq!(fixture.injected_count(), 2);
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 1);
}

#[tokio::test]
async fn release_all_is_an_ordered_barrier_against_delayed_key_down_retry() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let key_down = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        301,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    let release = fixture.signed(
        PermissionScope::InputKeyboard,
        2,
        302,
        ControlEvent::ReleaseAll {
            scope: common_control_proto::AuthenticatedInputScope::Keyboard,
        },
    );

    fixture.deliver(&key_down).await;
    fixture.deliver(&release).await;
    fixture.deliver(&key_down).await;

    assert_eq!(fixture.injected_count(), 2);
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 0);
}

#[tokio::test]
async fn release_all_barrier_skips_a_lost_reliable_transition_and_rejects_it_if_delayed() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let key_down = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        351,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    let delayed_key_up = fixture.signed(
        PermissionScope::InputKeyboard,
        2,
        352,
        ControlEvent::Key {
            key: 0x41,
            pressed: false,
        },
    );
    let release = fixture.signed(
        PermissionScope::InputKeyboard,
        3,
        353,
        ControlEvent::ReleaseAll {
            scope: common_control_proto::AuthenticatedInputScope::Keyboard,
        },
    );

    fixture.deliver(&key_down).await;
    fixture.deliver(&release).await;
    fixture.deliver(&delayed_key_up).await;

    assert_eq!(fixture.injected_count(), 2);
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 1);
}

#[tokio::test]
async fn release_all_barrier_cannot_jump_more_than_one_missing_reliable_transition() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let oversized_jump = fixture.signed(
        PermissionScope::InputKeyboard,
        3,
        371,
        ControlEvent::ReleaseAll {
            scope: common_control_proto::AuthenticatedInputScope::Keyboard,
        },
    );

    fixture
        .assert_rejected_without_new_injection(&oversized_jump, 0)
        .await;
}

#[tokio::test]
async fn realtime_traffic_does_not_evict_reliable_retry_acknowledgement() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let key_down = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        401,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    fixture.deliver(&key_down).await;
    for sequence in 1..=200_u64 {
        let realtime = fixture.signed(
            PermissionScope::InputPointer,
            sequence,
            1_000 + sequence,
            pointer_event(sequence as i32),
        );
        fixture.deliver(&realtime).await;
    }
    let before_retry = fixture.injected_count();

    fixture.deliver(&key_down).await;

    assert_eq!(fixture.injected_count(), before_retry);
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 0);
}

#[tokio::test]
async fn delayed_first_seen_event_is_rejected_after_its_short_deadline() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let mut delayed = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        501,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    let now = now_ms();
    delayed.payload.issued_at_ms = now.saturating_sub(3_000);
    delayed.payload.expires_at_ms = now.saturating_sub(1_000);
    resign(&fixture.controller, &mut delayed);

    fixture.deliver(&delayed).await;

    assert_eq!(fixture.injected_count(), 0);
}

#[tokio::test]
async fn reliable_release_injector_failure_releases_state_and_terminalizes_authorization() {
    let fixture = AuthorizedControlFixture::new(full_input_scopes()).await;
    let key_down = fixture.signed(
        PermissionScope::InputKeyboard,
        1,
        601,
        ControlEvent::Key {
            key: 0x41,
            pressed: true,
        },
    );
    fixture.deliver(&key_down).await;
    fixture.fail_next_release.store(true, Ordering::Release);
    let key_up = fixture.signed(
        PermissionScope::InputKeyboard,
        2,
        602,
        ControlEvent::Key {
            key: 0x41,
            pressed: false,
        },
    );

    fixture.deliver(&key_up).await;

    assert_eq!(
        fixture.injected_count(),
        2,
        "terminal retry must emit key-up"
    );
    let snapshot = fixture
        .state
        .session_authorizations
        .snapshot(&fixture.session_id)
        .await
        .expect("authorization snapshot");
    assert_eq!(
        snapshot.authorization_state,
        RemoteAuthorizationState::PolicyChanged
    );
    assert_eq!(fixture.control_audits_for(&fixture.session_id), 1);
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_millis() as u64
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
