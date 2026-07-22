use mrd_store_sqlite::{SecretBytes, SecretProtector};
use std::{os::windows::ffi::OsStrExt, path::PathBuf, slice, sync::Arc};
use windows::Win32::{
    Foundation::{CloseHandle, LocalFree, ERROR_ALREADY_EXISTS, HANDLE, HLOCAL},
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE,
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
    Security::{
        AclSizeInformation,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo,
            SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        EqualSid, GetAclInformation, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
        GetSecurityDescriptorOwner, IsWellKnownSid, WinLocalSystemSid, ACL, ACL_SIZE_INFORMATION,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
    },
    System::Com::CoTaskMemFree,
    UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
};
use zeroize::Zeroize;

/// Explicit ACL policy for protected machine-level product data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductDirectoryAclPolicy {
    sddl: String,
}

impl ProductDirectoryAclPolicy {
    /// Bootstrap policy used before the Windows service SID is provisioned by SCM installation.
    pub fn bootstrap() -> Self {
        Self {
            sddl: "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)".to_owned(),
        }
    }

    /// Installed-service policy scoped to one canonical per-service SID.
    pub fn installed_service(service_sid: &str) -> Result<Self, String> {
        let service_sid = canonical_service_sid(service_sid)?;
        Ok(Self {
            sddl: format!(
                "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1301bf;;;{service_sid})"
            ),
        })
    }

    pub fn sddl(&self) -> &str {
        &self.sddl
    }
}

fn canonical_service_sid(value: &str) -> Result<String, String> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 9 || parts[0] != "S" || parts[1] != "1" || parts[2] != "5" || parts[3] != "80"
    {
        return Err("service SID must be a per-service S-1-5-80 SID".to_owned());
    }
    let subauthorities = parts[4..]
        .iter()
        .map(|part| {
            part.parse::<u32>()
                .map_err(|_| "service SID contains an invalid subauthority".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if subauthorities.iter().all(|value| *value == 0) {
        return Err("service SID subauthorities cannot all be zero".to_owned());
    }
    Ok(format!(
        "S-1-5-80-{}-{}-{}-{}-{}",
        subauthorities[0],
        subauthorities[1],
        subauthorities[2],
        subauthorities[3],
        subauthorities[4]
    ))
}

/// DPAPI machine-scope protector. Product directory ACLs remain a separate mandatory boundary.
pub struct DpapiMachineProtector;

impl SecretProtector for DpapiMachineProtector {
    fn protect(&self, purpose: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plaintext)?;
        let entropy = blob(purpose)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptProtectData(
                &input,
                windows::core::w!("MiniRemoteDesktop machine secret"),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|error| format!("DPAPI protect failed: 0x{:08x}", error.code().0 as u32))?;
        }
        copy_and_free(&mut output, false)
    }

    fn unprotect(&self, purpose: &[u8], protected: &[u8]) -> Result<SecretBytes, String> {
        let input = blob(protected)?;
        let entropy = blob(purpose)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
            .map_err(|error| format!("DPAPI unprotect failed: 0x{:08x}", error.code().0 as u32))?;
        }
        copy_and_free(&mut output, true).map(SecretBytes::new)
    }
}

/// Creates the Windows production protector.
pub fn platform_secret_protector() -> Result<Arc<dyn SecretProtector>, String> {
    Ok(Arc::new(DpapiMachineProtector))
}

