use super::*;
use mrd_agent_ipc::{
    CancelConsent, ConsentCancelReason, ConsentDecision, ConsentRequest, DesktopKind, PeerBinding,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

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
    let authorization_deadline = registry
        .resolve(&request.session_id, 120)
        .unwrap()
        .expect("installed binding")
        .authorization_deadline;
    assert_eq!(
        registry.resolve(&request.session_id, 120).unwrap(),
        Some(TrustedSessionBinding {
            authority_generation: attempt,
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
            authorization_deadline,
            expected_issuer_key_id: ISSUER_KEY_ID,
        })
    );
    assert_eq!(registry.resolve(&request.session_id, 500).unwrap(), None);
}

#[tokio::test]
async fn monotonic_binding_deadline_wins_wall_clock_rollback() {
    let registry = ConsentAuthorityRegistry::new();
    let mut request = request([122; 16], 122, "monotonic-deadline");
    request.expires_at_ms = 125;
    request.authorization_expires_at_ms = 140;
    let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    assert_eq!(
        registry.resolve(&request.session_id, 121).unwrap(),
        None,
        "elapsed monotonic time must revoke authority even when wall time rolls back",
    );
    assert_eq!(
        registry.take_due(Instant::now(), 121).unwrap(),
        vec![AuthorityInvalidation {
            session_id: request.session_id,
            consent_request_id: request.request_id,
            authority_generation: attempt,
        }],
    );
    assert!(registry.take_due(Instant::now(), 121).unwrap().is_empty());
}

#[tokio::test]
async fn prompt_time_and_wall_rollback_do_not_restart_authority_lifetime() {
    let registry = ConsentAuthorityRegistry::new();
    let mut request = request([133; 16], 134, "anchored-deadline");
    request.expires_at_ms = 260;
    request.authorization_expires_at_ms = 310;
    let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());

    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(111),
        )
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        registry.take_due(Instant::now(), 111).unwrap(),
        vec![AuthorityInvalidation {
            session_id: request.session_id,
            consent_request_id: request.request_id,
            authority_generation: attempt,
        }],
        "local prompt latency must consume, rather than restart, authority lifetime",
    );
}

