use mrd_agent_ipc::{
    authorize_input_resource, decode_frame, encode_frame, read_frame, validate_consent_result,
    validate_execute_command, validate_input_event, write_frame, AgentCapability,
    AgentCapabilitySnapshot, AgentChallenge, AgentCommand, AgentCrashed, AgentEventContext,
    AgentHeartbeat, AgentProtocolState, AgentRegister, AgentRegistered, AgentStopping,
    AgentToService, AudioDirection, CommandOutcome, CommandResult, ConsentDecision, ConsentRequest,
    ConsentResult, ConsentValidationError, DesktopChanged, DesktopKind, ExecuteCommand,
    ExecuteGrant, ExecuteGrantClaims, ExecuteGrantVerifier, ExecutionContext, FileDirection,
    FrameError, GrantAudience, GrantValidationError, InputAck, InputAckOutcome, InputButton,
    InputEventEnvelope, InputEventPayload, InputFailure, InputKey, InputRejection, Locked,
    PeerBinding, RegistrationProofVerifier, ServiceToAgent, StopAgent, StopReason, StoppingReason,
    Unlocked, AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR,
    AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;

const AGENT_ID: [u8; 16] = [1; 16];
const REGISTRATION_ID: [u8; 16] = [2; 16];
const CHALLENGE_ID: [u8; 16] = [3; 16];
const RESOURCE_ID: [u8; 16] = [4; 16];
const COMMAND_ID: [u8; 16] = [5; 16];
const REQUEST_ID: [u8; 16] = [6; 16];
const GRANT_ID: [u8; 32] = [7; 32];
const AGENT_KEY_ID: [u8; 32] = [8; 32];
const PEER_KEY_ID: [u8; 32] = [9; 32];
const ISSUER_KEY_ID: [u8; 32] = [10; 32];
const COMMAND_DIGEST: [u8; 32] = [11; 32];
const OTHER_RESOURCE_ID: [u8; 16] = [17; 16];
const OTHER_GRANT_ID: [u8; 32] = [18; 32];

fn session_id() -> SessionId {
    SessionId("session-21".into())
}

fn peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId("peer-21".into()),
        key_id: PEER_KEY_ID,
    }
}

fn scopes(values: impl IntoIterator<Item = PermissionScope>) -> PermissionScopes {
    values.into_iter().collect()
}

fn consent_request() -> ConsentRequest {
    ConsentRequest {
        request_id: REQUEST_ID,
        session_id: session_id(),
        peer: peer(),
        requested_scopes: scopes([PermissionScope::ScreenView]),
        policy_revision: 3,
        windows_session_id: 7,
        issued_at_ms: 1_600,
        expires_at_ms: 2_000,
    }
}

fn consent_result() -> ConsentResult {
    ConsentResult {
        request_id: REQUEST_ID,
        session_id: session_id(),
        peer: peer(),
        policy_revision: 3,
        windows_session_id: 7,
        decision: ConsentDecision::Approved,
        approved_scopes: scopes([PermissionScope::ScreenView]),
        decided_at_ms: 1_700,
    }
}

fn register() -> AgentRegister {
    AgentRegister {
        agent_instance_id: AGENT_ID,
        process_id: 4_242,
        process_creation_time: 55,
        logon_sid_hash: [12; 32],
        windows_session_id: 7,
        agent_key_id: AGENT_KEY_ID,
        agent_nonce: [13; 32],
    }
}

fn challenge() -> AgentChallenge {
    AgentChallenge {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        challenge_id: CHALLENGE_ID,
        challenge_nonce: [14; 32],
        expected_agent_instance_id: AGENT_ID,
        expected_process_id: 4_242,
        expected_process_creation_time: 55,
        expected_logon_sid_hash: [12; 32],
        expected_windows_session_id: 7,
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
    }
}

fn registered() -> AgentRegistered {
    AgentRegistered {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        challenge_id: CHALLENGE_ID,
        agent_instance_id: AGENT_ID,
        accepted_protocol_major: AGENT_IPC_PROTOCOL_MAJOR,
        accepted_protocol_minor: 0,
        signed_at_ms: 1_500,
        signature: [15; 64],
    }
}