/// Resolves `%ProgramData%\MiniRemoteDesktop` without trusting an overridable environment variable.
pub fn protected_product_data_dir() -> Result<PathBuf, String> {
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, KF_FLAG_DEFAULT, None) }
        .map_err(|error| {
            format!(
                "ProgramData resolution failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
    let path = unsafe { raw.to_string() }
        .map(PathBuf::from)
        .map_err(|_| "ProgramData path is invalid UTF-16".to_owned());
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    path.map(|path| path.join("MiniRemoteDesktop"))
}

/// Creates or tightens the product directory under an explicit protected ACL policy.
pub fn ensure_protected_product_data_dir(
    policy: &ProductDirectoryAclPolicy,
) -> Result<PathBuf, String> {
    let path = protected_product_data_dir()?;
    let descriptor = security_descriptor_from_sddl(policy.sddl())?;
    apply_directory_descriptor(&path, descriptor.0)?;
    verify_protected_product_data_dir(policy)?;
    Ok(path)
}

/// Verifies the product directory policy without requesting ACL mutation rights.
pub fn verify_protected_product_data_dir(
    policy: &ProductDirectoryAclPolicy,
) -> Result<PathBuf, String> {
    let path = protected_product_data_dir()?;
    let descriptor = security_descriptor_from_sddl(policy.sddl())?;
    verify_directory_descriptor_at(&path, descriptor.0)?;
    Ok(path)
}

fn apply_directory_descriptor(
    path: &std::path::Path,
    descriptor: PSECURITY_DESCRIPTOR,
) -> Result<(), String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    if let Err(error) =
        unsafe { CreateDirectoryW(windows::core::PCWSTR(wide.as_ptr()), Some(&attributes)) }
    {
        if error.code() != windows::core::HRESULT::from_win32(ERROR_ALREADY_EXISTS.0) {
            return Err(format!(
                "protected product directory creation failed: 0x{:08x}",
                error.code().0 as u32
            ));
        }
    }

    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0 | WRITE_DAC.0 | WRITE_OWNER.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map(HandleGuard)
    .map_err(|error| {
        format!(
            "protected product directory open failed: 0x{:08x}",
            error.code().0 as u32
        )
    })?;
    ensure_plain_directory_handle(handle.0)?;

    let desired_owner = security_descriptor_owner(descriptor)?;
    let actual_descriptor = object_security_descriptor(handle.0, OWNER_SECURITY_INFORMATION)?;
    let actual_owner = security_descriptor_owner(actual_descriptor.0)?;
    let owner_matches = unsafe { EqualSid(actual_owner, desired_owner).is_ok() };
    let owner_is_system = unsafe { IsWellKnownSid(actual_owner, WinLocalSystemSid).as_bool() };
    if !owner_matches && !owner_is_system {
        return Err("protected product directory has an untrusted owner".to_owned());
    }

    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    unsafe {
        GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted).map_err(
            |error| {
                format!(
                    "security descriptor DACL extraction failed: 0x{:08x}",
                    error.code().0 as u32
                )
            },
        )?;
    }
    if !present.as_bool() || dacl.is_null() {
        return Err("security descriptor has no DACL".to_owned());
    }
    let status = unsafe {
        SetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(desired_owner),
            None,
            Some(dacl),
            None,
        )
    };
    if status.0 != 0 {
        return Err(format!(
            "protected product directory DACL failed: 0x{:08x}",
            status.0
        ));
    }
    let applied_descriptor = object_security_descriptor(
        handle.0,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
    )?;
    verify_security_descriptor(descriptor, applied_descriptor.0)?;
    Ok(())
}

fn verify_directory_descriptor_at(
    path: &std::path::Path,
    descriptor: PSECURITY_DESCRIPTOR,
) -> Result<(), String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            windows::core::PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map(HandleGuard)
    .map_err(|error| {
        format!(
            "protected product directory verification open failed: 0x{:08x}",
            error.code().0 as u32
        )
    })?;
    ensure_plain_directory_handle(handle.0)?;
    let actual = object_security_descriptor(
        handle.0,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
    )?;
    verify_security_descriptor(descriptor, actual.0)
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct SecurityDescriptorGuard(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(HLOCAL(self.0 .0)));
        }
    }
}