#[test]
fn due_bindings_return_exact_cleanup_identity_once() {
    let registry = ConsentAuthorityRegistry::new();
    let mut early = request([123; 16], 123, "early-session");
    early.authorization_expires_at_ms = 300;
    let early_attempt = prompt_attempt(registry.begin(early.clone(), context(110)).unwrap());
    registry
        .complete(
            early_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();
    let mut later = request([124; 16], 124, "later-session");
    later.authorization_expires_at_ms = 400;
    let later_attempt = prompt_attempt(registry.begin(later.clone(), context(110)).unwrap());
    registry
        .complete(
            later_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();

    assert_eq!(
        registry.take_due(Instant::now(), 300).unwrap(),
        vec![AuthorityInvalidation {
            session_id: early.session_id.clone(),
            consent_request_id: early.request_id,
            authority_generation: early_attempt,
        }],
    );
    assert!(registry.take_due(Instant::now(), 300).unwrap().is_empty());
    assert!(registry.resolve(&early.session_id, 120).unwrap().is_none());
    assert_eq!(
        registry
            .resolve(&later.session_id, 300)
            .unwrap()
            .expect("later authority remains")
            .authority_generation,
        later_attempt,
    );
}

#[test]
fn resolving_due_authority_does_not_discard_cleanup_identity() {
    let registry = ConsentAuthorityRegistry::new();
    let mut request = request([125; 16], 125, "resolve-due-session");
    request.authorization_expires_at_ms = 300;
    let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();

    assert!(registry
        .resolve(&request.session_id, 300)
        .unwrap()
        .is_none());
    assert_eq!(
        registry.take_due(Instant::now(), 300).unwrap(),
        vec![AuthorityInvalidation {
            session_id: request.session_id,
            consent_request_id: request.request_id,
            authority_generation: attempt,
        }],
        "authorization lookup must not silently lose the release_session identity",
    );
}

#[test]
fn beginning_another_consent_does_not_prune_cleanup_identity() {
    let registry = ConsentAuthorityRegistry::new();
    let mut expired = request([126; 16], 126, "expired-before-begin");
    expired.authorization_expires_at_ms = 300;
    let attempt = prompt_attempt(registry.begin(expired.clone(), context(110)).unwrap());
    registry
        .complete(
            attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();

    let mut unrelated = request([127; 16], 127, "unrelated-pending");
    unrelated.issued_at_ms = 250;
    unrelated.expires_at_ms = 350;
    unrelated.authorization_expires_at_ms = 400;
    registry.begin(unrelated, context(300)).unwrap();

    assert_eq!(
        registry.take_due(Instant::now(), 300).unwrap(),
        vec![AuthorityInvalidation {
            session_id: expired.session_id,
            consent_request_id: expired.request_id,
            authority_generation: attempt,
        }],
    );
}

#[test]
fn stale_deadline_cannot_remove_same_session_replacement() {
    let registry = ConsentAuthorityRegistry::new();
    let mut first = request([128; 16], 128, "deadline-replacement");
    first.authorization_expires_at_ms = 300;
    let first_attempt = prompt_attempt(registry.begin(first.clone(), context(110)).unwrap());
    registry
        .complete(
            first_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();
    let stale_deadline = registry
        .next_authority_deadline()
        .unwrap()
        .expect("first deadline");

    let replacement = request([129; 16], 129, "deadline-replacement");
    let replacement_attempt =
        prompt_attempt(registry.begin(replacement.clone(), context(130)).unwrap());
    registry
        .complete(
            replacement_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(140),
        )
        .unwrap();

    assert!(registry.take_due(stale_deadline, 150).unwrap().is_empty());
    assert_eq!(
        registry
            .resolve(&replacement.session_id, 150)
            .unwrap()
            .expect("replacement remains")
            .authority_generation,
        replacement_attempt,
    );
}

#[test]
fn desktop_mismatch_withdraws_only_nonmatching_bindings() {
    let registry = ConsentAuthorityRegistry::new();
    let matching = request([130; 16], 130, "desktop-matching");
    let matching_attempt = prompt_attempt(registry.begin(matching.clone(), context(110)).unwrap());
    registry
        .complete(
            matching_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();

    let mismatching = request([131; 16], 131, "desktop-mismatching");
    let mut changed_context = context(110);
    changed_context.desktop_epoch = 14;
    let mismatching_attempt = prompt_attempt(
        registry
            .begin(mismatching.clone(), changed_context.clone())
            .unwrap(),
    );
    changed_context.now_ms = 120;
    registry
        .complete(
            mismatching_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            changed_context,
        )
        .unwrap();

    assert_eq!(
        registry
            .take_desktop_mismatch(13, DesktopKind::Default)
            .unwrap(),
        vec![AuthorityInvalidation {
            session_id: mismatching.session_id.clone(),
            consent_request_id: mismatching.request_id,
            authority_generation: mismatching_attempt,
        }],
    );
    assert!(registry
        .resolve(&matching.session_id, 120)
        .unwrap()
        .is_some());
    assert!(registry
        .resolve(&mismatching.session_id, 120)
        .unwrap()
        .is_none());
}

#[test]
fn draining_live_bindings_retains_completed_replay_tombstones() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([132; 16], 132, "drained-session");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(110)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::InputPointer]),
                context(120),
            )
            .unwrap(),
    );

    assert_eq!(
        registry.drain().unwrap(),
        vec![AuthorityInvalidation {
            session_id: original.session_id.clone(),
            consent_request_id: original.request_id,
            authority_generation: attempt,
        }],
    );
    assert!(registry
        .resolve(&original.session_id, 130)
        .unwrap()
        .is_none());

    let mut replay = original;
    replay.request_token = 133;
    let ConsentBeginOutcome::Cached(cached) = registry.begin(replay, context(130)).unwrap() else {
        panic!("drain removed the replay tombstone");
    };
    assert_eq!(cached.decision, completion.result.decision);
    assert_eq!(cached.request_token, 133);
}