fn grant() -> ExecuteGrant {
    ExecuteGrant {
        claims: ExecuteGrantClaims {
            grant_id: GRANT_ID,
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            session_id: session_id(),
            peer: peer(),
            scopes: scopes([
                PermissionScope::ScreenView,
                PermissionScope::InputPointer,
                PermissionScope::InputKeyboard,
                PermissionScope::AudioListen,
                PermissionScope::AudioTalk,
                PermissionScope::ClipboardRead,
                PermissionScope::ClipboardWrite,
                PermissionScope::FileRead,
                PermissionScope::FileWrite,
            ]),
            policy_revision: 3,
            windows_session_id: 7,
            desktop_epoch: 4,
            desktop_kind: DesktopKind::Default,
            issued_at_ms: 1_000,
            not_before_ms: 1_000,
            expires_at_ms: 2_000,
            command_digest: COMMAND_DIGEST,
            audience: GrantAudience::SessionAgent,
        },
        issuer_key_id: ISSUER_KEY_ID,
        signature: [16; 64],
    }
}

fn execution_context(now_ms: u64) -> ExecutionContext {
    ExecutionContext {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        session_id: session_id(),
        peer: peer(),
        policy_revision: 3,
        windows_session_id: 7,
        desktop_epoch: 4,
        desktop_kind: DesktopKind::Default,
        now_ms,
        expected_issuer_key_id: ISSUER_KEY_ID,
    }
}

fn execute_command(command: AgentCommand) -> ExecuteCommand {
    let mut execute = ExecuteCommand {
        command_id: COMMAND_ID,
        grant: grant(),
        command,
    };
    execute.grant.claims.command_digest = execute.command_digest();
    execute
}

fn input_event(sequence: u64, event: InputEventPayload) -> InputEventEnvelope {
    InputEventEnvelope {
        session_id: session_id(),
        resource_id: RESOURCE_ID,
        start_grant_id: GRANT_ID,
        sequence,
        event,
    }
}

struct AcceptAllVerifier;

impl ExecuteGrantVerifier for AcceptAllVerifier {
    fn verify(
        &self,
        _issuer_key_id: &[u8; 32],
        _signing_bytes: &[u8],
        _signature: &[u8; 64],
    ) -> bool {
        true
    }
}

impl RegistrationProofVerifier for AcceptAllVerifier {
    fn verify(
        &self,
        _agent_key_id: &[u8; 32],
        _signing_bytes: &[u8],
        _signature: &[u8; 64],
    ) -> bool {
        true
    }
}

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = encode_frame(value).expect("encode frame");
    let decoded = decode_frame::<T>(&bytes).expect("decode frame");
    assert_eq!(decoded.protocol_major, AGENT_IPC_PROTOCOL_MAJOR);
    assert_eq!(&decoded.message, value);
    decoded.message
}

#[test]
fn registration_handshake_round_trips_required_identity_and_signature() {
    round_trip(&AgentToService::AgentRegister(register()));
    round_trip(&ServiceToAgent::AgentChallenge(challenge()));
    round_trip(&AgentToService::AgentRegistered(registered()));
}

#[test]
fn capability_and_consent_messages_round_trip() {
    let snapshot = AgentCapabilitySnapshot {
        agent_instance_id: AGENT_ID,
        registration_id: REGISTRATION_ID,
        windows_session_id: 7,
        revision: 2,
        desktop_epoch: 4,
        observed_at_ms: 1_600,
        capabilities: BTreeSet::from([
            AgentCapability::Capture,
            AgentCapability::Input,
            AgentCapability::Audio,
            AgentCapability::Clipboard,
            AgentCapability::File,
            AgentCapability::Render,
        ]),
    };
    round_trip(&AgentToService::AgentCapabilitySnapshot(snapshot));

    round_trip(&ServiceToAgent::ConsentRequest(consent_request()));
    round_trip(&AgentToService::ConsentResult(consent_result()));
}

