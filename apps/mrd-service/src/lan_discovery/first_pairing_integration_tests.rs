use super::*;
use mrd_identity::DeviceIdentity;
use mrd_ipc::{DecimalU64, IpcRequest, IpcResponse, LanPairingApproval};
use mrd_store_sqlite::{AeadSecretProtector, AuditDraft, PersistentStore};
use std::collections::BTreeMap;

fn signed_peer(peer: &DeviceIdentity, at: u64, nonce: u8) -> SignedLanAnnouncement {
    let endpoint: SocketAddr = "192.168.1.241:21116".parse().unwrap();
    SignedLanAnnouncement::sign(
        peer,
        1,
        LanAnnouncement {
            magic: DISCOVERY_MAGIC.to_owned(),
            app_id: DISCOVERY_APP_ID.to_owned(),
            instance_id: "first-pairing-peer".to_owned(),
            device_id: "lan-peer".to_owned(),
            device_name: "Paired desktop".to_owned(),
            device_type: "desktop".to_owned(),
            protocol_version: SIGNED_LAN_PROTOCOL_VERSION,
            discovery_port: endpoint.port(),
            transports: vec!["quic".to_owned()],
            service_build_id: Some("pairing-test".to_owned()),
            media_protocol_version: Some(3),
            media_capabilities: Vec::new(),
            mac_address: None,
            timestamp_ms: at,
        },
        endpoint,
        at + 5_000,
        [nonce; 16],
    )
    .unwrap()
}

#[tokio::test]
async fn formal_candidate_query_returns_only_service_verified_current_discovery() {
    let state = Arc::new(AppState::default());
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let at = now_ms();
    let signed = signed_peer(&peer, at, 41);
    let endpoint = signed.payload.discovery_endpoint;
    ingest_signed_lan_announcement(&state, signed.clone(), endpoint, at)
        .await
        .unwrap();
    assert!(ingest_signed_lan_announcement(&state, signed, endpoint, at)
        .await
        .is_err());
    let server = crate::ipc_server::IpcServer::new(state.clone());
    let IpcResponse::LanPairingCandidateList { candidates } = server
        .handle_request(IpcRequest::ListLanPairingCandidates)
        .await
    else {
        panic!("candidate query must be a public binding projection");
    };
    assert_eq!(
        candidates.len(),
        1,
        "a verified untrusted peer needs an explicit UI pairing candidate"
    );
    assert_eq!(candidates[0].peer_key_id, peer.key_id());
    assert_eq!(candidates[0].discovery_endpoint, endpoint.to_string());
    assert_eq!(
        candidates[0].permission_ceiling,
        vec![RemotePermissionScope::ScreenView]
    );
    assert!(!state.lan_discovery.snapshot().await.peers[0].p2p_available);
    assert!(state.sessions.lock().await.list_all().is_empty());
}

#[tokio::test]
async fn general_ipc_cannot_turn_a_displayed_candidate_into_machine_trust() {
    let state = Arc::new(AppState::default());
    let server = crate::ipc_server::IpcServer::new(state.clone());
    let response = server
        .handle_request(IpcRequest::ApproveLanPairing {
            approval: LanPairingApproval {
                candidate_id: "07".repeat(16),
                device_id: DeviceId("lan-peer".to_owned()),
                peer_key_id: "86".repeat(32),
                key_epoch: DecimalU64::new(1),
                discovery_endpoint: "192.168.1.241:21116".to_owned(),
                permission_ceiling: vec![RemotePermissionScope::ScreenView],
            },
        })
        .await;
    assert!(
        matches!(response, IpcResponse::Error { code, .. } if code == "E_PRODUCT_CALLER_DENIED")
    );
    assert!(state.sessions.lock().await.list_all().is_empty());
}

#[tokio::test]
async fn trusted_device_projection_uses_the_durable_permission_policy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pairing.sqlite");
    let protector = Arc::new(AeadSecretProtector::from_key([0x2A; 32]).unwrap());
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let store = PersistentStore::open(&path, protector.clone()).unwrap();
    store
        .insert_trusted_device_with_policy_and_audit(
            peer.key_id(),
            peer.public_key(),
            1,
            &["screen.view".to_owned()],
            AuditDraft {
                timestamp_ms: now_ms(),
                action: "trust.lan_paired".to_owned(),
                outcome: "allowed".to_owned(),
                session_id: None,
                actor_device_id: None,
                peer_device_id: None,
                transport_kind: Some("lan_quic".to_owned()),
                reason_code: None,
                details: BTreeMap::from([("peer_key_id".to_owned(), peer.key_id().to_owned())]),
            },
        )
        .unwrap();
    drop(store);
    let state = Arc::new(AppState::open_persistent(&path, protector).unwrap());
    let IpcResponse::TrustedDeviceList { devices } =
        crate::handlers::identity::list_trusted_devices(&state, false).await
    else {
        panic!("trusted devices projection must load sealed policy");
    };
    assert_eq!(devices.len(), 1);
    assert_eq!(
        devices[0].permission_ceiling,
        vec![RemotePermissionScope::ScreenView],
        "policy must survive store reopen and reach the product projection"
    );
}

