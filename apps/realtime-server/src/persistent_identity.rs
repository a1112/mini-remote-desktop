//! Stable server signatures with durable, exclusively owned counter reservations.

use mrd_identity::DeviceIdentity;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use thiserror::Error;
use zeroize::Zeroizing;

const COUNTER_BLOCK: u64 = 1_048_576;
const MAX_STATE_BYTES: u64 = 4_096;

#[derive(Debug, Error)]
pub enum PersistentIdentityError {
    #[error("realtime signing identity and counter files must both be configured")]
    InvalidConfiguration,
    #[error("realtime signing state is unavailable")]
    Unavailable,
    #[error("realtime signing state permissions are unsafe")]
    UnsafePermissions,
    #[error("realtime signing counter is owned by another process")]
    AlreadyLocked,
    #[error("realtime signing state is malformed or belongs to another identity")]
    InvalidState,
    #[error("realtime signing counter is exhausted")]
    CounterExhausted,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CounterState {
    format_version: u8,
    key_id: String,
    reserved_through: u64,
}

pub struct PersistentServerState {
    pub(crate) identity: DeviceIdentity,
    pub(crate) counter: ServerSigningCounter,
}

pub(crate) enum ServerSigningCounter {
    Ephemeral(u64),
    Persistent {
        file: File,
        state: CounterState,
        next: u64,
    },
}

impl PersistentServerState {
    pub fn from_env(deployed: bool) -> Result<Option<Self>, PersistentIdentityError> {
        let key = std::env::var_os("MRD_REALTIME_IDENTITY_PKCS8_FILE");
        let counter = std::env::var_os("MRD_REALTIME_COUNTER_FILE");
        match (key, counter) {
            (None, None) if !deployed => Ok(None),
            (Some(key), Some(counter)) => {
                // Production protected-storage validation is implemented for Unix hosts.
                if deployed && !cfg!(unix) {
                    return Err(PersistentIdentityError::UnsafePermissions);
                }
                Self::from_files(Path::new(&key), Path::new(&counter)).map(Some)
            }
            _ => Err(PersistentIdentityError::InvalidConfiguration),
        }
    }

    pub fn from_files(
        key_path: &Path,
        counter_path: &Path,
    ) -> Result<Self, PersistentIdentityError> {
        if !key_path.is_absolute() || !counter_path.is_absolute() || key_path == counter_path {
            return Err(PersistentIdentityError::InvalidConfiguration);
        }
        let mut key_file = open_protected(key_path, false, true)?;
        let mut key_bytes = Zeroizing::new(Vec::new());
        Read::by_ref(&mut key_file)
            .take(MAX_STATE_BYTES + 1)
            .read_to_end(&mut key_bytes)
            .map_err(|_| PersistentIdentityError::Unavailable)?;
        if key_bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(PersistentIdentityError::InvalidState);
        }
        let identity = load_pkcs8(&key_bytes)?;
        // No create option: a lost counter must never silently restart at one.
        let mut file = open_protected(counter_path, true, false)?;
        file.try_lock()
            .map_err(|_| PersistentIdentityError::AlreadyLocked)?;
        let mut encoded = String::new();
        Read::by_ref(&mut file)
            .take(MAX_STATE_BYTES + 1)
            .read_to_string(&mut encoded)
            .map_err(|_| PersistentIdentityError::InvalidState)?;
        if encoded.len() as u64 > MAX_STATE_BYTES {
            return Err(PersistentIdentityError::InvalidState);
        }
        let state: CounterState =
            serde_json::from_str(&encoded).map_err(|_| PersistentIdentityError::InvalidState)?;
        if state.format_version != 1 || state.key_id != identity.key_id() {
            return Err(PersistentIdentityError::InvalidState);
        }
        let next = state
            .reserved_through
            .checked_add(1)
            .ok_or(PersistentIdentityError::CounterExhausted)?;
        let mut counter = ServerSigningCounter::Persistent { file, state, next };
        counter.reserve()?;
        Ok(Self { identity, counter })
    }
}

impl ServerSigningCounter {
    fn reserve(&mut self) -> Result<(), PersistentIdentityError> {
        let Self::Persistent { file, state, .. } = self else {
            return Ok(());
        };
        let reserved_through = state
            .reserved_through
            .checked_add(COUNTER_BLOCK)
            .ok_or(PersistentIdentityError::CounterExhausted)?;
        let reservation = CounterState {
            format_version: 1,
            key_id: state.key_id.clone(),
            reserved_through,
        };
        let encoded =
            serde_json::to_vec(&reservation).map_err(|_| PersistentIdentityError::InvalidState)?;
        file.seek(SeekFrom::Start(0))
            .and_then(|_| file.set_len(0))
            .and_then(|_| file.write_all(&encoded))
            .and_then(|_| file.sync_all())
            .map_err(|_| PersistentIdentityError::Unavailable)?;
        state.reserved_through = reserved_through;
        Ok(())
    }

