use mrd_identity::public_key_id;
use mrd_ipc::{
    AuditEventsQueryV2, DecimalU64, IpcRequest, IpcResponse, RemoteAccessMode,
    RemoteAuthorizationState, RemoteCursorState, RemoteMediaState, RemotePermissionScope,
    RemoteReasonCode, RemoteRouteKind, RemoteRouteState, RouteCandidateState,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_service::{
    ipc_server::IpcServer,
    session_authorization::{VerifiedIncomingAuthorizationRequest, VerifiedSessionGrant},
    AppState,
};
use std::{sync::Arc, time::SystemTime};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("system time")
        .as_millis() as u64
}

fn outgoing_request(
    session_id: &str,
    peer_public_key: &[u8; 32],
) -> VerifiedIncomingAuthorizationRequest {
    let created_at_ms = now_ms();
    VerifiedIncomingAuthorizationRequest {
        session_id: SessionId(session_id.to_string()),
        peer_device_id: DeviceId("secure-lan-peer".to_string()),
        peer_key_id: public_key_id(peer_public_key),
        peer_key_epoch: 4,
        access_mode: RemoteAccessMode::Attended,
        requested_scopes: vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ],
        peer_permission_ceiling: vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ],
        machine_permission_ceiling: vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ],
        runtime_capabilities: vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ],
        transport_kind: "quic".to_string(),
        request_nonce: [0x21; 16],
        created_at_ms,
        expires_at_ms: created_at_ms + 60_000,
    }
}

async fn install_streaming_grant(
    state: &Arc<AppState>,
    session_id: &str,
) -> (SessionId, [u8; 32], u64) {
    let peer_public_key = [0x41; 32];
    let request = outgoing_request(session_id, &peer_public_key);
    let session_id = request.session_id.clone();
    let installed_at_ms = request.created_at_ms;
    state
        .session_authorizations
        .begin_outgoing(request)
        .await
        .expect("outgoing authorization");
    state
        .session_authorizations
        .bind_authenticated_peer_key(&session_id, &peer_public_key, installed_at_ms)
        .await
        .expect("authenticated peer key binding");
    state
        .session_authorizations
        .install_verified_grant(
            VerifiedSessionGrant {
                grant_id: format!("sha256:{}", "7a".repeat(32)),
                session_id: session_id.clone(),
                granted_scopes: vec![
                    RemotePermissionScope::ScreenView,
                    RemotePermissionScope::InputPointer,
                    RemotePermissionScope::InputKeyboard,
                ],
                issued_at_ms: installed_at_ms,
                expires_at_ms: installed_at_ms + 30_000,
                policy_revision: 9,
                route_constraint: "quic".to_string(),
                transport_fingerprint_sha256: [0xAB; 32],
            },
            installed_at_ms,
        )
        .await
        .expect("verified grant");
    state
        .session_authorizations
        .mark_streaming(&session_id, installed_at_ms)
        .await
        .expect("streaming snapshot");
    (session_id, peer_public_key, installed_at_ms)
}

#[tokio::test]
async fn route_evidence_projects_the_exact_verified_grant_and_connected_candidate() {
    let state = Arc::new(AppState::new());
    let (session_id, _, installed_at_ms) =
        install_streaming_grant(&state, "secure-route-evidence").await;
    let response = IpcServer::new(state)
        .handle_request(IpcRequest::GetRouteEvidence {
            session_id: session_id.clone(),
        })
        .await;

    let IpcResponse::RouteEvidence { evidence } = response else {
        panic!("expected authoritative route evidence, got {response:?}");
    };
    assert_eq!(evidence.session_id, session_id);
    assert_eq!(evidence.route_state, RemoteRouteState::Connected);
    assert_eq!(evidence.selected_route, Some(RemoteRouteKind::LanQuic));
    assert_eq!(evidence.policy_revision, DecimalU64::new(9));
    let expected_fingerprint = format!("sha256:{}", "ab".repeat(32));
    assert_eq!(
        evidence.transport_fingerprint_sha256.as_deref(),
        Some(expected_fingerprint.as_str())
    );
    assert!(evidence.observed_at_ms >= installed_at_ms);
    assert!(matches!(
        evidence.candidates.as_slice(),
        [candidate]
            if candidate.route == RemoteRouteKind::LanQuic
                && candidate.state == RouteCandidateState::Connected
                && candidate.started_at_ms == Some(installed_at_ms)
                && candidate.completed_at_ms == Some(installed_at_ms)
                && candidate.failure.is_none()
    ));
}

