//! Ephemeral password custody and publication fences. No transport, logs or disk secrets.
use mrd_ipc::{TemporaryAccessPassword, TemporaryAccessSecret, TemporaryAccessStatus};
use ring::{
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::Serialize;
use std::{
    num::NonZeroU32,
    sync::{Arc, Mutex},
};

/// Orders explicit user intent before any await; background work only borrows an epoch.
/// The disable cutoff remains monotonic even after a later explicit enable request.
pub(crate) struct TemporaryOperationEpoch {
    intent: Mutex<TemporaryOperationIntent>,
}
#[derive(Clone, Copy)]
struct TemporaryOperationIntent {
    current: u64,
    enabled: bool,
    disabled_at: Option<u64>,
}
impl Default for TemporaryOperationEpoch {
    fn default() -> Self {
        Self {
            intent: Mutex::new(TemporaryOperationIntent {
                current: 0,
                enabled: true,
                disabled_at: None,
            }),
        }
    }
}
impl TemporaryOperationEpoch {
    pub(crate) fn begin(&self, disable: bool) -> Result<u64, &'static str> {
        let mut intent = self
            .intent
            .lock()
            .map_err(|_| "temporary_operation_unavailable")?;
        let epoch = intent
            .current
            .checked_add(1)
            .ok_or("temporary_operation_exhausted")?;
        intent.current = epoch;
        intent.enabled = !disable;
        if disable {
            intent.disabled_at = Some(epoch);
        }
        Ok(epoch)
    }
    pub(crate) fn current(&self) -> Result<u64, &'static str> {
        self.intent
            .lock()
            .map(|intent| intent.current)
            .map_err(|_| "temporary_operation_unavailable")
    }
    pub(crate) fn is_current(&self, epoch: u64) -> bool {
        self.current() == Ok(epoch)
    }
    fn snapshot(&self) -> Option<TemporaryOperationIntent> {
        self.intent.lock().ok().map(|intent| *intent)
    }
    fn permits_enabled(&self) -> bool {
        self.intent
            .lock()
            .map(|intent| intent.enabled)
            .unwrap_or(false)
    }
    fn permits_existing_guest(&self, admitted_at: u64) -> bool {
        self.intent
            .lock()
            .map(|intent| {
                intent.enabled
                    && intent
                        .disabled_at
                        .is_none_or(|disabled| admitted_at > disabled)
            })
            .unwrap_or(false)
    }
}

use zeroize::Zeroizing;

pub const TEMPORARY_PASSWORD_TTL_MS: u64 = 600_000;
const PASSWORD_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const PASSWORD_ITERATIONS: u32 = 600_000;

/// The cleartext belongs only to this resident process and verified local UI replies.
pub struct TemporaryAccessState {
    enabled: bool,
    operation_epoch: Arc<TemporaryOperationEpoch>,
    applied_epoch: u64,
    rotation_requested: bool,
    generation: u64,
    published_generation: Option<u64>,
    expires_at_ms: Option<u64>,
    password: Option<Zeroizing<String>>,
    salt: Option<[u8; 16]>,
    verifier: Option<[u8; 32]>,
    guest_sessions:
        std::collections::BTreeMap<String, (TemporaryGuestAuthority, String, bool, u64)>,
}
impl Default for TemporaryAccessState {
    fn default() -> Self {
        Self {
            enabled: true,
            operation_epoch: Arc::new(TemporaryOperationEpoch::default()),
            applied_epoch: 0,
            rotation_requested: false,
            generation: 0,
            published_generation: None,
            expires_at_ms: None,
            password: None,
            salt: None,
            verifier: None,
            guest_sessions: Default::default(),
        }
    }
}
impl std::fmt::Debug for TemporaryAccessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemporaryAccessState")
            .field("enabled", &self.enabled)
            .field("generation", &self.generation)
            .field("password", &"REDACTED")
            .finish_non_exhaustive()
    }
}

