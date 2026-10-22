//! Kernel-bound verification of installed Windows IPC peers.
//! Endpoint names and caller supplied executable paths never establish trust.

use anyhow::{ensure, Context, Result};
use std::{
    ffi::c_void,
    mem::{offset_of, size_of},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use tokio::net::windows::named_pipe::NamedPipeClient;
use windows::{
    core::{Owned, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HANDLE, HLOCAL},
        Security::{
            AclSizeInformation,
            Authorization::{ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT},
            GetAce, GetAclInformation, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
        Storage::FileSystem::{
            CreateFileW, FileAttributeTagInfo, GetFileInformationByHandleEx,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING, READ_CONTROL,
        },
        System::{
            Com::CoTaskMemFree,
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT},
    },
};

const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
const MUTABLE_ACCESS: u32 = 0x1000_0000 | 0x4000_0000 | 0x000D_0000 | 0x0000_0156;
const INHERIT_ONLY: u8 = 0x08;

#[derive(Debug, Clone, Copy)]
/// Executable role allowed in the administrator-protected installation directory.
pub enum InstalledImage {
    /// Installed Rdesk UI only.
    Ui,
    /// Installed background service only.
    Service,
}

/// Holds the process object and installation files against PID reuse/renaming.
pub struct VerifiedInstalledProcess {
    _process: Owned<HANDLE>,
    _install_directory: Owned<HANDLE>,
    _image: Owned<HANDLE>,
    process_id: u32,
    image_path: PathBuf,
}

// These are owned kernel handles, never dereferenced pointers. Handles are
// process-wide and may be retained/dropped on a different Tokio worker thread.
unsafe impl Send for VerifiedInstalledProcess {}
unsafe impl Sync for VerifiedInstalledProcess {}

impl VerifiedInstalledProcess {
    /// Kernel process identifier checked against the connected pipe.
    pub fn process_id(&self) -> u32 {
        self.process_id
    }
    /// Actual OS process image, rather than a caller-supplied path.
    pub fn image_path(&self) -> &Path {
        &self.image_path
    }
}

/// System KnownFolder installation path; untrusted environment variables are ignored.
pub fn installed_product_directory() -> Result<PathBuf> {
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None)? };
    let folder = unsafe { raw.to_string() };
    unsafe { CoTaskMemFree(Some(raw.0.cast())) };
    Ok(PathBuf::from(folder?).join("MiniRemoteDesktop"))
}

fn image_name_is_allowed(path: &Path, directory: &Path, role: InstalledImage) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if !parent
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&directory.as_os_str().to_string_lossy())
    {
        return false;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match role {
        InstalledImage::Ui => {
            name.eq_ignore_ascii_case("Rdesk.exe") || name.eq_ignore_ascii_case("app.exe")
        }
        InstalledImage::Service => name.eq_ignore_ascii_case("mrd-service.exe"),
    }
}

/// Verify an OS process and pin its protected installation against replacement.
pub fn verify_installed_process(
    process_id: u32,
    role: InstalledImage,
) -> Result<VerifiedInstalledProcess> {
    ensure!(process_id != 0, "IPC peer process identity is unavailable");
    let process = unsafe {
        Owned::new(OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        )?)
    };
    let mut path = vec![0_u16; 32768];
    let mut length = path.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            *process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )?
    };
    ensure!(
        length > 0 && (length as usize) < path.len(),
        "IPC peer image path is invalid"
    );
    let image_path = PathBuf::from(String::from_utf16(&path[..length as usize])?);
    let directory = installed_product_directory()?;
    ensure!(
        image_name_is_allowed(&image_path, &directory, role),
        "IPC peer image is not an installed product executable"
    );
    // Check every ancestor without following a reparse point. Ancestors may
    // allow creation of siblings; deletion, ACL changes and replacement may
    // never be granted to an untrusted principal.
    for ancestor in directory.ancestors().skip(1) {
        let _ = protected_path_handle(ancestor, true, true)?;
    }
    let install_directory = protected_path_handle(&directory, true, false)?;
    let image = protected_path_handle(&image_path, false, false)?;
    Ok(VerifiedInstalledProcess {
        _process: process,
        _install_directory: install_directory,
        _image: image,
        process_id,
        image_path,
    })
}

pub(super) fn verify_pipe_server(pipe: &NamedPipeClient) -> Result<VerifiedInstalledProcess> {
    let mut process_id = 0;
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut process_id)? };
    verify_installed_process(process_id, InstalledImage::Service)
        .context("Cannot authenticate the installed background service")
}

fn trusted_sid(value: &str) -> bool {
    matches!(value, "S-1-5-18" | "S-1-5-32-544") || value == TRUSTED_INSTALLER
}

fn sid_string(sid: PSID) -> Result<String> {
    ensure!(!sid.0.is_null(), "Protected path has no owner");
    let mut text = PWSTR::null();
    unsafe { ConvertSidToStringSidW(sid, &mut text)? };
    let allocation = unsafe { Owned::new(HLOCAL(text.0.cast())) };
    let result = unsafe { text.to_string()? };
    drop(allocation);
    Ok(result)
}

