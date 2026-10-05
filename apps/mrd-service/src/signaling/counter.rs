//! Service-owned signing counters, encrypted independently of device credentials.

use mrd_store_sqlite::SecretProtector;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use thiserror::Error;
use zeroize::Zeroizing;

const COUNTER_PURPOSE: &[u8] = b"MRD_CLIENT_SIGNAL_COUNTER_V1\0";
const MARKER_PURPOSE: &[u8] = b"MRD_CLIENT_SIGNAL_COUNTER_INITIALIZED_V1\0";
const RESERVATION_SIZE: u64 = 1_048_576;
// Existing clients used small, in-memory counters. Start the first durable
// reservation above the entire practicable lifetime of that older sequence.
const LEGACY_MIGRATION_FLOOR: u64 = 1 << 48;
const MAX_PROTECTED_BYTES: u64 = 4_096;

#[derive(Debug, Error)]
pub enum SignalingCounterError {
    #[error("protected signaling counter configuration is invalid")]
    InvalidConfiguration,
    #[error("protected signaling counter is unavailable")]
    Unavailable,
    #[error("protected signaling counter permissions are unsafe")]
    UnsafePermissions,
    #[error("protected signaling counter is owned by another process")]
    AlreadyLocked,
    #[error("protected signaling counter is malformed or belongs to another identity")]
    InvalidState,
    #[error("protected signaling counter is exhausted")]
    Exhausted,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterState {
    format_version: u8,
    key_id: String,
    reserved_through: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InitializationMarker {
    format_version: u8,
    key_id: String,
}

struct Reservation {
    file: File,
    state: CounterState,
    next: u64,
}

/// One exclusively locked, durable sequence shared by all connections of a key.
/// Counter reservations are synced before any number in the block can be used.
pub struct PersistentSignalingCounter {
    key_id: String,
    purpose: Vec<u8>,
    protector: Arc<dyn SecretProtector>,
    reservation: Mutex<Reservation>,
}

impl PersistentSignalingCounter {
    /// The caller supplies the protected product directory and OS protector used
    /// for the machine identity. The SQLite security schema remains independent.
    pub fn open(
        directory: &Path,
        protector: Arc<dyn SecretProtector>,
        key_id: &str,
    ) -> Result<Self, SignalingCounterError> {
        if !directory.is_absolute()
            || key_id.is_empty()
            || key_id.len() > 256
            || !key_id.is_ascii()
            || key_id.chars().any(char::is_control)
        {
            return Err(SignalingCounterError::InvalidConfiguration);
        }
        verify_directory(directory)?;
        let (counter_path, marker_path) = state_paths(directory, key_id);
        let counter_exists = exists(&counter_path)?;
        let marker_exists = exists(&marker_path)?;
        // A marker distinguishes first migration from a lost counter. Neither
        // a missing counter nor a missing marker can silently reset a sequence.
        if counter_exists != marker_exists {
            return Err(SignalingCounterError::InvalidState);
        }
        let purpose = purpose_for(COUNTER_PURPOSE, key_id);
        let marker_purpose = purpose_for(MARKER_PURPOSE, key_id);
        let mut file = open_private(&counter_path, true, !counter_exists)?;
        file.try_lock()
            .map_err(|_| SignalingCounterError::AlreadyLocked)?;
        let state = if counter_exists {
            let mut marker = open_private(&marker_path, false, false)?;
            let initialized: InitializationMarker =
                read_protected(&mut marker, protector.as_ref(), &marker_purpose)?;
            if initialized.format_version != 1 || initialized.key_id != key_id {
                return Err(SignalingCounterError::InvalidState);
            }
            let state: CounterState = read_protected(&mut file, protector.as_ref(), &purpose)?;
            if state.format_version != 1
                || state.key_id != key_id
                || state.reserved_through < LEGACY_MIGRATION_FLOOR
            {
                return Err(SignalingCounterError::InvalidState);
            }
            state
        } else {
            let mut marker = open_private(&marker_path, true, true)?;
            write_protected(
                &mut marker,
                protector.as_ref(),
                &marker_purpose,
                &InitializationMarker {
                    format_version: 1,
                    key_id: key_id.into(),
                },
            )?;
            let state = CounterState {
                format_version: 1,
                key_id: key_id.into(),
                reserved_through: LEGACY_MIGRATION_FLOOR,
            };
            write_protected(&mut file, protector.as_ref(), &purpose, &state)?;
            // Make the initialization marker and counter entries durable too.
            #[cfg(unix)]
            File::open(directory)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| SignalingCounterError::Unavailable)?;
            state
        };
        let next = state
            .reserved_through
            .checked_add(1)
            .ok_or(SignalingCounterError::Exhausted)?;
        let mut reservation = Reservation { file, state, next };
        reserve(&mut reservation, protector.as_ref(), &purpose)?;
        Ok(Self {
            key_id: key_id.into(),
            purpose,
            protector,
            reservation: Mutex::new(reservation),
        })
    }

    pub(crate) fn key_id(&self) -> &str {
        &self.key_id
    }

    pub(crate) fn next(&self) -> Result<u64, SignalingCounterError> {
        let mut reservation = self
            .reservation
            .lock()
            .map_err(|_| SignalingCounterError::Unavailable)?;
        if reservation.next > reservation.state.reserved_through {
            reserve(&mut reservation, self.protector.as_ref(), &self.purpose)?;
        }
        let result = reservation.next;
        reservation.next = result
            .checked_add(1)
            .ok_or(SignalingCounterError::Exhausted)?;
        Ok(result)
    }
}

fn reserve(
    reservation: &mut Reservation,
    protector: &dyn SecretProtector,
    purpose: &[u8],
) -> Result<(), SignalingCounterError> {
    let reserved_through = reservation
        .state
        .reserved_through
        .checked_add(RESERVATION_SIZE)
        .ok_or(SignalingCounterError::Exhausted)?;
    let new_state = CounterState {
        format_version: 1,
        key_id: reservation.state.key_id.clone(),
        reserved_through,
    };
    write_protected(&mut reservation.file, protector, purpose, &new_state)?;
    reservation.state = new_state;
    Ok(())
}

fn purpose_for(prefix: &[u8], key_id: &str) -> Vec<u8> {
    let mut purpose = prefix.to_vec();
    purpose.extend_from_slice(key_id.as_bytes());
    purpose
}

fn state_paths(directory: &Path, key_id: &str) -> (PathBuf, PathBuf) {
    let digest = ring::digest::digest(&ring::digest::SHA256, key_id.as_bytes());
    let suffix: String = digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (
        directory.join(format!("signaling-counter-v1-{suffix}.protected")),
        directory.join(format!("signaling-counter-v1-{suffix}.initialized")),
    )
}

fn exists(path: &Path) -> Result<bool, SignalingCounterError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(SignalingCounterError::Unavailable),
    }
}

