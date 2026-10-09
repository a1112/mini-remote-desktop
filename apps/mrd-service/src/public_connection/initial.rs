//! First-time machine enrollment preserves the already selected durable identity.
use super::*;
use std::future::Future;

pub(super) async fn ensure_registration(
    state: &Arc<AppState>,
    api_url: &str,
) -> Result<bool, &'static str> {
    let api = api_url.to_owned();
    ensure_with(state, api_url, move |payload, identity| async move {
        mrd_device_registration::self_register_device(&api, &payload, identity.as_ref()).await
    })
    .await
}

async fn ensure_with<F, Fut>(
    state: &Arc<AppState>,
    api_url: &str,
    enroll: F,
) -> Result<bool, &'static str>
where
    F: FnOnce(DeviceRegistrationRequest, Arc<mrd_identity::DeviceIdentity>) -> Fut,
    Fut: Future<Output = Result<DeviceRegistrationResponse, &'static str>>,
{
    let _operation = state.public_connection.operation.lock().await;
    // The physical identity and existing credentials always win over first-time enrollment.
    if state.public_connection.registration().is_some() {
        return Ok(false);
    }
    let identity = state.device_identities.machine_identity();
    let storage_ready = state
        .public_connection
        .persistence
        .read()
        .unwrap()
        .as_ref()
        .is_some_and(|config| config.machine_key_id == identity.key_id());
    if !storage_ready {
        return Err("public_self_enrollment_storage_unavailable");
    }
    if state
        .public_connection
        .api_url
        .read()
        .unwrap()
        .trim_end_matches('/')
        != api_url.trim_end_matches('/')
        || validate_api_url(api_url).map_or(true, |url| url.scheme() != "https")
    {
        return Err("public_self_enrollment_origin_invalid");
    }
    let name = sysinfo::System::host_name()
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
        })
        .unwrap_or_else(|| "Rdesk Device".into());
    let payload = machine_payload(state, name);
    // Cancellation drops the HTTP future and operation guard; no detached enrollment task survives shutdown.
    let response = tokio::time::timeout(Duration::from_secs(15), enroll(payload, identity))
        .await
        .map_err(|_| "public_self_enrollment_transport")??;
    apply_self_registration(state, api_url, response).await?;
    Ok(true)
}

#[derive(Default)]
pub(super) struct EnrollmentRetry {
    failures: u8,
    next_attempt: Option<tokio::time::Instant>,
}
impl EnrollmentRetry {
    pub(super) fn can_attempt(&self, now: tokio::time::Instant) -> bool {
        self.next_attempt.is_none_or(|next| now >= next)
    }
    pub(super) fn failed(&mut self, now: tokio::time::Instant, error: &str) {
        self.failures = self.failures.saturating_add(1).min(5);
        let mut seconds = (30u64 << u32::from(self.failures - 1)).min(300);
        if error == "public_self_enrollment_rate_limited" {
            seconds = seconds.max(60);
        }
        if error == "public_self_enrollment_conflict" {
            seconds = 300;
        }
        let next = now + Duration::from_secs(seconds);
        self.next_attempt = Some(
            self.next_attempt
                .map_or(next, |previous| previous.max(next)),
        );
    }
    pub(super) fn succeeded(&mut self) {
        self.failures = 0;
        self.next_attempt = None;
    }
}
#[cfg(test)]
mod tests;