#[tokio::test]
async fn route_evidence_rejects_missing_and_unverified_sessions() {
    let state = Arc::new(AppState::new());
    let server = IpcServer::new(state.clone());
    let missing = server
        .handle_request(IpcRequest::GetRouteEvidence {
            session_id: SessionId("missing-secure-session".to_string()),
        })
        .await;
    assert!(matches!(
        missing,
        IpcResponse::Error { ref code, .. } if code == "E_REMOTE_SESSION_NOT_FOUND"
    ));

    let peer_public_key = [0x52; 32];
    let pending = state
        .session_authorizations
        .begin_outgoing(outgoing_request("unverified-route", &peer_public_key))
        .await
        .expect("unverified outgoing session");
    let unverified = server
        .handle_request(IpcRequest::GetRouteEvidence {
            session_id: pending.session_id.clone(),
        })
        .await;
    assert!(matches!(
        unverified,
        IpcResponse::RemoteAccessError {
            session_id: Some(ref session_id),
            ref failure,
            ..
        } if session_id == &pending.session_id
            && failure.code == RemoteReasonCode::PolicyChanged
    ));
    assert_eq!(
        state
            .session_authorizations
            .snapshot(&pending.session_id)
            .await
            .expect("pending session remains authoritative")
            .authorization_state,
        RemoteAuthorizationState::Authorizing
    );
}

#[tokio::test]
async fn audit_events_v2_use_an_exclusive_filtered_cursor_and_redact_arbitrary_details() {
    let state = Arc::new(AppState::new());
    let session_id = SessionId("audit-secure-session".to_string());
    let peer_device_id = DeviceId("audit-peer".to_string());
    let metadata = vec![
        (
            "peer_key_id".to_string(),
            format!("sha256:{}", "42".repeat(32)),
        ),
        ("authorization_state".to_string(), "granted".to_string()),
        ("access_mode".to_string(), "attended".to_string()),
        ("route_state".to_string(), "connected".to_string()),
        ("media_state".to_string(), "streaming".to_string()),
        (
            "requested_scopes".to_string(),
            "screen.view,input.pointer,input.keyboard".to_string(),
        ),
        (
            "granted_scopes".to_string(),
            "screen.view,input.pointer,input.keyboard".to_string(),
        ),
        ("policy_revision".to_string(), "9".to_string()),
        ("trust_revision".to_string(), "12".to_string()),
        (
            "credential_secret".to_string(),
            "must-never-leave-the-service".to_string(),
        ),
    ];
    state
        .audit_log
        .record(
            "session.authorization_grant",
            "allowed",
            Some(session_id.clone()),
            Some(DeviceId("local-device".to_string())),
            Some(peer_device_id.clone()),
            Some("quic".to_string()),
            None,
            metadata.clone(),
        )
        .expect("first matching audit event");
    state
        .audit_log
        .record(
            "session.authorization_grant",
            "allowed",
            Some(SessionId("other-session".to_string())),
            None,
            Some(peer_device_id.clone()),
            Some("quic".to_string()),
            None,
            Vec::new(),
        )
        .expect("non-matching audit event");
    state
        .audit_log
        .record(
            "session.authorization_grant",
            "allowed",
            Some(session_id.clone()),
            None,
            Some(peer_device_id.clone()),
            Some("quic".to_string()),
            Some("scope_denied".to_string()),
            metadata,
        )
        .expect("second matching audit event");

    let server = IpcServer::new(state);
    let query = |after_sequence| AuditEventsQueryV2 {
        after_sequence,
        limit: 1,
        session_id: Some(session_id.clone()),
        action: Some("session.authorization_grant".to_string()),
        outcome: Some("allowed".to_string()),
        peer_device_id: Some(peer_device_id.clone()),
    };
    let first = server
        .handle_request(IpcRequest::GetAuditEventsV2 {
            query: query(Some(DecimalU64::new(0))),
        })
        .await;
    let IpcResponse::AuditEventsV2 { page: first } = first else {
        panic!("expected first audit page, got {first:?}");
    };
    assert_eq!(first.cursor_state, RemoteCursorState::Current);
    assert!(first.chain_verified);
    assert!(first.has_more);
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.next_after_sequence, Some(first.events[0].sequence));
    let event = &first.events[0];
    assert_eq!(event.transport_kind, Some(RemoteRouteKind::LanQuic));
    let expected_peer_key_id = format!("sha256:{}", "42".repeat(32));
    assert_eq!(
        event.peer_key_id.as_deref(),
        Some(expected_peer_key_id.as_str())
    );
    assert_eq!(
        event.metadata.authorization_state,
        Some(RemoteAuthorizationState::Granted)
    );
    assert_eq!(event.metadata.access_mode, Some(RemoteAccessMode::Attended));
    assert_eq!(
        event.metadata.route_state,
        Some(RemoteRouteState::Connected)
    );
    assert_eq!(
        event.metadata.media_state,
        Some(RemoteMediaState::Streaming)
    );
    assert_eq!(event.metadata.policy_revision, Some(DecimalU64::new(9)));
    assert_eq!(event.metadata.trust_revision, Some(DecimalU64::new(12)));
    assert_eq!(
        event.metadata.granted_scopes,
        vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ]
    );
    let serialized = serde_json::to_string(&first).expect("serialize redacted audit page");
    assert!(!serialized.contains("must-never-leave-the-service"));
    assert!(!serialized.contains("credential_secret"));
    assert!(!serialized.contains("event_hash"));

    let second = server
        .handle_request(IpcRequest::GetAuditEventsV2 {
            query: query(first.next_after_sequence),
        })
        .await;
    let IpcResponse::AuditEventsV2 { page: second } = second else {
        panic!("expected second audit page, got {second:?}");
    };
    assert_eq!(second.events.len(), 1);
    assert!(second.events[0].sequence > first.events[0].sequence);
    assert_eq!(
        second.events[0].reason_code,
        Some(RemoteReasonCode::ScopeDenied)
    );
    assert!(!second.has_more);
}