#[test]
fn poisoned_registry_lookup_is_not_indistinguishable_from_missing_authority() {
    let registry = ConsentAuthorityRegistry::new();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = registry.state.lock().expect("lock registry");
        panic!("poison registry for fail-closed lookup test");
    }));

    assert_eq!(
        registry.resolve(&SessionId("unknown".to_owned()), 120),
        Err(ConsentRegistryError::Unavailable),
    );
}

#[test]
fn monotonic_deadline_overflow_fails_closed_before_pending_insert() {
    assert_eq!(
        checked_authority_deadline(Instant::now(), u64::MAX),
        Err(ConsentRegistryError::DeadlineOverflow),
    );
}

#[test]
fn far_future_request_is_inactive_before_deadline_construction() {
    let registry = ConsentAuthorityRegistry::new();
    let mut future = request([134; 16], 135, "far-future");
    future.issued_at_ms = 1_000_000;
    future.expires_at_ms = 1_000_100;
    future.authorization_expires_at_ms = 1_000_500;

    assert_eq!(
        registry.begin(future, context(110)),
        Err(ConsentRegistryError::InactiveRequest),
    );
}

#[test]
fn stale_authority_generation_cannot_invalidate_a_newer_binding() {
    let registry = ConsentAuthorityRegistry::new();
    let first = request([120; 16], 120, "generation-session");
    let first_attempt = prompt_attempt(registry.begin(first.clone(), context(110)).unwrap());
    registry
        .complete(
            first_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(120),
        )
        .unwrap();
    let stale = FreshAuthorityChange {
        session_id: first.session_id.clone(),
        consent_request_id: first.request_id,
        authority_generation: first_attempt,
    };

    let replacement = request([121; 16], 121, "generation-session");
    let replacement_attempt =
        prompt_attempt(registry.begin(replacement.clone(), context(130)).unwrap());
    registry
        .complete(
            replacement_attempt,
            ConsentDecision::Approved,
            scopes(&[PermissionScope::InputPointer]),
            context(140),
        )
        .unwrap();

    assert!(!registry.invalidate_fresh_authority(&stale).unwrap());
    let current = registry
        .resolve(&replacement.session_id, 140)
        .unwrap()
        .expect("replacement authority remains installed");
    assert_eq!(current.consent_request_id, replacement.request_id);
    assert_eq!(current.authority_generation, replacement_attempt);
    assert!(registry
        .invalidate_fresh_authority(&FreshAuthorityChange {
            session_id: replacement.session_id.clone(),
            consent_request_id: replacement.request_id,
            authority_generation: replacement_attempt,
        })
        .unwrap());
    assert_eq!(
        registry.resolve(&replacement.session_id, 140).unwrap(),
        None
    );
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
        .unwrap()
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
    assert_eq!(registry.resolve(&session_id, 150).unwrap(), Some(installed));
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
        assert_eq!(registry.resolve(&request.session_id, 120).unwrap(), None);

        let mut retry = request;
        retry.request_token += 100;
        assert!(matches!(
            registry.begin(retry, context(130)),
            Ok(ConsentBeginOutcome::Cached(_))
        ));
    }
}

#[test]
fn nonapproved_decision_with_scopes_is_tombstoned_without_authority() {
    let registry = ConsentAuthorityRegistry::new();
    let request = request([19; 16], 79, "nonapproved-with-scopes");
    let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Denied,
                scopes(&[PermissionScope::ScreenView]),
                context(120),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert!(completion.result.approved_scopes.is_empty());
    assert_eq!(
        completion.disposition,
        ConsentCompletionDisposition::Rejected(
            ConsentCompletionRejection::UnexpectedApprovedScopes
        )
    );
    assert_eq!(registry.resolve(&request.session_id, 120).unwrap(), None);
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
    assert_eq!(registry.resolve(&original.session_id, 120).unwrap(), None);

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
    assert_eq!(registry.resolve(&overflow.session_id, 140).unwrap(), None);
    assert!(registry
        .resolve(&SessionId("active-session-0".to_owned()), 140)
        .unwrap()
        .is_some());
    assert!(registry
        .resolve(&SessionId("active-session-63".to_owned()), 140)
        .unwrap()
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
            .unwrap()
            .consent_request_id,
        replacement.request_id
    );
}

