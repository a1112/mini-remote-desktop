use std::{ffi::c_void, mem::size_of, os::windows::io::RawHandle, ptr};

use anyhow::{Context, Result};
use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows::{
    core::{Owned, BOOL, PCWSTR, PWSTR},
    Win32::{
        Foundation::{HANDLE, HLOCAL},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING,
            SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

// FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE.
// Deliberately excludes FILE_APPEND_DATA / FILE_CREATE_PIPE_INSTANCE (0x4).
const MANAGEMENT_DATA_ACCESS: u32 = 0x0010_0083;

pub(super) fn validate_local_pipe_name(name: &str) -> Result<()> {
    anyhow::ensure!(
        name.starts_with(r"\\.\pipe\")
            && name.len() <= 512
            && !name.contains('\0')
            && !name.contains('/')
            && !name.contains(".."),
        "Management IPC requires a bounded local named pipe"
    );
    Ok(())
}

fn management_sddl(process_user: &str) -> String {
    format!("D:P(D;;GA;;;AN)(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x{MANAGEMENT_DATA_ACCESS:08X};;;IU)(A;;GA;;;{process_user})")
}

pub(super) fn management_sddl_for_current_process() -> Result<String> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)? };
    let token = unsafe { Owned::new(token) };
    let mut bytes = 0;
    let _ = unsafe { GetTokenInformation(*token, TokenUser, None, 0, &mut bytes) };
    anyhow::ensure!(
        bytes >= size_of::<TOKEN_USER>() as u32 && bytes <= 64 * 1024,
        "Invalid Windows process identity"
    );
    let words = (bytes as usize).div_ceil(size_of::<usize>());
    let mut information = vec![0usize; words];
    unsafe {
        GetTokenInformation(
            *token,
            TokenUser,
            Some(information.as_mut_ptr().cast()),
            bytes,
            &mut bytes,
        )?
    };
    let user = unsafe { &*information.as_ptr().cast::<TOKEN_USER>() };
    let mut user_sid = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut user_sid)? };
    let allocation = unsafe { Owned::new(HLOCAL(user_sid.0.cast())) };
    let sid = unsafe { user_sid.to_string()? };
    drop(allocation);
    Ok(management_sddl(&sid))
}

pub(super) fn create_pipe(name: &str, first: bool, sddl: &str) -> Result<NamedPipeServer> {
    validate_local_pipe_name(name)?;
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wide.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )?
    };
    let descriptor = unsafe { Owned::new(HLOCAL(descriptor.0)) };
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: BOOL::from(false),
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // The descriptor lives through synchronous CreateNamedPipeW; it is not retained.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            ptr::addr_of_mut!(attributes).cast::<c_void>(),
        )
    }
    .context("Cannot create protected service management pipe")
}