#[test]
fn input_event_and_structured_acknowledgments_round_trip_without_payload_echo() {
    let events = [
        InputEventPayload::MouseMove { x: 320, y: 240 },
        InputEventPayload::MouseButton {
            button: InputButton::X1,
            pressed: true,
        },
        InputEventPayload::MouseWheel { delta: -120 },
        InputEventPayload::MouseHorizontalWheel { delta: 120 },
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
        InputEventPayload::ReleaseAll,
    ];

    for (index, event) in events.into_iter().enumerate() {
        round_trip(&ServiceToAgent::InputEvent(input_event(
            index as u64 + 1,
            event,
        )));
    }

    let outcomes = [
        InputAckOutcome::Applied,
        InputAckOutcome::Rejected {
            reason: InputRejection::Policy,
        },
        InputAckOutcome::Rejected {
            reason: InputRejection::Grant,
        },
        InputAckOutcome::Rejected {
            reason: InputRejection::Unsupported,
        },
        InputAckOutcome::Rejected {
            reason: InputRejection::StaleDesktop,
        },
        InputAckOutcome::Rejected {
            reason: InputRejection::Replay,
        },
        InputAckOutcome::Rejected {
            reason: InputRejection::InvalidEvent,
        },
        InputAckOutcome::Failed {
            reason: InputFailure::Uipi,
        },
        InputAckOutcome::Failed {
            reason: InputFailure::Platform,
        },
    ];

    for (index, outcome) in outcomes.into_iter().enumerate() {
        let message = AgentToService::InputAck(InputAck {
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            session_id: session_id(),
            resource_id: RESOURCE_ID,
            start_grant_id: GRANT_ID,
            sequence: index as u64 + 1,
            event_commitment: [17; 32],
            outcome,
        });
        round_trip(&message);

        let json = serde_json::to_value(message).expect("serialize input acknowledgment");
        let serialized = serde_json::to_string(&json).expect("stringify input acknowledgment");
        for forbidden in [
            "event",
            "payload_echo",
            "message",
            "native_error",
            "coordinates",
            "key",
            "button",
        ] {
            assert!(
                !serialized.contains(&format!("\"{forbidden}\"")),
                "acknowledgment leaked forbidden field {forbidden}: {serialized}"
            );
        }
    }
}

#[test]
fn start_input_binds_the_exact_nonempty_input_scope_set_into_its_grant_digest() {
    let pointer = AgentCommand::StartInput {
        resource_id: RESOURCE_ID,
        input_scopes: scopes([PermissionScope::InputPointer]),
    };
    let keyboard = AgentCommand::StartInput {
        resource_id: RESOURCE_ID,
        input_scopes: scopes([PermissionScope::InputKeyboard]),
    };
    assert_ne!(pointer.digest(), keyboard.digest());
    assert_eq!(
        pointer.required_scopes(),
        scopes([PermissionScope::InputPointer])
    );

    let mut authorized = execute_command(pointer);
    authorized.grant.claims.scopes = scopes([PermissionScope::InputPointer]);
    assert!(
        validate_execute_command(&authorized, &execution_context(1_500), &AcceptAllVerifier)
            .is_ok()
    );

    for input_scopes in [
        PermissionScopes::new(),
        scopes([PermissionScope::ScreenView]),
        scopes([PermissionScope::InputPointer, PermissionScope::ScreenView]),
    ] {
        let invalid = execute_command(AgentCommand::StartInput {
            resource_id: RESOURCE_ID,
            input_scopes,
        });
        assert_eq!(
            validate_execute_command(&invalid, &execution_context(1_500), &AcceptAllVerifier),
            Err(GrantValidationError::InvalidInputScopes)
        );
    }
}