#[test]
fn tombstone_capacity_is_bounded_but_exact_replay_still_hits() {
    assert_eq!(MAX_CONSENT_TOMBSTONES, 4_096);
    let registry = ConsentAuthorityRegistry::new();
    let mut boundary_requests = Vec::new();
    for index in 1..=MAX_CONSENT_TOMBSTONES {
        let request = request(
            (index as u128).to_le_bytes(),
            index as u64,
            "bounded-session",
        );
        let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
        let completion = completed(
            registry
                .complete(
                    attempt,
                    ConsentDecision::Denied,
                    PermissionScopes::new(),
                    context(120),
                )
                .unwrap(),
        );
        if index == 1 || index == MAX_CONSENT_TOMBSTONES {
            boundary_requests.push((request, completion.result));
        }
    }

    let overflow = request(
        ((MAX_CONSENT_TOMBSTONES + 1) as u128).to_le_bytes(),
        5_000,
        "capacity-overflow",
    );
    assert_eq!(
        registry.begin(overflow, context(130)),
        Err(ConsentRegistryError::TombstoneCapacityExceeded)
    );
    for (offset, (mut exact, original_result)) in boundary_requests.into_iter().enumerate() {
        exact.request_token = 6_000 + offset as u64;
        let ConsentBeginOutcome::Cached(cached) =
            registry.begin(exact.clone(), context(130)).unwrap()
        else {
            panic!("boundary replay missed its completed tombstone");
        };
        assert_eq!(cached.request_token, exact.request_token);
        assert_eq!(cached.decided_at_ms, original_result.decided_at_ms);
        assert_eq!(cached.decision, original_result.decision);
    }
}

#[test]
fn expired_pending_attempts_become_tombstones_before_new_capacity_is_reserved() {
    assert_eq!(MAX_PENDING_CONSENTS, 32);
    let registry = ConsentAuthorityRegistry::new();
    let mut pending = Vec::new();
    for index in 1..=MAX_PENDING_CONSENTS {
        let request = request(
            (10_000_u128 + index as u128).to_le_bytes(),
            index as u64,
            &format!("expiring-pending-{index}"),
        );
        let attempt = prompt_attempt(registry.begin(request.clone(), context(110)).unwrap());
        pending.push((request, attempt));
    }

    let mut replacement = request([250; 16], 250, "post-expiry-prompt");
    replacement.issued_at_ms = 200;
    replacement.expires_at_ms = 300;
    replacement.authorization_expires_at_ms = 600;
    let replacement_attempt = prompt_attempt(registry.begin(replacement, context(210)).unwrap());
    assert!(pending
        .iter()
        .all(|(_, expired_attempt)| *expired_attempt != replacement_attempt));
    {
        let state = registry.state.lock().unwrap();
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending_attempts.len(), 1);
        assert_eq!(state.tombstones.len(), MAX_PENDING_CONSENTS);
    }

    for (_, expired_attempt) in &pending {
        assert_eq!(
            registry
                .complete(
                    *expired_attempt,
                    ConsentDecision::Approved,
                    scopes(&[PermissionScope::ScreenView]),
                    context(220),
                )
                .unwrap(),
            ConsentCompletionOutcome::Ignored
        );
    }

    let (mut exact, _) = pending.remove(0);
    exact.request_token = 8_000;
    let ConsentBeginOutcome::Cached(first_cached) =
        registry.begin(exact.clone(), context(230)).unwrap()
    else {
        panic!("expired pending request prompted again");
    };
    assert_eq!(first_cached.decision, ConsentDecision::Expired);
    assert!(first_cached.approved_scopes.is_empty());
    exact.request_token = 8_001;
    let ConsentBeginOutcome::Cached(second_cached) = registry.begin(exact, context(240)).unwrap()
    else {
        panic!("second expired replay prompted again");
    };
    assert_eq!(second_cached.request_token, 8_001);
    assert_eq!(second_cached.decided_at_ms, first_cached.decided_at_ms);
}

