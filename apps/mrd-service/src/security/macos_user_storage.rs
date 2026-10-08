//! User-session persistence protected by the login Keychain and private filesystem modes.

use security_framework::os::macos::keychain::SecKeychain;
use security_framework::random::SecRandom;
use std::{
    ffi::CStr,
    fs::{DirBuilder, OpenOptions},
    io::Write,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};
use zeroize::Zeroizing;

const KEYCHAIN_SERVICE: &str = "com.mini-remote-desktop.service.security-state-v2";
const KEYCHAIN_ACCOUNT: &str = "master-key-v1";
const ITEM_NOT_FOUND: i32 = -25300;
const DUPLICATE_ITEM: i32 = -25299;

/// Resolve the operating system account, without trusting an overridable HOME variable.
pub fn protected_product_data_dir() -> Result<PathBuf, String> {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        return Err("macOS runtime must run in the logged-in user's session".to_owned());
    }
    let mut account = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                &mut account,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if code != 0 || result.is_null() || account.pw_dir.is_null() {
            return Err("macOS account directory could not be resolved".to_owned());
        }
        let bytes = unsafe { CStr::from_ptr(account.pw_dir) }.to_bytes();
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
        if !home.is_absolute() {
            return Err("macOS account directory is not absolute".to_owned());
        }
        return Ok(home.join("Library/Application Support/MiniRemoteDesktop"));
    }
}

pub fn ensure_protected_product_data_dir() -> Result<PathBuf, String> {
    let path = protected_product_data_dir()?;
    verify_ancestors(&path, true)?;
    verify_protected_product_data_dir()
}

pub fn verify_protected_product_data_dir() -> Result<PathBuf, String> {
    let path = protected_product_data_dir()?;
    verify_ancestors(&path, false)?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "protected product directory is missing".to_owned())?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(
            "protected product directory must be owned by the current user with mode 0700"
                .to_owned(),
        );
    }
    for suffix in [
        "security-state-v2.sqlite3",
        "security-state-v2.sqlite3-wal",
        "security-state-v2.sqlite3-shm",
        ".keychain-initialized",
    ] {
        let file = path.join(suffix);
        match std::fs::symlink_metadata(&file) {
            Ok(_) => verify_owner_only_file(&file)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("protected product file could not be inspected".to_owned()),
        }
    }
    Ok(path)
}

fn verify_ancestors(path: &Path, create: bool) -> Result<(), String> {
    let uid = unsafe { libc::geteuid() };
    let mut current = PathBuf::new();
    for component in path.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err("protected product path contains an invalid component".to_owned());
        }
        current.push(component);
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                match DirBuilder::new().mode(0o700).create(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err("protected product directory creation failed".to_owned()),
                }
                std::fs::symlink_metadata(&current)
                    .map_err(|_| "protected product directory verification failed".to_owned())?
            }
            Err(_) => return Err("protected product directory could not be inspected".to_owned()),
        };
        if !metadata.is_dir()
            || (metadata.uid() != 0 && metadata.uid() != uid)
            || metadata.mode() & 0o022 != 0
        {
            return Err("protected product path contains an untrusted directory".to_owned());
        }
    }
    Ok(())
}

pub fn verify_owner_only_file(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "protected product file could not be inspected".to_owned())?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(
            "protected product file must be an owned regular file with mode 0600".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn load_master_key() -> Result<Zeroizing<[u8; 32]>, String> {
    load_master_key_with_interaction(false)
}

pub(super) fn load_master_key_for_user_start() -> Result<Zeroizing<[u8; 32]>, String> {
    load_master_key_with_interaction(true)
}

pub(super) fn authorize_master_key_access() -> Result<(), String> {
    // Used only by the explicit foreground CLI before the service runtime is
    // created. macOS owns the prompt and the user's allow/deny decision.
    drop(load_master_key_with_interaction(true)?);
    Ok(())
}

fn load_master_key_with_interaction(allow_prompt: bool) -> Result<Zeroizing<[u8; 32]>, String> {
    let directory = ensure_protected_product_data_dir()?;
    // Background services must fail promptly when the login Keychain is locked.
    let _interaction = if allow_prompt {
        None
    } else {
        Some(
            SecKeychain::disable_user_interaction()
                .map_err(|_| "macOS Keychain interaction policy could not be set".to_owned())?,
        )
    };
    let keychain =
        SecKeychain::default().map_err(|_| "macOS login Keychain is unavailable".to_owned())?;
    let load = || keychain.find_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
    let bytes = match load() {
        Ok((password, _)) => Zeroizing::new(password.to_vec()),
        Err(error) if error.code() == ITEM_NOT_FOUND => {
            if directory.join(".keychain-initialized").exists()
                || directory.join("security-state-v2.sqlite3").exists()
            {
                return Err("macOS Keychain master key is missing for initialized state".to_owned());
            }
            let mut generated = Zeroizing::new([0_u8; 32]);
            SecRandom::default()
                .copy_bytes(&mut *generated)
                .map_err(|_| "macOS master key entropy unavailable".to_owned())?;
            match keychain.add_generic_password(
                KEYCHAIN_SERVICE,
                KEYCHAIN_ACCOUNT,
                generated.as_ref(),
            ) {
                Ok(_) => {}
                Err(error) if error.code() == DUPLICATE_ITEM => {}
                Err(_) => return Err("macOS Keychain master key creation failed".to_owned()),
            }
            let (password, _) =
                load().map_err(|_| "macOS Keychain master key readback failed".to_owned())?;
            Zeroizing::new(password.to_vec())
        }
        Err(_) => return Err("macOS Keychain access is required; choose Start Backend in Rdesk or run mrd-service --authorize-keychain-and-run in the logged-in user session, then confirm the native prompt".to_owned()),
    };
    if bytes.len() != 32 {
        return Err("macOS Keychain master key is invalid".to_owned());
    }
    let marker = directory.join(".keychain-initialized");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&marker)
    {
        Ok(mut file) => {
            file.write_all(b"keychain-master-v1\n")
                .and_then(|_| file.sync_all())
                .map_err(|_| "macOS Keychain marker persistence failed".to_owned())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_owner_only_file(&marker)?
        }
        Err(_) => return Err("macOS Keychain marker persistence failed".to_owned()),
    }
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "macOS Keychain master key is invalid".to_owned())?;
    Ok(Zeroizing::new(key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn protected_files_reject_open_permissions_hard_links_and_symbolic_links() {
        let directory = std::env::temp_dir().join(format!(
            "mrd-private-file-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        DirBuilder::new().mode(0o700).create(&directory).unwrap();
        let file = directory.join("protected");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
            .unwrap();
        assert!(verify_owner_only_file(&file).is_ok());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(verify_owner_only_file(&file).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let hard_link = directory.join("hard-link");
        std::fs::hard_link(&file, &hard_link).unwrap();
        assert!(verify_owner_only_file(&file).is_err());
        std::fs::remove_file(hard_link).unwrap();
        let symbolic_link = directory.join("symbolic-link");
        std::os::unix::fs::symlink(&file, &symbolic_link).unwrap();
        assert!(verify_owner_only_file(&symbolic_link).is_err());
        assert!(verify_owner_only_file(&directory).is_err());
        std::fs::remove_file(symbolic_link).unwrap();
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