    pub(crate) fn next(&mut self) -> Result<u64, PersistentIdentityError> {
        if matches!(self, Self::Persistent { state, next, .. } if *next > state.reserved_through) {
            self.reserve()?;
        }
        let next = match self {
            Self::Ephemeral(next) | Self::Persistent { next, .. } => next,
        };
        let result = *next;
        *next = next
            .checked_add(1)
            .ok_or(PersistentIdentityError::CounterExhausted)?;
        Ok(result)
    }
}

fn load_pkcs8(bytes: &[u8]) -> Result<DeviceIdentity, PersistentIdentityError> {
    if let Ok(identity) = DeviceIdentity::from_pkcs8(bytes) {
        return Ok(identity);
    }
    // OpenSSL/systemd deployments commonly supply the RFC 8410 v1 encoding.
    // Accept only its exact Ed25519 seed form and add the derived public key.
    const PREFIX: &[u8] = &[
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    if bytes.len() != PREFIX.len() + 32 || !bytes.starts_with(PREFIX) {
        return Err(PersistentIdentityError::InvalidState);
    }
    use ring::signature::KeyPair;
    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&bytes[PREFIX.len()..])
        .map_err(|_| PersistentIdentityError::InvalidState)?;
    let mut canonical = Zeroizing::new(vec![
        0x30, 0x53, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ]);
    canonical.extend_from_slice(&bytes[PREFIX.len()..]);
    canonical.extend_from_slice(&[0xa1, 0x23, 0x03, 0x21, 0x00]);
    canonical.extend_from_slice(pair.public_key().as_ref());
    DeviceIdentity::from_pkcs8(&canonical).map_err(|_| PersistentIdentityError::InvalidState)
}