fn protected_path_handle(path: &Path, directory: bool, ancestor: bool) -> Result<Owned<HANDLE>> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        Owned::new(CreateFileW(
            PCWSTR(wide.as_ptr()),
            FILE_READ_ATTRIBUTES.0 | READ_CONTROL.0,
            // Directory updates by an administrator remain possible, but retaining
            // this handle without FILE_SHARE_DELETE pins its name for the connection.
            if directory {
                FILE_SHARE_READ | FILE_SHARE_WRITE
            } else {
                FILE_SHARE_READ
            },
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )?)
    };
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            *handle,
            FileAttributeTagInfo,
            ptr::addr_of_mut!(info).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )?
    };
    ensure!(
        info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 == 0,
        "Installed path may not contain a reparse point"
    );
    ensure!(
        (info.FileAttributes & 0x10 != 0) == directory,
        "Installed path object type is invalid"
    );
    let mut owner = PSID::default();
    let mut acl: *mut ACL = ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        GetSecurityInfo(
            *handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut acl),
            None,
            Some(&mut descriptor),
        )
        .ok()?
    };
    let allocation = unsafe { Owned::new(HLOCAL(descriptor.0)) };
    ensure!(
        trusted_sid(&sid_string(owner)?),
        "Installed path has an untrusted owner"
    );
    ensure!(!acl.is_null(), "Installed path must have a non-null DACL");
    let mut size = ACL_SIZE_INFORMATION::default();
    unsafe {
        GetAclInformation(
            acl,
            ptr::addr_of_mut!(size).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )?
    };
    let mask = if ancestor {
        MUTABLE_ACCESS & !0x6
    } else {
        MUTABLE_ACCESS
    };
    for index in 0..size.AceCount {
        let mut raw: *mut c_void = ptr::null_mut();
        unsafe { GetAce(acl, index, &mut raw)? };
        ensure!(!raw.is_null(), "Installed path ACL is malformed");
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if header.AceFlags & INHERIT_ONLY != 0 {
            continue;
        }
        // Deny ACEs cannot grant write access. All ordinary inherited readable
        // Users/Application Packages ACEs are accepted without broadening rights.
        if header.AceType == 1 || header.AceType == 6 {
            continue;
        }
        ensure!(
            header.AceSize as usize >= 8,
            "Installed path ACL is truncated"
        );
        let allowed_mask = unsafe { *raw.cast::<u8>().add(4).cast::<u32>() };
        if allowed_mask & mask == 0 {
            continue;
        }
        ensure!(
            header.AceType == 0 && header.AceSize as usize >= size_of::<ACCESS_ALLOWED_ACE>(),
            "Installed path has an unsupported mutable ACE"
        );
        let sid = PSID(unsafe {
            raw.cast::<u8>()
                .add(offset_of!(ACCESS_ALLOWED_ACE, SidStart))
                .cast()
        });
        ensure!(
            trusted_sid(&sid_string(sid)?),
            "Installed path is writable by an untrusted principal"
        );
    }
    drop(allocation);
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_executable_names_require_an_exact_directory_and_role() {
        let directory = Path::new(r"C:\Program Files\MiniRemoteDesktop");
        assert!(image_name_is_allowed(
            &directory.join("Rdesk.exe"),
            directory,
            InstalledImage::Ui
        ));
        assert!(image_name_is_allowed(
            &directory.join("app.exe"),
            directory,
            InstalledImage::Ui
        ));
        assert!(!image_name_is_allowed(
            &directory.join("mrd-service.exe"),
            directory,
            InstalledImage::Ui
        ));
        assert!(!image_name_is_allowed(
            Path::new(r"C:\Program Files\MiniRemoteDesktop-copy\Rdesk.exe"),
            directory,
            InstalledImage::Ui
        ));
    }
    #[test]
    fn actual_build_process_cannot_impersonate_an_installed_product_peer() {
        assert!(verify_installed_process(std::process::id(), InstalledImage::Ui).is_err());
        assert!(verify_installed_process(std::process::id(), InstalledImage::Service).is_err());
    }
    #[test]
    #[ignore = "requires the newly installed UI and service running under their real accounts"]
    fn live_installed_processes_pass_the_same_protected_image_boundary() {
        // Test-only inputs identify processes launched by the deployment check;
        // they never bypass any image, ACL, reparse or process-object verification.
        let ui_pid = std::env::var("MRD_TEST_INSTALLED_UI_PID")
            .expect("UI PID")
            .parse::<u32>()
            .unwrap();
        let service_pid = std::env::var("MRD_TEST_INSTALLED_SERVICE_PID")
            .expect("service PID")
            .parse::<u32>()
            .unwrap();
        assert_eq!(
            verify_installed_process(ui_pid, InstalledImage::Ui)
                .unwrap()
                .process_id(),
            ui_pid
        );
        assert_eq!(
            verify_installed_process(service_pid, InstalledImage::Service)
                .unwrap()
                .process_id(),
            service_pid
        );
    }
    #[test]
    fn ordinary_users_read_execute_does_not_imply_install_write_access() {
        assert_eq!(MUTABLE_ACCESS & 0x0012_00A9, 0);
    }
    #[tokio::test]
    async fn client_authenticates_kernel_server_before_sending_any_request() {
        let name = format!(
            r"\\.\pipe\product-server-identity-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let server = super::super::windows_management::create_pipe(
            &name,
            true,
            &super::super::windows_management::management_sddl_for_current_process().unwrap(),
        )
        .unwrap();
        let client = super::super::windows_management::connect_client(&name).unwrap();
        server.connect().await.unwrap();
        assert!(verify_pipe_server(&client).is_err());
        drop(client);
        let mut stream = crate::transport::IpcStream::Server(server);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), stream.recv_request())
                .await
                .unwrap()
                .is_err()
        );
    }
}
