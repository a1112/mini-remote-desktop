//! Automatic registration and credential recovery, serialized with manual changes.

use super::{apply_registration, machine_payload, AppState, Registration};
use mrd_device_registration::auto_enrollment::{self as self_enrollment, SelfEnrollmentError};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnrollmentFailure {
    Protocol(SelfEnrollmentError),
    ProtectedStorage,
}

impl EnrollmentFailure {
    pub(super) fn status_code(self) -> &'static str {
        match self {
            Self::Protocol(error) => error.status_code(),
            Self::ProtectedStorage => "public_credential_save_failed",
        }
    }
}

impl From<SelfEnrollmentError> for EnrollmentFailure {
    fn from(value: SelfEnrollmentError) -> Self {
        Self::Protocol(value)
    }
}

/// The caller owns `PublicConnectionState.operation` across this future.
pub(super) async fn enroll_missing(
    state: &Arc<AppState>,
    api_url: &str,
) -> Result<(), EnrollmentFailure> {
    if state.public_connection.registration().is_some() {
        return Ok(());
    }
    let identity = state.device_identities.machine_identity();
    // Prove only the machine identity whose OS-protected persistence is active.
    // A transient/debug identity must never claim a production device code.
    {
        let persistence = state.public_connection.persistence.read().unwrap();
        if persistence
            .as_ref()
            .is_none_or(|config| config.machine_key_id != identity.key_id())
        {
            return Err(EnrollmentFailure::ProtectedStorage);
        }
    }
    let name = sysinfo::System::host_name()
        .filter(|value| {
            !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        })
        .unwrap_or_else(|| "Rdesk Device".into());
    let payload = machine_payload(state, name);
    let registration = self_enrollment::self_register(
        api_url,
        &payload,
        identity.key_id(),
        identity.public_key(),
        |bytes| {
            identity
                .sign_context_bytes(self_enrollment::SIGNATURE_CONTEXT, bytes)
                .map_err(|_| SelfEnrollmentError::SigningFailed)
        },
    )
    .await?;
    apply_registration(state, api_url, registration)
        .await
        .map_err(|_| EnrollmentFailure::ProtectedStorage)
}

/// A network or protocol failure cannot select the identity recovery route.
pub(super) fn needs_existing_recovery(
    renewal: &Result<mrd_device_registration::DeviceRegistrationResponse, &'static str>,
) -> bool {
    matches!(
        renewal,
        Err(mrd_device_registration::DEVICE_CREDENTIAL_REJECTED)
    )
}

/// The caller owns `PublicConnectionState.operation` across this future.
/// Recovery retains the local assignment on every failure and proves only the
/// key selected by the configured protected credential store.
pub(super) async fn recover_existing(
    state: &Arc<AppState>,
    saved: &Registration,
) -> Result<(), EnrollmentFailure> {
    let identity = state.device_identities.machine_identity();
    {
        let persistence = state.public_connection.persistence.read().unwrap();
        let current = state.public_connection.registration();
        let api_url = state.public_connection.api_url.read().unwrap();
        let machine_serial = state.public_connection.machine_serial.read().unwrap();
        if persistence
            .as_ref()
            .is_none_or(|config| config.machine_key_id != identity.key_id())
            || current.as_ref().is_none_or(|current| {
                current.device_id != saved.device_id
                    || current.api_url != saved.api_url
                    || current.machine_serial != saved.machine_serial
            })
            || saved.api_url.trim_end_matches('/') != api_url.trim_end_matches('/')
            || (!saved.machine_serial.is_empty() && saved.machine_serial != *machine_serial)
        {
            return Err(EnrollmentFailure::ProtectedStorage);
        }
    }
    let registration = self_enrollment::self_register_existing(
        &saved.api_url,
        &machine_payload(state, saved.device_name.clone()),
        identity.key_id(),
        identity.public_key(),
        &saved.device_id,
        |bytes| {
            identity
                .sign_context_bytes(self_enrollment::SIGNATURE_CONTEXT, bytes)
                .map_err(|_| SelfEnrollmentError::SigningFailed)
        },
    )
    .await?;
    // Defense in depth before either credentials or the device registry changes.
    if registration.device_id != saved.device_id {
        return Err(SelfEnrollmentError::InvalidResponse.into());
    }
    apply_registration(state, &saved.api_url, registration)
        .await
        .map_err(|_| EnrollmentFailure::ProtectedStorage)
}

/// Notifications and API polling cannot turn an enrollment failure into a busy
/// claim loop. The supervisor's regular tick retries at or after this deadline.
#[derive(Default)]
pub(super) struct EnrollmentRetry {
    failures: u8,
    next_attempt: Option<tokio::time::Instant>,
}

impl EnrollmentRetry {
    pub(super) fn ready(&self) -> bool {
        self.next_attempt
            .is_none_or(|next| tokio::time::Instant::now() >= next)
    }

    pub(super) fn reset(&mut self) {
        self.failures = 0;
        self.next_attempt = None;
    }