#[test]
fn expired_pending_attempt_past_authorization_expiry_releases_its_reservation() {
    let registry = ConsentAuthorityRegistry::new();
    let mut expired = request([249; 16], 249, "fully-expired-pending");
    expired.authorization_expires_at_ms = expired.expires_at_ms;
    let attempt = prompt_attempt(registry.begin(expired.clone(), context(110)).unwrap());

    let mut replacement = request([248; 16], 248, "after-full-expiry");
    replacement.issued_at_ms = 200;
    replacement.expires_at_ms = 300;
    replacement.authorization_expires_at_ms = 600;
    assert!(matches!(
        registry.begin(replacement, context(210)),
        Ok(ConsentBeginOutcome::Prompt(_))
    ));
    assert_eq!(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(220),
            )
            .unwrap(),
        ConsentCompletionOutcome::Ignored
    );
    assert_eq!(
        registry.begin(expired, context(220)),
        Err(ConsentRegistryError::InactiveRequest)
    );
}

#[test]
fn explicit_expiry_cleanup_frees_capacity_without_discarding_owner_identity() {
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
    assert!(registry.resolve(&first.session_id, 499).unwrap().is_some());

    let mut next = request([51; 16], 51, "replacement-session");
    next.issued_at_ms = 500;
    next.expires_at_ms = 600;
    next.authorization_expires_at_ms = 800;
    assert_eq!(
        registry.take_due(Instant::now(), 510).unwrap(),
        vec![AuthorityInvalidation {
            session_id: first.session_id.clone(),
            consent_request_id: first.request_id,
            authority_generation: attempt,
        }],
    );
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
    assert_eq!(registry.resolve(&first.session_id, 520).unwrap(), None);
    assert!(registry.resolve(&next.session_id, 520).unwrap().is_some());
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
    assert_eq!(registry.resolve(&original.session_id, 121).unwrap(), None);

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
fn cancel_after_clock_rollback_is_stable_and_consumes_the_attempt() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([63; 16], 73, "rollback-cancel");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(190)).unwrap());
    let cancel = CancelConsent {
        request_token: original.request_token,
        request_id: original.request_id,
        session_id: original.session_id.clone(),
        reason: ConsentCancelReason::CallerAborted,
    };
    let ConsentCancelOutcome::Cancelled(cancelled) = registry.cancel(&cancel, 50).unwrap() else {
        panic!("exact rollback cancel was ignored");
    };
    assert!(cancelled.decided_at_ms >= original.issued_at_ms);
    assert!(cancelled.decided_at_ms < original.expires_at_ms);
    assert_eq!(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(195),
            )
            .unwrap(),
        ConsentCompletionOutcome::Ignored
    );

    let mut retry = original;
    retry.request_token = 74;
    let ConsentBeginOutcome::Cached(cached) = registry.begin(retry, context(195)).unwrap() else {
        panic!("rollback cancellation prompted again");
    };
    assert_eq!(cached.request_token, 74);
    assert_eq!(cached.decided_at_ms, cancelled.decided_at_ms);
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
    assert_eq!(
        registry.resolve(&local_change.session_id, 120).unwrap(),
        None
    );

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
    assert_eq!(registry.resolve(&expired.session_id, 200).unwrap(), None);

    let secure = request([72; 16], 72, "secure-desktop");
    let mut secure_context = context(110);
    secure_context.desktop_kind = DesktopKind::Secure;
    assert_eq!(
        registry.begin(secure, secure_context),
        Err(ConsentRegistryError::InvalidLocalContext)
    );
}

#[test]
fn completion_clock_rollback_cannot_approve_or_install_authority() {
    let registry = ConsentAuthorityRegistry::new();
    let original = request([73; 16], 75, "rollback-completion");
    let attempt = prompt_attempt(registry.begin(original.clone(), context(190)).unwrap());
    let completion = completed(
        registry
            .complete(
                attempt,
                ConsentDecision::Approved,
                scopes(&[PermissionScope::ScreenView]),
                context(110),
            )
            .unwrap(),
    );
    assert_eq!(completion.result.decision, ConsentDecision::Dismissed);
    assert_eq!(
        completion.disposition,
        ConsentCompletionDisposition::Rejected(ConsentCompletionRejection::InvalidLocalContext)
    );
    assert!(!completion.binding_changed);
    assert_eq!(registry.resolve(&original.session_id, 190).unwrap(), None);

    let mut retry = original;
    retry.request_token = 76;
    assert!(matches!(
        registry.begin(retry, context(195)),
        Ok(ConsentBeginOutcome::Cached(ConsentResult {
            decision: ConsentDecision::Dismissed,
            ..
        }))
    ));
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
    assert_eq!(registry.resolve(&empty.session_id, 120).unwrap(), None);

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
        assert_eq!(registry.resolve(&request.session_id, 120).unwrap(), None);
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
        Err(ConsentRegistryError::Unavailable)
    );
    assert_eq!(
        registry.begin(request([110; 16], 110, "poisoned"), context(110)),
        Err(ConsentRegistryError::Unavailable)
    );
}

