use super::{first_pairing::VerifiedLanPairingPeer, now_ms, resolve_authenticated_peer_trust};
use crate::app_state::{
    redact_audit_correlation_id, AppState, AuthenticatedPeerTrust, DeviceIdentityRegistryError,
};
use mrd_ipc::{IpcResponse, LanPairingApproval, LanPairingCandidate, RemotePermissionScope};
use mrd_store_sqlite::{AuditDraft, StoreError};
use std::{collections::BTreeMap, sync::Arc, time::Instant};

pub(crate) async fn list_candidates(state: &Arc<AppState>) -> IpcResponse {
    let _gate = state.authorization_security_gate.lock().await;
    let candidates = state
        .lan_discovery
        .first_pairing
        .lock()
        .await
        .snapshot(now_ms(), Instant::now());
    let mut current = Vec::<LanPairingCandidate>::new();
    for candidate in candidates {
        let approval = approval_for_candidate(&candidate);
        let peer = match state.lan_discovery.first_pairing.lock().await.resolve(
            &approval,
            now_ms(),
            Instant::now(),
        ) {
            Ok(peer) => peer,
            Err(_) => continue,
        };
        match resolve_authenticated_peer_trust(
            state,
            &peer.peer_key_id,
            &peer.public_key,
            peer.key_epoch,
        )
        .await
        {
            Ok(AuthenticatedPeerTrust::Untrusted) => current.push(candidate),
            Ok(_) => state
                .lan_discovery
                .first_pairing
                .lock()
                .await
                .remove_peer(&peer.peer_key_id),
            Err(_) => return security_error(state),
        }
    }
    IpcResponse::LanPairingCandidateList {
        candidates: current,
    }
}

fn approval_for_candidate(candidate: &LanPairingCandidate) -> LanPairingApproval {
    LanPairingApproval {
        candidate_id: candidate.candidate_id.clone(),
        device_id: candidate.device_id.clone(),
        peer_key_id: candidate.peer_key_id.clone(),
        key_epoch: candidate.key_epoch,
        discovery_endpoint: candidate.discovery_endpoint.clone(),
        permission_ceiling: vec![RemotePermissionScope::ScreenView],
    }
}