fn verify_directory(path: &Path) -> Result<(), SignalingCounterError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| SignalingCounterError::Unavailable)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(SignalingCounterError::UnsafePermissions);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
            return Err(SignalingCounterError::UnsafePermissions);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(SignalingCounterError::UnsafePermissions);
        }
    }
    Ok(())
}

fn open_private(path: &Path, write: bool, create: bool) -> Result<File, SignalingCounterError> {
    let mut options = OpenOptions::new();
    options.read(true).write(write).create_new(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options
        .open(path)
        .map_err(|_| SignalingCounterError::Unavailable)?;
    let metadata = file
        .metadata()
        .map_err(|_| SignalingCounterError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > MAX_PROTECTED_BYTES {
        return Err(SignalingCounterError::InvalidState);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(SignalingCounterError::UnsafePermissions);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(SignalingCounterError::UnsafePermissions);
        }
    }
    Ok(file)
}

fn read_protected<T: serde::de::DeserializeOwned>(
    file: &mut File,
    protector: &dyn SecretProtector,
    purpose: &[u8],
) -> Result<T, SignalingCounterError> {
    let mut protected = Vec::new();
    Read::by_ref(file)
        .take(MAX_PROTECTED_BYTES + 1)
        .read_to_end(&mut protected)
        .map_err(|_| SignalingCounterError::Unavailable)?;
    if protected.len() as u64 > MAX_PROTECTED_BYTES {
        return Err(SignalingCounterError::InvalidState);
    }
    let plaintext = protector
        .unprotect(purpose, &protected)
        .map_err(|_| SignalingCounterError::InvalidState)?;
    serde_json::from_slice(plaintext.as_ref()).map_err(|_| SignalingCounterError::InvalidState)
}

fn write_protected<T: Serialize>(
    file: &mut File,
    protector: &dyn SecretProtector,
    purpose: &[u8],
    value: &T,
) -> Result<(), SignalingCounterError> {
    let plaintext =
        Zeroizing::new(serde_json::to_vec(value).map_err(|_| SignalingCounterError::InvalidState)?);
    let protected = protector
        .protect(purpose, &plaintext)
        .map_err(|_| SignalingCounterError::Unavailable)?;
    if protected.is_empty() || protected.len() as u64 > MAX_PROTECTED_BYTES {
        return Err(SignalingCounterError::InvalidState);
    }
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.set_len(0))
        .and_then(|_| file.write_all(&protected))
        .and_then(|_| file.sync_all())
        .map_err(|_| SignalingCounterError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_store_sqlite::AeadSecretProtector;

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            use ring::rand::SecureRandom;
            let mut random = [0u8; 16];
            ring::rand::SystemRandom::new().fill(&mut random).unwrap();
            let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let path = std::env::temp_dir().join(format!("mrd-signing-counter-{suffix}"));
            std::fs::create_dir(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn protector() -> Arc<dyn SecretProtector> {
        Arc::new(AeadSecretProtector::from_key([92; 32]).unwrap())
    }

    #[test]
    fn restart_skips_the_entire_reserved_block_and_exclusive_lock_is_released() {
        let directory = TestDirectory::new();
        let key = "counter-machine-key";
        let first = PersistentSignalingCounter::open(&directory.0, protector(), key).unwrap();
        assert_eq!(first.next().unwrap(), LEGACY_MIGRATION_FLOOR + 1);
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector(), key),
            Err(SignalingCounterError::AlreadyLocked)
        ));
        drop(first);
        let second = PersistentSignalingCounter::open(&directory.0, protector(), key).unwrap();
        assert_eq!(
            second.next().unwrap(),
            LEGACY_MIGRATION_FLOOR + RESERVATION_SIZE + 1
        );
    }

    #[test]
    fn encrypted_counter_rejects_tampering_foreign_purpose_and_missing_files() {
        let directory = TestDirectory::new();
        let key = "counter-machine-key";
        let protector = protector();
        let counter =
            PersistentSignalingCounter::open(&directory.0, protector.clone(), key).unwrap();
        drop(counter);
        let (path, marker) = state_paths(&directory.0, key);
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(!bytes
            .windows(key.len())
            .any(|window| window == key.as_bytes()));
        assert!(protector
            .unprotect(
                &purpose_for(b"MRD_PUBLIC_DEVICE_CREDENTIAL_V1\0", key),
                &bytes
            )
            .is_err());
        assert!(protector
            .unprotect(&purpose_for(MARKER_PURPOSE, key), &bytes)
            .is_err());
        let original = bytes.clone();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector.clone(), key),
            Err(SignalingCounterError::InvalidState)
        ));
        std::fs::write(&path, original).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector.clone(), key),
            Err(SignalingCounterError::InvalidState)
        ));
        assert!(marker.exists());
    }

    #[test]
    fn initialization_marker_is_authenticated_and_mandatory_after_migration() {
        let directory = TestDirectory::new();
        let key = "counter-machine-key";
        let protector = protector();
        drop(PersistentSignalingCounter::open(&directory.0, protector.clone(), key).unwrap());
        let (path, marker) = state_paths(&directory.0, key);
        let original = std::fs::read(&marker).unwrap();
        std::fs::write(&marker, std::fs::read(&path).unwrap()).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector.clone(), key),
            Err(SignalingCounterError::InvalidState)
        ));
        std::fs::write(&marker, original).unwrap();
        std::fs::remove_file(&marker).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector, key),
            Err(SignalingCounterError::InvalidState)
        ));
    }

    #[test]
    fn shared_threads_allocate_unique_numbers_and_refill_is_durable() {
        let directory = TestDirectory::new();
        let key = "counter-machine-key";
        let counter =
            Arc::new(PersistentSignalingCounter::open(&directory.0, protector(), key).unwrap());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let counter = counter.clone();
                std::thread::spawn(move || {
                    (0..128)
                        .map(|_| counter.next().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut values: Vec<_> = threads
            .into_iter()
            .flat_map(|thread| thread.join().unwrap())
            .collect();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), 1024);
        assert_eq!(values[0], LEGACY_MIGRATION_FLOOR + 1);
        {
            let mut state = counter.reservation.lock().unwrap();
            state.next = state.state.reserved_through;
        }
        let boundary = counter.next().unwrap();
        assert_eq!(counter.next().unwrap(), boundary + 1);
        drop(counter);
        let restarted = PersistentSignalingCounter::open(&directory.0, protector(), key).unwrap();
        assert_eq!(
            restarted.next().unwrap(),
            LEGACY_MIGRATION_FLOOR + 2 * RESERVATION_SIZE + 1
        );
        {
            let mut state = restarted.reservation.lock().unwrap();
            state.state.reserved_through = u64::MAX;
            state.next = u64::MAX;
        }
        assert!(matches!(
            restarted.next(),
            Err(SignalingCounterError::Exhausted)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn counter_rejects_open_permissions_symlinks_and_hard_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let directory = TestDirectory::new();
        let key = "counter-machine-key";
        drop(PersistentSignalingCounter::open(&directory.0, protector(), key).unwrap());
        let (path, _) = state_paths(&directory.0, key);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector(), key),
            Err(SignalingCounterError::UnsafePermissions)
        ));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let alias = directory.0.join("linked-counter");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&directory.0, protector(), key),
            Err(SignalingCounterError::UnsafePermissions)
        ));
        std::fs::remove_file(&alias).unwrap();
        std::fs::rename(&path, &alias).unwrap();
        symlink(&alias, &path).unwrap();
        assert!(PersistentSignalingCounter::open(&directory.0, protector(), key).is_err());
        let linked_directory = directory.0.join("linked-directory");
        symlink(&directory.0, &linked_directory).unwrap();
        assert!(matches!(
            PersistentSignalingCounter::open(&linked_directory, protector(), key),
            Err(SignalingCounterError::UnsafePermissions)
        ));
    }
}
