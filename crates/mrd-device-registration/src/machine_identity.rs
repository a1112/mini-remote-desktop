/// Extract a canonical hardware UUID from the platform expert's property output.
pub fn platform_uuid(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let (property, value) = line.split_once('=')?;
        if property.trim() != "\"IOPlatformUUID\"" {
            return None;
        }
        canonical_uuid(value.trim().trim_matches('"'))
    })
}

pub fn canonical_uuid(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || !bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
        || bytes
            .iter()
            .filter(|byte| **byte != b'-')
            .all(|byte| *byte == b'0')
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

/// Resolve a stable physical device identifier without trusting hostname or UI input.
/// The optional fallback must be a public key ID from an OS-protected persistent store.
pub fn stable_machine_identity(protected_key_id: Option<&str>) -> Result<String, String> {
    #[cfg(windows)]
    let hardware = stable_windows_identity();
    #[cfg(target_os = "macos")]
    let hardware = stable_macos_identity();
    #[cfg(target_os = "linux")]
    let hardware = stable_linux_identity();
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let hardware: Result<String, String> = Err("native device identity is unavailable".to_owned());
    hardware.or_else(|_| protected_installation_identity(protected_key_id))
}

fn protected_installation_identity(key_id: Option<&str>) -> Result<String, String> {
    let key_id = key_id
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| {
            "stable device identity is unavailable; protected installation identity is required"
                .to_owned()
        })?;
    Ok(format!("mrd-machine-key:{}", key_id.to_ascii_lowercase()))
}

#[cfg(windows)]
fn stable_windows_identity() -> Result<String, String> {
    use winreg::{
        enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY},
        RegKey,
    };
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            r"SOFTWARE\Microsoft\Cryptography",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .map_err(|_| "Windows machine identity registry key is unavailable".to_owned())?;
    let value: String = key
        .get_value("MachineGuid")
        .map_err(|_| "Windows machine identity registry value is unavailable".to_owned())?;
    let uuid = canonical_uuid(value.trim())
        .ok_or_else(|| "Windows machine identity registry value is invalid".to_owned())?;
    Ok(format!("windows-{uuid}"))
}

#[cfg(target_os = "linux")]
fn stable_linux_identity() -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;
    let path = std::path::Path::new("/etc/machine-id");
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|_| "Linux machine identity is unavailable".to_owned())?;
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || metadata.len() > 64
    {
        return Err("Linux machine identity file is untrusted".to_owned());
    }
    let value = std::fs::read_to_string(path)
        .map_err(|_| "Linux machine identity cannot be read".to_owned())?;
    let value = value.trim();
    if value.len() != 32
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err("Linux machine identity is invalid".to_owned());
    }
    Ok(format!("linux-{}", value.to_ascii_lowercase()))
}

#[cfg(target_os = "macos")]
pub fn stable_macos_identity() -> Result<String, String> {
    use security_framework::os::macos::keychain::SecKeychain;
    use std::process::Command;
    const SERVICE: &str = "com.mini-remote-desktop.device-installation-v1";
    const ACCOUNT: &str = "installation-identity";
    const NOT_FOUND: i32 = -25300;
    const DUPLICATE: i32 = -25299;
    // The physical UUID is available to both the shell and the user service.
    // Protected registrations persist their first serial independently, so
    // refreshing credentials never follows a later hardware identity change.
    if let Some(uuid) = Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| platform_uuid(&output))
    {
        return Ok(format!("macos-{uuid}"));
    }
    let _interaction = SecKeychain::disable_user_interaction()
        .map_err(|_| "macOS Keychain interaction policy unavailable".to_owned())?;
    let keychain =
        SecKeychain::default().map_err(|_| "macOS login Keychain unavailable".to_owned())?;
    let load = || keychain.find_generic_password(SERVICE, ACCOUNT);
    match load() {
        Ok((password, _)) => return validate_saved_identity(&password),
        Err(error) if error.code() == NOT_FOUND => {}
        Err(_) => return Err("macOS installation identity cannot be accessed".to_owned()),
    }
    let uuid = {
        Command::new("/usr/bin/uuidgen")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|output| canonical_uuid(output.trim()))
    }
    .ok_or_else(|| "macOS installation identity could not be generated".to_owned())?;
    let identity = format!("macos-{uuid}");
    // Creation is atomic: a concurrent registration can never overwrite the
    // installation identity that another process has already persisted.
    match keychain.add_generic_password(SERVICE, ACCOUNT, identity.as_bytes()) {
        Ok(_) => {}
        Err(error) if error.code() == DUPLICATE => {}
        Err(_) => return Err("macOS installation identity persistence failed".to_owned()),
    }
    let (password, _) =
        load().map_err(|_| "macOS installation identity readback failed".to_owned())?;
    validate_saved_identity(&password)
}

#[cfg(target_os = "macos")]
fn validate_saved_identity(bytes: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| "macOS installation identity is invalid".to_owned())?;
    let uuid = value
        .strip_prefix("macos-")
        .and_then(canonical_uuid)
        .ok_or_else(|| "macOS installation identity is invalid".to_owned())?;
    Ok(format!("macos-{uuid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_fallback_requires_a_valid_protected_public_key_id() {
        assert!(protected_installation_identity(None).is_err());
        assert!(protected_installation_identity(Some("hostname")).is_err());
        assert!(protected_installation_identity(Some("../secret")).is_err());
        let key = "A".repeat(64);
        assert_eq!(
            protected_installation_identity(Some(&key)).unwrap(),
            format!("mrd-machine-key:{}", "a".repeat(64))
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_machine_identity_is_stable_and_independent_of_hostname() {
        let first = stable_machine_identity(None).expect("OS MachineGuid");
        assert!(first.starts_with("windows-"));
        assert!(
            stable_machine_identity(None).unwrap() == first,
            "OS device identity changed between reads"
        );
        assert!(
            stable_machine_identity(Some(&"f".repeat(64))).unwrap() == first,
            "A protected fallback replaced an available OS device identity"
        );
    }

    #[test]
    fn hardware_uuid_is_independent_of_hostname_and_output_whitespace() {
        let first = r#"  "IOPlatformUUID" = "D43DA8ED-33EB-41DF-8E08-B6FC4CB18A59" "#;
        let renamed = r#""IOPlatformSerialNumber" = "My-Mac"
          "IOPlatformUUID" = "d43da8ed-33eb-41df-8e08-b6fc4cb18a59""#;
        assert_eq!(
            platform_uuid(first),
            Some("d43da8ed-33eb-41df-8e08-b6fc4cb18a59".to_owned())
        );
        assert_eq!(platform_uuid(first), platform_uuid(renamed));
    }

    #[test]
    fn malformed_nil_and_unrelated_identifiers_are_rejected() {
        for bad in [
            "MacBook",
            "00000000-0000-0000-0000-000000000000",
            "../D43DA8ED-33EB-41DF-8E08-B6FC4CB18A59",
            "D43DA8ED_33EB_41DF_8E08_B6FC4CB18A59",
        ] {
            assert!(canonical_uuid(bad).is_none());
        }
        assert!(platform_uuid(
            r#""IOPlatformSerialNumber" = "D43DA8ED-33EB-41DF-8E08-B6FC4CB18A59""#
        )
        .is_none());
    }
}