#[tokio::test]
async fn audit_events_v2_requires_a_snapshot_reset_when_the_cursor_predates_retention() {
    let state = Arc::new(AppState::new());
    for index in 0..1_001_u64 {
        state
            .audit_log
            .record(
                "security.retention_probe",
                "observed",
                Some(SessionId(format!("retention-{index}"))),
                None,
                None,
                None,
                None,
                Vec::new(),
            )
            .expect("bounded in-memory audit append");
    }

    let response = IpcServer::new(state)
        .handle_request(IpcRequest::GetAuditEventsV2 {
            query: AuditEventsQueryV2 {
                after_sequence: Some(DecimalU64::new(0)),
                limit: 16,
                session_id: None,
                action: None,
                outcome: None,
                peer_device_id: None,
            },
        })
        .await;
    let IpcResponse::AuditEventsV2 { page } = response else {
        panic!("expected reset-required audit page, got {response:?}");
    };
    assert_eq!(page.cursor_state, RemoteCursorState::ResetRequired);
    assert!(page.events.is_empty());
    assert_eq!(page.next_after_sequence, Some(DecimalU64::new(1_001)));
    assert!(!page.has_more);
    assert!(!page.chain_verified);
}

#[tokio::test]
async fn audit_events_v2_projects_secure_lan_start_and_stop_bindings_for_the_product_gate() {
    let state = Arc::new(AppState::new());
    let session_id = SessionId("secure-lifecycle-audit".to_string());
    let actor_device_id = DeviceId("secure-controller".to_string());
    let peer_device_id = DeviceId("secure-target".to_string());
    for action in ["session.start_lan", "session.stop"] {
        state
            .audit_log
            .record(
                action,
                "success",
                Some(session_id.clone()),
                Some(actor_device_id.clone()),
                Some(peer_device_id.clone()),
                Some("quic".to_string()),
                None,
                Vec::new(),
            )
            .expect("secure lifecycle audit append");
    }

    let response = IpcServer::new(state)
        .handle_request(IpcRequest::GetAuditEventsV2 {
            query: AuditEventsQueryV2 {
                after_sequence: Some(DecimalU64::new(0)),
                limit: 16,
                session_id: Some(session_id.clone()),
                action: None,
                outcome: None,
                peer_device_id: None,
            },
        })
        .await;
    let IpcResponse::AuditEventsV2 { page } = response else {
        panic!("expected secure lifecycle audit page, got {response:?}");
    };
    assert!(page.chain_verified);
    assert!(!page.has_more);
    assert_eq!(page.events.len(), 2);
    assert_eq!(
        page.events
            .iter()
            .map(|event| event.action.as_str())
            .collect::<Vec<_>>(),
        vec!["session.start_lan", "session.stop"]
    );
    assert!(page.events.iter().all(|event| {
        event.outcome == "success"
            && event.session_id.as_ref() == Some(&session_id)
            && event.actor_device_id.as_ref() == Some(&actor_device_id)
            && event.peer_device_id.as_ref() == Some(&peer_device_id)
            && event.transport_kind == Some(RemoteRouteKind::LanQuic)
    }));
}
