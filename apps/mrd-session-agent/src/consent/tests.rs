use super::*;
use mrd_agent_ipc::{
    CancelConsent, ConsentCancelReason, ConsentDecision, ConsentRequest, DesktopKind, PeerBinding,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};

const REGISTRATION_ID: [u8; 16] = [7; 16];
const ISSUER_KEY_ID: [u8; 32] = [9; 32];

fn scopes(values: &[PermissionScope]) -> PermissionScopes {
    values.iter().copied().collect()
}

fn request(request_id: [u8; 16], token: u64, session: &str) -> ConsentRequest {
    ConsentRequest {
        request_token: token,
        request_id,
        session_id: SessionId(session.to_owned()),
        peer: PeerBinding {
            device_id: DeviceId("trusted-peer".to_owned()),
            key_id: [5; 32],
        },
        requested_scopes: scopes(&[PermissionScope::ScreenView, PermissionScope::InputPointer]),
        policy_revision: 3,
        windows_session_id: 7,
        issued_at_ms: 100,
        expires_at_ms: 200,
        authorization_expires_at_ms: 500,
    }
}

fn context(now_ms: u64) -> TrustedConsentContext {
    TrustedConsentContext {
        registration_id: REGISTRATION_ID,
        registration_epoch: 11,
        windows_session_id: 7,
        desktop_epoch: 13,
        desktop_kind: DesktopKind::Default,
        expected_issuer_key_id: ISSUER_KEY_ID,
        now_ms,
    }
}

fn prompt_attempt(outcome: ConsentBeginOutcome) -> u64 {
    match outcome {
        ConsentBeginOutcome::Prompt(prompt) => prompt.attempt_id,
        other => panic!("expected prompt, got {other:?}"),
    }
}

fn completed(outcome: ConsentCompletionOutcome) -> ConsentCompletion {
    match outcome {
        ConsentCompletionOutcome::Completed(completed) => completed,
        ConsentCompletionOutcome::Ignored => panic!("completion was ignored"),
    }
}

#[test]
fn approved_completion_installs_the_exact_local_binding() {
    let registry = ConsentAuthorityRegistry::new();
    let request = request([1; 16], 41, "session-a");
    let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());

    let completed = registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();
    let ConsentCompletionOutcome::Completed(completed) = completed else {
        panic!("completion was ignored");
    };

    assert_eq!(completed.result.request_token, 41);
    assert_eq!(completed.result.decided_at_ms, 120);
    assert_eq!(completed.result.decision, ConsentDecision::Approved);
    assert!(completed.binding_changed);
    assert_eq!(
        completed.disposition,
        ConsentCompletionDisposition::Approved
    );
    assert_eq!(
        registry.resolve(&request.session_id, 120),
        Some(TrustedSessionBinding {
            consent_request_id: request.request_id,
            registration_id: REGISTRATION_ID,
            registration_epoch: 11,
            session_id: request.session_id.clone(),
            peer: request.peer.clone(),
            approved_scopes: scopes(&[PermissionScope::InputPointer]),
            policy_revision: request.policy_revision,
            windows_session_id: 7,
            desktop_epoch: 13,
            desktop_kind: DesktopKind::Default,
            authorization_expires_at_ms: 500,
            expected_issuer_key_id: ISSUER_KEY_ID,
        })
    );
    assert_eq!(registry.resolve(&request.session_id, 500), None);
}

#[test]
fn completed_replay_is_cached_without_a_second_prompt() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([2; 16], 51, "session-replay");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    let first = registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::ScreenView]),
            context(121),
        )
        .unwrap();
    let ConsentCompletionOutcome::Completed(first) = first else {
        panic!("completion was ignored");
    };

    let installed = registry
        .resolve(&original.session_id, 121)
        .expect("approval installs one binding");
    let session_id = original.session_id.clone();
    let mut retry = original;
    retry.request_token = 52;
    let ConsentBeginOutcome::Cached(cached) = registry.begin(retry, context(150)).unwrap() else {
        panic!("completed retry prompted again");
    };
    let mut expected = first.result;
    expected.request_token = 52;
    assert_eq!(cached, expected);
    assert_eq!(cached.decided_at_ms, 121);
    assert_eq!(registry.resolve(&session_id, 150), Some(installed));
}

