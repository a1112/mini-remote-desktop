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
        || [b'0', b'f'].iter().any(|placeholder| {
            bytes
                .iter()
                .filter(|byte| **byte != b'-')
                .all(|byte| byte.to_ascii_lowercase() == *placeholder)
        })
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

/// Firmware sometimes reports the field label or a vendor placeholder as a serial.
/// Keep only bounded, printable serials with enough content to identify a board.
#[cfg(any(windows, target_os = "linux", test))]
fn canonical_board_serial(value: &str) -> Option<String> {
    let value = value.trim();
    if !(3..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return None;
    }
    let normalized: String = value
        .bytes()
        .filter(u8::is_ascii_alphanumeric)
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect();
    if normalized.len() < 3
        || normalized
            .bytes()
            .all(|byte| byte == normalized.as_bytes()[0])
        || matches!(
            normalized.as_str(),
            "tobefilledbyoem"
                | "defaultstring"
                | "systemserialnumber"
                | "baseboardserialnumber"
                | "boardserialnumber"
                | "chassisserialnumber"
                | "serialnumber"
                | "unknown"
                | "none"
                | "notapplicable"
                | "notavailable"
                | "notspecified"
                | "notpresent"
                | "noserialnumber"
                | "invalid"
                | "oem"
                | "null"
                | "default"
                | "undefined"
                | "0123456789"
                | "123456789"
                | "1234567890"
        )
    {
        return None;
    }
    Some(value.to_ascii_uppercase())
}

#[cfg(any(windows, target_os = "linux", test))]
fn board_machine_identity(platform: &str, serial: &str) -> String {
    let identity = format!("{platform}-board-{serial}");
    if identity.len() <= 128 {
        return identity;
    }
    // Preserve the entire serial. The colon-delimited hash namespace cannot
    // collide with a literal serial in the existing `platform-board-` namespace.
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    digest.update(b"mrd-machine-identity/board-serial/v1\0");
    digest.update(platform.as_bytes());
    digest.update(&[0]);
    digest.update(serial.as_bytes());
    let hex: String = digest
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{platform}-board:sha256:{hex}")
}

#[cfg(any(windows, target_os = "linux", test))]
fn prefer_hardware_identity(
    hardware: Option<String>,
    os_identity: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    hardware.map_or_else(os_identity, Ok)
}

/// Parse the RawSMBIOSData returned by the Windows RSMB firmware provider.
/// All bounds are checked before reading fields or the trailing string table.
#[cfg(any(windows, test))]
fn smbios_hardware_identity(raw: &[u8]) -> Option<String> {
    const RAW_HEADER_LEN: usize = 8;
    let length = u32::from_le_bytes(raw.get(4..8)?.try_into().ok()?) as usize;
    let table = raw.get(RAW_HEADER_LEN..RAW_HEADER_LEN.checked_add(length)?)?;
    let uuid_is_little_endian = (raw[1], raw[2]) >= (2, 6);
    let mut offset = 0;
    let mut uuid = None;
    let mut board = None;
    while offset < table.len() {
        let header = table.get(offset..offset.checked_add(4)?)?;
        let kind = header[0];
        let formatted_length = usize::from(header[1]);
        if formatted_length < 4 {
            return None;
        }
        let formatted_end = offset.checked_add(formatted_length)?;
        let formatted = table.get(offset..formatted_end)?;
        let strings_end = table
            .get(formatted_end..)?
            .windows(2)
            .position(|s| s == [0, 0])?;
        let strings = table.get(formatted_end..formatted_end.checked_add(strings_end)?)?;
        match kind {
            1 if uuid.is_none() && formatted.len() >= 25 => {
                let bytes: [u8; 16] = formatted[8..24].try_into().ok()?;
                uuid = smbios_uuid(bytes, uuid_is_little_endian);
            }
            2 if board.is_none() && formatted.len() >= 8 => {
                let index = usize::from(formatted[7]);
                if index > 0 {
                    board = strings
                        .split(|byte| *byte == 0)
                        .nth(index - 1)
                        .and_then(|serial| std::str::from_utf8(serial).ok())
                        .and_then(canonical_board_serial);
                }
            }
            _ => {}
        }
        if kind == 127 {
            break;
        }
        offset = formatted_end.checked_add(strings_end)?.checked_add(2)?;
    }
    uuid.map(|uuid| format!("windows-hardware-{uuid}"))
        .or_else(|| board.map(|serial| board_machine_identity("windows", &serial)))
}