    pub(super) fn defer(&mut self, error: EnrollmentFailure) {
        let delay = self.delay(error);
        self.failures = self.failures.saturating_add(1);
        self.next_attempt = Some(tokio::time::Instant::now() + delay);
    }

    fn delay(&self, error: EnrollmentFailure) -> Duration {
        let exponential = (30u64 << self.failures.min(4)).min(300);
        let minimum = match error {
            EnrollmentFailure::Protocol(SelfEnrollmentError::RateLimited) => 120,
            EnrollmentFailure::Protocol(SelfEnrollmentError::ProtocolUnavailable) => 300,
            _ => 30,
        };
        Duration::from_secs(exponential.max(minimum))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_credential_rejection_selects_identity_recovery() {
        assert!(needs_existing_recovery(&Err(
            mrd_device_registration::DEVICE_CREDENTIAL_REJECTED
        )));
        for error in [
            "连接服务器失败，请稍后重试",
            "注册请求过于频繁，请稍后重试",
            "设备注册失败，请检查服务器配置后重试",
            "服务器返回的设备登记响应无效",
        ] {
            assert!(!needs_existing_recovery(&Err(error)));
        }
        assert!(!needs_existing_recovery(&Ok(
            mrd_device_registration::DeviceRegistrationResponse {
                device_id: "123456789".into(),
                device_name: "Office".into(),
                access_token: "new.access.token".into(),
                refresh_token: Some("new.refresh.token".into()),
            }
        )));
    }

    fn saved_registration(state: &Arc<AppState>) -> Registration {
        Registration {
            device_id: "legacy-device-42".into(),
            device_name: "Office".into(),
            access_token: "existing.access.token".into(),
            refresh_token: Some("existing.refresh.token".into()),
            api_url: "https://127.0.0.1:9/api/v1".into(),
            machine_serial: state
                .public_connection
                .machine_serial
                .read()
                .unwrap()
                .clone(),
        }
    }

    fn assert_existing_identity_retained(state: &Arc<AppState>) {
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(saved.device_id, "legacy-device-42");
        assert_eq!(saved.access_token, "existing.access.token");
        assert_eq!(
            saved.refresh_token.as_deref(),
            Some("existing.refresh.token")
        );
    }

    #[tokio::test]
    async fn recovery_rejects_unprotected_or_different_machine_keys_without_identity_loss() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::new());
        *state.public_connection.api_url.write().unwrap() = "https://127.0.0.1:9/api/v1".into();
        *state.public_connection.registration.write().unwrap() = Some(saved_registration(&state));
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(
            recover_existing(&state, &saved).await,
            Err(EnrollmentFailure::ProtectedStorage)
        );
        assert_existing_identity_retained(&state);
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([95; 32]).unwrap()),
                &"a".repeat(64),
            )
            .unwrap();
        *state.public_connection.api_url.write().unwrap() = "https://127.0.0.1:9/api/v1".into();
        *state.public_connection.registration.write().unwrap() = Some(saved_registration(&state));
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(
            recover_existing(&state, &saved).await,
            Err(EnrollmentFailure::ProtectedStorage)
        );
        assert_existing_identity_retained(&state);
    }

    #[tokio::test]
    async fn failed_recovery_preserves_durable_credentials_and_rejects_identity_substitution() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::new());
        let key_id = state
            .device_identities
            .machine_identity()
            .key_id()
            .to_owned();
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([96; 32]).unwrap()),
                &key_id,
            )
            .unwrap();
        *state.public_connection.api_url.write().unwrap() = "https://127.0.0.1:9/api/v1".into();
        state
            .public_connection
            .save(saved_registration(&state))
            .unwrap();
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(
            recover_existing(&state, &saved).await,
            Err(EnrollmentFailure::Protocol(
                SelfEnrollmentError::ConnectionFailed
            ))
        );
        assert_existing_identity_retained(&state);
        let persisted = state
            .public_connection
            .persistence
            .read()
            .unwrap()
            .as_ref()
            .unwrap()
            .load()
            .unwrap()
            .unwrap();
        assert_eq!(persisted.device_id, saved.device_id);
        assert_eq!(persisted.access_token, saved.access_token);
        assert_eq!(persisted.refresh_token, saved.refresh_token);
        for mutation in 0..3 {
            let mut substitution = saved.clone();
            match mutation {
                0 => substitution.device_id = "987654321".into(),
                1 => substitution.api_url = "https://other.example/api/v1".into(),
                2 => substitution.machine_serial = "different-machine".into(),
                _ => unreachable!(),
            }
            assert_eq!(
                recover_existing(&state, &substitution).await,
                Err(EnrollmentFailure::ProtectedStorage)
            );
            assert_existing_identity_retained(&state);
        }
    }

    #[test]
    fn retry_deadline_is_capped_and_survives_notifications() {
        let mut retry = EnrollmentRetry::default();
        assert!(retry.ready());
        for expected in [30, 60, 120, 240, 300, 300] {
            let error = EnrollmentFailure::Protocol(SelfEnrollmentError::ConnectionFailed);
            assert_eq!(retry.delay(error), Duration::from_secs(expected));
            retry.defer(error);
            assert!(!retry.ready());
        }
        retry.reset();
        assert!(retry.ready());
        assert_eq!(
            retry.delay(EnrollmentFailure::Protocol(
                SelfEnrollmentError::RateLimited
            )),
            Duration::from_secs(120)
        );
        assert_eq!(
            retry.delay(EnrollmentFailure::Protocol(
                SelfEnrollmentError::ProtocolUnavailable
            )),
            Duration::from_secs(300)
        );
    }

    #[tokio::test]
    async fn existing_device_never_requests_a_new_code() {
        let state = Arc::new(AppState::new());
        *state.public_connection.registration.write().unwrap() = Some(super::super::Registration {
            device_id: "1501515774".into(),
            device_name: "LCX_ACE".into(),
            access_token: "existing.access.token".into(),
            refresh_token: Some("existing.refresh.token".into()),
            api_url: "https://example.com/api/v1".into(),
            machine_serial: "existing-machine".into(),
        });
        // Even with no configured storage and an unreachable server, the helper
        // returns before constructing an HTTP client or touching credentials.
        enroll_missing(&state, "https://127.0.0.1:9/api/v1")
            .await
            .unwrap();
        let saved = state.public_connection.registration().unwrap();
        assert_eq!(saved.device_id, "1501515774");
        assert_eq!(saved.access_token, "existing.access.token");
        assert_eq!(
            saved.refresh_token.as_deref(),
            Some("existing.refresh.token")
        );
    }

    #[tokio::test]
    async fn first_enrollment_requires_matching_protected_machine_identity() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::new());
        assert_eq!(
            enroll_missing(&state, "https://127.0.0.1:9/api/v1").await,
            Err(EnrollmentFailure::ProtectedStorage)
        );
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([92; 32]).unwrap()),
                &"a".repeat(64),
            )
            .unwrap();
        assert_eq!(
            enroll_missing(&state, "https://127.0.0.1:9/api/v1").await,
            Err(EnrollmentFailure::ProtectedStorage)
        );
        assert!(state.public_connection.registration().is_none());
    }

    #[tokio::test]
    async fn a_failed_durable_save_never_claims_a_device_code() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::new());
        let key_id = state
            .device_identities
            .machine_identity()
            .key_id()
            .to_owned();
        state
            .public_connection
            .configure_persistence(
                directory.path().to_path_buf(),
                Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([93; 32]).unwrap()),
                &key_id,
            )
            .unwrap();
        // The final file cannot replace this directory. This exercises a real
        // write/rename failure after the server has already issued credentials.
        std::fs::create_dir(directory.path().join("public-device-v1.protected")).unwrap();
        let result = apply_registration(
            &state,
            super::super::DEFAULT_PUBLIC_API_URL,
            mrd_device_registration::DeviceRegistrationResponse {
                device_id: "0123456789".into(),
                device_name: "Office".into(),
                access_token: "new.access.token".into(),
                refresh_token: Some("new.refresh.token".into()),
            },
        )
        .await;
        assert!(result.is_err());
        assert!(state.public_connection.registration().is_none());
        assert!(!state.devices.lock().await.is_registered());
        let status = state
            .public_connection
            .snapshot(&crate::signaling::SignalingRuntimeSnapshot::default());
        assert!(!status.device_registered && status.device_id.is_none());
        let temporary_files = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "tmp")
            })
            .count();
        assert_eq!(temporary_files, 0);
    }

    #[tokio::test]
    async fn successful_first_registration_survives_restart_without_exposing_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let state = Arc::new(AppState::new());
        let key_id = state
            .device_identities
            .machine_identity()
            .key_id()
            .to_owned();
        let protector =
            Arc::new(mrd_store_sqlite::AeadSecretProtector::from_key([94; 32]).unwrap());
        state
            .public_connection
            .configure_persistence(directory.path().to_path_buf(), protector.clone(), &key_id)
            .unwrap();
        apply_registration(
            &state,
            super::super::DEFAULT_PUBLIC_API_URL,
            mrd_device_registration::DeviceRegistrationResponse {
                device_id: "0123456789".into(),
                device_name: "Office".into(),
                access_token: "new.access.token".into(),
                refresh_token: Some("new.refresh.token".into()),
            },
        )
        .await
        .unwrap();
        let restarted = super::super::PublicConnectionState::default();
        restarted
            .configure_persistence(directory.path().to_path_buf(), protector, &key_id)
            .unwrap();
        let saved = restarted.registration().unwrap();
        assert_eq!(saved.device_id, "0123456789");
        assert_eq!(saved.access_token, "new.access.token");
        assert_eq!(saved.refresh_token.as_deref(), Some("new.refresh.token"));
        let status = serde_json::to_string(
            &restarted.snapshot(&crate::signaling::SignalingRuntimeSnapshot::default()),
        )
        .unwrap();
        assert!(status.contains("0123456789"));
        assert!(!status.contains("new.access.token") && !status.contains("new.refresh.token"));
    }
}