/// Canonical signed publication fields; the password is never a field.
#[derive(Serialize)]
pub struct TemporaryAccessDocument {
    pub device_id: String,
    pub auth_version: u64,
    pub generation: u64,
    pub enabled: bool,
    pub expires_at_ms: Option<u64>,
    pub salt: Option<String>,
    pub verifier: Option<String>,
    pub allowed_scopes: Vec<String>,
}
impl TemporaryAccessState {
    pub(crate) fn with_operation_epoch(mut self, epoch: Arc<TemporaryOperationEpoch>) -> Self {
        self.operation_epoch = epoch;
        self
    }
    pub(crate) fn apply_operation_epoch(&mut self, epoch: u64) -> Result<(), &'static str> {
        if !self.operation_epoch.is_current(epoch) {
            return Err("temporary_publication_superseded");
        }
        self.applied_epoch = epoch;
        Ok(())
    }
    pub(crate) fn publication_epoch_current(&self) -> bool {
        self.operation_epoch.is_current(self.applied_epoch)
    }
    pub fn rotate(&mut self, generation: u64, now: u64) -> Result<(), &'static str> {
        self.check_generation(generation)?;
        let expiry = now
            .checked_add(TEMPORARY_PASSWORD_TTL_MS)
            .ok_or("temporary_expiry_invalid")?;
        let mut entropy = Zeroizing::new([0u8; 24]);
        SystemRandom::new()
            .fill(entropy.as_mut())
            .map_err(|_| "temporary_entropy_unavailable")?;
        let password = Zeroizing::new(
            entropy[..8]
                .iter()
                .map(|byte| char::from(PASSWORD_ALPHABET[usize::from(byte & 31)]))
                .collect::<String>(),
        );
        let mut salt = [0u8; 16];
        salt.copy_from_slice(&entropy[8..]);
        let verifier = password_verifier(password.as_bytes(), &salt);
        self.password = Some(password);
        self.salt = Some(salt);
        self.verifier = Some(verifier);
        self.enabled = true;
        self.rotation_requested = false;
        self.generation = generation;
        self.published_generation = None;
        self.expires_at_ms = Some(expiry);
        Ok(())
    }
    pub fn disable(&mut self, generation: u64) -> Result<(), &'static str> {
        self.check_generation(generation)?;
        self.freeze();
        self.generation = generation;
        Ok(())
    }
    /// Admission and password reads freeze before any network await.
    pub fn freeze(&mut self) {
        self.enabled = false;
        self.rotation_requested = false;
        self.published_generation = None;
        self.expires_at_ms = None;
        self.password = None;
        self.salt = None;
        self.verifier = None;
    }
    fn check_generation(&self, generation: u64) -> Result<(), &'static str> {
        if generation <= self.generation || generation > i64::MAX as u64 {
            Err("temporary_generation_invalid")
        } else {
            Ok(())
        }
    }
    pub fn invalidate_publication(&mut self) {
        self.rotation_requested = true;
        self.published_generation = None;
    }
    pub fn mark_published(&mut self, generation: u64) -> bool {
        if generation != self.generation
            || self.rotation_requested
            || !self.publication_epoch_current()
        {
            return false;
        }
        self.published_generation = Some(generation);
        true
    }
    pub fn status(&mut self, now: u64, online: bool) -> TemporaryAccessStatus {
        // All response fields describe one linearized user intent, even if a new
        // request starts while this snapshot is being assembled.
        let intent = self.operation_epoch.snapshot();
        let enabled = self.enabled && intent.is_some_and(|intent| intent.enabled);
        let epoch_current = intent.is_some_and(|intent| intent.current == self.applied_epoch);
        let expired = self.expires_at_ms.is_some_and(|expires| now >= expires);
        if expired {
            self.password = None;
            self.salt = None;
            self.verifier = None;
            self.published_generation = None;
        }
        let ready = enabled
            && epoch_current
            && !self.rotation_requested
            && self.password.is_some()
            && self.published_generation == Some(self.generation)
            && online
            && !expired;
        let reason = if !enabled {
            Some("disabled")
        } else if expired {
            Some("expired")
        } else if self.generation == 0 {
            Some("device_unregistered")
        } else if !online {
            Some("signaling_offline")
        } else if !epoch_current || self.published_generation != Some(self.generation) {
            Some("publication_pending")
        } else {
            None
        };
        TemporaryAccessStatus {
            enabled,
            ready,
            generation: self.generation,
            expires_at_ms: self.expires_at_ms,
            reason: reason.map(str::to_owned),
        }
    }
    pub fn secret(&mut self, now: u64, online: bool) -> TemporaryAccessSecret {
        let status = self.status(now, online);
        let password = if status.ready {
            self.password
                .as_ref()
                .map(|value| TemporaryAccessPassword::from(value.as_str().to_owned()))
        } else {
            None
        };
        TemporaryAccessSecret { status, password }
    }
    pub fn access_document(
        &self,
        device: &str,
        auth: u64,
    ) -> Result<TemporaryAccessDocument, &'static str> {
        if device.is_empty()
            || device.len() > 64
            || auth == 0
            || auth > i64::MAX as u64
            || self.generation == 0
        {
            return Err("temporary_identity_invalid");
        }
        if self.enabled
            && (self.password.is_none() || self.salt.is_none() || self.verifier.is_none())
        {
            return Err("temporary_password_unavailable");
        }
        Ok(TemporaryAccessDocument {
            device_id: device.to_owned(),
            auth_version: auth,
            generation: self.generation,
            enabled: self.enabled,
            expires_at_ms: self.expires_at_ms,
            salt: self.salt.as_ref().map(|value| hex(value)),
            verifier: self.verifier.as_ref().map(|value| hex(value)),
            allowed_scopes: if self.enabled {
                vec![
                    "input.keyboard".into(),
                    "input.pointer".into(),
                    "screen.view".into(),
                ]
            } else {
                vec![]
            },
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn enabled(&self) -> bool {
        self.enabled && self.operation_epoch.permits_enabled()
    }
    pub fn needs_rotation(&self, now: u64) -> bool {
        self.enabled
            && (self.rotation_requested
                || self.password.is_none()
                || self.expires_at_ms.is_none_or(|expires| now >= expires))
    }
    pub fn needs_publication(&self) -> bool {
        self.generation > 0 && self.published_generation != Some(self.generation)
    }
}
fn password_verifier(password: &[u8], salt: &[u8]) -> [u8; 32] {
    let mut derived = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PASSWORD_ITERATIONS).expect("nonzero PBKDF2 iterations"),
        salt,
        password,
        &mut derived,
    );
    derived
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TemporaryGuestAuthority {
    pub generation: u64,
    pub target_auth_version: u64,
}
impl TemporaryGuestAuthority {
    pub(crate) fn from_wire(
        kind: Option<&str>,
        generation: Option<u64>,
        auth: Option<u64>,
    ) -> Result<Option<Self>, &'static str> {
        match (kind, generation, auth) {
            (None, None, None) => Ok(None),
            (Some("temporary_password"), Some(generation), Some(target_auth_version))
                if generation > 0
                    && generation <= i64::MAX as u64
                    && target_auth_version > 0
                    && target_auth_version <= i64::MAX as u64 =>
            {
                Ok(Some(Self {
                    generation,
                    target_auth_version,
                }))
            }
            _ => Err("temporary_authority_invalid"),
        }
    }
}
#[derive(serde::Serialize)]
pub struct TemporaryPublicationProof {
    pub key_id: String,
    pub public_key: String,
    pub access_json: String,
    pub signature: String,
}
impl TemporaryAccessState {
    pub fn authorize_guest(
        &mut self,
        session: &str,
        authority: TemporaryGuestAuthority,
        commitment: &str,
        approved: bool,
        now: u64,
        online: bool,
        auth: u64,
    ) -> bool {
        if !self.enabled() || auth == 0 || authority.target_auth_version != auth {
            return false;
        }
        if approved {
            let Some((saved, bound, already_approved, admitted_at)) =
                self.guest_sessions.get(session).cloned()
            else {
                return false;
            };
            if saved != authority
                || bound != commitment
                || !self.operation_epoch.permits_existing_guest(admitted_at)
            {
                return false;
            }
            if !already_approved
                && (authority.generation != self.generation || !self.status(now, online).ready)
            {
                return false;
            }
            self.guest_sessions.insert(
                session.to_owned(),
                (authority, commitment.to_owned(), true, admitted_at),
            );
            return true;
        }
        if authority.generation != self.generation || !self.status(now, online).ready {
            return false;
        }
        if let Some((saved, bound, _, admitted_at)) = self.guest_sessions.get(session) {
            return *saved == authority
                && bound == commitment
                && self.operation_epoch.permits_existing_guest(*admitted_at);
        }
        if self.guest_sessions.len() >= 128 {
            return false;
        }
        self.guest_sessions.insert(
            session.to_owned(),
            (authority, commitment.to_owned(), false, self.applied_epoch),
        );
        true
    }
    pub(crate) fn guest_session_ids_before_epoch(&self, epoch: u64) -> Vec<String> {
        self.guest_sessions
            .iter()
            .filter(|(_, (_, _, _, admitted))| *admitted < epoch)
            .map(|(session, _)| session.clone())
            .collect()
    }
    pub fn guest_session_ids(&self) -> Vec<String> {
        self.guest_sessions.keys().cloned().collect()
    }
    pub fn has_guest_session(&self, session: &str) -> bool {
        self.guest_sessions.contains_key(session)
    }
    pub fn forget_guest_session(&mut self, session: &str) {
        self.guest_sessions.remove(session);
    }
}
pub(crate) fn signed_publication(
    identity: &mrd_identity::DeviceIdentity,
    endpoint: &str,
    doc: &TemporaryAccessDocument,
) -> Result<TemporaryPublicationProof, &'static str> {
    let access_json = serde_json::to_string(doc).map_err(|_| "temporary_document_invalid")?;
    let hash = hex(ring::digest::digest(&ring::digest::SHA256, access_json.as_bytes()).as_ref());
    let canonical = format!("POST\n{endpoint}\n{}\n{hash}", identity.key_id());
    let signature = identity
        .sign_context_bytes("MRD_DEVICE_TEMPORARY_ACCESS_V1", canonical.as_bytes())
        .map_err(|_| "temporary_proof_unavailable")?;
    Ok(TemporaryPublicationProof {
        key_id: identity.key_id().to_owned(),
        public_key: hex(identity.public_key()),
        access_json,
        signature: hex(&signature),
    })
}
pub(crate) fn decoded_auth_version(token: &str) -> Option<u64> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use serde::de::{MapAccess, Visitor};
    struct UniqueClaims;
    impl<'de> Visitor<'de> for UniqueClaims {
        type Value = std::collections::BTreeMap<String, serde_json::Value>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique device claims")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut values = std::collections::BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                if values.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate device claim"));
                }
            }
            Ok(values)
        }
    }
    if token.len() > 4096 {
        return None;
    }
    let mut parts = token.split('.');
    let _ = parts.next()?;
    let encoded = parts.next()?;
    let _ = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let payload = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).ok()?);
    let mut deserializer = serde_json::Deserializer::from_slice(payload.as_ref());
    let claims = serde::de::Deserializer::deserialize_map(&mut deserializer, UniqueClaims).ok()?;
    deserializer.end().ok()?;
    if claims.get("token_type")?.as_str()? != "device" || claims.get("role")?.as_str()? != "device"
    {
        return None;
    }
    let version = claims.get("auth_version")?.as_u64()?;
    (version > 0 && version <= i64::MAX as u64).then_some(version)
}
#[cfg(test)]
fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.len() % 2 != 0 {
        return None;
    }
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).ok())
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_access_status_fields_share_one_concurrent_intent_snapshot() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let epoch = Arc::new(TemporaryOperationEpoch::default());
        let mut state = TemporaryAccessState::default().with_operation_epoch(epoch.clone());
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        let stop = Arc::new(AtomicBool::new(false));
        let writer_epoch = epoch.clone();
        let writer_stop = stop.clone();
        let writer = std::thread::spawn(move || {
            while !writer_stop.load(Ordering::Acquire) {
                writer_epoch.begin(true).unwrap();
                writer_epoch.begin(false).unwrap();
            }
        });
        let mut inconsistent = None;
        for _ in 0..100_000 {
            state.applied_epoch = epoch.current().unwrap();
            let status = state.status(1001, true);
            if status.ready && (!status.enabled || status.reason.is_some()) {
                inconsistent = Some(status);
                break;
            }
        }
        stop.store(true, Ordering::Release);
        writer.join().unwrap();
        assert!(
            inconsistent.is_none(),
            "response fields must share one intent: {inconsistent:?}"
        );
    }

    #[test]
    fn request_epoch_fences_reads_and_guests_before_local_freeze() {
        let epoch = Arc::new(TemporaryOperationEpoch::default());
        let mut state = TemporaryAccessState::default().with_operation_epoch(epoch.clone());
        state.rotate(1, 1000).unwrap();
        assert!(state.mark_published(1));
        let authority = TemporaryGuestAuthority {
            generation: 1,
            target_auth_version: 2,
        };
        assert!(state.authorize_guest("approved-old", authority, "bound", false, 1001, true, 2));
        assert!(state.authorize_guest("approved-old", authority, "bound", true, 1001, true, 2));
        let disabled = epoch.begin(true).unwrap();
        assert!(state.secret(1001, true).password.is_none());
        assert!(!state.status(1001, true).enabled);
        assert!(!state.mark_published(1));
        assert!(!state.authorize_guest(
            "new-after-disable",
            authority,
            "bound",
            false,
            1001,
            true,
            2
        ));
        assert!(!state.authorize_guest("approved-old", authority, "bound", true, 1001, true, 2));
        let rotate = epoch.begin(false).unwrap();
        assert!(rotate > disabled);
        assert!(state.apply_operation_epoch(disabled).is_err());
        state.apply_operation_epoch(rotate).unwrap();
        state.rotate(2, 1002).unwrap();
        state.mark_published(2);
        assert!(state.status(1003, true).ready);
        assert!(!state.authorize_guest("approved-old", authority, "bound", true, 1003, true, 2));
        let current = TemporaryGuestAuthority {
            generation: 2,
            target_auth_version: 2,
        };
        assert!(state.authorize_guest(
            "new-after-explicit-rotate",
            current,
            "new-bound",
            false,
            1003,
            true,
            2
        ));
        assert_eq!(
            state.guest_session_ids_before_epoch(disabled),
            vec!["approved-old".to_owned()]
        );
    }

    #[test]
    fn refresh_epoch_retains_only_already_approved_grants() {
        let epoch = Arc::new(TemporaryOperationEpoch::default());
        let mut state = TemporaryAccessState::default().with_operation_epoch(epoch.clone());
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        let authority = TemporaryGuestAuthority {
            generation: 1,
            target_auth_version: 2,
        };
        for session in ["approved", "pending"] {
            assert!(state.authorize_guest(session, authority, "bound", false, 1001, true, 2));
        }
        assert!(state.authorize_guest("approved", authority, "bound", true, 1001, true, 2));
        let rotate = epoch.begin(false).unwrap();
        assert!(state.secret(1001, true).password.is_none());
        assert!(state.authorize_guest("approved", authority, "bound", true, 1001, true, 2));
        assert!(!state.authorize_guest("pending", authority, "bound", true, 1001, true, 2));
        state.apply_operation_epoch(rotate).unwrap();
        state.invalidate_publication();
        state.rotate(2, 1002).unwrap();
        state.mark_published(2);
        assert!(state.authorize_guest("approved", authority, "bound", true, 1003, true, 2));
        assert!(!state.authorize_guest("pending", authority, "bound", true, 1003, true, 2));
    }

    #[test]
    fn temporary_access_is_not_ready_until_exact_publish_and_online_signal() {
        let mut state = TemporaryAccessState::default();
        state.rotate(7, 1_000).unwrap();
        assert!(!state.status(1_001, true).ready);
        assert!(state.secret(1_001, true).password.is_none());
        assert!(!state.mark_published(6));
        assert!(state.mark_published(7));
        assert!(!state.status(1_001, false).ready);
        assert!(state.status(1_001, true).ready);
        assert_eq!(
            state
                .secret(1_001, true)
                .password
                .unwrap()
                .into_secret()
                .len(),
            8
        );
    }

    #[test]
    fn temporary_access_rotation_freezes_old_password_and_expiry_erases_secret() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1_000).unwrap();
        state.mark_published(1);
        let old = state.secret(1_001, true).password.unwrap().into_secret();
        state.rotate(2, 2_000).unwrap();
        assert!(!state.status(2_001, true).ready);
        assert!(!state.mark_published(1));
        state.mark_published(2);
        let secret = state.secret(2_001, true);
        assert_ne!(old, secret.password.as_ref().unwrap().secret());
        assert!(!format!("{secret:?}").contains(secret.password.as_ref().unwrap().secret()));
        assert!(state.secret(602_000, true).password.is_none());
        assert!(!state.status(602_000, true).ready);
        state.disable(3).unwrap();
        assert!(!state.status(602_001, true).enabled);
        assert!(state.secret(602_001, true).password.is_none());
    }

    #[test]
    fn temporary_access_generation_never_moves_backwards() {
        let mut state = TemporaryAccessState::default();
        state.rotate(9, 1_000).unwrap();
        assert!(state.rotate(9, 1_001).is_err());
        assert!(state.disable(8).is_err());
        assert_eq!(state.status(1_001, true).generation, 9);
    }

    #[test]
    fn temporary_access_verifier_matches_shared_pbkdf2_vector_and_never_contains_password() {
        let verifier = password_verifier(b"ABCDEFGH", &[17; 16]);
        assert_eq!(
            hex(&verifier),
            "f7482a0b52084989fe6540af912f17b2d316a6f4610e280536b6b813cc852bce"
        );
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1_000).unwrap();
        let password = state.password.as_ref().unwrap().as_str().to_owned();
        let access = state.access_document("0123456789", 2).unwrap();
        assert!(!serde_json::to_string(&access).unwrap().contains(&password));
        assert_eq!(
            access.allowed_scopes,
            ["input.keyboard", "input.pointer", "screen.view"]
        );
        assert_eq!(access.expires_at_ms, Some(601_000));
        assert_eq!(access.auth_version, 2);
        assert_eq!(access.salt.as_ref().unwrap().len(), 32);
    }

    #[test]
    fn guest_admission_requires_published_current_generation_and_freezes_on_disable() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        let authority = TemporaryGuestAuthority {
            generation: 1,
            target_auth_version: 2,
        };
        assert!(!state.authorize_guest("s", authority, "commitment", false, 1001, true, 2));
        state.mark_published(1);
        assert!(state.authorize_guest("s", authority, "commitment", false, 1001, true, 2));
        assert!(state.authorize_guest("s", authority, "commitment", true, 1001, true, 2));
        state.rotate(2, 2000).unwrap();
        assert!(state.authorize_guest("s", authority, "commitment", true, 2001, true, 2));
        assert!(!state.authorize_guest("new", authority, "commitment", false, 2001, true, 2));
        assert!(!state.authorize_guest("s", authority, "changed", true, 2001, true, 2));
        assert!(!state.authorize_guest("s", authority, "commitment", true, 2001, true, 3));
        state.freeze();
        assert!(!state.authorize_guest("s", authority, "commitment", true, 2001, true, 2));
    }
    #[test]
    fn guest_approved_authority_cannot_be_created_without_local_admission() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        assert!(!state.authorize_guest(
            "unknown",
            TemporaryGuestAuthority {
                generation: 1,
                target_auth_version: 2
            },
            "commitment",
            true,
            1001,
            true,
            2
        ));
    }
    #[test]
    fn temporary_publication_is_bound_to_url_document_and_machine_key() {
        let identity = mrd_identity::DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        let doc = state.access_document("0123456789", 2).unwrap();
        let proof = signed_publication(
            &identity,
            "https://server.example/api/v1/devices/temporary-access",
            &doc,
        )
        .unwrap();
        let hash =
            hex(ring::digest::digest(&ring::digest::SHA256, proof.access_json.as_bytes()).as_ref());
        let canonical = format!(
            "POST\nhttps://server.example/api/v1/devices/temporary-access\n{}\n{hash}",
            identity.key_id()
        );
        assert_eq!(proof.key_id, identity.key_id());
        assert_eq!(proof.public_key, hex(identity.public_key()));
        let signature = decode_hex(&proof.signature).unwrap();
        mrd_identity::verify_context_bytes(
            identity.public_key(),
            "MRD_DEVICE_TEMPORARY_ACCESS_V1",
            canonical.as_bytes(),
            &signature,
        )
        .unwrap();
        assert!(mrd_identity::verify_context_bytes(
            identity.public_key(),
            "MRD_DEVICE_TEMPORARY_ACCESS_V1",
            b"wrong-url",
            &signature
        )
        .is_err());
    }
    #[test]
    fn decoded_auth_version_is_positive_and_never_a_user_login_token() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        let jwt = |value: &str| format!("e30.{}.signature", URL_SAFE_NO_PAD.encode(value));
        assert_eq!(
            decoded_auth_version(&jwt(
                r#"{"token_type":"device","role":"device","auth_version":2}"#
            )),
            Some(2)
        );
        for payload in [
            r#"{"auth_version":0}"#,
            r#"{"auth_version":-1}"#,
            r#"{"auth_version":"2"}"#,
            r#"{"auth_version":2,"auth_version":3}"#,
        ] {
            assert_eq!(decoded_auth_version(&jwt(payload)), None);
        }
    }

    #[test]
    fn rotating_password_cannot_promote_an_old_pending_session_to_approved() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        let authority = TemporaryGuestAuthority {
            generation: 1,
            target_auth_version: 2,
        };
        assert!(state.authorize_guest("pending", authority, "commitment", false, 1001, true, 2));
        state.rotate(2, 2000).unwrap();
        assert!(!state.authorize_guest("pending", authority, "commitment", true, 2001, true, 2));
    }

    #[test]
    fn requested_rotation_is_retried_after_network_failure_instead_of_republishing_old_password() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        state.invalidate_publication();
        assert!(state.needs_rotation(1001));
        assert!(!state.status(1001, true).ready);
    }

    #[test]
    fn typed_guest_authority_metadata_rejects_mixed_null_and_unknown_fields() {
        assert_eq!(
            TemporaryGuestAuthority::from_wire(None, None, None),
            Ok(None)
        );
        assert_eq!(
            TemporaryGuestAuthority::from_wire(Some("temporary_password"), Some(3), Some(2)),
            Ok(Some(TemporaryGuestAuthority {
                generation: 3,
                target_auth_version: 2
            }))
        );
        for (kind, generation, auth) in [
            (Some("account"), Some(1), Some(2)),
            (None, Some(1), Some(2)),
            (Some("temporary_password"), Some(0), Some(2)),
            (Some("temporary_password"), None, Some(2)),
            (Some("temporary_password"), Some(1), None),
        ] {
            assert!(TemporaryGuestAuthority::from_wire(kind, generation, auth).is_err());
        }
    }
    #[test]
    fn old_publication_ack_cannot_reveal_password_while_rotation_is_pending() {
        let mut state = TemporaryAccessState::default();
        state.rotate(1, 1000).unwrap();
        state.mark_published(1);
        state.invalidate_publication();
        state.mark_published(1);
        assert!(!state.status(1001, true).ready);
        assert!(state.secret(1001, true).password.is_none());
    }

    #[test]
    fn python_publisher_signature_fixture_matches_rust_context_verification() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../Rdesk-Server/tests/fixtures/temporary-access-publication-v1.json"
        ))
        .unwrap();
        assert_eq!(fixture["pbkdf2_iterations"], 600000);
        let public_key = decode_hex(fixture["public_key"].as_str().unwrap()).unwrap();
        let signature = decode_hex(fixture["signature"].as_str().unwrap()).unwrap();
        mrd_identity::verify_context_bytes(
            &public_key,
            "MRD_DEVICE_TEMPORARY_ACCESS_V1",
            fixture["canonical_utf8"].as_str().unwrap().as_bytes(),
            &signature,
        )
        .unwrap();
    }
}