#[test]
fn input_event_commitment_and_validation_bind_the_authorized_resource() {
    let mut start = execute_command(AgentCommand::StartInput {
        resource_id: RESOURCE_ID,
        input_scopes: scopes([PermissionScope::InputPointer]),
    });
    start.grant.claims.scopes = scopes([PermissionScope::InputPointer]);
    let authorized =
        validate_execute_command(&start, &execution_context(1_500), &AcceptAllVerifier)
            .expect("authorized start-input command");
    let resource = authorize_input_resource(authorized).expect("typed input resource");

    let envelope = input_event(1, InputEventPayload::MouseMove { x: 11, y: 12 });
    let validated = validate_input_event(&envelope, &resource, &execution_context(1_500))
        .expect("resource-bound input event");
    assert_eq!(validated.sequence(), 1);
    assert_eq!(validated.event(), &envelope.event);

    let baseline = envelope.commitment().expect("input event commitment");
    let mut variants = Vec::new();
    let mut changed = envelope.clone();
    changed.session_id = SessionId("other-session".into());
    variants.push(changed);
    let mut changed = envelope.clone();
    changed.resource_id = OTHER_RESOURCE_ID;
    variants.push(changed);
    let mut changed = envelope.clone();
    changed.start_grant_id = OTHER_GRANT_ID;
    variants.push(changed);
    let mut changed = envelope.clone();
    changed.sequence += 1;
    variants.push(changed);
    let mut changed = envelope.clone();
    changed.event = InputEventPayload::MouseMove { x: 12, y: 11 };
    variants.push(changed);
    for changed in variants {
        assert_ne!(changed.commitment().unwrap(), baseline);
    }

    for mismatched in [
        {
            let mut value = envelope.clone();
            value.session_id = SessionId("other-session".into());
            value
        },
        {
            let mut value = envelope.clone();
            value.resource_id = OTHER_RESOURCE_ID;
            value
        },
        {
            let mut value = envelope.clone();
            value.start_grant_id = OTHER_GRANT_ID;
            value
        },
    ] {
        assert_eq!(
            validate_input_event(&mismatched, &resource, &execution_context(1_500)),
            Err(InputRejection::Grant)
        );
    }

    let keyboard = input_event(
        2,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0x41 },
            pressed: true,
        },
    );
    assert_eq!(
        validate_input_event(&keyboard, &resource, &execution_context(1_500)),
        Err(InputRejection::Grant)
    );

    let mut changed_policy = execution_context(1_500);
    changed_policy.policy_revision += 1;
    assert_eq!(
        validate_input_event(&envelope, &resource, &changed_policy),
        Err(InputRejection::Policy)
    );

    let mut changed_desktop = execution_context(1_500);
    changed_desktop.desktop_epoch += 1;
    assert_eq!(
        validate_input_event(&envelope, &resource, &changed_desktop),
        Err(InputRejection::StaleDesktop)
    );
}

#[test]
fn input_event_shape_rejects_sentinels_invalid_sequences_and_invalid_keys() {
    let mut value = input_event(1, InputEventPayload::MouseMove { x: 1, y: 2 });
    assert!(value.validate_shape().is_ok());

    value.session_id = SessionId(String::new());
    assert_eq!(value.validate_shape(), Err(InputRejection::InvalidEvent));
    value = input_event(1, InputEventPayload::MouseMove { x: 1, y: 2 });
    value.resource_id = [0; 16];
    assert_eq!(value.validate_shape(), Err(InputRejection::InvalidEvent));
    value = input_event(1, InputEventPayload::MouseMove { x: 1, y: 2 });
    value.start_grant_id = [0; 32];
    assert_eq!(value.validate_shape(), Err(InputRejection::InvalidEvent));

    for sequence in [0, u64::MAX] {
        value = input_event(sequence, InputEventPayload::MouseMove { x: 1, y: 2 });
        assert_eq!(value.validate_shape(), Err(InputRejection::InvalidEvent));
    }

    value = input_event(
        1,
        InputEventPayload::Key {
            key: InputKey::VirtualKey { code: 0 },
            pressed: true,
        },
    );
    assert_eq!(value.validate_shape(), Err(InputRejection::InvalidEvent));
}

#[test]
fn consent_result_is_correlated_and_cannot_expand_approved_scopes() {
    let validated = validate_consent_result(&consent_request(), &consent_result(), 1_700)
        .expect("bound consent result");
    assert_eq!(
        validated.approved_scopes(),
        &scopes([PermissionScope::ScreenView])
    );

    let mut escalated = consent_result();
    escalated
        .approved_scopes
        .insert(PermissionScope::InputKeyboard);
    assert_eq!(
        validate_consent_result(&consent_request(), &escalated, 1_700),
        Err(ConsentValidationError::ScopeEscalation)
    );

    let mut mismatched = consent_result();
    mismatched.session_id = SessionId("other-session".into());
    assert_eq!(
        validate_consent_result(&consent_request(), &mismatched, 1_700),
        Err(ConsentValidationError::RequestMismatch)
    );

    let mut denied_with_scopes = consent_result();
    denied_with_scopes.decision = ConsentDecision::Denied;
    assert_eq!(
        validate_consent_result(&consent_request(), &denied_with_scopes, 1_700),
        Err(ConsentValidationError::UnexpectedApprovedScopes)
    );

    assert_eq!(
        validate_consent_result(&consent_request(), &consent_result(), 2_000),
        Err(ConsentValidationError::Expired)
    );
}