struct PendingBackend {
    started: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}

struct ImmediateApprovalBackend;

impl ConsentBackend for ImmediateApprovalBackend {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let scopes = prompt.requested_scopes().clone();
        Box::pin(async move { ConsentBackendDecision::Approved(scopes) })
    }
}

struct FutureDropGuard(Arc<AtomicBool>);

impl Drop for FutureDropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl ConsentBackend for PendingBackend {
    fn is_available(&self) -> bool {
        true
    }

    fn prompt(
        &self,
        _prompt: ConsentPrompt,
        _abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let started = Arc::clone(&self.started);
        let dropped = Arc::clone(&self.dropped);
        Box::pin(async move {
            let _guard = FutureDropGuard(dropped);
            started.notify_one();
            std::future::pending().await
        })
    }
}

fn pending_manager() -> (ConsentManager, Arc<Notify>, Arc<AtomicBool>) {
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let manager = ConsentManager::new(Arc::new(PendingBackend {
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
    }));
    (manager, started, dropped)
}

#[tokio::test]
async fn manager_uses_one_instant_anchor_for_prompt_and_authority_deadlines() {
    let (mut manager, _started, _dropped) = pending_manager();
    let mut request = request([134; 16], 134, "shared-manager-anchor");
    request.expires_at_ms = 500;
    request.authorization_expires_at_ms = 500;
    manager.begin(request.clone(), context(110)).unwrap();

    let prompt_deadline = manager
        .active
        .as_ref()
        .expect("active prompt")
        .prompt
        .deadline;
    let authorization_deadline = manager
        .registry
        .state
        .lock()
        .expect("registry state")
        .pending
        .get(&request.request_id)
        .expect("pending consent")
        .authorization_deadline;
    assert_eq!(
        prompt_deadline, authorization_deadline,
        "one manager begin call must not sample two different monotonic anchors",
    );
}

#[tokio::test]
async fn monotonic_deadline_wins_when_exact_cancel_is_already_ready() {
    let (mut manager, started, _dropped) = pending_manager();
    let request = request([111; 16], 111, "deadline-cancel-race");
    manager.begin(request.clone(), context(110)).unwrap();
    started.notified().await;
    manager
        .active
        .as_mut()
        .expect("active prompt")
        .prompt
        .deadline = Instant::now() - std::time::Duration::from_millis(1);
    let mut abort = manager
        .active
        .as_ref()
        .expect("active prompt")
        .abort
        .subscribe();

    let results = manager
        .cancel(
            &CancelConsent {
                request_token: request.request_token,
                request_id: request.request_id,
                session_id: request.session_id.clone(),
                reason: ConsentCancelReason::CallerAborted,
            },
            Instant::now(),
            110,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].decision, ConsentDecision::Expired);
    abort.changed().await.unwrap();
    assert_eq!(
        *abort.borrow_and_update(),
        Some(ConsentAbortReason::PromptExpired)
    );
    assert_eq!(
        manager.active.as_ref().map(|active| active.phase),
        Some(ActivePromptPhase::Closing)
    );
    manager
        .shutdown(ConsentAbortReason::RuntimeStopping, 110)
        .await
        .unwrap();
}

