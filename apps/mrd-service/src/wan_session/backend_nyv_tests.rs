use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signer as _, SigningKey};
use mrd_relay_control::{
    RelayDirectoryCandidate, RelayDirectoryEndpoint, RelayDirectoryPayload,
    RelayDirectoryTransport, RelayReservation, SignedRelayDirectory,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const NOW: u64 = 1_777_000_000_000;
const KEY_ID: &str = "nyv-test-directory";

struct Fixture {
    context: RelayAccessContext,
    keys: BTreeMap<String, Vec<u8>>,
    body: Vec<u8>,
}

fn fixture(issued_at_ms: u64, expires_at_ms: u64) -> Fixture {
    let key = SigningKey::from_bytes(&[0x31; 32]);
    let context = RelayAccessContext::for_generation("nyv-session", 29, "target-1", 0).unwrap();
    let urls = vec!["turn:relay.example.test:3478?transport=udp".to_owned()];
    let payload = RelayDirectoryPayload {
        format_version: 1,
        policy_revision: 29,
        directory_id: "nyv-directory".into(),
        issued_at_ms,
        expires_at_ms,
        session_id: "nyv-session".into(),
        intended_peer_digest: crate::relay::relay_peer_digest("target-1").unwrap(),
        candidates: vec![RelayDirectoryCandidate {
            node_id: "relay-a".into(),
            region: "test-region".into(),
            failure_domain: "test-host".into(),
            endpoints: vec![RelayDirectoryEndpoint {
                transport: RelayDirectoryTransport::Udp,
                host: "relay.example.test".into(),
                port: 3478,
            }],
            capabilities: 1,
            load_class: 1,
            selection_reason: "preferred-region".into(),
            reservation: RelayReservation {
                reservation_id: "nyv-reservation".into(),
                expires_at_ms,
            },
        }],
    };
    let signature = key.sign(&payload.canonical_signing_bytes().unwrap());
    let directory = SignedRelayDirectory {
        payload,
        signing_key_id: KEY_ID.into(),
        signature_b64: STANDARD.encode(signature.to_bytes()),
    };
    let body = serde_json::to_vec(&json!({
        "generation": 0,
        "directory_id": "nyv-directory",
        "relay_url_digest": hex_digest(&crate::relay::urls_digest(&urls)),
        "directory": directory,
        "credentials": [{
            "node_id": "relay-a", "urls": urls,
            "username": "synthetic-user", "credential": "synthetic-credential",
            "expires_at_unix_seconds": expires_at_ms / 1_000 + 60,
        }]
    }))
    .unwrap();
    Fixture {
        context,
        keys: BTreeMap::from([(KEY_ID.into(), key.verifying_key().to_bytes().to_vec())]),
        body,
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().try_into().unwrap()
}

fn alter(f: &mut Fixture, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut body = serde_json::from_slice(&f.body).unwrap();
    edit(&mut body);
    f.body = serde_json::to_vec(&body).unwrap();
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_uses_the_same_signed_bytes_when_local_time_becomes_current() {
    let f = fixture(NOW + 425, NOW + 30_000);
    let original = f.body.clone();
    let start = Instant::now();
    let verified = verify_access_response_before(
        &f.context,
        &f.keys,
        &f.body,
        || NOW + elapsed_ms(start),
        start + Duration::from_secs(5),
    )
    .await
    .expect("a strictly current, fully verified response should be accepted");
    assert_eq!(elapsed_ms(start), 425);
    assert_eq!(verified.directory().payload().issued_at_ms, NOW + 425);
    assert_eq!(verified.generation(), Some(0));
    assert_eq!(f.body, original);
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_persistent_future_is_bounded_by_two_seconds() {
    let f = fixture(NOW + 30_000, NOW + 60_000);
    let start = Instant::now();
    let error = verify_access_response_before(
        &f.context,
        &f.keys,
        &f.body,
        || NOW + elapsed_ms(start),
        start + Duration::from_secs(5),
    )
    .await
    .unwrap_err();
    assert_eq!(error, WanSessionBackendError::DirectoryNotYetValid);
    assert_eq!(elapsed_ms(start), 2_000);
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_operation_deadline_is_original_and_strict() {
    for budget_ms in [0, 100] {
        let f = fixture(
            if budget_ms == 0 { NOW - 1 } else { NOW + 425 },
            NOW + 30_000,
        );
        let start = Instant::now();
        let error = verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || NOW + elapsed_ms(start),
            start + Duration::from_millis(budget_ms),
        )
        .await
        .unwrap_err();
        assert_eq!(error, WanSessionBackendError::DeadlineExceeded);
        assert_eq!(elapsed_ms(start), budget_ms);
    }
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_the_original_setup_fence_cancels_the_same_future() {
    let f = fixture(NOW + 425, NOW + 30_000);
    let start = Instant::now();
    let result = tokio::time::timeout_at(
        start + Duration::from_millis(75),
        verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || NOW + elapsed_ms(start),
            start + Duration::from_secs(5),
        ),
    )
    .await;
    assert!(
        result.is_err(),
        "setup must cancel the waiting operation, not renew it"
    );
    assert_eq!(elapsed_ms(start), 75);
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_future_signature_and_altered_body_are_never_authority() {
    for case in [
        "signature",
        "signed_payload",
        "credential",
        "peer",
        "policy",
    ] {
        let mut f = fixture(NOW + 200, NOW + 30_000);
        alter(&mut f, |body| match case {
            "signature" => {
                let mut signature = STANDARD
                    .decode(body["directory"]["signature_b64"].as_str().unwrap())
                    .unwrap();
                signature[0] ^= 1;
                body["directory"]["signature_b64"] = json!(STANDARD.encode(signature));
            }
            "signed_payload" => {
                body["directory"]["payload"]["candidates"][0]["endpoints"][0]["port"] = json!(3479)
            }
            "credential" => {
                body["credentials"][0]["urls"] =
                    json!(["turn:foreign.example.test:3478?transport=udp"])
            }
            "peer" => body["directory"]["payload"]["intended_peer_digest"] = json!("foreign-peer"),
            "policy" => body["directory"]["payload"]["policy_revision"] = json!(30),
            _ => unreachable!(),
        });
        let start = Instant::now();
        let error = verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || NOW + elapsed_ms(start),
            start + Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(error, WanSessionBackendError::BindingMismatch, "{case}");
        assert_eq!(elapsed_ms(start), 200, "{case}");
    }
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_non_nyv_failures_do_not_wait() {
    for case in ["json", "expired", "credential"] {
        let mut f = fixture(
            NOW - 1_000,
            if case == "expired" { NOW } else { NOW + 30_000 },
        );
        if case == "json" {
            f.body = b"malformed-private-body".to_vec();
        }
        if case == "credential" {
            alter(&mut f, |body| {
                body["credentials"][0]["expires_at_unix_seconds"] = json!(NOW / 1000)
            });
        }
        let start = Instant::now();
        let error = verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || NOW,
            start + Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            if case == "json" {
                WanSessionBackendError::InvalidResponse
            } else {
                WanSessionBackendError::BindingMismatch
            },
            "{case}"
        );
        assert_eq!(elapsed_ms(start), 0);
        assert!(!format!("{error:?} {error}").contains("private-body"));
    }
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_clock_jump_to_actual_expiry_is_rejected() {
    let f = fixture(NOW + 425, NOW + 30_000);
    let wall = Arc::new(AtomicU64::new(NOW));
    let wall_for_task = wall.clone();
    let start = Instant::now();
    let task = tokio::spawn(async move {
        verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || wall_for_task.load(Ordering::Acquire),
            start + Duration::from_secs(5),
        )
        .await
    });
    tokio::task::yield_now().await;
    wall.store(NOW + 30_000, Ordering::Release);
    tokio::time::advance(Duration::from_millis(25)).await;
    assert_eq!(
        task.await.unwrap().unwrap_err(),
        WanSessionBackendError::BindingMismatch
    );
    assert_eq!(elapsed_ms(start), 25);
}

#[tokio::test(start_paused = true)]
async fn nyv_reverify_cancellation_has_no_detached_retry() {
    let f = fixture(NOW + 30_000, NOW + 60_000);
    let calls = Arc::new(AtomicU64::new(0));
    let calls_for_task = calls.clone();
    let start = Instant::now();
    let task = tokio::spawn(async move {
        verify_access_response_before(
            &f.context,
            &f.keys,
            &f.body,
            || {
                calls_for_task.fetch_add(1, Ordering::AcqRel);
                NOW + elapsed_ms(start)
            },
            start + Duration::from_secs(5),
        )
        .await
    });
    tokio::task::yield_now().await;
    assert!(
        !task.is_finished(),
        "the response is awaiting strict local validity"
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let stopped_at = calls.load(Ordering::Acquire);
    tokio::time::advance(Duration::from_secs(10)).await;
    tokio::task::yield_now().await;
    assert_eq!(calls.load(Ordering::Acquire), stopped_at);
}
