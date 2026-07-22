//! Windows private named-pipe creation and OS peer identity inspection.

use super::{AgentCallerKind, ObservedAgentIdentity};
use mrd_agent_ipc::hash_windows_logon_sid;
use std::{
    ffi::c_void,
    mem::{size_of, size_of_val},
    os::windows::io::AsRawHandle,
    ptr,
};
use thiserror::Error;
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use windows::{
    core::{Owned, BOOL, PCWSTR, PWSTR},
    Win32::{
        Foundation::{FILETIME, HANDLE, HLOCAL},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            CheckTokenMembership, CreateWellKnownSid, GetLengthSid, GetTokenInformation,
            IsValidSecurityDescriptor, IsValidSid, RevertToSelf, SecurityAnonymous,
            TokenImpersonationLevel, TokenLogonSid, TokenSessionId, TokenUser, WinAnonymousSid,
            WinInteractiveSid, WinNetworkSid, WinRemoteLogonIdSid, PSECURITY_DESCRIPTOR, PSID,
            SECURITY_ATTRIBUTES, SECURITY_IMPERSONATION_LEVEL, SID_AND_ATTRIBUTES, TOKEN_GROUPS,
            TOKEN_QUERY, TOKEN_USER, WELL_KNOWN_SID_TYPE,
        },
        System::{
            Pipes::{
                GetNamedPipeClientProcessId, GetNamedPipeClientSessionId,
                ImpersonateNamedPipeClient,
            },
            SystemServices::SE_GROUP_LOGON_ID,
            Threading::{
                GetCurrentProcess, GetCurrentThread, GetProcessTimes, OpenProcess,
                OpenProcessToken, OpenThreadToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    },
};

const MAX_PIPE_NAME_BYTES: usize = 512;
const MAX_TOKEN_INFORMATION_BYTES: usize = 64 * 1024;
const SECURITY_MAX_SID_SIZE: usize = 68;
const AGENT_PIPE_ACCESS_MASK: u32 = 0x0010_0183;

/// Failures creating a protected pipe or verifying its connected peer.
#[derive(Debug, Error)]
pub enum WindowsAgentPipeError {
    /// Pipe name is not a bounded local MRD agent endpoint.
    #[error("Windows agent pipe name is invalid")]
    InvalidPipeName,
    /// Expected or observed SID bytes are malformed.
    #[error("Windows agent SID is invalid")]
    InvalidSid,
    /// Token information is malformed or exceeds the local bound.
    #[error("Windows agent token information is invalid")]
    InvalidTokenInformation,
    /// Pipe and process token observations disagree.
    #[error("Windows pipe client identity does not match its process token")]
    PeerIdentityMismatch,
    /// Pipe caller is anonymous, network-only, or non-interactive.
    #[error("Windows pipe client is not an interactive local logon")]
    UntrustedCaller,
    /// Windows security or process API failed.
    #[error(transparent)]
    Windows(#[from] windows::core::Error),
    /// Tokio could not create or operate the named pipe.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// One protected, single-instance Windows agent pipe.
pub struct WindowsAgentPipe {
    server: NamedPipeServer,
    expected: ExpectedWindowsPeer,
}

impl WindowsAgentPipe {
    /// Create a local-only first pipe instance bound to one launcher process.
    pub fn create_for_process(
        pipe_name: &str,
        expected_process: &VerifiedWindowsProcess,
    ) -> Result<Self, WindowsAgentPipeError> {
        let expected_logon_sid = expected_process.logon_sid();
        validate_pipe_name(pipe_name)?;
        validate_sid_bytes(expected_logon_sid)?;

        let service_user_sid = current_process_user_sid()?;
        let service_user = sid_string(&service_user_sid)?;
        let expected_logon = sid_string(expected_logon_sid)?;
        let sddl = format!(
            "D:P(D;;GA;;;AN)(D;;GA;;;NU)(A;;GA;;;{service_user})(A;;0x{AGENT_PIPE_ACCESS_MASK:08X};;;{expected_logon})"
        );
        let descriptor = security_descriptor_from_sddl(&sddl)?;
        let descriptor_ptr = PSECURITY_DESCRIPTOR(descriptor.0);
        if !unsafe { IsValidSecurityDescriptor(descriptor_ptr) }.as_bool() {
            return Err(WindowsAgentPipeError::InvalidTokenInformation);
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL::from(false),
        };
        let mut options = ServerOptions::new();
        options
            .pipe_mode(PipeMode::Byte)
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .max_instances(1);
        // SAFETY: `attributes` and its owned security descriptor remain alive
        // for the synchronous CreateNamedPipeW call. Tokio does not retain it.
        let server = unsafe {
            options.create_with_security_attributes_raw(
                pipe_name,
                ptr::addr_of_mut!(attributes).cast::<c_void>(),
            )?
        };
        Ok(Self {
            server,
            expected: ExpectedWindowsPeer {
                process_id: expected_process.process_id,
                process_creation_time: expected_process.process_creation_time,
                logon_sid_hash: expected_process.logon_sid_hash,
                windows_session_id: expected_process.windows_session_id,
            },
        })
    }

    /// Wait for the sole local client to connect.
    pub async fn connect(&mut self) -> Result<(), WindowsAgentPipeError> {
        self.server.connect().await.map_err(Into::into)
    }

    /// Inspect pipe-effective and process-primary tokens without trusting protocol fields.
    pub fn inspect_peer(&self) -> Result<VerifiedWindowsAgentPeer, WindowsAgentPipeError> {
        let peer = inspect_connected_peer(&self.server)?;
        if peer.identity.process_id != self.expected.process_id
            || peer.identity.process_creation_time != self.expected.process_creation_time
            || peer.identity.logon_sid_hash != self.expected.logon_sid_hash
            || peer.identity.windows_session_id != self.expected.windows_session_id
        {
            return Err(WindowsAgentPipeError::PeerIdentityMismatch);
        }
        Ok(peer)
    }

    /// Consume the wrapper and retain the connected asynchronous stream.
    pub fn into_stream(self) -> NamedPipeServer {
        self.server
    }
}

struct ExpectedWindowsPeer {
    process_id: u32,
    process_creation_time: u64,
    logon_sid_hash: [u8; 32],
    windows_session_id: u32,
}

/// Verified peer identity plus a process-object handle pinning against PID reuse.
pub struct VerifiedWindowsAgentPeer {
    identity: ObservedAgentIdentity,
    process: Owned<HANDLE>,
}

// Windows kernel process handles are process-wide and may be retained/dropped
// from another thread; this wrapper never dereferences the raw handle value.
unsafe impl Send for VerifiedWindowsAgentPeer {}

/// Launcher-owned process observation captured before bootstrap disclosure.
pub struct VerifiedWindowsProcess {
    process_id: u32,
    process_creation_time: u64,
    logon_sid: Vec<u8>,
    logon_sid_hash: [u8; 32],
    windows_session_id: u32,
    process: Owned<HANDLE>,
}

// Windows kernel process handles are process-wide and may be retained/dropped
// from another thread; this wrapper never dereferences the raw handle value.
unsafe impl Send for VerifiedWindowsProcess {}

impl VerifiedWindowsProcess {
    /// Process identifier bound to the retained process object.
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Creation `FILETIME` bound to the retained process object.
    pub fn process_creation_time(&self) -> u64 {
        self.process_creation_time
    }

    /// Validated raw logon SID used only for local DACL construction.
    pub fn logon_sid(&self) -> &[u8] {
        &self.logon_sid
    }

    /// Domain-separated logon SID digest used in agent IPC.
    pub fn logon_sid_hash(&self) -> &[u8; 32] {
        &self.logon_sid_hash
    }

    /// Primary-token Windows session id.
    pub fn windows_session_id(&self) -> u32 {
        self.windows_session_id
    }

    /// Whether the process object remains pinned against PID reuse.
    pub fn holds_process_object(&self) -> bool {
        !self.process.is_invalid()
    }
}

/// Inspect and retain one launcher-created process before sending bootstrap secrets.
pub fn inspect_windows_process(
    process_id: u32,
) -> Result<VerifiedWindowsProcess, WindowsAgentPipeError> {
    if process_id == 0 {
        return Err(WindowsAgentPipeError::PeerIdentityMismatch);
    }
    let process_handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)? };
    // SAFETY: OpenProcess returned a new owned process handle.
    let process = unsafe { Owned::new(process_handle) };
    let process_creation_time = process_creation_time(*process)?;
    let token = open_process_token(*process)?;
    let snapshot = token_snapshot(&token, false)?;
    let logon_sid_hash =
        hash_windows_logon_sid(&snapshot.logon_sid).ok_or(WindowsAgentPipeError::InvalidSid)?;
    Ok(VerifiedWindowsProcess {
        process_id,
        process_creation_time,
        logon_sid: snapshot.logon_sid,
        logon_sid_hash,
        windows_session_id: snapshot.session_id,
        process,
    })
}

impl VerifiedWindowsAgentPeer {
    /// Trusted identity derived from the pipe and process tokens.
    pub fn identity(&self) -> &ObservedAgentIdentity {
        &self.identity
    }

    /// Whether the process object remains pinned for this verification lifetime.
    pub fn holds_process_object(&self) -> bool {
        !self.process.is_invalid()
    }

    /// Clone the compact identity while retaining this guard in the caller.
    pub fn cloned_identity(&self) -> ObservedAgentIdentity {
        self.identity.clone()
    }
}

/// Return the current process token's binary logon SID.
pub fn current_process_logon_sid() -> Result<Vec<u8>, WindowsAgentPipeError> {
    let token = open_process_token(unsafe { GetCurrentProcess() })?;
    token_logon_sid(&token)
}

fn current_process_user_sid() -> Result<Vec<u8>, WindowsAgentPipeError> {
    let token = open_process_token(unsafe { GetCurrentProcess() })?;
    token_user_sid(&token)
}

fn inspect_connected_peer(
    server: &NamedPipeServer,
) -> Result<VerifiedWindowsAgentPeer, WindowsAgentPipeError> {
    let pipe_handle = HANDLE(server.as_raw_handle());
    let mut process_id = 0_u32;
    let mut pipe_session_id = 0_u32;
    unsafe {
        GetNamedPipeClientProcessId(pipe_handle, &mut process_id)?;
        GetNamedPipeClientSessionId(pipe_handle, &mut pipe_session_id)?;
        ImpersonateNamedPipeClient(pipe_handle)?;
    }
    if process_id == 0 || pipe_session_id == 0 {
        abort_impersonation();
        return Err(WindowsAgentPipeError::PeerIdentityMismatch);
    }

    let impersonation = ImpersonationGuard::new();
    let effective_token = open_thread_token()?;
    let effective = token_snapshot(&effective_token, true)?;
    impersonation.revert();

    let process_handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id)? };
    // SAFETY: OpenProcess returned a new owned process handle.
    let process = unsafe { Owned::new(process_handle) };
    let creation_time = process_creation_time(*process)?;
    let primary_token = open_process_token(*process)?;
    let primary = token_snapshot(&primary_token, false)?;

    if effective.user_sid != primary.user_sid
        || effective.logon_sid != primary.logon_sid
        || effective.session_id != primary.session_id
        || effective.session_id != pipe_session_id
    {
        return Err(WindowsAgentPipeError::PeerIdentityMismatch);
    }
    if effective.caller_kind != AgentCallerKind::InteractiveUser {
        return Err(WindowsAgentPipeError::UntrustedCaller);
    }
    let logon_sid_hash =
        hash_windows_logon_sid(&effective.logon_sid).ok_or(WindowsAgentPipeError::InvalidSid)?;
    Ok(VerifiedWindowsAgentPeer {
        identity: ObservedAgentIdentity {
            caller_kind: effective.caller_kind,
            process_id,
            process_creation_time: creation_time,
            logon_sid_hash,
            windows_session_id: pipe_session_id,
        },
        process,
    })
}