#[tokio::test]
async fn poisoned_registry_cannot_skip_backend_abort_and_join() {
    let (mut manager, started, dropped) = pending_manager();
    manager
        .begin(request([112; 16], 112, "poison-shutdown"), context(110))
        .unwrap();
    started.notified().await;
    let registry = manager.registry();
    let _ = std::panic::catch_unwind(|| {
        let _guard = registry.state.lock().unwrap();
        panic!("poison manager registry");
    });

    assert_eq!(
        manager
            .shutdown(ConsentAbortReason::RuntimeStopping, 110)
            .await,
        Err(ConsentRegistryError::Unavailable)
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert!(manager.active.is_none());
    assert!(manager.queued.is_empty());
}

#[tokio::test]
async fn manager_owned_registry_installs_only_timely_approval() {
    let mut manager = ConsentManager::new(Arc::new(ImmediateApprovalBackend));
    let request = request([113; 16], 113, "manager-approval");
    manager.begin(request.clone(), context(110)).unwrap();
    let completion = manager.next_completion().await.unwrap();
    let completion = manager.complete(completion, context(120)).unwrap();
    assert_eq!(completion.results.len(), 1);
    assert_eq!(completion.results[0].decision, ConsentDecision::Approved);
    let change = completion
        .fresh_authority_change
        .expect("fresh approval reports its authority generation");
    assert_eq!(change.session_id, request.session_id);
    assert_eq!(change.consent_request_id, request.request_id);
    assert!(manager
        .registry
        .resolve(&request.session_id, 120)
        .unwrap()
        .is_some());
    assert!(matches!(manager.next_deadline(), Ok(Some(_))));
}

#[tokio::test]
async fn fresh_authority_waits_for_runtime_cleanup_before_promoting_next_prompt() {
    let mut manager = ConsentManager::new(Arc::new(ImmediateApprovalBackend));
    manager
        .begin(
            request([122; 16], 122, "cleanup-before-next-a"),
            context(110),
        )
        .unwrap();
    manager
        .begin(
            request([123; 16], 123, "cleanup-before-next-b"),
            context(110),
        )
        .unwrap();
    let backend_completion = manager.next_completion().await.unwrap();

    let completion = manager.complete(backend_completion, context(120)).unwrap();

    assert!(completion.fresh_authority_change.is_some());
    assert!(
        manager.active.is_none(),
        "next prompt must not start before runtime acknowledges targeted cleanup"
    );
    let resumed = manager.resume_after_fresh_authority(context(120)).unwrap();
    assert!(resumed.is_empty());
    assert!(manager.active.is_some());
}

#[tokio::test]
async fn approved_completion_that_loses_cancel_race_cannot_install_binding() {
    let mut manager = ConsentManager::new(Arc::new(ImmediateApprovalBackend));
    let request = request([114; 16], 114, "manager-cancel-late");
    manager.begin(request.clone(), context(110)).unwrap();
    let cancelled = manager
        .cancel(
            &CancelConsent {
                request_token: request.request_token,
                request_id: request.request_id,
                session_id: request.session_id.clone(),
                reason: ConsentCancelReason::CallerAborted,
            },
            Instant::now(),
            110,
        )
        .unwrap();
    assert_eq!(cancelled[0].decision, ConsentDecision::Dismissed);
    let completion = manager.next_completion().await.unwrap();
    assert!(manager
        .complete(completion, context(120))
        .unwrap()
        .results
        .is_empty());
    assert_eq!(
        manager.registry.resolve(&request.session_id, 120).unwrap(),
        None
    );
}

#[tokio::test]
async fn approved_completion_that_loses_deadline_race_cannot_install_binding() {
    let mut manager = ConsentManager::new(Arc::new(ImmediateApprovalBackend));
    let request = request([115; 16], 115, "manager-deadline-late");
    manager.begin(request.clone(), context(110)).unwrap();
    manager
        .active
        .as_mut()
        .expect("active prompt")
        .prompt
        .deadline = Instant::now() - std::time::Duration::from_millis(1);
    let completion = manager.next_completion().await.unwrap();
    let completion = manager.complete(completion, context(110)).unwrap();
    assert_eq!(completion.results.len(), 1);
    assert_eq!(completion.results[0].decision, ConsentDecision::Expired);
    assert!(completion.fresh_authority_change.is_none());
    assert_eq!(
        manager.registry.resolve(&request.session_id, 120).unwrap(),
        None
    );
}