#[test]
fn every_product_command_round_trips_with_an_execute_grant() {
    let commands = [
        AgentCommand::StartCapture {
            resource_id: RESOURCE_ID,
            display_id: 1,
        },
        AgentCommand::StopCapture {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StartInput {
            resource_id: RESOURCE_ID,
            input_scopes: scopes([
                PermissionScope::InputPointer,
                PermissionScope::InputKeyboard,
            ]),
        },
        AgentCommand::StopInput {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StartAudio {
            resource_id: RESOURCE_ID,
            direction: AudioDirection::Listen,
        },
        AgentCommand::StopAudio {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StartClipboard {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StopClipboard {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StartFile {
            resource_id: RESOURCE_ID,
            direction: FileDirection::Download,
        },
        AgentCommand::StopFile {
            resource_id: RESOURCE_ID,
        },
        AgentCommand::StartRender {
            resource_id: RESOURCE_ID,
            display_id: 1,
        },
        AgentCommand::StopRender {
            resource_id: RESOURCE_ID,
        },
    ];

    for command in commands {
        let execute = execute_command(command);
        round_trip(&ServiceToAgent::Execute(Box::new(execute.clone())));
        let now_ms = if execute.is_cleanup() { 2_000 } else { 1_500 };
        validate_execute_command(&execute, &execution_context(now_ms), &AcceptAllVerifier)
            .expect("every command has a validator-derived authorization context");
    }
}

fn event_context(sequence: u64) -> AgentEventContext {
    AgentEventContext {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        windows_session_id: 7,
        desktop_epoch: 4,
        sequence,
        observed_at_ms: 1_800 + sequence,
    }
}

#[test]
fn lifecycle_events_and_shutdown_messages_round_trip() {
    let messages = [
        AgentToService::DesktopChanged(DesktopChanged {
            context: event_context(1),
            previous_desktop_epoch: 3,
            desktop: DesktopKind::Default,
        }),
        AgentToService::Locked(Locked {
            context: event_context(2),
        }),
        AgentToService::Unlocked(Unlocked {
            context: event_context(3),
        }),
        AgentToService::AgentStopping(AgentStopping {
            context: event_context(4),
            reason: StoppingReason::ServiceRequest,
        }),
        AgentToService::AgentCrashed(AgentCrashed {
            context: event_context(5),
            exit_code: Some(-1),
        }),
        AgentToService::AgentHeartbeat(AgentHeartbeat {
            context: event_context(6),
        }),
        AgentToService::CommandResult(CommandResult {
            registration_id: REGISTRATION_ID,
            command_id: COMMAND_ID,
            outcome: CommandOutcome::Completed,
            completed_at_ms: 1_900,
        }),
    ];

    for message in messages {
        round_trip(&message);
    }

    round_trip(&ServiceToAgent::StopAgent(StopAgent {
        request_id: REQUEST_ID,
        deadline_ms: 2_500,
        reason: StopReason::ServiceShutdown,
    }));
}

#[test]
fn frame_rejects_unknown_major_before_decoding_the_body() {
    let mut bytes = encode_frame(&AgentToService::AgentRegister(register())).unwrap();
    bytes[4..6].copy_from_slice(&(AGENT_IPC_PROTOCOL_MAJOR + 1).to_le_bytes());
    bytes.truncate(8);

    assert!(matches!(
        decode_frame::<AgentToService>(&bytes),
        Err(FrameError::UnsupportedMajor { .. })
    ));
}

#[test]
fn frame_rejects_payloads_over_the_control_plane_limit() {
    let oversized = vec![0_u8; AGENT_IPC_MAX_FRAME_BYTES + 1];
    assert!(matches!(
        encode_frame(&oversized),
        Err(FrameError::FrameTooLarge { .. })
    ));
}

#[tokio::test]
async fn async_framing_round_trips_without_transport_specific_types() {
    let (mut writer, mut reader) = tokio::io::duplex(8 * 1024);
    let expected = AgentToService::AgentRegister(register());
    let send = tokio::spawn(async move { write_frame(&mut writer, &expected).await });

    let received = read_frame::<_, AgentToService>(&mut reader)
        .await
        .expect("read frame");
    send.await.unwrap().unwrap();
    assert_eq!(received.message, AgentToService::AgentRegister(register()));
}

#[test]
fn registration_state_rejects_duplicate_register_and_consumes_challenge() {
    let mut state = AgentProtocolState::new();
    state.accept_register(register()).unwrap();
    assert!(matches!(
        state.accept_register(register()),
        Err(mrd_agent_ipc::RegistrationError::DuplicateRegistration)
    ));

    state.issue_challenge(challenge()).unwrap();
    let identity = state
        .complete_registration(registered(), 1_500, &AcceptAllVerifier)
        .unwrap();
    assert_eq!(identity.windows_session_id, 7);
    assert!(state.is_registered());
    assert!(state
        .complete_registration(registered(), 1_500, &AcceptAllVerifier)
        .is_err());
}

#[test]
fn registration_rejects_sentinel_identity_and_overlong_challenge() {
    let mut state = AgentProtocolState::new();
    let mut invalid_register = register();
    invalid_register.agent_instance_id = [0; 16];
    assert_eq!(
        state.accept_register(invalid_register),
        Err(mrd_agent_ipc::RegistrationError::InvalidRegistrationShape)
    );

    state.accept_register(register()).unwrap();
    let mut overlong = challenge();
    overlong.expires_at_ms =
        overlong.issued_at_ms + AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS + 1;
    assert_eq!(
        state.issue_challenge(overlong),
        Err(mrd_agent_ipc::RegistrationError::ChallengeLifetimeExceeded)
    );
    state.issue_challenge(challenge()).unwrap();
}

#[test]
fn failed_registration_proof_consumes_the_one_shot_challenge() {
    let mut state = AgentProtocolState::new();
    state.accept_register(register()).unwrap();
    state.issue_challenge(challenge()).unwrap();
    let mut mismatched = registered();
    mismatched.challenge_id = [99; 16];

    assert_eq!(
        state.complete_registration(mismatched, 1_500, &AcceptAllVerifier),
        Err(mrd_agent_ipc::RegistrationError::ProofMismatch)
    );
    assert_eq!(
        state.complete_registration(registered(), 1_500, &AcceptAllVerifier),
        Err(mrd_agent_ipc::RegistrationError::ChallengeConsumed)
    );
}

#[test]
fn execute_grant_is_bound_to_all_authorization_context() {
    let execute = execute_command(AgentCommand::StartCapture {
        resource_id: RESOURCE_ID,
        display_id: 1,
    });
    let authorized =
        validate_execute_command(&execute, &execution_context(1_500), &AcceptAllVerifier)
            .expect("valid grant-bound command");
    assert_eq!(authorized.grant_id(), &GRANT_ID);

    let mut expired = execution_context(2_000);
    assert_eq!(
        validate_execute_command(&execute, &expired, &AcceptAllVerifier),
        Err(GrantValidationError::Expired)
    );

    expired.now_ms = 1_500;
    expired.session_id = SessionId("wrong-session".into());
    assert_eq!(
        validate_execute_command(&execute, &expired, &AcceptAllVerifier),
        Err(GrantValidationError::SessionMismatch)
    );
}

#[test]
fn an_expired_grant_only_remains_usable_for_its_bound_cleanup_command() {
    let execute = execute_command(AgentCommand::StopCapture {
        resource_id: RESOURCE_ID,
    });
    let context = execution_context(2_000);
    assert!(validate_execute_command(&execute, &context, &AcceptAllVerifier).is_ok());

    let mut substituted = execute.clone();
    substituted.command = AgentCommand::StopCapture {
        resource_id: [99; 16],
    };
    assert_eq!(
        validate_execute_command(&substituted, &context, &AcceptAllVerifier),
        Err(GrantValidationError::CommandMismatch)
    );
}

#[test]
fn execute_grant_digest_covers_the_command_id() {
    let execute = execute_command(AgentCommand::StartCapture {
        resource_id: RESOURCE_ID,
        display_id: 1,
    });
    let mut substituted = execute.clone();
    substituted.command_id = [77; 16];

    assert_eq!(
        validate_execute_command(&substituted, &execution_context(1_500), &AcceptAllVerifier,),
        Err(GrantValidationError::CommandMismatch)
    );
}

#[test]
fn execute_grant_rejects_each_mismatched_authorization_binding() {
    let execute = execute_command(AgentCommand::StartCapture {
        resource_id: RESOURCE_ID,
        display_id: 1,
    });

    let mut context = execution_context(1_500);
    context.registration_epoch += 1;
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::RegistrationEpochMismatch)
    );

    let mut context = execution_context(1_500);
    context.peer.device_id = DeviceId("other-peer".into());
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::PeerDeviceMismatch)
    );

    let mut context = execution_context(1_500);
    context.peer.key_id = [88; 32];
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::PeerKeyMismatch)
    );

    let mut context = execution_context(1_500);
    context.policy_revision += 1;
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::PolicyRevisionMismatch)
    );

    let mut context = execution_context(1_500);
    context.windows_session_id += 1;
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::WindowsSessionMismatch)
    );

    let mut context = execution_context(1_500);
    context.desktop_epoch += 1;
    assert_eq!(
        validate_execute_command(&execute, &context, &AcceptAllVerifier),
        Err(GrantValidationError::DesktopEpochMismatch)
    );

    let mut under_scoped = execute.clone();
    under_scoped
        .grant
        .claims
        .scopes
        .remove(&PermissionScope::ScreenView);
    assert_eq!(
        validate_execute_command(&under_scoped, &execution_context(1_500), &AcceptAllVerifier),
        Err(GrantValidationError::InsufficientScopes)
    );

    let mut oversized_session = execute.clone();
    oversized_session.grant.claims.session_id = SessionId("s".repeat(257));
    assert_eq!(
        validate_execute_command(
            &oversized_session,
            &execution_context(1_500),
            &AcceptAllVerifier,
        ),
        Err(GrantValidationError::InvalidSessionId)
    );

    let mut oversized_peer = execute;
    oversized_peer.grant.claims.peer.device_id = DeviceId("p".repeat(257));
    assert_eq!(
        validate_execute_command(
            &oversized_peer,
            &execution_context(1_500),
            &AcceptAllVerifier,
        ),
        Err(GrantValidationError::InvalidPeerDeviceId)
    );
}