fn open_protected(
    path: &Path,
    writable: bool,
    credential: bool,
) -> Result<File, PersistentIdentityError> {
    let mut options = OpenOptions::new();
    options.read(true).write(writable);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(path)
        .map_err(|_| PersistentIdentityError::Unavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| PersistentIdentityError::Unavailable)?;
    if !metadata.is_file() {
        return Err(PersistentIdentityError::UnsafePermissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = unsafe { libc::geteuid() };
        if metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
            // systemd credentials can be root-owned 0440 with an ACL granting
            // this service access; their protected credential directory limits access.
            let systemd_credential = credential
                && std::env::var_os("CREDENTIALS_DIRECTORY")
                    .and_then(|directory| {
                        let directory = Path::new(&directory).canonicalize().ok()?;
                        if path.parent()?.canonicalize().ok()? != directory {
                            return None;
                        }
                        let parent = directory.metadata().ok()?;
                        Some(
                            (parent.uid() == 0 || parent.uid() == uid)
                                && parent.mode() & 0o027 == 0
                                && (metadata.uid() == 0 || metadata.uid() == uid)
                                && metadata.mode() & 0o227 == 0,
                        )
                    })
                    .unwrap_or(false);
            if !systemd_credential {
                return Err(PersistentIdentityError::UnsafePermissions);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (credential, metadata);
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectionId, CoreConfig, RealtimeCore, RejectAllBackendTokens};
    use mrd_proto::DeviceId;
    use mrd_signal_proto::SignalReplayGuard;
    use ring::rand::SystemRandom;
    use std::sync::Arc;

    fn fixture() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        String,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let key = directory.path().join("identity.pk8");
        let counter = directory.path().join("counter.json");
        let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        std::fs::write(&key, identity.private_pkcs8()).unwrap();
        std::fs::write(
            &counter,
            serde_json::to_vec(&CounterState {
                format_version: 1,
                key_id: identity.key_id().into(),
                reserved_through: 0,
            })
            .unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::set_permissions(&counter, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        (directory, key, counter, identity.key_id().into())
    }

    fn config() -> CoreConfig {
        CoreConfig {
            server_device_id: DeviceId("signal-server".into()),
            challenge_ttl_ms: 10_000,
            presence_ttl_ms: 30_000,
            route_ttl_ms: 60_000,
            max_connections: 8,
            max_messages_per_window: 64,
            rate_window_ms: 1_000,
        }
    }

    #[test]
    fn restarted_core_preserves_identity_and_existing_client_replay_guard() {
        let (_directory, key, counter, key_id) = fixture();
        let mut replay = SignalReplayGuard::new(16, 16);
        let mut previous = None;
        let mut previous_counter = 0;
        let peer = DeviceId("device-peer".into());
        for index in 0..3 {
            let state = PersistentServerState::from_files(&key, &counter).unwrap();
            assert_eq!(state.identity.key_id(), key_id);
            let mut core = RealtimeCore::with_persistent_state(
                config(),
                Arc::new(RejectAllBackendTokens),
                state,
            )
            .unwrap();
            let signed = core
                .sign_registered(
                    peer.clone(),
                    ConnectionId::from_bytes([index + 1; 16]).unwrap(),
                    1_000,
                )
                .unwrap();
            assert!(signed.payload.claims.counter > previous_counter);
            assert_eq!(signed.payload.claims.issuer_key_id, key_id);
            signed.verify_for(&peer, 1_001, &mut replay).unwrap();
            if let Some(old) = previous.as_ref() {
                let old: &mrd_signal_proto::Registered = old;
                assert!(old.verify_for(&peer, 1_001, &mut replay).is_err());
            }
            previous_counter = signed.payload.claims.counter;
            previous = Some(signed);
            drop(core);
        }
    }

    #[test]
    fn counter_has_one_process_owner_and_releases_its_lock_on_shutdown() {
        let (_directory, key, counter, _) = fixture();
        let first = PersistentServerState::from_files(&key, &counter).unwrap();
        assert!(matches!(
            PersistentServerState::from_files(&key, &counter),
            Err(PersistentIdentityError::AlreadyLocked)
        ));
        drop(first);
        let stored: CounterState =
            serde_json::from_slice(&std::fs::read(&counter).unwrap()).unwrap();
        assert_eq!(stored.reserved_through, COUNTER_BLOCK);
        assert!(PersistentServerState::from_files(&key, &counter).is_ok());
    }

    #[test]
    fn missing_corrupt_and_foreign_identity_counters_never_reset() {
        let (_directory, key, counter, _) = fixture();
        for bytes in [
            b"".to_vec(),
            b"{broken".to_vec(),
            br#"{"format_version":1,"key_id":"different","reserved_through":42}"#.to_vec(),
        ] {
            std::fs::write(&counter, &bytes).unwrap();
            assert!(matches!(
                PersistentServerState::from_files(&key, &counter),
                Err(PersistentIdentityError::InvalidState)
            ));
            assert_eq!(std::fs::read(&counter).unwrap(), bytes);
        }
        std::fs::remove_file(&counter).unwrap();
        assert!(matches!(
            PersistentServerState::from_files(&key, &counter),
            Err(PersistentIdentityError::Unavailable)
        ));
        assert!(!counter.exists());
    }

    #[test]
    fn counter_block_refill_is_durable_before_use_and_exhaustion_fails_closed() {
        let (_directory, key, counter, key_id) = fixture();
        let mut state = PersistentServerState::from_files(&key, &counter).unwrap();
        assert_eq!(state.counter.next().unwrap(), 1);
        if let ServerSigningCounter::Persistent { next, .. } = &mut state.counter {
            *next = COUNTER_BLOCK + 1;
        }
        assert_eq!(state.counter.next().unwrap(), COUNTER_BLOCK + 1);
        drop(state);
        let stored: CounterState =
            serde_json::from_slice(&std::fs::read(&counter).unwrap()).unwrap();
        assert_eq!(stored.reserved_through, COUNTER_BLOCK * 2);
        std::fs::write(
            &counter,
            serde_json::to_vec(&CounterState {
                format_version: 1,
                key_id,
                reserved_through: u64::MAX,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            PersistentServerState::from_files(&key, &counter),
            Err(PersistentIdentityError::CounterExhausted)
        ));
    }

    #[test]
    fn openssl_v1_seed_encoding_maps_to_the_same_ed25519_identity() {
        let original = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
        let mut encoded = Zeroizing::new(vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ]);
        encoded.extend_from_slice(&original.private_pkcs8()[16..48]);
        assert_eq!(load_pkcs8(&encoded).unwrap().key_id(), original.key_id());
        encoded[11] = 0x71;
        assert!(load_pkcs8(&encoded).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn generic_readable_or_symlinked_key_and_counter_files_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let (directory, key, counter, _) = fixture();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o440)).unwrap();
        assert!(matches!(
            PersistentServerState::from_files(&key, &counter),
            Err(PersistentIdentityError::UnsafePermissions)
        ));
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&counter, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            PersistentServerState::from_files(&key, &counter),
            Err(PersistentIdentityError::UnsafePermissions)
        ));
        std::fs::set_permissions(&counter, std::fs::Permissions::from_mode(0o600)).unwrap();
        let linked = directory.path().join("linked-key.pk8");
        symlink(&key, &linked).unwrap();
        assert!(PersistentServerState::from_files(&linked, &counter).is_err());
        let linked = directory.path().join("linked-counter.json");
        symlink(&counter, &linked).unwrap();
        assert!(PersistentServerState::from_files(&key, &linked).is_err());
    }
}