#[test]
fn same_request_id_with_different_semantics_is_a_replay_conflict() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([3; 16], 61, "session-conflict");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Dismissed,
            PermissionScopes::new(),
            context(120),
        )
        .unwrap();

    let mut conflict = original;
    conflict.request_token = 62;
    conflict.policy_revision += 1;
    assert_eq!(
        registry.begin(conflict, context(130)),
        Err(ConsentRegistryError::ConsentReplayConflict)
    );
}

#[test]
fn nonapproved_decisions_never_install_authority() {
    for (index, decision) in [
        ConsentDecision::Denied,
        ConsentDecision::Dismissed,
        ConsentDecision::Expired,
    ]
    .into_iter()
    .enumerate()
    {
        let registry = ConsentAuthorityRegistry::new();
        let request = request([(index + 10) as u8; 16], 70 + index as u64, "session-no");
        let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
        let completion = completed(
            registry
                .complete(attempt, decision, PermissionScopes::new(), context(120))
                .unwrap(),
        );
        assert_eq!(completion.result.decision, decision);
        assert!(!completion.binding_changed);
        assert_eq!(
            completion.disposition,
            ConsentCompletionDisposition::NonApproved
        );
        assert_eq!(registry.resolve(&request.session_id, 120), None);

        let mut retry = request;
        retry.request_token += 100;
        assert!(matches!(
            registry.begin(retry, context(130)),
            Ok(ConsentBeginOutcome::Cached(_))
        ));
    }
}

#[test]
fn scope_escalation_is_tombstoned_as_nonapproved() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([20; 16], 80, "session-scope");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::InputKeyboard]),
                context(120),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert!(completion.result.approved_scopes.is_empty());
    assert_eq!(
        completion.disposition,
        ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::ScopeEscalation)
    );
    assert_eq!(registry.resolve(&original.session_id, 120), None);

    let mut retry = original;
    retry.request_token = 81;
    let ConsentBeginOutcome::Cached(cached) = registry.begin(retry, context(130)).unwrap() else {
        panic!("scope escalation left a re-prompt hole");
    };
    assert_eq!(cached.decision, ConsentDecision::Dismissed);
    assert!(cached.approved_scopes.is_empty());
}

#[test]
fn every_authorization_semantic_participates_in_the_replay_fingerprint() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([21; 16], 90, "session-fingerprint");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Denied,
            PermissionScopes::new(),
            context(120),
        )
        .unwrap();

    let mut conflicts = Vec::new();
    let mut changed = original.clone();
    changed.session_id = SessionId("different-session".to_owned());
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.peer.device_id = DeviceId("different-peer".to_owned());
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.requested_scopes = scopes(&[PermissionScope::ScreenView]);
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.policy_revision += 1;
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.windows_session_id += 1;
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.issued_at_ms += 1;
    conflicts.push(changed);
    let mut changed = original.clone();
    changed.expires_at_ms += 1;
    conflicts.push(changed);
    let mut changed = original;
    changed.authorization_expires_at_ms += 1;
    conflicts.push(changed);

    for mut conflict in conflicts {
        conflict.request_token += 1;
        assert_eq!(
            registry.begin(conflict, context(130)),
            Err(ConsentRegistryError::ConsentReplayConflict)
        );
    }
}

#[test]
fn live_binding_capacity_never_evicts_existing_authority_or_leaks_approval() {
    let registry = ConsentAuthorityRegistry::new();
    for index in 0..MAX_ACTIVE_BINDINGS {
        let id = 30 + index as u8;
        let request = request([id; 16], u64::from(id), &format!("active-session-{index}"));
        let attempt = prompt_attempt(registry.begin(request, context(110)).unwrap());
        let completion = completed(
            registry
                .complete(
                    attempt,
                    ConsentDecision::Approved,
                    scopes(&[PermissionScope::ScreenView]),
                    context(120),
                )
                .unwrap(),
        );
        assert_eq!(completion.result.decision, ConsentDecision::Approved);
    }

    let overflow = request([100; 16], 100, "overflow-session");
    let attempt = prompt_attempt(registry.begin(overflow.clone(), context(130)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(140),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert!(!completion.binding_changed);
    assert_eq!(
        completion.disposition,
        ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::BindingCapacityExceeded)
    );
    assert_eq!(registry.resolve(&overflow.session_id, 140), None);
    assert!(registry
        .resolve(&SessionId("active-session-0".to_owned()), 140)
        .is_some());
    assert!(registry
        .resolve(&SessionId("active-session-63".to_owned()), 140)
        .is_some());

    let replacement = request([101; 16], 101, "active-session-0");
    let attempt = prompt_attempt(registry.begin(replacement.clone(), context(150)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::InputPointer]),
                context(160),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Approved);
    assert!(completion.binding_changed);
    assert_eq!(
        registry
            .resolve(&replacement.session_id, 160)
            .unwrap()
            .consent_request_id,
        replacement.request_id
    );
}