#[test]
fn ordinary_session_agent_rejects_all_non_default_desktop_starts() {
    let mut capture = execute_command(AgentCommand::StartCapture {
        resource_id: RESOURCE_ID,
        display_id: 1,
    });
    capture.grant.claims.desktop_kind = DesktopKind::Secure;
    let mut secure_context = execution_context(1_500);
    secure_context.desktop_kind = DesktopKind::Secure;
    capture
        .grant
        .claims
        .scopes
        .insert(PermissionScope::SecureDesktopView);
    assert_eq!(
        validate_execute_command(&capture, &secure_context, &AcceptAllVerifier),
        Err(GrantValidationError::UnsupportedDesktop)
    );

    let mut input = execute_command(AgentCommand::StartInput {
        resource_id: RESOURCE_ID,
        input_scopes: scopes([
            PermissionScope::InputPointer,
            PermissionScope::InputKeyboard,
        ]),
    });
    input.grant.claims.desktop_kind = DesktopKind::Winlogon;
    let mut winlogon_context = execution_context(1_500);
    winlogon_context.desktop_kind = DesktopKind::Winlogon;
    input
        .grant
        .claims
        .scopes
        .insert(PermissionScope::SecureDesktopControl);
    assert_eq!(
        validate_execute_command(&input, &winlogon_context, &AcceptAllVerifier),
        Err(GrantValidationError::UnsupportedDesktop)
    );

    let mut unknown = execute_command(AgentCommand::StartCapture {
        resource_id: RESOURCE_ID,
        display_id: 1,
    });
    unknown.grant.claims.desktop_kind = DesktopKind::Unknown;
    let mut unknown_context = execution_context(1_500);
    unknown_context.desktop_kind = DesktopKind::Unknown;
    assert_eq!(
        validate_execute_command(&unknown, &unknown_context, &AcceptAllVerifier),
        Err(GrantValidationError::UnsupportedDesktop)
    );
}