struct TokenSnapshot {
    user_sid: Vec<u8>,
    logon_sid: Vec<u8>,
    session_id: u32,
    caller_kind: AgentCallerKind,
}

fn token_snapshot(
    token: &Owned<HANDLE>,
    require_impersonation: bool,
) -> Result<TokenSnapshot, WindowsAgentPipeError> {
    if require_impersonation {
        let level = token_scalar::<SECURITY_IMPERSONATION_LEVEL>(token, TokenImpersonationLevel)?;
        if level == SecurityAnonymous {
            return Err(WindowsAgentPipeError::UntrustedCaller);
        }
    }
    let caller_kind = if require_impersonation {
        let anonymous = token_has_well_known_sid(token, WinAnonymousSid)?;
        let network = token_has_well_known_sid(token, WinNetworkSid)?;
        let interactive = token_has_well_known_sid(token, WinInteractiveSid)?
            || token_has_well_known_sid(token, WinRemoteLogonIdSid)?;
        if anonymous {
            AgentCallerKind::Anonymous
        } else if network {
            AgentCallerKind::Network
        } else if interactive {
            AgentCallerKind::InteractiveUser
        } else {
            AgentCallerKind::NonInteractive
        }
    } else {
        AgentCallerKind::NonInteractive
    };
    Ok(TokenSnapshot {
        user_sid: token_user_sid(token)?,
        logon_sid: token_logon_sid(token)?,
        session_id: token_scalar::<u32>(token, TokenSessionId)?,
        caller_kind,
    })
}