#[test]
fn tombstone_capacity_is_bounded_but_exact_replay_still_hits() {
    assert_eq!(MAX_CONSENT_TOMBSTONES, 4_096);
    assert_eq!(MAX_PENDING_CONSENTS, 32);
    assert_eq!(MAX_ACTIVE_BINDINGS, 64);
    let registry = ConsentAuthorityRegistry::with_limits(2, 2, 2);
    let mut completed_requests = Vec::new();
    for id in [40, 41] {
        let request = request([id; 16], u64::from(id), "bounded-session");
        let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
        registry
            .complete(
                attempt,
                ConsentDecision::Denied,
                PermissionScopes::new(),
                context(120),
            )
            .unwrap();
        completed_requests.push(request);
    }

    assert_eq!(
        registry.begin(request([42; 16], 42, "new-session"), context(130)),
        Err(ConsentRegistryError::TombstoneCapacityExceeded)
    );
    let mut exact = completed_requests.remove(0);
    exact.request_token = 140;
    assert!(matches!(
        registry.begin(exact, context(130)),
        Ok(ConsentBeginOutcome::Cached(_))
    ));
}

#[test]
fn expired_authority_and_tombstones_are_pruned_before_capacity_checks() {
    let registry = ConsentAuthorityRegistry::with_limits(1, 1, 1);
    let first = request([50; 16], 50, "expired-session");
    let attempt = prompt_attempt(registry.begin(first.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::ScreenView]),
            context(120),
        )
        .unwrap();
    assert!(registry.resolve(&first.session_id, 499).is_some());

    let mut next = request([51; 16], 51, "replacement-session");
    next.issued_at_ms = 500;
    next.expires_at_ms = 600;
    next.authorization_expires_at_ms = 800;
    let attempt = prompt_attempt(registry.begin(next.clone(), context(510)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(520),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Approved);
    assert_eq!(registry.resolve(&first.session_id, 520), None);
    assert!(registry.resolve(&next.session_id, 520).is_some());
}

#[test]
fn exact_cancel_is_tombstoned_and_the_late_attempt_is_ignored() {
    let registry = ConsentAuthorityRegistry::with_limits(2, 1, 2);
    let original = request([60; 16], 60, "cancelled-session");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    let cancel = CancelConsent {
        request_token: original.request_token,
        request_id: original.request_id,
        session_id: original.session_id.clone(),
        reason: ConsentCancelReason::CallerAborted,
    };
    let ConsentCancelOutcome::Cancelled(cancelled) = registry.cancel(&cancel, 120).unwrap() else {
        panic!("exact cancel was ignored");
    };
    assert_eq!(cancelled.decision, ConsentDecision::Dismissed);
    assert_eq!(cancelled.decided_at_ms, 120);
    assert_eq!(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(121),
            )
            .unwrap(),
        ConsentCompletionOutcome::Ignored
    );
    assert_eq!(registry.resolve(&original.session_id, 121), None);

    let mut retry = original;
    retry.request_token = 61;
    let ConsentBeginOutcome::Cached(cached) = registry.begin(retry, context(130)).unwrap() else {
        panic!("cancelled request was prompted again");
    };
    assert_eq!(cached.request_token, 61);
    assert_eq!(cached.decided_at_ms, 120);
    assert_eq!(cached.decision, ConsentDecision::Dismissed);

    let next = request([61; 16], 62, "next-session");
    let next_attempt = prompt_attempt(registry.begin(next, context(130)).unwrap());
    assert_ne!(next_attempt, attempt);
}

#[test]
fn inexact_cancel_does_not_consume_the_pending_attempt() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([62; 16], 70, "exact-cancel");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    let wrong = CancelConsent {
        request_token: original.request_token + 1,
        request_id: original.request_id,
        session_id: original.session_id.clone(),
        reason: ConsentCancelReason::TimedOut,
    };
    assert_eq!(
        registry.cancel(&wrong, 120).unwrap(),
        ConsentCancelOutcome::Ignored
    );
    assert!(matches!(
        registry
            .complete(
                attempt,
                ConsentDecision::Denied,
                PermissionScopes::new(),
                context(121),
            )
            .unwrap(),
        ConsentCompletionOutcome::Completed(_)
    ));
}