#[test]
fn messages_reject_unknown_or_secret_bearing_fields() {
    let mut value = serde_json::to_value(AgentToService::AgentRegister(register())).unwrap();
    let payload = value
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .expect("tagged payload object");
    payload.insert("private_key".into(), Value::String("forbidden".into()));

    assert!(serde_json::from_value::<AgentToService>(value).is_err());

    let mut outer = serde_json::to_value(ServiceToAgent::AgentChallenge(challenge())).unwrap();
    outer
        .as_object_mut()
        .unwrap()
        .insert("private_key".into(), Value::String("forbidden".into()));
    assert!(serde_json::from_value::<ServiceToAgent>(outer).is_err());

    let mut input = serde_json::to_value(ServiceToAgent::InputEvent(input_event(
        1,
        InputEventPayload::MouseMove { x: 1, y: 2 },
    )))
    .unwrap();
    input
        .get_mut("payload")
        .and_then(Value::as_object_mut)
        .unwrap()
        .insert("native_error".into(), Value::String("forbidden".into()));
    assert!(serde_json::from_value::<ServiceToAgent>(input).is_err());

    let mut ack = serde_json::to_value(AgentToService::InputAck(InputAck {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        session_id: session_id(),
        resource_id: RESOURCE_ID,
        start_grant_id: GRANT_ID,
        sequence: 1,
        event_commitment: [17; 32],
        outcome: InputAckOutcome::Applied,
    }))
    .unwrap();
    ack.get_mut("payload")
        .and_then(Value::as_object_mut)
        .unwrap()
        .insert("event".into(), serde_json::json!({ "kind": "key" }));
    assert!(serde_json::from_value::<AgentToService>(ack).is_err());
}