async fn pending_pairing_state() -> (
    tempfile::TempDir,
    Arc<AppState>,
    DeviceIdentity,
    mrd_ipc::LanPairingApproval,
) {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(
        AppState::open_persistent(
            directory.path().join("approval.sqlite"),
            Arc::new(AeadSecretProtector::from_key([0x35; 32]).unwrap()),
        )
        .unwrap(),
    );
    let peer = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let at = now_ms();
    let signed = signed_peer(&peer, at, 61);
    let endpoint = signed.payload.discovery_endpoint;
    ingest_signed_lan_announcement(&state, signed, endpoint, at)
        .await
        .unwrap();
    let IpcResponse::LanPairingCandidateList { candidates } =
        pairing_approval::list_candidates(&state).await
    else {
        panic!("verified candidate");
    };
    let candidate = &candidates[0];
    let approval = mrd_ipc::LanPairingApproval {
        candidate_id: candidate.candidate_id.clone(),
        device_id: candidate.device_id.clone(),
        peer_key_id: candidate.peer_key_id.clone(),
        key_epoch: candidate.key_epoch,
        discovery_endpoint: candidate.discovery_endpoint.clone(),
        permission_ceiling: vec![RemotePermissionScope::ScreenView],
    };
    (directory, state, peer, approval)
}

#[tokio::test]
async fn verified_approval_body_commits_only_screen_policy_and_audit_without_a_session() {
    let (_directory, state, peer, approval) = pending_pairing_state().await;
    // The OS admission layer is tested separately; this closure represents its
    // already verified actor and lets this test exercise the real DB transaction.
    let response = pairing_approval::approve_candidate(&state, approval.clone(), || Ok(())).await;
    let IpcResponse::TrustedDeviceUpdated { device } = response else {
        panic!("atomic pairing should succeed: {response:?}");
    };
    assert_eq!(
        device.permission_ceiling,
        vec![RemotePermissionScope::ScreenView]
    );
    assert_eq!(
        state
            .device_identities
            .permission_ceiling(peer.key_id())
            .unwrap(),
        vec![RemotePermissionScope::ScreenView]
    );
    assert!(state.lan_discovery.snapshot().await.peers[0].p2p_available);
    assert!(state.sessions.lock().await.list_all().is_empty());
    assert!(matches!(
        pairing_approval::approve_candidate(&state, approval, || Ok(())).await,
        IpcResponse::Error { .. }
    ));
    assert_eq!(
        state
            .device_identities
            .trusted_records(false)
            .unwrap()
            .len(),
        1
    );
    let events = state
        .audit_log
        .query(&mrd_ipc::AuditLogQuery {
            action: Some("trust.lan_paired".to_owned()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, "allowed");
}

#[tokio::test]
async fn changed_local_actor_never_commits_first_pairing_policy() {
    let (_directory, state, peer, approval) = pending_pairing_state().await;
    let result = pairing_approval::approve_candidate(&state, approval, || {
        Err(mrd_store_sqlite::StoreError::TrustTransition(
            "active desktop changed".to_owned(),
        ))
    })
    .await;
    assert!(matches!(result, IpcResponse::Error { .. }));
    assert_eq!(
        state
            .device_identities
            .authenticated_peer_trust(peer.key_id(), peer.public_key(), 1)
            .unwrap(),
        AuthenticatedPeerTrust::Untrusted
    );
    assert!(state
        .device_identities
        .permission_ceiling(peer.key_id())
        .unwrap()
        .is_empty());
    assert!(state.security_is_healthy());
}

#[tokio::test]
async fn widening_first_pairing_scopes_is_audited_and_leaves_no_trust() {
    let (_directory, state, peer, mut approval) = pending_pairing_state().await;
    approval
        .permission_ceiling
        .push(RemotePermissionScope::InputPointer);
    assert!(matches!(
        pairing_approval::approve_candidate(&state, approval, || Ok(())).await,
        IpcResponse::Error { .. }
    ));
    assert_eq!(
        state
            .device_identities
            .authenticated_peer_trust(peer.key_id(), peer.public_key(), 1)
            .unwrap(),
        AuthenticatedPeerTrust::Untrusted
    );
    let events = state
        .audit_log
        .query(&mrd_ipc::AuditLogQuery {
            action: Some("trust.lan_pairing".to_owned()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, "denied");
}

#[tokio::test]
async fn outgoing_lan_authorization_restricts_requests_to_the_sealed_pairing_ceiling() {
    let (_directory, state, peer, approval) = pending_pairing_state().await;
    assert!(matches!(
        pairing_approval::approve_candidate(&state, approval, || Ok(())).await,
        IpcResponse::TrustedDeviceUpdated { .. }
    ));
    let session_id = SessionId("first-pairing-ceiling".to_owned());
    let at = now_ms();
    let scopes = vec![
        RemotePermissionScope::ScreenView,
        RemotePermissionScope::InputPointer,
    ];
    begin_outgoing_authorization_under_security_gate(
        &state,
        &session_id,
        peer.key_id(),
        peer.public_key(),
        1,
        crate::session_authorization::VerifiedIncomingAuthorizationRequest {
            session_id: session_id.clone(),
            peer_device_id: DeviceId("lan-peer".to_owned()),
            peer_key_id: peer.key_id().to_owned(),
            peer_key_epoch: 1,
            access_mode: RemoteAccessMode::Attended,
            requested_scopes: scopes.clone(),
            peer_permission_ceiling: scopes.clone(),
            machine_permission_ceiling: scopes.clone(),
            runtime_capabilities: scopes.clone(),
            transport_kind: "quic".to_owned(),
            request_nonce: [31; 16],
            created_at_ms: at,
            expires_at_ms: at + 5000,
        },
    )
    .await
    .unwrap();
    let grant = crate::session_authorization::VerifiedSessionGrant {
        grant_id: "signed-pairing-ceiling-grant".to_owned(),
        session_id: session_id.clone(),
        policy_revision: 1,
        granted_scopes: scopes,
        issued_at_ms: at,
        expires_at_ms: at + 5000,
        transport_fingerprint_sha256: [5; 32],
        route_constraint: "quic".to_owned(),
    };
    assert!(
        state
            .session_authorizations
            .install_verified_grant(grant, at + 1)
            .await
            .is_err(),
        "a verified transport grant cannot expand the durable screen-only pairing policy"
    );
}