#[test]
fn changed_local_generation_and_prompt_expiry_cannot_install_authority() {
    let registry = ConsentAuthorityRegistry::new();
    let local_change = request([70; 16], 70, "local-change");
    let attempt = prompt_attempt(registry.begin(local_change.clone(), context(110)).unwrap());
    let mut changed = context(120);
    changed.desktop_epoch += 1;
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                changed,
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert_eq!(registry.resolve(&local_change.session_id, 120), None);

    let expired = request([71; 16], 71, "prompt-expired");
    let attempt = prompt_attempt(registry.begin(expired.clone(), context(110)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(200),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Expired);
    assert_eq!(registry.resolve(&expired.session_id, 200), None);

    let secure = request([72; 16], 72, "secure-desktop");
    let mut secure_context = context(110);
    secure_context.desktop_kind = DesktopKind::Secure;
    assert_eq!(
        registry.begin(secure, secure_context),
        Err(ConsentRegistryError::InvalidLocalContext)
    );
}

#[test]
fn pending_capacity_is_bounded_and_attempt_ids_are_never_reused() {
    let registry = ConsentAuthorityRegistry::new();
    let mut attempts = Vec::new();
    for index in 0..MAX_PENDING_CONSENTS {
        let id = 120 + index as u8;
        attempts.push(prompt_attempt(
            registry
                .begin(
                    request([id; 16], u64::from(id), &format!("pending-{index}")),
                    context(100),
                )
                .unwrap(),
        ));
    }
    assert!(attempts.iter().all(|attempt| *attempt != 0));
    let mut unique = attempts.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), MAX_PENDING_CONSENTS);
    assert_eq!(
        registry.begin(request([200; 16], 200, "pending-overflow"), context(110)),
        Err(ConsentRegistryError::PendingCapacityExceeded)
    );

    registry
        .complete(
            attempts[0],
            ConsentDecision::Denied,
            PermissionScopes::new(),
            context(120),
        )
        .unwrap();
    let next_attempt = prompt_attempt(
        registry
            .begin(request([200; 16], 200, "pending-overflow"), context(130))
            .unwrap(),
    );
    assert!(!attempts.contains(&next_attempt));
}

#[test]
fn empty_approval_and_each_changed_local_identity_fail_closed() {
    let registry = ConsentAuthorityRegistry::new();
    let empty = request([90; 16], 90, "empty-approval");
    let attempt = prompt_attempt(registry.begin(empty.clone(), context(110)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                PermissionScopes::new(),
                context(120),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert_eq!(registry.resolve(&empty.session_id, 120), None);

    for (index, mutate) in [
        |context: &mut TrustedConsentContext| context.registration_id[0] ^= 1,
        |context: &mut TrustedConsentContext| context.registration_epoch += 1,
        |context: &mut TrustedConsentContext| context.windows_session_id += 1,
        |context: &mut TrustedConsentContext| context.desktop_epoch += 1,
        |context: &mut TrustedConsentContext| context.desktop_kind = DesktopKind::Secure,
        |context: &mut TrustedConsentContext| context.expected_issuer_key_id[0] ^= 1,
    ]
    .into_iter()
    .enumerate()
    {
        let request = request(
            [(index + 91) as u8; 16],
            91 + index as u64,
            &format!("local-mismatch-{index}"),
        );
        let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
        let mut changed = context(120);
        mutate(&mut changed);
        let completion = completed(
            registry
                .complete(
                    attempt,
                    ConsentDecision::Approved,
                    scopes(&[PermissionScope::ScreenView]),
                    changed,
                )
                .unwrap(),
        );
        assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
        assert!(!completion.binding_changed);
        assert_eq!(registry.resolve(&request.session_id, 120), None);
    }
}

#[test]
fn poisoned_registry_never_returns_or_installs_authority() {
    let registry = ConsentAuthorityRegistry::new();
    let _ = std::panic::catch_unwind(|| {
        let _guard = registry.state.lock().unwrap();
        panic!("poison consent registry for fail-closed contract");
    });

    assert_eq!(
        registry.resolve(&SessionId("any-session".to_owned()), 110),
        None
    );
    assert_eq!(
        registry.begin(request([110; 16], 110, "poisoned"), context(110)),
        Err(ConsentRegistryError::Unavailable)
    );
}