#[test]
fn serialized_control_messages_contain_no_secret_material_fields() {
    let fixtures = [
        serde_json::to_value(AgentToService::AgentRegister(register())).unwrap(),
        serde_json::to_value(ServiceToAgent::AgentChallenge(challenge())).unwrap(),
        serde_json::to_value(AgentToService::AgentRegistered(registered())).unwrap(),
        serde_json::to_value(ServiceToAgent::Execute(Box::new(ExecuteCommand {
            command_id: COMMAND_ID,
            grant: grant(),
            command: AgentCommand::StopCapture {
                resource_id: RESOURCE_ID,
            },
        })))
        .unwrap(),
        serde_json::to_value(ServiceToAgent::InputEvent(input_event(
            1,
            InputEventPayload::MouseMove { x: 1, y: 2 },
        )))
        .unwrap(),
        serde_json::to_value(AgentToService::InputAck(InputAck {
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            session_id: session_id(),
            resource_id: RESOURCE_ID,
            start_grant_id: GRANT_ID,
            sequence: 1,
            event_commitment: [17; 32],
            outcome: InputAckOutcome::Rejected {
                reason: InputRejection::Policy,
            },
        }))
        .unwrap(),
    ];
    let forbidden = [
        "private_key",
        "secret_key",
        "password",
        "credential",
        "unattended_secret",
        "access_token",
        "refresh_token",
        "cookie",
        "raw_sid",
        "windows_token",
        "clipboard_content",
        "file_content",
        "media_frame",
    ];

    fn visit(value: &Value, forbidden: &[&str]) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    assert!(!forbidden.contains(&key.as_str()), "forbidden field: {key}");
                    visit(child, forbidden);
                }
            }
            Value::Array(values) => {
                for child in values {
                    visit(child, forbidden);
                }
            }
            _ => {}
        }
    }

    for fixture in &fixtures {
        visit(fixture, &forbidden);
    }
}