#[cfg(any(windows, test))]
fn smbios_uuid(mut bytes: [u8; 16], little_endian: bool) -> Option<String> {
    // SMBIOS 2.6 standardized little-endian encoding for the first three fields.
    if little_endian {
        bytes[0..4].reverse();
        bytes[4..6].reverse();
        bytes[6..8].reverse();
    }
    canonical_uuid(&format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    ))
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

/// A persisted hardware registration remains valid only on its original device.
/// Legacy OS/installation identities retain their existing serial and code.
pub fn validate_saved_machine_identity(
    saved_serial: &str,
    protected_key_id: Option<&str>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if saved_serial.starts_with("macos-") {
        return validate_saved_macos_identity_binding(
            saved_serial,
            macos_hardware_identity,
            existing_macos_installation_identity,
        );
    }
    validate_saved_identity_binding(saved_serial, || stable_machine_identity(protected_key_id))
}

fn validate_saved_identity_binding(
    saved_serial: &str,
    current_identity: impl FnOnce() -> Result<String, String>,
) -> Result<(), String> {
    let hardware_bound = [
        "macos-",
        "windows-hardware-",
        "windows-board-",
        "windows-board:sha256:",
        "linux-hardware-",
        "linux-board-",
        "linux-board:sha256:",
    ]
    .iter()
    .any(|prefix| saved_serial.starts_with(prefix));
    if !hardware_bound {
        return Ok(());
    }
    match current_identity() {
        Ok(current) if current == saved_serial => Ok(()),
        _ => {
            Err("saved device registration belongs to different or unavailable hardware".to_owned())
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn validate_saved_macos_identity_binding(
    saved_serial: &str,
    hardware_identity: impl FnOnce() -> Result<String, String>,
    installation_identity: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<(), String> {
    let failure =
        || "saved device registration belongs to different or unavailable hardware".to_owned();
    if !saved_serial.starts_with("macos-installation-") {
        if hardware_identity().is_ok_and(|current| current == saved_serial) {
            return Ok(());
        }
    }
    // Older releases put randomly generated Keychain UUIDs in the hardware
    // namespace. Recognize those only by reading the preexisting protected item.
    // Never create an identity, migrate the item, or prompt during validation.
    let saved_fallback = macos_installation_uuid(saved_serial).ok_or_else(failure)?;
    match installation_identity() {
        Ok(Some(current))
            if macos_installation_uuid(&current).as_deref() == Some(saved_fallback.as_str()) =>
        {
            Ok(())
        }
        _ => Err(failure()),
    }
}

#[cfg(any(target_os = "macos", test))]
fn macos_installation_uuid(value: &str) -> Option<String> {
    value
        .strip_prefix("macos-installation-")
        .or_else(|| value.strip_prefix("macos-"))
        .and_then(canonical_uuid)
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
    prefer_hardware_identity(windows_firmware_identity(), windows_os_identity)
}

#[cfg(windows)]
fn windows_firmware_identity() -> Option<String> {
    use std::ffi::c_void;
    // This stable kernel32 API returns firmware SMBIOS data directly, without
    // spawning WMI/PowerShell or depending on their installation and timeouts.
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemFirmwareTable(
            provider: u32,
            table: u32,
            buffer: *mut c_void,
            buffer_size: u32,
        ) -> u32;
    }
    const PROVIDER: u32 = u32::from_be_bytes(*b"RSMB");
    const MAX_TABLE_SIZE: u32 = 1024 * 1024;
    // SAFETY: the null/zero call is the API's documented buffer-size query.
    let size = unsafe { GetSystemFirmwareTable(PROVIDER, 0, std::ptr::null_mut(), 0) };
    if !(8..=MAX_TABLE_SIZE).contains(&size) {
        return None;
    }
    let mut raw = vec![0u8; size as usize];
    // SAFETY: the initialized buffer has `size` bytes and remains allocated
    // through the synchronous call. A changed firmware size is rejected below.
    let written =
        unsafe { GetSystemFirmwareTable(PROVIDER, 0, raw.as_mut_ptr().cast::<c_void>(), size) };
    if written == 0 || written > size {
        return None;
    }
    smbios_hardware_identity(&raw[..written as usize])
}

#[cfg(windows)]
fn windows_os_identity() -> Result<String, String> {
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
    prefer_hardware_identity(linux_hardware_identity(), linux_os_identity)
}

#[cfg(target_os = "linux")]
fn linux_hardware_identity() -> Option<String> {
    // /sys/class/dmi/id is normally a symlink into /sys/devices. Resolve that
    // kernel-provided path, while rejecting redirects outside the sysfs tree.
    trusted_linux_dmi_value("product_uuid")
        .and_then(|value| canonical_uuid(value.trim()))
        .map(|uuid| format!("linux-hardware-{uuid}"))
        .or_else(|| {
            trusted_linux_dmi_value("board_serial")
                .and_then(|value| canonical_board_serial(&value))
                .map(|serial| board_machine_identity("linux", &serial))
        })
}

#[cfg(any(target_os = "linux", all(test, unix)))]
fn trusted_linux_dmi_value(name: &str) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;
    let path = std::fs::canonicalize(std::path::Path::new("/sys/class/dmi/id").join(name)).ok()?;
    if !path.starts_with("/sys/devices") {
        return None;
    }
    // The resolved file and every parent below /sys must be root-owned and
    // protected against writes by other users. Normal sysfs symlinks are allowed
    // only through the canonical path above, never in the final open path.
    for ancestor in path
        .ancestors()
        .take_while(|path| *path != std::path::Path::new("/"))
    {
        let metadata = std::fs::symlink_metadata(ancestor).ok()?;
        if metadata.file_type().is_symlink()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || (ancestor == path && !metadata.is_file())
            || (ancestor != path && !metadata.is_dir())
        {
            return None;
        }
    }
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return None;
    }
    // sysfs metadata often reports a page size instead of the text's length.
    let mut value = String::new();
    file.take(4097).read_to_string(&mut value).ok()?;
    (value.len() <= 4096).then_some(value)
}

#[cfg(target_os = "linux")]
fn linux_os_identity() -> Result<String, String> {
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
const MACOS_INSTALLATION_SERVICE: &str = "com.mini-remote-desktop.device-installation-v1";
#[cfg(target_os = "macos")]
const MACOS_INSTALLATION_ACCOUNT: &str = "installation-identity";

#[cfg(target_os = "macos")]
fn macos_hardware_identity() -> Result<String, String> {
    use std::process::Command;
    Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| platform_uuid(&output))
        .map(|uuid| format!("macos-{uuid}"))
        .ok_or_else(|| "macOS hardware identity is unavailable".to_owned())
}

#[cfg(target_os = "macos")]
fn existing_macos_installation_identity() -> Result<Option<String>, String> {
    use security_framework::os::macos::keychain::SecKeychain;
    const NOT_FOUND: i32 = -25300;
    let _interaction = SecKeychain::disable_user_interaction()
        .map_err(|_| "macOS installation identity cannot be accessed".to_owned())?;
    let keychain = SecKeychain::default()
        .map_err(|_| "macOS installation identity cannot be accessed".to_owned())?;
    match keychain.find_generic_password(MACOS_INSTALLATION_SERVICE, MACOS_INSTALLATION_ACCOUNT) {
        Ok((password, _)) => validate_saved_identity(&password).map(Some),
        Err(error) if error.code() == NOT_FOUND => Ok(None),
        Err(_) => Err("macOS installation identity cannot be accessed".to_owned()),
    }
}

#[cfg(target_os = "macos")]
pub fn stable_macos_identity() -> Result<String, String> {
    use security_framework::os::macos::keychain::SecKeychain;
    use std::process::Command;
    const NOT_FOUND: i32 = -25300;
    const DUPLICATE: i32 = -25299;
    if let Ok(hardware) = macos_hardware_identity() {
        return Ok(hardware);
    }
    let _interaction = SecKeychain::disable_user_interaction()
        .map_err(|_| "macOS Keychain interaction policy unavailable".to_owned())?;
    let keychain =
        SecKeychain::default().map_err(|_| "macOS login Keychain unavailable".to_owned())?;
    let load =
        || keychain.find_generic_password(MACOS_INSTALLATION_SERVICE, MACOS_INSTALLATION_ACCOUNT);
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
    let identity = format!("macos-installation-{uuid}");
    // Creation is atomic: a concurrent registration can never overwrite the
    // installation identity that another process has already persisted.
    match keychain.add_generic_password(
        MACOS_INSTALLATION_SERVICE,
        MACOS_INSTALLATION_ACCOUNT,
        identity.as_bytes(),
    ) {
        Ok(_) => {}
        Err(error) if error.code() == DUPLICATE => {}
        Err(_) => return Err("macOS installation identity persistence failed".to_owned()),
    }
    let (password, _) =
        load().map_err(|_| "macOS installation identity readback failed".to_owned())?;
    validate_saved_identity(&password)
}

#[cfg(any(target_os = "macos", test))]
fn validate_saved_identity(bytes: &[u8]) -> Result<String, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|_| "macOS installation identity is invalid".to_owned())?;
    let uuid = macos_installation_uuid(value)
        .ok_or_else(|| "macOS installation identity is invalid".to_owned())?;
    // Registration must retain the exact legacy namespace already bound to its
    // signing key. Normalize legacy/new namespaces only for binding comparisons.
    let namespace = if value.starts_with("macos-installation-") {
        "macos-installation-"
    } else {
        "macos-"
    };
    Ok(format!("{namespace}{uuid}"))
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
        let first = stable_machine_identity(None).expect("firmware identity or OS MachineGuid");
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
            "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
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

    #[test]
    fn board_serial_rejects_vendor_placeholders_and_canonicalizes_real_values() {
        for serial in [
            "",
            "To Be Filled By O.E.M.",
            "Default string",
            "System Serial Number",
            "NOT AVAILABLE",
            "Unknown",
            "0000000000",
            "FFFFFFFF",
            "1234567890",
            "ABC\nDEF",
        ] {
            assert!(
                canonical_board_serial(serial).is_none(),
                "accepted placeholder"
            );
        }
        assert_eq!(
            canonical_board_serial("  Board-7a92  "),
            Some("BOARD-7A92".into())
        );
    }

    #[test]
    fn board_identity_preserves_literal_ids_until_the_backend_length_boundary() {
        for (platform, max_literal_serial) in [("windows", 114), ("linux", 116)] {
            let serial = "AB".repeat(max_literal_serial / 2);
            let identity = board_machine_identity(platform, &serial);
            assert_eq!(identity, format!("{platform}-board-{serial}"));
            assert_eq!(identity.len(), 128);
            for length in max_literal_serial + 1..=128 {
                let serial: String = "AB".repeat(64).chars().take(length).collect();
                let identity = board_machine_identity(platform, &serial);
                assert!(identity.starts_with(&format!("{platform}-board:sha256:")));
                assert!(!identity.starts_with(&format!("{platform}-board-")));
                assert!(identity.len() <= 128);
            }
        }
    }

    #[test]
    fn long_board_identities_hash_the_entire_serial_with_platform_domain_separation() {
        let first = format!("{}CD", "AB".repeat(63));
        let second = format!("{}CE", "AB".repeat(63));
        let windows = board_machine_identity("windows", &first);
        assert_eq!(windows, board_machine_identity("windows", &first));
        assert_ne!(windows, board_machine_identity("windows", &second));
        assert_ne!(
            windows.rsplit(':').next(),
            board_machine_identity("linux", &first).rsplit(':').next()
        );
    }

    #[test]
    fn available_hardware_has_priority_over_the_os_installation_identity() {
        let actual = prefer_hardware_identity(Some("physical-test-board".into()), || {
            panic!("must not query OS identity when firmware identity is available")
        });
        assert_eq!(actual.unwrap(), "physical-test-board");
        assert_eq!(
            prefer_hardware_identity(None, || Ok("os-installation-test".into())).unwrap(),
            "os-installation-test"
        );
        assert!(prefer_hardware_identity(None, || Err("unavailable".into())).is_err());
    }

    #[test]
    fn saved_hardware_registration_is_valid_only_on_the_original_machine() {
        for prefix in [
            "macos-",
            "windows-hardware-",
            "windows-board-",
            "windows-board:sha256:",
            "linux-hardware-",
            "linux-board-",
            "linux-board:sha256:",
        ] {
            let saved = format!("{prefix}fixture-original");
            assert!(validate_saved_identity_binding(&saved, || Ok(saved.clone())).is_ok());
            for current in [
                format!("{prefix}fixture-another"),
                "windows-fixture-os-identity".to_owned(),
                "linux-fixture-os-identity".to_owned(),
                format!("mrd-machine-key:{}", "a".repeat(64)),
            ] {
                assert!(validate_saved_identity_binding(&saved, || Ok(current)).is_err());
            }
            assert!(
                validate_saved_identity_binding(&saved, || Err("probe unavailable".into()))
                    .is_err()
            );
        }
    }

    #[test]
    fn saved_legacy_os_and_installation_registrations_keep_their_identity() {
        for saved in [
            "windows-fixture-os-identity",
            "linux-fixture-os-identity",
            "mrd-machine-key:fixture-protected-key",
            "legacy-fixture",
        ] {
            assert!(validate_saved_identity_binding(saved, || {
                panic!("legacy registration must retain its serial without a hardware probe")
            })
            .is_ok());
        }
    }

    #[test]
    fn macos_keychain_fallback_uses_a_namespace_distinct_from_hardware() {
        let uuid = "01234567-89ab-cdef-0123-456789abcdef";
        let canonical = format!("macos-installation-{uuid}");
        for saved in [format!("macos-{uuid}"), canonical.clone()] {
            assert_eq!(validate_saved_identity(saved.as_bytes()).unwrap(), saved);
            assert_eq!(macos_installation_uuid(&saved), Some(uuid.into()));
        }
        assert!(validate_saved_identity(b"macos-installation-not-a-uuid").is_err());
    }

    #[test]
    fn legacy_macos_fallback_registration_value_survives_lost_public_credentials() {
        let existing = "macos-01234567-89ab-cdef-0123-456789abcdef";
        // This is the same readback path used when the Keychain fallback exists,
        // public credentials are missing, and the native probe is unavailable.
        let reconstructed = validate_saved_identity(existing.as_bytes()).unwrap();
        assert_eq!(reconstructed, existing);
        assert!(!reconstructed.starts_with("macos-installation-"));
        assert!(validate_saved_macos_identity_binding(
            &reconstructed,
            || Err("probe unavailable".into()),
            || Ok(Some(reconstructed.clone()))
        )
        .is_ok());
    }

    #[test]
    fn macos_native_registration_requires_the_same_motherboard_without_fallback_reads() {
        let original = "macos-01234567-89ab-cdef-0123-456789abcdef";
        assert!(validate_saved_macos_identity_binding(
            original,
            || Ok(original.into()),
            || panic!("native match must not read or prompt for Keychain fallback")
        )
        .is_ok());
        let another = "macos-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let unrelated_fallback = "macos-installation-11111111-2222-3333-4444-555555555555";
        assert!(validate_saved_macos_identity_binding(
            original,
            || Ok(another.into()),
            || Ok(Some(unrelated_fallback.into()))
        )
        .is_err());
        assert!(validate_saved_macos_identity_binding(
            original,
            || Err("probe unavailable".into()),
            || Ok(None)
        )
        .is_err());
        assert!(validate_saved_macos_identity_binding(
            original,
            || Ok(another.into()),
            || Err("protected fallback unreadable".into())
        )
        .is_err());
    }

    #[test]
    fn legacy_macos_fallback_survives_a_recovered_probe_only_with_the_saved_protected_item() {
        let original = "macos-01234567-89ab-cdef-0123-456789abcdef";
        let saved_fallback = "macos-installation-01234567-89ab-cdef-0123-456789abcdef";
        for protected_item in [original, saved_fallback] {
            for probe in [
                Ok("macos-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
                Err("probe unavailable".into()),
            ] {
                assert!(validate_saved_macos_identity_binding(
                    original,
                    || probe,
                    || Ok(Some(protected_item.into()))
                )
                .is_ok());
            }
        }
        assert!(validate_saved_macos_identity_binding(
            saved_fallback,
            || panic!("installation registration only checks its preexisting protected item"),
            || Ok(Some(saved_fallback.into()))
        )
        .is_ok());
        for item in [Ok(None), Err("protected fallback unreadable".into())] {
            assert!(validate_saved_macos_identity_binding(
                original,
                || Ok("macos-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
                || item
            )
            .is_err());
        }
    }

    #[test]
    #[cfg(unix)]
    fn linux_dmi_reads_reject_paths_outside_the_kernel_device_tree() {
        for path in ["/etc/passwd", "/etc/machine-id", "../../../../etc/passwd"] {
            assert!(trusted_linux_dmi_value(path).is_none());
        }
    }

    fn raw_smbios(major: u8, minor: u8, records: &[Vec<u8>]) -> Vec<u8> {
        let table: Vec<u8> = records.iter().flatten().copied().collect();
        let mut raw = vec![0, major, minor, 0];
        raw.extend_from_slice(&(table.len() as u32).to_le_bytes());
        raw.extend_from_slice(&table);
        raw
    }

    fn system_record(uuid: [u8; 16]) -> Vec<u8> {
        let mut record = vec![0; 25];
        record[0] = 1;
        record[1] = 25;
        record[8..24].copy_from_slice(&uuid);
        record.extend_from_slice(&[0, 0]);
        record
    }

    fn board_record(serial: &str) -> Vec<u8> {
        let mut record = vec![2, 8, 0, 0, 1, 2, 3, 4];
        record.extend_from_slice(b"Test manufacturer\0Test product\0v1\0");
        record.extend_from_slice(serial.as_bytes());
        record.extend_from_slice(&[0, 0]);
        record
    }

    #[test]
    fn smbios_uuid_respects_version_endianness_and_ignores_record_order() {
        let uuid = [
            0x67, 0x45, 0x23, 0x01, 0xab, 0x89, 0xef, 0xcd, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ];
        let records = [board_record("BOARD-7A92"), system_record(uuid)];
        assert_eq!(
            smbios_hardware_identity(&raw_smbios(2, 6, &records)),
            Some("windows-hardware-01234567-89ab-cdef-0123-456789abcdef".into())
        );
        assert_eq!(
            smbios_hardware_identity(&raw_smbios(2, 5, &records)),
            Some("windows-hardware-67452301-ab89-efcd-0123-456789abcdef".into())
        );
    }

    #[test]
    fn smbios_falls_back_to_valid_board_serial_when_uuid_is_unset() {
        for uuid in [[0; 16], [0xff; 16]] {
            let raw = raw_smbios(3, 0, &[system_record(uuid), board_record("board-7a92")]);
            assert_eq!(
                smbios_hardware_identity(&raw),
                Some("windows-board-BOARD-7A92".into())
            );
            let raw = raw_smbios(
                3,
                0,
                &[system_record(uuid), board_record("To Be Filled By O.E.M.")],
            );
            assert!(smbios_hardware_identity(&raw).is_none());
        }
    }

    #[test]
    fn truncated_or_malformed_smbios_is_rejected_without_unchecked_reads() {
        let raw = raw_smbios(3, 0, &[board_record("BOARD-7A92")]);
        for end in 0..raw.len() {
            assert!(smbios_hardware_identity(&raw[..end]).is_none());
        }
        for record in [
            vec![2, 3, 0, 0, 0, 0],
            vec![2, 255, 0, 0, 0, 0],
            vec![2, 8, 0, 0, 0, 0, 0, 1, b'A', b'B', b'C'],
        ] {
            assert!(smbios_hardware_identity(&raw_smbios(3, 0, &[record])).is_none());
        }
    }
}