pub(super) fn connect_client(name: &str) -> Result<NamedPipeClient> {
    validate_local_pipe_name(name)?;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            MANAGEMENT_DATA_ACCESS,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )
    }
    .map_err(|error| {
        let status = error.code().0 as u32;
        if status & 0xffff_0000 == 0x8007_0000 {
            std::io::Error::from_raw_os_error((status & 0xffff) as i32)
        } else {
            std::io::Error::other(error)
        }
    })?;
    // Tokio consumes the handle even if I/O registration fails.
    unsafe { NamedPipeClient::from_raw_handle(handle.0 as RawHandle) }
        .context("Cannot register management IPC client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_management_rights_exclude_pipe_creation_and_acl_changes() {
        assert_eq!(MANAGEMENT_DATA_ACCESS, 0x0010_0083);
        assert_eq!(
            MANAGEMENT_DATA_ACCESS & (0x4 | 0x0004_0000 | 0x0008_0000),
            0
        );
        let sddl = management_sddl("S-1-5-18");
        assert!(sddl.contains("(A;;0x00100083;;;IU)"));
        assert!(sddl.contains("(D;;GA;;;NU)"));
        assert!(sddl.contains("(D;;GA;;;AN)"));
    }

    #[test]
    fn management_endpoints_reject_network_or_malformed_pipe_names() {
        for name in [
            r"\\remote\pipe\mrd-service-management",
            r"\\.\pipe\..\escape",
            "bad\0pipe",
        ] {
            assert!(validate_local_pipe_name(name).is_err());
        }
        assert!(validate_local_pipe_name(r"\\.\pipe\mrd-service-management").is_ok());
    }

    #[tokio::test]
    async fn management_pipe_connects_and_exchanges_messages_without_generic_write() {
        let name = format!(
            r"\\.\pipe\mrd-management-data-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        // Simulate the installed service's SYSTEM-owned ACL. The caller must
        // connect through IU data rights, which omit FILE_CREATE_PIPE_INSTANCE.
        let server = create_pipe(&name, true, &management_sddl("S-1-5-18")).unwrap();
        let mut client = crate::transport::IpcStream::Client(connect_client(&name).unwrap());
        server.connect().await.unwrap();
        let mut server = crate::transport::IpcStream::Server(server);
        client
            .send_request(&crate::IpcRequest::ServiceHealth)
            .await
            .unwrap();
        assert!(matches!(
            server.recv_request().await.unwrap(),
            crate::IpcRequest::ServiceHealth
        ));
        server
            .send_response(&crate::IpcResponse::Ack)
            .await
            .unwrap();
        assert!(matches!(
            client.recv_response().await.unwrap(),
            crate::IpcResponse::Ack
        ));
    }

    #[tokio::test]
    async fn ordinary_interactive_data_rights_do_not_allow_creating_another_pipe_instance() {
        let name = format!(
            r"\\.\pipe\mrd-product-no-create-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let _server = create_pipe(&name, true, &management_sddl("S-1-5-18")).unwrap();
        let _client = connect_client(&name).unwrap();
        // A restricted/ordinary token receives IU data rights and no create-instance
        // right. Elevated administrators legitimately have the separate BA grant.
        let instance = ServerOptions::new()
            .first_pipe_instance(false)
            .create(&name);
        match instance {
            Err(error) => assert_eq!(error.raw_os_error(), Some(5)),
            Ok(_) => {
                use windows::Win32::Security::{
                    CheckTokenMembership, CreateWellKnownSid, WinBuiltinAdministratorsSid, PSID,
                    SECURITY_MAX_SID_SIZE,
                };
                let mut sid = [0_u8; SECURITY_MAX_SID_SIZE as usize];
                let mut length = sid.len() as u32;
                let mut administrator = BOOL::default();
                unsafe {
                    CreateWellKnownSid(
                        WinBuiltinAdministratorsSid,
                        None,
                        Some(PSID(sid.as_mut_ptr().cast())),
                        &mut length,
                    )
                    .unwrap();
                    CheckTokenMembership(None, PSID(sid.as_mut_ptr().cast()), &mut administrator)
                        .unwrap();
                }
                assert!(
                    administrator.as_bool(),
                    "Ordinary caller received FILE_CREATE_PIPE_INSTANCE"
                );
            }
        }
    }

    #[tokio::test]
    async fn management_binding_rejects_an_existing_first_instance() {
        let endpoint = crate::transport::IpcEndpoint::named_pipe(format!(
            r"\\.\pipe\mrd-management-collision-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _existing = ServerOptions::new()
            .first_pipe_instance(true)
            .create(endpoint.pipe_name())
            .unwrap();
        assert!(
            crate::transport::IpcServer::bind_management_with_endpoint(endpoint)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn management_binding_cannot_replace_the_core_endpoint() {
        assert!(crate::transport::IpcServer::bind_management_with_endpoint(
            crate::transport::IpcEndpoint::service_from_env_or_default()
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn missing_management_pipe_preserves_the_native_io_error() {
        let name = format!(
            r"\\.\pipe\mrd-management-missing-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let error = match connect_client(&name) {
            Ok(_) => panic!("missing pipe connected unexpectedly"),
            Err(error) => error,
        };
        let io = error
            .downcast_ref::<std::io::Error>()
            .expect("management client must preserve Win32 I/O error classification");
        assert_eq!(io.raw_os_error(), Some(2));
    }
}