/// Called only after dispatch authenticates the installed product caller.
/// The late actor check runs under the actual SQLite writer lock and must not
/// access the store again. Pairing never installs a session or media grant.
pub(crate) async fn approve_candidate(
    state: &Arc<AppState>,
    approval: LanPairingApproval,
    late_actor_check: impl FnOnce() -> Result<(), StoreError> + Send + 'static,
) -> IpcResponse {
    let _gate = state.authorization_security_gate.lock().await;
    let peer = match resolve_candidate(state, &approval).await {
        Ok(peer) => peer,
        Err(_) => {
            return deny(
                state,
                &approval,
                "pairing_candidate_invalid",
                "配对信息已变更或失效，请刷新候选后重新核对",
            )
            .await
        }
    };
    match resolve_authenticated_peer_trust(
        state,
        &peer.peer_key_id,
        &peer.public_key,
        peer.key_epoch,
    )
    .await
    {
        Ok(AuthenticatedPeerTrust::Untrusted) => {}
        Ok(_) => {
            return deny(
                state,
                &approval,
                "pairing_existing_identity",
                "该密钥已存在信任记录，首次配对不能覆盖或恢复它",
            )
            .await
        }
        Err(_) => return security_error(state),
    }
    // A sealed-store read can wait for another writer. Do not use a candidate
    // that expired while the asynchronous lookup was in progress.
    if resolve_candidate(state, &approval).await.is_err() {
        return deny(
            state,
            &approval,
            "pairing_candidate_expired",
            "配对候选已失效，请刷新后重试",
        )
        .await;
    }
    let actor_device_id = state
        .devices
        .lock()
        .await
        .get_local_device()
        .map(|(id, _)| redact_audit_correlation_id(id.0.clone()));
    let draft = AuditDraft {
        timestamp_ms: now_ms(),
        action: "trust.lan_paired".to_owned(),
        outcome: "attempted".to_owned(),
        session_id: None,
        actor_device_id,
        peer_device_id: Some(redact_audit_correlation_id(peer.device_id.0.clone())),
        transport_kind: Some("lan_quic".to_owned()),
        reason_code: None,
        details: BTreeMap::from([
            ("peer_key_id".to_owned(), peer.peer_key_id.clone()),
            ("key_epoch".to_owned(), peer.key_epoch.to_string()),
            (
                "discovery_endpoint".to_owned(),
                peer.discovery_endpoint.to_string(),
            ),
            ("permission_ceiling".to_owned(), "screen.view".to_owned()),
        ]),
    };
    let registry = state.device_identities.clone();
    let write_peer = peer.clone();
    let result = tokio::task::spawn_blocking(move || {
        let valid_until = write_peer.valid_until;
        let expires_at_ms = write_peer.expires_at_ms;
        registry.approve_lan_peer_with_policy_guarded(
            &write_peer.peer_key_id,
            &write_peer.public_key,
            write_peer.key_epoch,
            &[RemotePermissionScope::ScreenView],
            draft,
            move || {
                if Instant::now() >= valid_until || now_ms() >= expires_at_ms {
                    return Err(StoreError::TrustTransition(
                        "pairing candidate expired before insertion".to_owned(),
                    ));
                }
                late_actor_check()
            },
        )
    })
    .await;
    match result {
        Ok(Ok((record, _audit))) => {
            state
                .lan_discovery
                .first_pairing
                .lock()
                .await
                .remove_peer(&peer.peer_key_id);
            state
                .lan_discovery
                .peers
                .lock()
                .await
                .mark_paired_binding(&peer);
            state.lan_discovery.peer_changed.notify_one();
            let mut device = crate::handlers::identity::project_trust_record_with_policy(
                record,
                vec![RemotePermissionScope::ScreenView],
            );
            device.display_name = Some(peer.device_name);
            device.approved_at_ms = Some(device.updated_at_ms);
            IpcResponse::TrustedDeviceUpdated { device }
        }
        Ok(Err(DeviceIdentityRegistryError::Store(StoreError::TrustTransition(_)))) => {
            deny(
                state,
                &approval,
                "pairing_admission_changed",
                "配对信息或活动桌面已变更，请重新确认",
            )
            .await
        }
        Ok(Err(DeviceIdentityRegistryError::Store(StoreError::Database(error))))
            if error
                .sqlite_error()
                .is_some_and(|error| error.extended_code & 0xff == 19) =>
        {
            deny(
                state,
                &approval,
                "pairing_existing_identity",
                "该密钥已经配对，不能重复覆盖信任",
            )
            .await
        }
        Ok(Err(DeviceIdentityRegistryError::AuthenticatedPeerRequired)) => {
            deny(
                state,
                &approval,
                "pairing_persistent_store_required",
                "首次配对需要正式安装的后台服务",
            )
            .await
        }
        _ => security_error(state),
    }
}

async fn resolve_candidate(
    state: &Arc<AppState>,
    approval: &LanPairingApproval,
) -> anyhow::Result<VerifiedLanPairingPeer> {
    state
        .lan_discovery
        .first_pairing
        .lock()
        .await
        .resolve(approval, now_ms(), Instant::now())
}

async fn deny(
    state: &AppState,
    approval: &LanPairingApproval,
    reason: &str,
    message: &str,
) -> IpcResponse {
    let key = approval.peer_key_id.as_bytes();
    let details = if key.len() == 64
        && key
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        vec![("peer_key_id".to_owned(), approval.peer_key_id.clone())]
    } else {
        Vec::new()
    };
    if state
        .audit_log
        .record(
            "trust.lan_pairing",
            "denied",
            None,
            None,
            None,
            Some("lan_quic".to_owned()),
            Some(reason.to_owned()),
            details,
        )
        .is_err()
    {
        return security_error(state);
    }
    IpcResponse::Error {
        code: "E_LAN_PAIRING_REJECTED".to_owned(),
        message: message.to_owned(),
    }
}

fn security_error(state: &AppState) -> IpcResponse {
    state.mark_security_unhealthy();
    IpcResponse::Error {
        code: "E_SECURITY_STORE_UNAVAILABLE".to_owned(),
        message: "配对安全状态不可用，请检查后台服务".to_owned(),
    }
}
