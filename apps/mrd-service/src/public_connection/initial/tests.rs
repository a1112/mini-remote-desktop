use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

fn configured() -> (Arc<AppState>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let state = Arc::new(AppState::new());
    state
        .public_connection
        .configure_persistence(
            dir.path().to_path_buf(),
            Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([97; 32]).unwrap()),
            state.device_identities.machine_key_id().unwrap(),
        )
        .unwrap();
    (state, dir)
}
fn response(code: &str) -> DeviceRegistrationResponse {
    DeviceRegistrationResponse {
        device_id: code.into(),
        device_name: "Guest PC".into(),
        access_token: "new.access.token".into(),
        refresh_token: Some("new.refresh.token".into()),
    }
}
#[tokio::test]
async fn first_registration_uses_the_selected_machine_key_and_saves_without_an_account() {
    let (state, dir) = configured();
    let key = state.device_identities.machine_key_id().unwrap().to_owned();
    let expected_serial = state
        .public_connection
        .machine_serial
        .read()
        .unwrap()
        .clone();
    assert!(ensure_with(
        &state,
        DEFAULT_PUBLIC_API_URL,
        move |payload, identity| async move {
            assert_eq!(identity.key_id(), key);
            assert_eq!(payload.motherboard_serial, expected_serial);
            Ok(response("0123456789"))
        }
    )
    .await
    .unwrap());
    let saved = state
        .public_connection
        .registration()
        .expect("a first-time machine code must be persisted");
    assert_eq!(saved.device_id, "0123456789");
    assert_eq!(saved.refresh_token.as_deref(), Some("new.refresh.token"));
    let encrypted = std::fs::read(dir.path().join("public-device-v1.protected")).unwrap();
    assert!(!encrypted
        .windows(b"new.refresh.token".len())
        .any(|v| v == b"new.refresh.token"));
    assert_eq!(
        state.device_identities.machine_key_id(),
        Some(key_for(&state).as_str())
    );
}
fn key_for(state: &AppState) -> String {
    state
        .device_identities
        .machine_identity()
        .key_id()
        .to_owned()
}
#[tokio::test]
async fn signed_self_registration_recovers_same_key_nine_digit_code_without_local_registration() {
    let (state, _dir) = configured();
    let original_key = key_for(&state);
    assert!(ensure_with(&state, DEFAULT_PUBLIC_API_URL, |_, _| async {
        Ok(response("753662296"))
    })
    .await
    .unwrap());
    assert_eq!(
        state.public_connection.registration().unwrap().device_id,
        "753662296"
    );
    assert_eq!(key_for(&state), original_key);
}
#[tokio::test]
async fn existing_identity_never_calls_anonymous_registration_or_changes_protected_bytes() {
    let (state, dir) = configured();
    let original_key = key_for(&state);
    state
        .public_connection
        .save(Registration {
            device_id: "917753264".into(),
            device_name: "Existing Mac".into(),
            access_token: "old.access.token".into(),
            refresh_token: Some("old.refresh.token".into()),
            api_url: DEFAULT_PUBLIC_API_URL.into(),
            machine_serial: state
                .public_connection
                .machine_serial
                .read()
                .unwrap()
                .clone(),
        })
        .unwrap();
    let path = dir.path().join("public-device-v1.protected");
    let before = std::fs::read(&path).unwrap();
    let invoked = Arc::new(AtomicBool::new(false));
    let observed = invoked.clone();
    assert!(
        !ensure_with(&state, DEFAULT_PUBLIC_API_URL, move |_, _| async move {
            observed.store(true, Ordering::SeqCst);
            Ok(response("9999999999"))
        })
        .await
        .unwrap()
    );
    assert!(!invoked.load(Ordering::SeqCst));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let saved = state.public_connection.registration().unwrap();
    assert_eq!(saved.device_id, "917753264");
    assert_eq!(saved.access_token, "old.access.token");
    assert_eq!(saved.refresh_token.as_deref(), Some("old.refresh.token"));
    assert_eq!(key_for(&state), original_key);
}
#[tokio::test]
async fn first_registration_requires_protected_storage_before_any_network_call() {
    let state = Arc::new(AppState::new());
    let invoked = Arc::new(AtomicBool::new(false));
    let observed = invoked.clone();
    assert_eq!(
        ensure_with(&state, DEFAULT_PUBLIC_API_URL, move |_, _| async move {
            observed.store(true, Ordering::SeqCst);
            Ok(response("0123456789"))
        })
        .await
        .unwrap_err(),
        "public_self_enrollment_storage_unavailable"
    );
    assert!(!invoked.load(Ordering::SeqCst));
    assert!(state.public_connection.registration().is_none());
}
#[tokio::test]
async fn cancelling_initial_registration_releases_operation_and_saves_nothing() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let (state, _dir) = configured();
    let dropped = Arc::new(AtomicBool::new(false));
    let observed = dropped.clone();
    let outcome = tokio::time::timeout(
        Duration::from_millis(10),
        ensure_with(&state, DEFAULT_PUBLIC_API_URL, move |_, _| async move {
            let _guard = Dropped(observed);
            std::future::pending::<Result<DeviceRegistrationResponse, &'static str>>().await
        }),
    )
    .await;
    assert!(
        outcome.is_err(),
        "a stalled HTTP future must be cancellable"
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert!(state.public_connection.operation.try_lock().is_ok());
    assert!(state.public_connection.registration().is_none());
}
#[test]
fn failed_registration_notifications_cannot_skip_backoff_and_rate_limits_wait_at_least_one_minute()
{
    let start = tokio::time::Instant::now();
    let mut retry = EnrollmentRetry::default();
    assert!(retry.can_attempt(start));
    retry.failed(start, "public_self_enrollment_rate_limited");
    assert!(!retry.can_attempt(start + Duration::from_secs(59)));
    assert!(retry.can_attempt(start + Duration::from_secs(60)));
    for _ in 0..100 {
        retry.failed(start, "public_self_enrollment_transport");
    }
    assert!(!retry.can_attempt(start + Duration::from_secs(299)));
    assert!(retry.can_attempt(start + Duration::from_secs(300)));
    retry.succeeded();
    assert!(retry.can_attempt(start));
}
#[tokio::test]
async fn otp_application_still_refuses_new_nine_digit_identity() {
    let (state, _dir) = configured();
    assert!(
        apply_registration(&state, DEFAULT_PUBLIC_API_URL, response("753662296"))
            .await
            .is_err()
    );
    assert!(state.public_connection.registration().is_none());
}