fn security_descriptor_from_sddl(sddl: &str) -> Result<SecurityDescriptorGuard, String> {
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            windows::core::PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|error| {
            format!(
                "security descriptor creation failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
    }
    if descriptor.0.is_null() {
        return Err("security descriptor creation returned no descriptor".to_owned());
    }
    Ok(SecurityDescriptorGuard(descriptor))
}

fn verify_security_descriptor(
    expected: PSECURITY_DESCRIPTOR,
    actual: PSECURITY_DESCRIPTOR,
) -> Result<(), String> {
    let expected_owner = security_descriptor_owner(expected)?;
    let actual_owner = security_descriptor_owner(actual)?;
    if unsafe { EqualSid(expected_owner, actual_owner).is_err() } {
        return Err(
            "protected product directory security descriptor does not match policy".to_owned(),
        );
    }
    ensure_protected_dacl(expected)?;
    ensure_protected_dacl(actual)?;
    if dacl_bytes(expected)? != dacl_bytes(actual)? {
        return Err(
            "protected product directory security descriptor does not match policy".to_owned(),
        );
    }
    Ok(())
}

fn ensure_protected_dacl(descriptor: PSECURITY_DESCRIPTOR) -> Result<(), String> {
    let mut control = 0_u16;
    let mut revision = 0_u32;
    unsafe {
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).map_err(|error| {
            format!(
                "security descriptor control read failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
    }
    if control & SE_DACL_PROTECTED.0 == 0 {
        return Err("protected product directory DACL is not protected".to_owned());
    }
    Ok(())
}

fn dacl_bytes(descriptor: PSECURITY_DESCRIPTOR) -> Result<Vec<u8>, String> {
    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    unsafe {
        GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted).map_err(
            |error| {
                format!(
                    "security descriptor DACL extraction failed: 0x{:08x}",
                    error.code().0 as u32
                )
            },
        )?;
    }
    if !present.as_bool() || dacl.is_null() {
        return Err("security descriptor has no DACL".to_owned());
    }
    let mut information = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
        .map_err(|error| {
            format!(
                "security descriptor ACL size read failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
        Ok(slice::from_raw_parts(dacl.cast::<u8>(), information.AclBytesInUse as usize).to_vec())
    }
}

fn object_security_descriptor(
    handle: HANDLE,
    information: windows::Win32::Security::OBJECT_SECURITY_INFORMATION,
) -> Result<SecurityDescriptorGuard, String> {
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            information,
            None,
            None,
            None,
            None,
            Some(&mut descriptor),
        )
    };
    if status.0 != 0 || descriptor.0.is_null() {
        return Err(format!(
            "protected product directory security read failed: 0x{:08x}",
            status.0
        ));
    }
    Ok(SecurityDescriptorGuard(descriptor))
}

fn ensure_plain_directory_handle(handle: HANDLE) -> Result<(), String> {
    let mut information = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileAttributeTagInfo,
            (&mut information as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
        .map_err(|error| {
            format!(
                "protected product directory attribute read failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
    }
    if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err("protected product directory is a reparse point".to_owned());
    }
    if information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err("protected product data path is not a directory".to_owned());
    }
    Ok(())
}

fn security_descriptor_owner(descriptor: PSECURITY_DESCRIPTOR) -> Result<PSID, String> {
    let mut owner = PSID::default();
    let mut defaulted = windows::core::BOOL::default();
    unsafe {
        GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted).map_err(|error| {
            format!(
                "security descriptor owner extraction failed: 0x{:08x}",
                error.code().0 as u32
            )
        })?;
    }
    if owner.0.is_null() {
        return Err("security descriptor has no owner".to_owned());
    }
    Ok(owner)
}

fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, String> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| "secret input exceeds DPAPI size limit".to_owned())?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

fn copy_and_free(
    output: &mut CRYPT_INTEGER_BLOB,
    clear_before_free: bool,
) -> Result<Vec<u8>, String> {
    if output.cbData > 0 && output.pbData.is_null() {
        return Err("DPAPI returned an invalid output buffer".to_owned());
    }
    let mut bytes = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec()
    };
    unsafe {
        if clear_before_free && !output.pbData.is_null() {
            slice::from_raw_parts_mut(output.pbData, output.cbData as usize).zeroize();
        }
        let result = if output.pbData.is_null() {
            HLOCAL::default()
        } else {
            LocalFree(Some(HLOCAL(output.pbData.cast())))
        };
        output.cbData = 0;
        output.pbData = std::ptr::null_mut();
        if !result.is_invalid() {
            if clear_before_free {
                bytes.zeroize();
            }
            return Err("DPAPI output buffer release failed".to_owned());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    use windows::{
        core::PWSTR,
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::{
                Authorization::{ConvertSidToStringSidW, SetNamedSecurityInfoW},
                GetSecurityDescriptorOwner, GetTokenInformation, TokenUser,
                OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        root: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("mrd-service-security-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir(&root).unwrap();
            Self { root }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn test_descriptor(sddl: &str) -> PSECURITY_DESCRIPTOR {
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                windows::core::PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .unwrap();
        }
        descriptor
    }

    fn free_test_descriptor(descriptor: PSECURITY_DESCRIPTOR) {
        unsafe {
            LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }

    fn current_user_sid_string() -> String {
        let mut token = HANDLE::default();
        unsafe {
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).unwrap();
        }
        let mut required = 0;
        let first = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut required) };
        assert!(first.is_err());
        assert!(required >= std::mem::size_of::<TOKEN_USER>() as u32);
        let mut buffer = vec![0_u8; required as usize];
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                required,
                &mut required,
            )
            .unwrap();
            CloseHandle(token).unwrap();
        }
        let token_user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut string_sid = PWSTR::null();
        unsafe {
            ConvertSidToStringSidW(token_user.User.Sid, &mut string_sid).unwrap();
        }
        let value = unsafe { string_sid.to_string() }.unwrap();
        unsafe {
            LocalFree(Some(HLOCAL(string_sid.0.cast())));
        }
        value
    }

    fn set_directory_descriptor(path: &Path, descriptor: PSECURITY_DESCRIPTOR) {
        let mut owner = windows::Win32::Security::PSID::default();
        let mut owner_defaulted = windows::core::BOOL::default();
        let mut present = windows::core::BOOL::default();
        let mut dacl_defaulted = windows::core::BOOL::default();
        let mut dacl: *mut ACL = std::ptr::null_mut();
        unsafe {
            GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted).unwrap();
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut dacl_defaulted)
                .unwrap();
        }
        assert!(present.as_bool());
        assert!(!owner.0.is_null());
        assert!(!dacl.is_null());
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let status = unsafe {
            SetNamedSecurityInfoW(
                windows::core::PCWSTR(wide.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION
                    | DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION,
                Some(owner),
                None,
                Some(dacl),
                None,
            )
        };
        assert_eq!(status.0, 0);
    }

    fn create_junction(link: &Path, target: &Path) {
        let link = link.to_string_lossy().replace('\'', "''");
        let target = target.to_string_lossy().replace('\'', "''");
        let script = format!(
            "New-Item -ItemType Junction -Path '{link}' -Target '{target}' -ErrorAction Stop | Out-Null"
        );
        let output = Command::new("powershell")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn security_dpapi_round_trip_is_purpose_bound() {
        let protector = DpapiMachineProtector;
        let protected = protector.protect(b"identity", b"private material").unwrap();
        assert_ne!(protected, b"private material");
        assert_eq!(
            protector
                .unprotect(b"identity", &protected)
                .unwrap()
                .as_ref(),
            b"private material"
        );
        assert!(protector.unprotect(b"audit", &protected).is_err());
    }

    #[test]
    fn security_dpapi_rejects_tampering() {
        let protector = DpapiMachineProtector;
        let mut protected = protector.protect(b"identity", b"private material").unwrap();
        let index = protected.len() / 2;
        protected[index] ^= 0x80;
        assert!(protector.unprotect(b"identity", &protected).is_err());
    }

    #[test]
    fn security_product_data_path_is_machine_wide() {
        let path = protected_product_data_dir().unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("MiniRemoteDesktop")
        );
        assert!(path.is_absolute());
    }

    #[test]
    fn security_directory_policy_is_explicit_and_service_sid_scoped() {
        const SERVICE_SID: &str = "S-1-5-80-2970612574-78537857-698502321-558674196-1451644582";

        let bootstrap = ProductDirectoryAclPolicy::bootstrap();
        assert_eq!(bootstrap.sddl(), "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)");

        let installed = ProductDirectoryAclPolicy::installed_service(SERVICE_SID).unwrap();
        assert_eq!(
            installed.sddl(),
            format!("O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1301bf;;;{SERVICE_SID})")
        );
    }

    #[test]
    fn security_directory_policy_rejects_non_service_and_injected_sids() {
        for invalid in [
            "S-1-5-32-545",
            "S-1-5-80-0",
            "S-1-5-80-0-0-0-0-0",
            "S-1-5-80-1-2-3-4-5)(A;;FA;;;WD",
            "S-1-5-80-1-2-3-4",
        ] {
            assert!(
                ProductDirectoryAclPolicy::installed_service(invalid).is_err(),
                "accepted invalid service SID: {invalid}"
            );
        }
    }

    #[test]
    fn security_directory_descriptor_verification_rejects_acl_drift() {
        let current_sid = current_user_sid_string();
        let desired = test_descriptor(&format!("O:{current_sid}D:P(A;OICI;FA;;;{current_sid})"));
        let weak = test_descriptor(&format!(
            "O:{current_sid}D:P(A;OICI;FA;;;{current_sid})(A;OICI;FR;;;WD)"
        ));

        assert!(verify_security_descriptor(desired, desired).is_ok());
        let result = verify_security_descriptor(desired, weak);
        assert!(
            matches!(&result, Err(error) if error.contains("does not match")),
            "unexpected result: {result:?}"
        );

        free_test_descriptor(weak);
        free_test_descriptor(desired);
    }

    #[test]
    fn security_directory_rejects_junction_before_acl_application() {
        let test = TestDirectory::new();
        let target = test.path("target");
        let junction = test.path("junction");
        fs::create_dir(&target).unwrap();
        create_junction(&junction, &target);

        let current_sid = current_user_sid_string();
        let descriptor = test_descriptor(&format!("O:{current_sid}D:P(A;OICI;FA;;;{current_sid})"));
        let result = apply_directory_descriptor(&junction, descriptor);
        free_test_descriptor(descriptor);

        assert!(
            matches!(&result, Err(error) if error.contains("reparse point")),
            "unexpected result: {result:?}"
        );
    }

    #[test]
    fn security_directory_rejects_existing_untrusted_owner_without_mutation() {
        let test = TestDirectory::new();
        let directory = test.path("preowned");
        fs::create_dir(&directory).unwrap();

        let current_sid = current_user_sid_string();
        let preexisting =
            test_descriptor(&format!("O:{current_sid}D:P(A;OICI;FA;;;{current_sid})"));
        set_directory_descriptor(&directory, preexisting);
        free_test_descriptor(preexisting);

        let desired = test_descriptor("O:BAD:P(A;OICI;FA;;;WD)");
        let result = apply_directory_descriptor(&directory, desired);
        free_test_descriptor(desired);

        assert!(
            matches!(&result, Err(error) if error.contains("untrusted owner")),
            "unexpected result: {result:?}"
        );
    }

    #[test]
    fn security_directory_rejects_a_non_directory_leaf() {
        let test = TestDirectory::new();
        let file = test.path("not-a-directory");
        fs::write(&file, b"not a directory").unwrap();

        let current_sid = current_user_sid_string();
        let desired = test_descriptor(&format!("O:{current_sid}D:P(A;OICI;FA;;;{current_sid})"));
        let result = apply_directory_descriptor(&file, desired);
        free_test_descriptor(desired);

        assert!(
            matches!(&result, Err(error) if error.contains("not a directory")),
            "unexpected result: {result:?}"
        );
    }

    #[test]
    fn security_directory_runtime_verification_detects_acl_drift() {
        let test = TestDirectory::new();
        let directory = test.path("verified");
        let current_sid = current_user_sid_string();
        let desired = test_descriptor(&format!("O:{current_sid}D:P(A;OICI;FA;;;{current_sid})"));

        apply_directory_descriptor(&directory, desired).unwrap();
        verify_directory_descriptor_at(&directory, desired).unwrap();

        let weak = test_descriptor(&format!(
            "O:{current_sid}D:P(A;OICI;FA;;;{current_sid})(A;OICI;FR;;;WD)"
        ));
        set_directory_descriptor(&directory, weak);
        let result = verify_directory_descriptor_at(&directory, desired);
        assert!(
            matches!(&result, Err(error) if error.contains("does not match")),
            "unexpected result: {result:?}"
        );

        free_test_descriptor(weak);
        free_test_descriptor(desired);
    }
}