fn open_process_token(process: HANDLE) -> Result<Owned<HANDLE>, WindowsAgentPipeError> {
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token)? };
    // SAFETY: OpenProcessToken returned a new owned token handle.
    Ok(unsafe { Owned::new(token) })
}

fn open_thread_token() -> Result<Owned<HANDLE>, WindowsAgentPipeError> {
    let mut token = HANDLE::default();
    unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token)? };
    // SAFETY: OpenThreadToken returned a new owned token handle.
    Ok(unsafe { Owned::new(token) })
}

fn process_creation_time(process: HANDLE) -> Result<u64, WindowsAgentPipeError> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user)? };
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

fn token_user_sid(token: &Owned<HANDLE>) -> Result<Vec<u8>, WindowsAgentPipeError> {
    let buffer = token_information(token, TokenUser)?;
    if buffer.byte_len < size_of::<TOKEN_USER>() {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    // SAFETY: token information storage is usize-aligned and size-checked.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    copy_valid_sid(user.User.Sid)
}

fn token_logon_sid(token: &Owned<HANDLE>) -> Result<Vec<u8>, WindowsAgentPipeError> {
    let buffer = token_information(token, TokenLogonSid)?;
    if buffer.byte_len < size_of::<TOKEN_GROUPS>() {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    // SAFETY: token information storage is usize-aligned and size-checked.
    let groups = unsafe { &*buffer.as_ptr().cast::<TOKEN_GROUPS>() };
    let first = ptr::addr_of!(groups.Groups).cast::<SID_AND_ATTRIBUTES>();
    let offset = first as usize - buffer.as_ptr() as usize;
    let count = usize::try_from(groups.GroupCount)
        .map_err(|_| WindowsAgentPipeError::InvalidTokenInformation)?;
    let available = buffer.byte_len.saturating_sub(offset) / size_of::<SID_AND_ATTRIBUTES>();
    if count == 0 || count > available || count > 1_024 {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    // SAFETY: count is bounded by the returned token-information buffer.
    let entries = unsafe { std::slice::from_raw_parts(first, count) };
    let logon_mask = SE_GROUP_LOGON_ID as u32;
    let mut matching = entries
        .iter()
        .filter(|entry| entry.Attributes & logon_mask == logon_mask);
    let sid = matching
        .next()
        .ok_or(WindowsAgentPipeError::InvalidTokenInformation)?;
    if matching.next().is_some() {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    copy_valid_sid(sid.Sid)
}

fn token_scalar<T: Copy>(
    token: &Owned<HANDLE>,
    class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Result<T, WindowsAgentPipeError> {
    let buffer = token_information(token, class)?;
    if buffer.byte_len < size_of::<T>() {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    // SAFETY: storage is usize-aligned; all requested scalar types fit that alignment.
    Ok(unsafe { *buffer.as_ptr().cast::<T>() })
}

struct AlignedTokenBuffer {
    words: Vec<usize>,
    byte_len: usize,
}

impl AlignedTokenBuffer {
    fn as_ptr(&self) -> *const u8 {
        self.words.as_ptr().cast()
    }
}

fn token_information(
    token: &Owned<HANDLE>,
    class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
) -> Result<AlignedTokenBuffer, WindowsAgentPipeError> {
    let mut required = 0_u32;
    let _ = unsafe { GetTokenInformation(**token, class, None, 0, &mut required) };
    let required =
        usize::try_from(required).map_err(|_| WindowsAgentPipeError::InvalidTokenInformation)?;
    if required == 0 || required > MAX_TOKEN_INFORMATION_BYTES {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    let word_count = required.div_ceil(size_of::<usize>());
    let mut words = vec![0_usize; word_count];
    let mut returned = required as u32;
    unsafe {
        GetTokenInformation(
            **token,
            class,
            Some(words.as_mut_ptr().cast()),
            required as u32,
            &mut returned,
        )?
    };
    let returned =
        usize::try_from(returned).map_err(|_| WindowsAgentPipeError::InvalidTokenInformation)?;
    if returned == 0 || returned > size_of_val(words.as_slice()) {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    Ok(AlignedTokenBuffer {
        words,
        byte_len: returned,
    })
}

fn token_has_well_known_sid(
    token: &Owned<HANDLE>,
    kind: WELL_KNOWN_SID_TYPE,
) -> Result<bool, WindowsAgentPipeError> {
    let mut sid = [0_u8; SECURITY_MAX_SID_SIZE];
    let mut sid_len = sid.len() as u32;
    let sid_ptr = PSID(sid.as_mut_ptr().cast());
    unsafe { CreateWellKnownSid(kind, None, Some(sid_ptr), &mut sid_len)? };
    let mut member = BOOL::default();
    unsafe { CheckTokenMembership(Some(**token), sid_ptr, &mut member)? };
    Ok(member.as_bool())
}

fn copy_valid_sid(sid: PSID) -> Result<Vec<u8>, WindowsAgentPipeError> {
    if sid.0.is_null() || !unsafe { IsValidSid(sid) }.as_bool() {
        return Err(WindowsAgentPipeError::InvalidSid);
    }
    let length = unsafe { GetLengthSid(sid) } as usize;
    if !(8..=SECURITY_MAX_SID_SIZE).contains(&length) {
        return Err(WindowsAgentPipeError::InvalidSid);
    }
    // SAFETY: IsValidSid and GetLengthSid validated this exact range.
    Ok(unsafe { std::slice::from_raw_parts(sid.0.cast::<u8>(), length) }.to_vec())
}

fn validate_sid_bytes(sid: &[u8]) -> Result<(), WindowsAgentPipeError> {
    if !(8..=SECURITY_MAX_SID_SIZE).contains(&sid.len()) {
        return Err(WindowsAgentPipeError::InvalidSid);
    }
    let sid_ptr = PSID(sid.as_ptr().cast_mut().cast());
    if !unsafe { IsValidSid(sid_ptr) }.as_bool()
        || unsafe { GetLengthSid(sid_ptr) } as usize != sid.len()
    {
        return Err(WindowsAgentPipeError::InvalidSid);
    }
    Ok(())
}

fn sid_string(sid: &[u8]) -> Result<String, WindowsAgentPipeError> {
    validate_sid_bytes(sid)?;
    let mut value = PWSTR::null();
    unsafe { ConvertSidToStringSidW(PSID(sid.as_ptr().cast_mut().cast()), &mut value)? };
    // SAFETY: ConvertSidToStringSidW returned LocalAlloc-owned memory.
    let allocated = unsafe { Owned::new(HLOCAL(value.0.cast())) };
    let string = unsafe { value.to_string() }.map_err(|_| WindowsAgentPipeError::InvalidSid)?;
    drop(allocated);
    Ok(string)
}

fn security_descriptor_from_sddl(sddl: &str) -> Result<Owned<HLOCAL>, WindowsAgentPipeError> {
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
    if descriptor.0.is_null() {
        return Err(WindowsAgentPipeError::InvalidTokenInformation);
    }
    // SAFETY: conversion returned LocalAlloc-owned memory.
    Ok(unsafe { Owned::new(HLOCAL(descriptor.0)) })
}

fn validate_pipe_name(pipe_name: &str) -> Result<(), WindowsAgentPipeError> {
    if !pipe_name.starts_with(r"\\.\pipe\mrd-agent-")
        || pipe_name.len() > MAX_PIPE_NAME_BYTES
        || pipe_name.contains('/')
        || pipe_name.contains("..")
        || pipe_name.contains('\0')
    {
        return Err(WindowsAgentPipeError::InvalidPipeName);
    }
    Ok(())
}

struct ImpersonationGuard {
    active: bool,
}

impl ImpersonationGuard {
    fn new() -> Self {
        Self { active: true }
    }

    fn revert(mut self) {
        if unsafe { RevertToSelf() }.is_err() {
            std::process::abort();
        }
        self.active = false;
    }
}

impl Drop for ImpersonationGuard {
    fn drop(&mut self) {
        if self.active && unsafe { RevertToSelf() }.is_err() {
            std::process::abort();
        }
    }
}

fn abort_impersonation() {
    if unsafe { RevertToSelf() }.is_err() {
        std::process::abort();
    }
}
