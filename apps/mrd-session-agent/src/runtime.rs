//! Fail-closed interactive-agent registration and event loop.

use crate::capabilities::AgentCapabilities;
use crate::consent::{
    AuthorityInvalidation, BackendCompletion, ConsentAbortReason, ConsentBackend, ConsentManager,
    ConsentManagerBeginOutcome, ConsentRegistryError, TrustedConsentContext, TrustedSessionBinding,
};
use crate::input::InputBackend;
use mrd_agent_ipc::{
    decode_frame, registration_proof_signing_bytes, validate_execute_command, write_frame,
    AgentCapabilitySnapshot, AgentChallenge, AgentEventContext, AgentHeartbeat, AgentRegister,
    AgentRegistered, AgentStopping, AgentToService, AuthorizedCommand, CancelConsent,
    CommandOutcome, CommandResult, ConsentDecision, ConsentRequest, ConsentResult, DesktopKind,
    ExecuteGrantVerifier, ExecutionContext, FrameError, InputAck, InputAckOutcome, PeerBinding,
    RegisteredAgentIdentity, RenderBoundaryMetrics, ServiceToAgent, StoppingReason,
    AGENT_IPC_FRAME_HEADER_BYTES, AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR,
    AGENT_IPC_PROTOCOL_MINOR, AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS,
};
use mrd_proto::SessionId;
#[cfg(unix)]
use std::path::{Component, Path};
use std::{
    collections::{HashMap, HashSet},
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};

const INBOUND_QUEUE_CAPACITY: usize = 32;
const OUTBOUND_QUEUE_CAPACITY: usize = 32;
const REPLAY_LEDGER_CAPACITY: usize = 4_096;
const MAX_CLEANUP_RECEIPTS: usize = 256;
const CLEANUP_RECEIPT_RETENTION: Duration = Duration::from_secs(5 * 60);
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
const CAPTURE_PUMP_INTERVAL: Duration = Duration::from_millis(8);

struct OutboundFrame {
    message: AgentToService,
    deadline: Option<Instant>,
    completion: Option<oneshot::Sender<OutboundWriteResult>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundWriteResult {
    Flushed { completed_at: Instant },
    DeadlineExceeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterTerminal {
    Failed,
}

struct OutboundWriter {
    sender: Option<mpsc::Sender<OutboundFrame>>,
    terminal_deadline: watch::Sender<Option<Instant>>,
    terminal: watch::Receiver<Option<WriterTerminal>>,
    task: Option<JoinHandle<()>>,
}

impl OutboundWriter {
    fn spawn<W>(mut writer: W) -> Self
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (sender, mut receiver) = mpsc::channel::<OutboundFrame>(OUTBOUND_QUEUE_CAPACITY);
        let (terminal_deadline, mut terminal_deadline_rx) = watch::channel(None);
        let (terminal_sender, terminal) = watch::channel(None);
        let task = tokio::spawn(async move {
            loop {
                let frame = match receive_before_terminal_deadline(
                    &mut receiver,
                    &mut terminal_deadline_rx,
                )
                .await
                {
                    Ok(Some(frame)) => frame,
                    Ok(None) => return,
                    Err(()) => {
                        terminal_sender.send_replace(Some(WriterTerminal::Failed));
                        return;
                    }
                };
                let terminal_frame = frame.deadline.is_some();
                let result = write_frame_under_terminal_deadline(
                    &mut writer,
                    &frame.message,
                    &mut terminal_deadline_rx,
                )
                .await;
                if let Some(completion) = frame.completion {
                    let _ = completion.send(result);
                }
                if terminal_frame || !matches!(result, OutboundWriteResult::Flushed { .. }) {
                    if !matches!(result, OutboundWriteResult::Flushed { .. }) {
                        terminal_sender.send_replace(Some(WriterTerminal::Failed));
                    }
                    return;
                }
            }
        });
        Self {
            sender: Some(sender),
            terminal_deadline,
            terminal,
            task: Some(task),
        }
    }

    fn enqueue(&self, message: AgentToService) -> Result<(), AgentRuntimeError> {
        self.sender
            .as_ref()
            .ok_or(AgentRuntimeError::OutboundUnavailable)?
            .try_send(OutboundFrame {
                message,
                deadline: None,
                completion: None,
            })
            .map_err(|_| AgentRuntimeError::OutboundUnavailable)
    }

    fn sender(&self) -> Result<mpsc::Sender<OutboundFrame>, AgentRuntimeError> {
        self.sender
            .as_ref()
            .cloned()
            .ok_or(AgentRuntimeError::OutboundUnavailable)
    }

    fn arm_terminal_deadline(&mut self, deadline: Instant) -> Result<(), AgentRuntimeError> {
        if self
            .task
            .as_ref()
            .is_none_or(tokio::task::JoinHandle::is_finished)
            || self.terminal_deadline.borrow().is_some()
        {
            return Err(AgentRuntimeError::OutboundUnavailable);
        }
        self.terminal_deadline.send_replace(Some(deadline));
        Ok(())
    }

    fn terminal_subscription(&self) -> watch::Receiver<Option<WriterTerminal>> {
        self.terminal.clone()
    }

    async fn terminal_changed(&mut self) -> WriterTerminal {
        match self.terminal.changed().await {
            Ok(()) => self
                .terminal
                .borrow_and_update()
                .unwrap_or(WriterTerminal::Failed),
            Err(_) => WriterTerminal::Failed,
        }
    }

    async fn close_and_join(&mut self) -> Result<(), AgentRuntimeError> {
        self.sender.take();
        match self.task.take() {
            Some(task) => task
                .await
                .map_err(|_| AgentRuntimeError::OutboundUnavailable),
            None => Ok(()),
        }
    }

    async fn abort_and_join(&mut self) {
        self.sender.take();
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for OutboundWriter {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

struct ReaderTaskGuard {
    task: JoinHandle<()>,
}

impl ReaderTaskGuard {
    fn new(task: JoinHandle<()>) -> Self {
        Self { task }
    }

    async fn abort_and_join(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for ReaderTaskGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn receive_before_terminal_deadline(
    receiver: &mut mpsc::Receiver<OutboundFrame>,
    terminal_deadline: &mut watch::Receiver<Option<Instant>>,
) -> Result<Option<OutboundFrame>, ()> {
    loop {
        let deadline = *terminal_deadline.borrow_and_update();
        match deadline {
            Some(deadline) => {
                return tokio::select! {
                    biased;
                    _ = tokio::time::sleep_until(deadline) => Err(()),
                    frame = receiver.recv() => Ok(frame),
                };
            }
            None => {
                tokio::select! {
                    biased;
                    changed = terminal_deadline.changed() => {
                        if changed.is_err() {
                            return Err(());
                        }
                    }
                    frame = receiver.recv() => return Ok(frame),
                }
            }
        }
    }
}

async fn write_frame_under_terminal_deadline<W>(
    writer: &mut W,
    message: &AgentToService,
    terminal_deadline: &mut watch::Receiver<Option<Instant>>,
) -> OutboundWriteResult
where
    W: AsyncWrite + Unpin,
{
    let mut write = std::pin::pin!(write_frame(writer, message));
    loop {
        let deadline = *terminal_deadline.borrow_and_update();
        match deadline {
            Some(deadline) => {
                let guarded_write = std::future::poll_fn(|context| {
                    if Instant::now() >= deadline {
                        return std::task::Poll::Ready(OutboundWriteResult::DeadlineExceeded);
                    }
                    match std::future::Future::poll(write.as_mut(), context) {
                        std::task::Poll::Ready(Ok(())) => {
                            let completed_at = Instant::now();
                            if completed_at < deadline {
                                std::task::Poll::Ready(OutboundWriteResult::Flushed {
                                    completed_at,
                                })
                            } else {
                                std::task::Poll::Ready(OutboundWriteResult::DeadlineExceeded)
                            }
                        }
                        std::task::Poll::Ready(Err(_)) => {
                            std::task::Poll::Ready(OutboundWriteResult::Failed)
                        }
                        std::task::Poll::Pending => std::task::Poll::Pending,
                    }
                });
                tokio::pin!(guarded_write);
                return tokio::select! {
                    biased;
                    _ = tokio::time::sleep_until(deadline) => {
                        OutboundWriteResult::DeadlineExceeded
                    }
                    result = &mut guarded_write => result,
                };
            }
            None => {
                tokio::select! {
                    biased;
                    changed = terminal_deadline.changed() => {
                        if changed.is_err() {
                            return OutboundWriteResult::Failed;
                        }
                    }
                    result = &mut write => {
                        return match result {
                            Ok(()) => OutboundWriteResult::Flushed {
                                completed_at: Instant::now(),
                            },
                            Err(_) => OutboundWriteResult::Failed,
                        };
                    }
                }
            }
        }
    }
}

async fn wait_for_writer_terminal(terminal: &mut watch::Receiver<Option<WriterTerminal>>) {
    if terminal.borrow_and_update().is_some() {
        return;
    }
    let _ = terminal.changed().await;
}

/// Platform-local endpoint; network transports are deliberately unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateAgentEndpoint {
    value: PathBuf,
}

impl PrivateAgentEndpoint {
    /// Parse a Windows named pipe or absolute Unix-domain socket path.
    pub fn parse(value: impl AsRef<str>) -> Result<Self, PrivateEndpointError> {
        let value = value.as_ref().trim();
        if value.is_empty() || value.len() > 512 {
            return Err(PrivateEndpointError::InvalidEndpoint);
        }

        #[cfg(windows)]
        if !value.starts_with(r"\\.\pipe\")
            || value == r"\\.\pipe\"
            || value.contains('/')
            || value.contains("..")
        {
            return Err(PrivateEndpointError::InvalidEndpoint);
        }

        #[cfg(unix)]
        {
            let path = Path::new(value);
            if !path.is_absolute()
                || path
                    .components()
                    .any(|component| component == Component::ParentDir)
            {
                return Err(PrivateEndpointError::InvalidEndpoint);
            }
        }

        #[cfg(not(any(windows, unix)))]
        return Err(PrivateEndpointError::UnsupportedPlatform);

        #[allow(unreachable_code)]
        Ok(Self {
            value: PathBuf::from(value),
        })
    }

    #[cfg(windows)]
    fn as_pipe_name(&self) -> &str {
        self.value
            .to_str()
            .expect("validated Windows named-pipe path is UTF-8")
    }

    #[cfg(unix)]
    fn as_socket_path(&self) -> &Path {
        &self.value
    }
}

/// Failures before an authenticated local stream is established.
#[derive(Debug, Error)]
pub enum PrivateEndpointError {
    /// Endpoint is empty, oversized, relative, traversing, or not platform-local.
    #[error("agent endpoint is not a valid private local endpoint")]
    InvalidEndpoint,
    /// This target has no private endpoint implementation yet.
    #[error("private agent endpoints are unsupported on this platform")]
    UnsupportedPlatform,
    /// The configured local endpoint could not be opened.
    #[error("private agent endpoint connection failed")]
    Io(#[from] std::io::Error),
}

/// Connected platform-local stream type.
#[cfg(windows)]
pub type PrivateAgentStream = tokio::net::windows::named_pipe::NamedPipeClient;
/// Connected platform-local stream type.
#[cfg(unix)]
pub type PrivateAgentStream = tokio::net::UnixStream;

/// Connect only to the validated platform-local endpoint.
#[cfg(windows)]
pub async fn connect_private_endpoint(
    endpoint: &PrivateAgentEndpoint,
) -> Result<PrivateAgentStream, PrivateEndpointError> {
    use std::os::windows::io::RawHandle;
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, FILE_READ_ATTRIBUTES, FILE_READ_DATA,
            FILE_SHARE_MODE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, OPEN_EXISTING,
            SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, SYNCHRONIZE,
        },
    };

    let wide: Vec<u16> = endpoint
        .as_pipe_name()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let desired_access = FILE_READ_DATA.0
        | FILE_WRITE_DATA.0
        | FILE_READ_ATTRIBUTES.0
        | FILE_WRITE_ATTRIBUTES.0
        | SYNCHRONIZE.0;
    let flags = FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            desired_access,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .map_err(|_| PrivateEndpointError::Io(std::io::Error::last_os_error()))?;
    // SAFETY: CreateFileW returned the sole owned overlapped pipe handle. Tokio
    // takes ownership and closes it; the raw value is not used again.
    unsafe { PrivateAgentStream::from_raw_handle(handle.0 as RawHandle) }
        .map_err(PrivateEndpointError::Io)
}

/// Connect only to the validated platform-local endpoint.
#[cfg(unix)]
pub async fn connect_private_endpoint(
    endpoint: &PrivateAgentEndpoint,
) -> Result<PrivateAgentStream, PrivateEndpointError> {
    tokio::net::UnixStream::connect(endpoint.as_socket_path())
        .await
        .map_err(PrivateEndpointError::Io)
}

/// Immutable process and interactive-session identity for one agent launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescriptor {
    agent_instance_id: [u8; 16],
    process_id: u32,
    process_creation_time: u64,
    logon_sid_hash: [u8; 32],
    windows_session_id: u32,
    agent_nonce: [u8; 32],
    desktop_epoch: u64,
}

impl SessionDescriptor {
    /// Construct and validate immutable identity supplied by the platform launcher.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_instance_id: [u8; 16],
        process_id: u32,
        process_creation_time: u64,
        logon_sid_hash: [u8; 32],
        windows_session_id: u32,
        agent_nonce: [u8; 32],
        desktop_epoch: u64,
    ) -> Result<Self, AgentRuntimeError> {
        let descriptor = Self {
            agent_instance_id,
            process_id,
            process_creation_time,
            logon_sid_hash,
            windows_session_id,
            agent_nonce,
            desktop_epoch,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    fn validate(&self) -> Result<(), AgentRuntimeError> {
        if self.agent_instance_id.iter().all(|byte| *byte == 0)
            || self.process_id == 0
            || self.process_creation_time == 0
            || self.logon_sid_hash.iter().all(|byte| *byte == 0)
            || self.windows_session_id == 0
            || self.agent_nonce.iter().all(|byte| *byte == 0)
            || self.desktop_epoch == 0
        {
            return Err(AgentRuntimeError::InvalidConfiguration);
        }
        Ok(())
    }

    fn register(&self, agent_key_id: [u8; 32]) -> AgentRegister {
        AgentRegister {
            agent_instance_id: self.agent_instance_id,
            process_id: self.process_id,
            process_creation_time: self.process_creation_time,
            logon_sid_hash: self.logon_sid_hash,
            windows_session_id: self.windows_session_id,
            agent_key_id,
            agent_nonce: self.agent_nonce,
        }
    }
}

/// Runtime timing and immutable session configuration.
#[derive(Debug, Clone)]
pub struct AgentRuntimeConfig {
    /// Immutable identity for this process launch.
    pub session: SessionDescriptor,
    /// Monotonic interval between liveness events.
    pub heartbeat_interval: Duration,
    /// Maximum time to wait for the one registration challenge.
    pub handshake_timeout: Duration,
}

/// Wall-clock source used for signed windows and event timestamps.
pub trait AgentClock: Send + Sync {
    /// Current Unix epoch time in milliseconds.
    fn now_ms(&self) -> u64;
}

/// Production wall clock.
#[derive(Debug, Default)]
pub struct SystemClock;

impl AgentClock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

/// Registration-signing failures that do not expose secret material.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum RegistrationSigningError {
    /// No authenticated signing bootstrap is available.
    #[error("registration signer is unavailable")]
    Unavailable,
    /// The configured signer could not produce a proof.
    #[error("registration signer failed")]
    Failed,
}

/// Process-local signer provisioned by the authenticated launcher.
pub trait RegistrationSigner: Send + Sync {
    /// Identifier of the pre-trusted verification key.
    fn key_id(&self) -> [u8; 32];
    /// Sign the canonical registration transcript.
    fn sign(&self, message: &[u8]) -> Result<[u8; 64], RegistrationSigningError>;
}

/// Trusted platform observation used to invalidate grants across desktop changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustedDesktopState {
    /// Monotonic desktop generation observed by the platform adapter.
    pub desktop_epoch: u64,
    /// Current input desktop kind.
    pub desktop_kind: DesktopKind,
}

/// Independent source of current desktop state.
pub trait TrustedDesktopStateSource: Send + Sync {
    /// Return the current trusted state from a fast in-memory snapshot, or
    /// `None` when it cannot be established.
    ///
    /// This method must not perform platform I/O or wait for a native desktop
    /// probe. Native watchers update the snapshot in the background.
    fn current_state(&self) -> Option<TrustedDesktopState>;

    /// Subscribe to trusted-state changes.
    ///
    /// The source must keep the corresponding sender alive for its own trusted
    /// lifetime and publish a new watch revision after updating trusted state
    /// for every possible identity change. The payload is only a wake-up hint;
    /// callers re-read [`Self::current_state`] after every notification. A
    /// closed receiver means desktop authority is unavailable and causes the
    /// runtime to revoke local authority and fail-stop. Subscription must also
    /// be an immediate in-memory operation and must not perform platform I/O or
    /// wait for native state.
    fn subscribe(&self) -> tokio::sync::watch::Receiver<()>;
}

/// A capture worker failed or returned a resource with mismatched identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("authorized capture resource failed")]
pub struct CaptureAccessUnitsError;

/// Product backend boundary that can receive only validated commands.
pub trait AuthorizedCommandExecutor: Send {
    /// Capabilities implemented by this exact backend.
    fn capabilities(&self) -> AgentCapabilities;
    /// Execute one already-authorized command without blocking the event loop.
    fn execute(&mut self, command: AuthorizedCommand) -> CommandOutcome;
    /// Drain a bounded batch of encoded frames from authorized capture resources.
    /// `Err` signals a failed capture worker or a resource identity mismatch.
    fn capture_access_units(
        &mut self,
    ) -> Result<Vec<crate::media::EncodedMediaAccessUnit>, CaptureAccessUnitsError> {
        Ok(Vec::new())
    }
    /// Whether a capture worker exists and needs the bounded frame-pump timer.
    fn has_capture_resources(&self) -> bool {
        false
    }
    /// Route one authenticated encoded unit to an already-authorized render resource.
    fn render_access_unit(&mut self, _unit: mrd_agent_ipc::RenderAccessUnit) -> bool {
        false
    }
    /// Snapshot cumulative metrics for live render resources.
    fn render_metrics(&self) -> Vec<crate::render::RenderAdapterMetrics> {
        Vec::new()
    }
    /// Revoke every product resource owned by one invalidated logical session.
    fn revoke_session(&mut self, _session_id: &SessionId) -> bool {
        true
    }
}

/// Normal termination reason for the agent event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentExit {
    /// A valid StopAgent message requested immediate graceful shutdown.
    StoppedByService,
    /// The private machine-service stream disconnected.
    ServiceDisconnected,
}

/// Agent runtime configuration, framing, and protocol failures.
#[derive(Debug, Error)]
pub enum AgentRuntimeError {
    /// Immutable identity, signer, or timing configuration is invalid.
    #[error("agent runtime configuration is invalid")]
    InvalidConfiguration,
    /// The machine service did not challenge registration in time.
    #[error("agent registration challenge timed out")]
    HandshakeTimeout,
    /// The private stream closed before registration completed.
    #[error("machine service disconnected during registration")]
    DisconnectedDuringHandshake,
    /// Registration received a message other than the one expected challenge.
    #[error("agent registration expected exactly one challenge")]
    ExpectedChallenge,
    /// Challenge identity, shape, or time window did not match this process.
    #[error("agent registration challenge is invalid")]
    InvalidChallenge,
    /// Registration signing failed.
    #[error(transparent)]
    RegistrationSigning(#[from] RegistrationSigningError),
    /// Private control framing failed.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// The event sequence reached its terminal sentinel.
    #[error("agent event sequence exhausted")]
    EventSequenceExhausted,
    /// The service sent a message unsupported by the Task 22 shell.
    #[error("unsupported service-to-agent message")]
    UnsupportedMessage,
    /// A StopAgent message carried an invalid request identity or deadline.
    #[error("StopAgent request is invalid")]
    InvalidStopRequest,
    /// A grant or command identifier was reused for different semantics.
    #[error("execute command replay identity conflict")]
    ReplayConflict,
    /// The bounded replay ledger is full and cannot safely evict active history.
    #[error("execute command replay ledger is full")]
    ReplayCapacityExceeded,
    /// A product-enabled runtime could not establish current desktop state.
    #[error("trusted desktop state is unavailable")]
    DesktopStateUnavailable,
    /// The bounded agent-local consent authority could not safely progress.
    #[error("agent-local consent authority is unavailable")]
    ConsentStateUnavailable,
    /// A fresh local authority could not synchronously retire old input state.
    #[error("session input cleanup failed")]
    InputCleanupFailed,
    /// A revoked session retained a desktop-bound media resource.
    #[error("session media cleanup failed")]
    MediaCleanupFailed,
    /// Capture failed or emitted a frame outside its live consent binding.
    #[error("session capture failed")]
    MediaCaptureFailed,
    /// An encoded frame did not match a live authorized render resource.
    #[error("render access unit is not authorized for a live resource")]
    InvalidRenderAccessUnit,
    /// A render adapter returned malformed cumulative metrics.
    #[error("render boundary metrics are invalid")]
    InvalidRenderMetrics,
    /// The bounded post-registration writer stopped or could not accept output.
    #[error("agent outbound channel is unavailable")]
    OutboundUnavailable,
}

/// Startup failures spanning local endpoint connection and the agent runtime.
#[derive(Debug, Error)]
pub enum AgentStartError {
    /// The configured private endpoint could not be validated or opened.
    #[error(transparent)]
    Endpoint(#[from] PrivateEndpointError),
    /// Registration or the connected event loop failed.
    #[error(transparent)]
    Runtime(#[from] AgentRuntimeError),
}

struct AttendedAuthority {
    manager: ConsentManager,
    verifier: Arc<dyn ExecuteGrantVerifier + Send + Sync>,
    desktop_state: Arc<dyn TrustedDesktopStateSource>,
    expected_issuer_key_id: [u8; 32],
    executor: Box<dyn AuthorizedCommandExecutor>,
}

/// A successful Start owns this identity for the rest of this registration.
/// Only Stop may resolve the original authority after consent revocation, and
/// only confirmed native cleanup permits an acknowledgement without execution.
struct CleanupReceipt {
    binding: TrustedSessionBinding,
    capability: mrd_agent_ipc::AgentCapability,
    grant_window: (u64, u64, u64),
    retained_until: Instant,
    retired: bool,
}

/// One connected session-agent runtime.
///
/// Legacy split authority builders are intentionally absent:
///
/// ```compile_fail
/// use mrd_session_agent::runtime::AgentRuntime;
/// fn bypass(runtime: AgentRuntime) {
///     let _ = runtime.with_execution_security(todo!(), todo!(), todo!(), todo!());
/// }
/// ```
///
/// ```compile_fail
/// use mrd_session_agent::runtime::AgentRuntime;
/// fn replace_manager(runtime: AgentRuntime) {
///     let _ = runtime.with_consent_backend(todo!(), todo!(), [1; 32]);
/// }
/// ```
pub struct AgentRuntime {
    config: AgentRuntimeConfig,
    clock: Arc<dyn AgentClock>,
    signer: Arc<dyn RegistrationSigner>,
    authority: Option<AttendedAuthority>,
    input: Option<Box<dyn InputBackend>>,
    last_desktop_state: Option<TrustedDesktopState>,
    replay: ReplayLedger,
    cleanup_receipts: HashMap<[u8; 16], CleanupReceipt>,
    started_resource_ids: HashSet<[u8; 16]>,
    event_sequence: u64,
    cleanup_complete: bool,
}

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }
        if let Some(authority) = self.authority.as_ref() {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                let _ = authority.manager.drain_authority();
            }));
        }
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _ = self.release_input();
        }));
    }
}

impl AgentRuntime {
    /// Construct the fail-closed shell runtime.
    pub fn new(
        config: AgentRuntimeConfig,
        clock: Arc<dyn AgentClock>,
        signer: Arc<dyn RegistrationSigner>,
    ) -> Result<Self, AgentRuntimeError> {
        config.session.validate()?;
        if config.heartbeat_interval.is_zero()
            || config.handshake_timeout.is_zero()
            || signer.key_id().iter().all(|byte| *byte == 0)
        {
            return Err(AgentRuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            config,
            clock,
            signer,
            authority: None,
            input: None,
            last_desktop_state: None,
            replay: ReplayLedger::new(REPLAY_LEDGER_CAPACITY),
            cleanup_receipts: HashMap::new(),
            started_resource_ids: HashSet::new(),
            event_sequence: 0,
            cleanup_complete: false,
        })
    }

    /// Atomically install the attended-consent and command authority product path.
    ///
    /// Consent, execution, input, capabilities, and heartbeat observations all
    /// share this exact desktop source and the manager-owned binding registry.
    pub fn with_attended_authority(
        mut self,
        backend: Arc<dyn ConsentBackend>,
        verifier: Arc<dyn ExecuteGrantVerifier + Send + Sync>,
        desktop_state: Arc<dyn TrustedDesktopStateSource>,
        expected_issuer_key_id: [u8; 32],
        executor: Box<dyn AuthorizedCommandExecutor>,
    ) -> Result<Self, AgentRuntimeError> {
        if self.authority.is_some() || expected_issuer_key_id.iter().all(|byte| *byte == 0) {
            return Err(AgentRuntimeError::InvalidConfiguration);
        }
        self.authority = Some(AttendedAuthority {
            manager: ConsentManager::new(backend),
            verifier,
            desktop_state,
            expected_issuer_key_id,
            executor,
        });
        Ok(self)
    }

    /// Install the platform input backend used by validated StartInput resources.
    pub fn with_input_backend(mut self, input: Box<dyn InputBackend>) -> Self {
        self.input = Some(input);
        self
    }

    /// Connect to a validated local endpoint and run the agent.
    pub async fn run_at_endpoint(
        self,
        endpoint: &PrivateAgentEndpoint,
    ) -> Result<AgentExit, AgentStartError> {
        let stream = connect_private_endpoint(endpoint).await?;
        self.run(stream).await.map_err(AgentStartError::Runtime)
    }

    /// Run registration and the registered event loop on an already-private stream.
    pub async fn run<S>(mut self, stream: S) -> Result<AgentExit, AgentRuntimeError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (reader, writer) = tokio::io::split(stream);
        let (inbound_tx, mut inbound_rx) = mpsc::channel(INBOUND_QUEUE_CAPACITY);
        let mut reader_task = ReaderTaskGuard::new(tokio::spawn(read_loop(reader, inbound_tx)));

        let result = self.run_connected(writer, &mut inbound_rx).await;
        let shutdown_reason = match &result {
            Ok(AgentExit::StoppedByService) => ConsentAbortReason::RuntimeStopping,
            Ok(AgentExit::ServiceDisconnected) | Err(_) => ConsentAbortReason::ServiceDisconnected,
        };
        let final_cleanup = self.terminal_cleanup(shutdown_reason).await;
        reader_task.abort_and_join().await;
        match (result, final_cleanup) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(exit), Ok(())) => Ok(exit),
        }
    }

    async fn run_connected<W>(
        &mut self,
        mut writer: W,
        inbound: &mut mpsc::Receiver<InboundEvent>,
    ) -> Result<AgentExit, AgentRuntimeError>
    where
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let mut desktop_changes = self
            .authority
            .as_ref()
            .map(|authority| authority.desktop_state.subscribe());
        let identity = self.register(&mut writer, inbound).await?;
        let mut outbound = OutboundWriter::spawn(writer);
        let mut capability_revision = 1_u64;
        let result = async {
            let baseline_desktop = self.resolve_desktop_state()?;
            self.last_desktop_state = Some(baseline_desktop);
            self.send_capabilities(
                &outbound,
                &identity,
                capability_revision,
                baseline_desktop,
            )?;

            let mut heartbeat = tokio::time::interval_at(
                Instant::now() + self.config.heartbeat_interval,
                self.config.heartbeat_interval,
            );
            heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut capture = tokio::time::interval(CAPTURE_PUMP_INTERVAL);
            capture.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
            if desktop_changes
                .as_ref()
                .is_some_and(|changes| changes.has_changed().is_err())
            {
                return match self
                    .terminal_cleanup(ConsentAbortReason::ServiceDisconnected)
                    .await
                {
                    Ok(()) => Err(AgentRuntimeError::DesktopStateUnavailable),
                    Err(error) => Err(error),
                };
            }
            let consent_deadline = self
                .authority
                .as_ref()
                .map(|authority| authority.manager.next_deadline())
                .transpose()
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?
                .flatten();
            let event = {
                let capture_enabled = self.authority.as_ref().is_some_and(|authority| authority.executor.has_capture_resources());
                let consent_completion = async {
                    match self.authority.as_mut() {
                        Some(authority) => authority.manager.next_completion().await,
                        None => std::future::pending().await,
                    }
                };
                let consent_deadline_wait = async move {
                    match consent_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                };
                let desktop_change = async {
                    match desktop_changes.as_mut() {
                        Some(changes) => Some(changes.changed().await),
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    biased;
                    _ = outbound.terminal_changed() => RegisteredLoopEvent::WriterTerminal,
                    _ = consent_deadline_wait => RegisteredLoopEvent::ConsentDeadline,
                    completion = consent_completion => RegisteredLoopEvent::Consent(completion),
                    _ = heartbeat.tick() => RegisteredLoopEvent::Heartbeat,
                    inbound_event = inbound.recv() => RegisteredLoopEvent::Inbound(inbound_event),
                    change = desktop_change => RegisteredLoopEvent::DesktopChanged(change),
                    _ = capture.tick(), if capture_enabled => RegisteredLoopEvent::Capture,
                }
            };
            let due_sessions = if !matches!(&event, RegisteredLoopEvent::WriterTerminal) {
                self.reconcile_due_authority()?
            } else {
                Vec::new()
            };
            if matches!(
                &event,
                RegisteredLoopEvent::Inbound(_)
                    | RegisteredLoopEvent::Heartbeat
                    | RegisteredLoopEvent::Consent(Some(_))
                    | RegisteredLoopEvent::ConsentDeadline
                    | RegisteredLoopEvent::Capture
            ) {
                let desktop = self.current_desktop_state()?;
                self.reconcile_desktop_authority(&outbound, desktop)?;
                if self.last_desktop_state != Some(desktop) {
                    self.last_desktop_state = Some(desktop);
                    capability_revision = capability_revision
                        .checked_add(1)
                        .ok_or(AgentRuntimeError::EventSequenceExhausted)?;
                    self.send_capabilities(
                        &outbound,
                        &identity,
                        capability_revision,
                        desktop,
                    )?;
                }
            }
                match event {
                RegisteredLoopEvent::Inbound(inbound_event) => match inbound_event {
                    Some(InboundEvent::Message(ServiceToAgent::StopAgent(stop))) => {
                        let stop_anchor = Instant::now();
                        let now_ms = self.clock.now_ms();
                        if stop.request_id.iter().all(|byte| *byte == 0)
                            || stop.deadline_ms <= now_ms
                        {
                            return Err(AgentRuntimeError::InvalidStopRequest);
                        }
                        let stop_deadline = stop_anchor
                            .checked_add(Duration::from_millis(
                                stop.deadline_ms.saturating_sub(now_ms),
                            ))
                            .ok_or(AgentRuntimeError::InvalidStopRequest)?;
                        self.terminal_cleanup(ConsentAbortReason::RuntimeStopping)
                            .await?;
                        if Instant::now() >= stop_deadline {
                            return Err(AgentRuntimeError::OutboundUnavailable);
                        }
                        let desktop = self.current_desktop_state()?;
                        let context = self.next_event_context(&identity, desktop)?;
                        let stopping = AgentToService::AgentStopping(AgentStopping {
                                context,
                                reason: StoppingReason::ServiceRequest,
                            });
                        outbound.arm_terminal_deadline(stop_deadline)?;
                        let sender = outbound.sender()?;
                        let mut writer_terminal = outbound.terminal_subscription();
                        let permit = tokio::select! {
                            biased;
                            _ = tokio::time::sleep_until(stop_deadline) => {
                                return Err(AgentRuntimeError::OutboundUnavailable);
                            }
                            _ = wait_for_writer_terminal(&mut writer_terminal) => {
                                return Err(AgentRuntimeError::OutboundUnavailable);
                            }
                            inbound_event = inbound.recv() => {
                                return match inbound_event {
                                    Some(InboundEvent::Disconnected) | None => Ok(AgentExit::ServiceDisconnected),
                                    Some(InboundEvent::Failed(error)) => Err(error.into()),
                                    Some(InboundEvent::Message(_)) => Err(AgentRuntimeError::UnsupportedMessage),
                                };
                            }
                            permit = sender.reserve_owned() => {
                                permit.map_err(|_| AgentRuntimeError::OutboundUnavailable)?
                            }
                        };
                        let (completion_sender, completion) = oneshot::channel();
                        permit.send(OutboundFrame {
                            message: stopping,
                            deadline: Some(stop_deadline),
                            completion: Some(completion_sender),
                        });
                        let flush_result = tokio::select! {
                            biased;
                            result = completion => match result {
                                Ok(OutboundWriteResult::Flushed { completed_at })
                                    if completed_at < stop_deadline =>
                                {
                                    Ok(AgentExit::StoppedByService)
                                }
                                Ok(
                                    OutboundWriteResult::Flushed { .. }
                                    | OutboundWriteResult::DeadlineExceeded
                                    | OutboundWriteResult::Failed,
                                )
                                | Err(_) => Err(AgentRuntimeError::OutboundUnavailable),
                            },
                            _ = wait_for_writer_terminal(&mut writer_terminal) => {
                                Err(AgentRuntimeError::OutboundUnavailable)
                            },
                        };
                        return flush_result;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::Execute(execute))) => {
                        self.handle_execute(&outbound, &identity, &execute).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::ConsentRequest(request))) => {
                        self.handle_managed_consent(&outbound, &identity, request)
                            .await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::CancelConsent(cancel))) => {
                        self.handle_managed_cancel(&outbound, cancel).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::InputEvent(envelope))) => {
                        self.handle_input(&outbound, &identity, envelope).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::RenderAccessUnit(unit))) => {
                        let accepted = self
                            .authority
                            .as_mut()
                            .is_some_and(|authority| authority.executor.render_access_unit(unit));
                        if !accepted {
                            return Err(AgentRuntimeError::InvalidRenderAccessUnit);
                        }
                    }
                    Some(InboundEvent::Message(ServiceToAgent::AgentChallenge(_))) => {
                        return Err(AgentRuntimeError::UnsupportedMessage);
                    }
                    Some(InboundEvent::Failed(error)) => return Err(error.into()),
                    Some(InboundEvent::Disconnected) => {
                        self.terminal_cleanup(ConsentAbortReason::ServiceDisconnected)
                            .await?;
                        return Ok(AgentExit::ServiceDisconnected);
                    }
                    None => {
                        self.terminal_cleanup(ConsentAbortReason::ServiceDisconnected)
                            .await?;
                        return Ok(AgentExit::ServiceDisconnected);
                    }
                },
                RegisteredLoopEvent::Consent(Some(completion)) => {
                    self.handle_consent_completion(
                        &outbound,
                        &identity,
                        completion,
                        &due_sessions,
                    )
                        .await?;
                }
                RegisteredLoopEvent::Consent(None) => {
                    return Err(AgentRuntimeError::ConsentStateUnavailable);
                }
                RegisteredLoopEvent::ConsentDeadline => {
                    self.handle_consent_deadline(&outbound).await?;
                }
                RegisteredLoopEvent::DesktopChanged(Some(Ok(()))) => {
                    let desktop = self.current_desktop_state()?;
                    self.reconcile_desktop_authority(&outbound, desktop)?;
                    if self.last_desktop_state != Some(desktop) {
                        self.last_desktop_state = Some(desktop);
                        capability_revision = capability_revision
                            .checked_add(1)
                            .ok_or(AgentRuntimeError::EventSequenceExhausted)?;
                        self.send_capabilities(
                            &outbound,
                            &identity,
                            capability_revision,
                            desktop,
                        )?;
                    }
                }
                RegisteredLoopEvent::DesktopChanged(Some(Err(_)) | None) => {
                    return match self
                        .terminal_cleanup(ConsentAbortReason::ServiceDisconnected)
                        .await
                    {
                        Ok(()) => Err(AgentRuntimeError::DesktopStateUnavailable),
                        Err(error) => Err(error),
                    };
                }
                RegisteredLoopEvent::Heartbeat => {
                    let desktop = self.current_desktop_state()?;
                    let context = self.next_event_context(&identity, desktop)?;
                    outbound.enqueue(AgentToService::AgentHeartbeat(AgentHeartbeat { context }))?;
                    let render_metrics = self
                        .authority
                        .as_ref()
                        .map(|authority| authority.executor.render_metrics())
                        .unwrap_or_default();
                    for metrics in render_metrics {
                        let message = RenderBoundaryMetrics {
                            context: self.next_event_context(&identity, desktop)?,
                            resource_id: metrics.resource_id,
                            session_id: metrics.session_id.0,
                            decoder_backend: metrics.decoder_backend,
                            enqueued_units: metrics.enqueued_units,
                            queue_replacements: metrics.queue_replacements,
                            decoded_frames: metrics.decoded_frames,
                            presented_frames: metrics.presented_frames,
                        };
                        if !message.is_valid() {
                            return Err(AgentRuntimeError::InvalidRenderMetrics);
                        }
                        outbound.enqueue(AgentToService::RenderBoundaryMetrics(message))?;
                    }
                }
                RegisteredLoopEvent::Capture => {
                    self.pump_capture_access_units(&outbound, &identity)?;
                }
                RegisteredLoopEvent::WriterTerminal => {
                    return Err(AgentRuntimeError::OutboundUnavailable);
                }
                }
            }
        }
        .await;
        match &result {
            Ok(AgentExit::StoppedByService) => {
                outbound.close_and_join().await?;
            }
            _ => outbound.abort_and_join().await,
        }
        result
    }

    async fn register<W>(
        &mut self,
        writer: &mut W,
        inbound: &mut mpsc::Receiver<InboundEvent>,
    ) -> Result<RegisteredAgentIdentity, AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let register = self.config.session.register(self.signer.key_id());
        write_frame(writer, &AgentToService::AgentRegister(register.clone())).await?;

        let inbound_event = tokio::time::timeout(self.config.handshake_timeout, inbound.recv())
            .await
            .map_err(|_| AgentRuntimeError::HandshakeTimeout)?
            .ok_or(AgentRuntimeError::DisconnectedDuringHandshake)?;
        let challenge = match inbound_event {
            InboundEvent::Message(ServiceToAgent::AgentChallenge(challenge)) => challenge,
            InboundEvent::Message(_) => return Err(AgentRuntimeError::ExpectedChallenge),
            InboundEvent::Failed(error) => return Err(error.into()),
            InboundEvent::Disconnected => {
                return Err(AgentRuntimeError::DisconnectedDuringHandshake);
            }
        };
        let now_ms = self.clock.now_ms();
        validate_challenge(&register, &challenge, now_ms)?;

        let mut proof = AgentRegistered {
            registration_id: challenge.registration_id,
            registration_epoch: challenge.registration_epoch,
            challenge_id: challenge.challenge_id,
            agent_instance_id: register.agent_instance_id,
            accepted_protocol_major: AGENT_IPC_PROTOCOL_MAJOR,
            accepted_protocol_minor: AGENT_IPC_PROTOCOL_MINOR,
            signed_at_ms: now_ms,
            signature: [0; 64],
        };
        let signing_bytes = registration_proof_signing_bytes(&register, &challenge, &proof);
        proof.signature = self.signer.sign(&signing_bytes)?;
        if proof.signature.iter().all(|byte| *byte == 0) {
            return Err(RegistrationSigningError::Failed.into());
        }
        write_frame(writer, &AgentToService::AgentRegistered(proof)).await?;

        Ok(RegisteredAgentIdentity {
            agent_instance_id: register.agent_instance_id,
            process_id: register.process_id,
            process_creation_time: register.process_creation_time,
            logon_sid_hash: register.logon_sid_hash,
            windows_session_id: register.windows_session_id,
            agent_key_id: register.agent_key_id,
            registration_id: challenge.registration_id,
            registration_epoch: challenge.registration_epoch,
            protocol_major: AGENT_IPC_PROTOCOL_MAJOR,
            protocol_minor: AGENT_IPC_PROTOCOL_MINOR,
        })
    }

    fn send_capabilities(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
        revision: u64,
        state: TrustedDesktopState,
    ) -> Result<(), AgentRuntimeError> {
        let desktop_epoch = state.desktop_epoch;
        let mut capabilities = if state.desktop_kind == DesktopKind::Default {
            self.generic_executor_capabilities()
        } else {
            AgentCapabilities::empty()
        };
        if self.authority.is_some()
            && state.desktop_kind == DesktopKind::Default
            && self
                .input
                .as_ref()
                .is_some_and(|input| input.is_available())
            && capabilities
                .as_set()
                .iter()
                .all(|capability| *capability != mrd_agent_ipc::AgentCapability::Input)
        {
            let mut advertised = capabilities.as_set().clone();
            advertised.insert(mrd_agent_ipc::AgentCapability::Input);
            capabilities = AgentCapabilities::from_implemented(advertised);
        }
        if self.authority.as_ref().is_some_and(|authority| {
            authority.manager.is_available() && state.desktop_kind == DesktopKind::Default
        }) {
            let mut advertised = capabilities.as_set().clone();
            advertised.insert(mrd_agent_ipc::AgentCapability::Consent);
            capabilities = AgentCapabilities::from_implemented(advertised);
        }
        writer.enqueue(AgentToService::AgentCapabilitySnapshot(
            AgentCapabilitySnapshot {
                agent_instance_id: identity.agent_instance_id,
                registration_id: identity.registration_id,
                windows_session_id: identity.windows_session_id,
                revision,
                desktop_epoch,
                observed_at_ms: self.clock.now_ms(),
                capabilities: capabilities.as_set().clone(),
            },
        ))?;
        Ok(())
    }

    async fn handle_execute(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
        execute: &mrd_agent_ipc::ExecuteCommand,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let (authorized, start_input_blocked) = if self.authority.is_some() {
            let desktop = self.current_desktop_state()?;
            self.reconcile_desktop_authority(writer, desktop)?;
            self.prune_cleanup_receipts(Instant::now());
            let authority = self
                .authority
                .as_ref()
                .ok_or(AgentRuntimeError::ConsentStateUnavailable)?;
            let start_input_blocked = authority.manager.has_active_prompt()
                && matches!(
                    &execute.command,
                    mrd_agent_ipc::AgentCommand::StartInput { .. }
                );
            let (binding, retired) = if execute.command.is_cleanup() {
                self.cleanup_receipts
                    .get(execute.command.resource_id())
                    .filter(|receipt| {
                        receipt.capability == execute.command.required_capability()
                            && receipt.grant_window
                                == (
                                    execute.grant.claims.issued_at_ms,
                                    execute.grant.claims.not_before_ms,
                                    execute.grant.claims.expires_at_ms,
                                )
                            && binding_matches_registration(&receipt.binding, identity)
                            && (receipt.retired
                                || binding_matches_runtime(&receipt.binding, identity, desktop))
                    })
                    .map_or((None, false), |receipt| {
                        (Some(receipt.binding.clone()), receipt.retired)
                    })
            } else {
                (
                    authority
                        .manager
                        .resolve_binding(&execute.grant.claims.session_id, now_ms)
                        .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?,
                    false,
                )
            };
            let authorized = if let Some(binding) = binding {
                if retired || binding_matches_runtime(&binding, identity, desktop) {
                    let context = ExecutionContext {
                        registration_id: binding.registration_id,
                        registration_epoch: binding.registration_epoch,
                        session_id: binding.session_id.clone(),
                        peer: binding.peer.clone(),
                        policy_revision: binding.policy_revision,
                        windows_session_id: binding.windows_session_id,
                        desktop_epoch: binding.desktop_epoch,
                        desktop_kind: binding.desktop_kind,
                        now_ms,
                        expected_issuer_key_id: binding.expected_issuer_key_id,
                        authorization_scopes: binding.approved_scopes.clone(),
                        authorization_expires_at_ms: binding.authorization_expires_at_ms,
                    };
                    match validate_execute_command(execute, &context, authority.verifier.as_ref()) {
                        Ok(command) => Some((command, binding, retired)),
                        Err(reason) => {
                            tracing::warn!(?reason, "agent rejected execute authorization");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };
            (authorized, start_input_blocked)
        } else {
            (None, false)
        };
        let outcome = match authorized {
            Some((authorized, binding, retired)) => {
                self.execute_once(authorized, binding, retired, start_input_blocked)?
            }
            None => CommandOutcome::Rejected,
        };

        writer.enqueue(AgentToService::CommandResult(CommandResult {
            request_token: execute.request_token,
            registration_id: identity.registration_id,
            command_id: execute.command_id,
            outcome,
            completed_at_ms: self.clock.now_ms(),
        }))?;
        Ok(())
    }

    async fn handle_input(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
        envelope: mrd_agent_ipc::InputEventEnvelope,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let outcome = if self.input.is_some() && self.authority.is_some() {
            let desktop = self.current_desktop_state()?;
            self.reconcile_desktop_authority(writer, desktop)?;
            let prompt_active = self
                .authority
                .as_ref()
                .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                .manager
                .has_active_prompt();
            if prompt_active {
                InputAckOutcome::Rejected {
                    reason: mrd_agent_ipc::InputRejection::Grant,
                }
            } else {
                let binding = self
                    .authority
                    .as_ref()
                    .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                    .manager
                    .resolve_binding(&envelope.session_id, now_ms)
                    .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
                if let Some(binding) = binding {
                    if binding_matches_runtime(&binding, identity, desktop) {
                        // Resolve trusted binding state before sampling the time
                        // used at the actual injection boundary.
                        let now_ms = self.clock.now_ms();
                        let context = ExecutionContext {
                            registration_id: binding.registration_id,
                            registration_epoch: binding.registration_epoch,
                            session_id: binding.session_id,
                            peer: binding.peer,
                            policy_revision: binding.policy_revision,
                            windows_session_id: binding.windows_session_id,
                            desktop_epoch: desktop.desktop_epoch,
                            desktop_kind: desktop.desktop_kind,
                            now_ms,
                            expected_issuer_key_id: binding.expected_issuer_key_id,
                            authorization_scopes: binding.approved_scopes,
                            authorization_expires_at_ms: binding.authorization_expires_at_ms,
                        };
                        let input = self
                            .input
                            .as_mut()
                            .ok_or(AgentRuntimeError::InputCleanupFailed)?;
                        let outcome = input.handle(&envelope, &context);
                        if matches!(
                            outcome,
                            InputAckOutcome::Rejected {
                                reason: mrd_agent_ipc::InputRejection::StaleDesktop
                            }
                        ) {
                            input
                                .release_all()
                                .map_err(|_| AgentRuntimeError::InputCleanupFailed)?;
                        }
                        outcome
                    } else {
                        InputAckOutcome::Rejected {
                            reason: mrd_agent_ipc::InputRejection::Grant,
                        }
                    }
                } else {
                    InputAckOutcome::Rejected {
                        reason: mrd_agent_ipc::InputRejection::Grant,
                    }
                }
            }
        } else {
            InputAckOutcome::Rejected {
                reason: mrd_agent_ipc::InputRejection::Unsupported,
            }
        };
        writer.enqueue(AgentToService::InputAck(InputAck {
            request_token: envelope.request_token,
            registration_id: identity.registration_id,
            registration_epoch: identity.registration_epoch,
            session_id: envelope.session_id.clone(),
            resource_id: envelope.resource_id,
            start_grant_id: envelope.start_grant_id,
            sequence: envelope.sequence,
            event_commitment: envelope.commitment().unwrap_or([0; 32]),
            outcome,
        }))?;
        Ok(())
    }

    async fn handle_managed_consent(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
        request: ConsentRequest,
    ) -> Result<(), AgentRuntimeError> {
        if self.authority.is_none() {
            return self.handle_consent_without_manager(writer, request).await;
        }
        let context = self.trusted_consent_context(identity)?;
        let due = self
            .authority
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
            .manager
            .expire_due(Instant::now(), context.now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in due {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        let admission = self
            .authority
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
            .manager
            .admit(request.clone(), context.clone());
        let immediate = match admission {
            Ok(ConsentManagerBeginOutcome::Cached(result)) => vec![result],
            Ok(ConsentManagerBeginOutcome::PromptAdmitted) => {
                let needs_activation = self
                    .authority
                    .as_ref()
                    .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                    .manager
                    .needs_activation();
                if needs_activation {
                    self.release_input()
                        .map_err(|_| AgentRuntimeError::InputCleanupFailed)?;
                    let activation_context = self.trusted_consent_context(identity)?;
                    self.authority
                        .as_mut()
                        .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                        .manager
                        .activate(activation_context)
                        .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?
                } else {
                    Vec::new()
                }
            }
            Err(ConsentRegistryError::InactiveRequest) => vec![coarse_consent_result(
                &request,
                ConsentDecision::Expired,
                self.clock.now_ms(),
            )],
            Err(
                ConsentRegistryError::InvalidLocalContext
                | ConsentRegistryError::PendingCapacityExceeded,
            ) => vec![coarse_consent_result(
                &request,
                ConsentDecision::Dismissed,
                self.clock.now_ms(),
            )],
            Err(_) => return Err(AgentRuntimeError::ConsentStateUnavailable),
        };
        for result in immediate {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        Ok(())
    }

    async fn handle_consent_completion(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
        completion: BackendCompletion,
        already_released_sessions: &[SessionId],
    ) -> Result<(), AgentRuntimeError> {
        let context = self.trusted_consent_context(identity)?;
        let mut completion = {
            let authority = self
                .authority
                .as_mut()
                .ok_or(AgentRuntimeError::ConsentStateUnavailable)?;
            authority
                .manager
                .complete(completion, context.clone())
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?
        };
        if let Some(change) = completion.fresh_authority_change.as_ref() {
            let cleanup_failed = !already_released_sessions.contains(&change.session_id)
                && self
                    .input
                    .as_mut()
                    .is_some_and(|input| input.release_session(&change.session_id).is_err());
            if cleanup_failed {
                let invalidated = self
                    .authority
                    .as_ref()
                    .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                    .manager
                    .invalidate_fresh_authority(change)
                    .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
                return if invalidated {
                    Err(AgentRuntimeError::InputCleanupFailed)
                } else {
                    Err(AgentRuntimeError::ConsentStateUnavailable)
                };
            }
            for receipt in self.cleanup_receipts.values_mut() {
                if receipt.capability == mrd_agent_ipc::AgentCapability::Input
                    && receipt.binding.session_id == change.session_id
                {
                    receipt.retired = true;
                }
            }
        }
        if completion.fresh_authority_change.is_some() {
            let resume_context = self.trusted_consent_context(identity)?;
            let mut resumed = self
                .authority
                .as_mut()
                .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
                .manager
                .resume_after_fresh_authority(resume_context)
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
            completion.results.append(&mut resumed);
        }
        for result in completion.results {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        Ok(())
    }

    async fn handle_consent_deadline(
        &mut self,
        writer: &OutboundWriter,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let results = self
            .authority
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
            .manager
            .expire_due(Instant::now(), now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in results {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        Ok(())
    }

    async fn handle_managed_cancel(
        &mut self,
        writer: &OutboundWriter,
        cancel: CancelConsent,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let Some(authority) = &mut self.authority else {
            // Cleanup is deliberately safe to consume when no manager exists.
            return Ok(());
        };
        let results = authority
            .manager
            .cancel(&cancel, Instant::now(), now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in results {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        Ok(())
    }

    fn trusted_consent_context(
        &self,
        identity: &RegisteredAgentIdentity,
    ) -> Result<TrustedConsentContext, AgentRuntimeError> {
        let expected_issuer_key_id = self
            .authority
            .as_ref()
            .map(|authority| authority.expected_issuer_key_id)
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?;
        let desktop = self.resolve_desktop_state()?;
        Ok(TrustedConsentContext {
            registration_id: identity.registration_id,
            registration_epoch: identity.registration_epoch,
            windows_session_id: identity.windows_session_id,
            desktop_epoch: desktop.desktop_epoch,
            desktop_kind: desktop.desktop_kind,
            expected_issuer_key_id,
            now_ms: self.clock.now_ms(),
        })
    }

    async fn shutdown_consent(
        &mut self,
        reason: ConsentAbortReason,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        if let Some(authority) = &mut self.authority {
            authority
                .manager
                .shutdown(reason, now_ms)
                .await
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        }
        Ok(())
    }

    async fn terminal_cleanup(
        &mut self,
        reason: ConsentAbortReason,
    ) -> Result<(), AgentRuntimeError> {
        if self.cleanup_complete {
            return Ok(());
        }
        let mut first_error = None;
        let invalidations = match self.authority.as_ref() {
            Some(authority) => match authority.manager.drain_authority() {
                Ok(invalidations) => invalidations,
                Err(_) => {
                    first_error = Some(AgentRuntimeError::ConsentStateUnavailable);
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        if let Err(error) = self.release_authority_invalidations(invalidations) {
            first_error.get_or_insert(error);
        }
        if self.release_input().is_err() {
            first_error.get_or_insert(AgentRuntimeError::InputCleanupFailed);
        }
        if let Err(error) = self.shutdown_consent(reason).await {
            first_error.get_or_insert(error);
        }
        match first_error {
            Some(error) => Err(error),
            None => {
                self.cleanup_complete = true;
                Ok(())
            }
        }
    }

    async fn handle_consent_without_manager(
        &mut self,
        writer: &OutboundWriter,
        request: ConsentRequest,
    ) -> Result<(), AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let decision = if now_ms >= request.issued_at_ms && now_ms < request.expires_at_ms {
            ConsentDecision::Dismissed
        } else {
            ConsentDecision::Expired
        };
        writer.enqueue(AgentToService::ConsentResult(coarse_consent_result(
            &request, decision, now_ms,
        )))?;
        Ok(())
    }

    fn execute_once(
        &mut self,
        authorized: AuthorizedCommand,
        mut binding: TrustedSessionBinding,
        retired: bool,
        start_input_blocked: bool,
    ) -> Result<CommandOutcome, AgentRuntimeError> {
        let grant_id = *authorized.grant_id();
        let command_id = *authorized.command_id();
        let fingerprint = SemanticFingerprint::from_authorized(&authorized);
        match self.replay.reserve(grant_id, command_id, fingerprint)? {
            ReplayReservation::First => {
                let resource_id = *authorized.command().resource_id();
                let is_cleanup = authorized.command().is_cleanup();
                let capability = authorized.command().required_capability();
                let retained_until = binding.authorization_deadline + CLEANUP_RECEIPT_RETENTION;
                let grant_window = (
                    authorized.grant().claims().issued_at_ms,
                    authorized.grant().claims().not_before_ms,
                    authorized.grant().claims().expires_at_ms,
                );
                // Stop is bounded by the scopes and lifetime of this exact
                // successful Start, rather than a later consent for the session.
                binding.approved_scopes = authorized.grant().claims().scopes.clone();
                binding.authorization_expires_at_ms = authorized.grant().claims().expires_at_ms;
                let capabilities = if is_cleanup && retired {
                    AgentCapabilities::empty()
                } else {
                    self.generic_executor_capabilities()
                };
                let outcome = if is_cleanup && retired {
                    CommandOutcome::AlreadyStopped
                } else if (!is_cleanup
                    && (self.cleanup_receipts.len() >= MAX_CLEANUP_RECEIPTS
                        || self.started_resource_ids.len() >= REPLAY_LEDGER_CAPACITY
                        || self.started_resource_ids.contains(&resource_id)))
                    || (start_input_blocked
                        && matches!(
                            authorized.command(),
                            mrd_agent_ipc::AgentCommand::StartInput { .. }
                        ))
                {
                    CommandOutcome::Rejected
                } else if let Some(input) = &mut self.input {
                    match authorized.command().clone() {
                        mrd_agent_ipc::AgentCommand::StartInput { .. } => input
                            .start(authorized)
                            .map(|()| CommandOutcome::Completed)
                            .unwrap_or(CommandOutcome::Rejected),
                        mrd_agent_ipc::AgentCommand::StopInput { resource_id } => {
                            command_outcome(input.stop(&resource_id))
                        }
                        _ if capabilities.supports_command(authorized.command()) => self
                            .authority
                            .as_mut()
                            .map_or(CommandOutcome::Rejected, |authority| {
                                authority.executor.execute(authorized)
                            }),
                        _ => CommandOutcome::Rejected,
                    }
                } else if matches!(
                    authorized.command(),
                    mrd_agent_ipc::AgentCommand::StartInput { .. }
                        | mrd_agent_ipc::AgentCommand::StopInput { .. }
                ) {
                    CommandOutcome::Rejected
                } else if capabilities.supports_command(authorized.command()) {
                    self.authority
                        .as_mut()
                        .map_or(CommandOutcome::Rejected, |authority| {
                            authority.executor.execute(authorized)
                        })
                } else {
                    CommandOutcome::Rejected
                };
                if !is_cleanup && outcome == CommandOutcome::Completed {
                    self.started_resource_ids.insert(resource_id);
                    self.cleanup_receipts.insert(
                        resource_id,
                        CleanupReceipt {
                            binding,
                            capability,
                            grant_window,
                            retained_until,
                            retired: false,
                        },
                    );
                } else if is_cleanup
                    && matches!(
                        outcome,
                        CommandOutcome::Completed | CommandOutcome::AlreadyStopped
                    )
                {
                    if let Some(receipt) = self.cleanup_receipts.get_mut(&resource_id) {
                        receipt.retired = true;
                    }
                }
                self.replay.complete(command_id, outcome);
                Ok(outcome)
            }
            ReplayReservation::Cached(outcome) => Ok(outcome),
        }
    }

    fn prune_cleanup_receipts(&mut self, now: Instant) {
        // Unconfirmed resources are never evicted to make room for new work.
        // The registration-lifetime used-id set also survives receipt expiry.
        self.cleanup_receipts
            .retain(|_, receipt| !receipt.retired || now < receipt.retained_until);
    }

    fn release_input(&mut self) -> Result<(), mrd_input::InputError> {
        self.input
            .as_mut()
            .map_or(Ok(()), |input| input.release_all())?;
        for receipt in self.cleanup_receipts.values_mut() {
            if receipt.capability == mrd_agent_ipc::AgentCapability::Input {
                receipt.retired = true;
            }
        }
        Ok(())
    }

    fn release_authority_invalidations(
        &mut self,
        invalidations: Vec<AuthorityInvalidation>,
    ) -> Result<(), AgentRuntimeError> {
        let mut input_failed = false;
        let mut media_failed = false;
        for invalidation in invalidations {
            let session_input_failed = self
                .input
                .as_mut()
                .is_some_and(|input| input.release_session(&invalidation.session_id).is_err());
            let session_media_failed = self.authority.as_mut().is_some_and(|authority| {
                !authority.executor.revoke_session(&invalidation.session_id)
            });
            input_failed |= session_input_failed;
            media_failed |= session_media_failed;
            if !session_input_failed && !session_media_failed {
                for receipt in self.cleanup_receipts.values_mut() {
                    if receipt.binding.session_id == invalidation.session_id
                        && receipt.binding.authority_generation == invalidation.authority_generation
                        && receipt.binding.consent_request_id == invalidation.consent_request_id
                    {
                        receipt.retired = true;
                    }
                }
            }
        }
        if input_failed {
            Err(AgentRuntimeError::InputCleanupFailed)
        } else if media_failed {
            Err(AgentRuntimeError::MediaCleanupFailed)
        } else {
            Ok(())
        }
    }

    fn reconcile_desktop_authority(
        &mut self,
        writer: &OutboundWriter,
        desktop: TrustedDesktopState,
    ) -> Result<(), AgentRuntimeError> {
        let desktop_changed = self.last_desktop_state.is_some_and(|last| last != desktop);
        let (had_active_prompt, prompt_results, invalidations) = match self.authority.as_mut() {
            Some(authority) => {
                let had_active_prompt = authority.manager.has_active_prompt();
                let prompt_results = authority
                    .manager
                    .invalidate_desktop_prompts(
                        desktop.desktop_epoch,
                        desktop.desktop_kind,
                        self.clock.now_ms(),
                    )
                    .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
                let invalidations = authority
                    .manager
                    .take_desktop_mismatch(desktop.desktop_epoch, desktop.desktop_kind)
                    .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
                (had_active_prompt, prompt_results, invalidations)
            }
            None => (false, Vec::new(), Vec::new()),
        };
        let prompt_invalidated = !prompt_results.is_empty();
        self.release_authority_invalidations(invalidations)?;
        if (desktop_changed && had_active_prompt) || prompt_invalidated {
            self.release_input()
                .map_err(|_| AgentRuntimeError::InputCleanupFailed)?;
        }
        for result in prompt_results {
            writer.enqueue(AgentToService::ConsentResult(result))?;
        }
        Ok(())
    }

    fn reconcile_due_authority(&mut self) -> Result<Vec<SessionId>, AgentRuntimeError> {
        let now_ms = self.clock.now_ms();
        let invalidations = match self.authority.as_ref() {
            Some(authority) => authority
                .manager
                .take_due_authority(Instant::now(), now_ms)
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?,
            None => Vec::new(),
        };
        let sessions = invalidations
            .iter()
            .map(|invalidation| invalidation.session_id.clone())
            .collect();
        self.release_authority_invalidations(invalidations)?;
        Ok(sessions)
    }

    fn generic_executor_capabilities(&self) -> AgentCapabilities {
        let Some(authority) = self.authority.as_ref() else {
            return AgentCapabilities::empty();
        };
        AgentCapabilities::from_implemented(
            authority
                .executor
                .capabilities()
                .as_set()
                .iter()
                .copied()
                .filter(|capability| {
                    !matches!(
                        capability,
                        mrd_agent_ipc::AgentCapability::Input
                            | mrd_agent_ipc::AgentCapability::Consent
                    )
                }),
        )
    }

    fn next_event_context(
        &mut self,
        identity: &RegisteredAgentIdentity,
        desktop: TrustedDesktopState,
    ) -> Result<AgentEventContext, AgentRuntimeError> {
        self.event_sequence = self
            .event_sequence
            .checked_add(1)
            .filter(|sequence| *sequence != u64::MAX)
            .ok_or(AgentRuntimeError::EventSequenceExhausted)?;
        Ok(AgentEventContext {
            registration_id: identity.registration_id,
            registration_epoch: identity.registration_epoch,
            windows_session_id: identity.windows_session_id,
            desktop_epoch: desktop.desktop_epoch,
            sequence: self.event_sequence,
            observed_at_ms: self.clock.now_ms(),
        })
    }

    fn pump_capture_access_units(
        &mut self,
        writer: &OutboundWriter,
        identity: &RegisteredAgentIdentity,
    ) -> Result<(), AgentRuntimeError> {
        let desktop = self.current_desktop_state()?;
        let units = match self.authority.as_mut() {
            Some(authority) => authority
                .executor
                .capture_access_units()
                .map_err(|_| AgentRuntimeError::MediaCaptureFailed)?,
            None => return Ok(()),
        };
        for unit in units {
            let binding = self
                .authority
                .as_ref()
                .expect("capture authority")
                .manager
                .resolve_binding(unit.session_id(), self.clock.now_ms())
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
            if !binding.as_ref().is_some_and(|binding| {
                binding_matches_runtime(binding, identity, desktop)
                    && binding
                        .approved_scopes
                        .contains(&mrd_session::PermissionScope::ScreenView)
            }) {
                return Err(AgentRuntimeError::MediaCaptureFailed);
            }
            let context = self.next_event_context(identity, desktop)?;
            let unit = unit
                .into_ipc(context, mrd_agent_ipc::MediaCodec::H264)
                .ok_or(AgentRuntimeError::MediaCaptureFailed)?;
            writer.enqueue(AgentToService::MediaAccessUnit(unit))?;
        }
        Ok(())
    }

    fn current_desktop_state(&self) -> Result<TrustedDesktopState, AgentRuntimeError> {
        self.resolve_desktop_state()
    }

    fn resolve_desktop_state(&self) -> Result<TrustedDesktopState, AgentRuntimeError> {
        match self.authority.as_ref() {
            Some(authority) => validated_desktop_state(authority.desktop_state.as_ref()),
            None => Ok(TrustedDesktopState {
                desktop_epoch: self.config.session.desktop_epoch,
                desktop_kind: DesktopKind::Default,
            }),
        }
    }
}

fn binding_matches_runtime(
    binding: &TrustedSessionBinding,
    identity: &RegisteredAgentIdentity,
    desktop: TrustedDesktopState,
) -> bool {
    binding_matches_registration(binding, identity)
        && binding.desktop_epoch == desktop.desktop_epoch
        && binding.desktop_kind == desktop.desktop_kind
        && binding.desktop_kind == DesktopKind::Default
}

fn coarse_consent_result(
    request: &ConsentRequest,
    decision: ConsentDecision,
    now_ms: u64,
) -> ConsentResult {
    ConsentResult {
        request_token: request.request_token,
        request_id: request.request_id,
        session_id: request.session_id.clone(),
        peer: request.peer.clone(),
        policy_revision: request.policy_revision,
        windows_session_id: request.windows_session_id,
        decision,
        approved_scopes: Default::default(),
        decided_at_ms: now_ms
            .max(request.issued_at_ms)
            .min(request.expires_at_ms.saturating_sub(1)),
    }
}

fn binding_matches_registration(
    binding: &TrustedSessionBinding,
    identity: &RegisteredAgentIdentity,
) -> bool {
    binding.registration_id == identity.registration_id
        && binding.registration_epoch == identity.registration_epoch
        && binding.windows_session_id == identity.windows_session_id
}

fn command_outcome(outcome: InputAckOutcome) -> CommandOutcome {
    match outcome {
        InputAckOutcome::Applied => CommandOutcome::Completed,
        InputAckOutcome::Rejected { .. } => CommandOutcome::Rejected,
        InputAckOutcome::Failed { .. } => CommandOutcome::Failed,
    }
}

fn validated_desktop_state(
    source: &dyn TrustedDesktopStateSource,
) -> Result<TrustedDesktopState, AgentRuntimeError> {
    source
        .current_state()
        .filter(|state| state.desktop_epoch != 0)
        .ok_or(AgentRuntimeError::DesktopStateUnavailable)
}

fn validate_challenge(
    register: &AgentRegister,
    challenge: &AgentChallenge,
    now_ms: u64,
) -> Result<(), AgentRuntimeError> {
    if challenge.registration_id.iter().all(|byte| *byte == 0)
        || challenge.registration_epoch == 0
        || challenge.challenge_id.iter().all(|byte| *byte == 0)
        || challenge.challenge_nonce.iter().all(|byte| *byte == 0)
        || challenge.issued_at_ms == 0
        || challenge.expires_at_ms <= challenge.issued_at_ms
        || challenge
            .expires_at_ms
            .saturating_sub(challenge.issued_at_ms)
            > AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS
        || now_ms < challenge.issued_at_ms
        || now_ms >= challenge.expires_at_ms
        || challenge.expected_agent_instance_id != register.agent_instance_id
        || challenge.expected_process_id != register.process_id
        || challenge.expected_process_creation_time != register.process_creation_time
        || challenge.expected_logon_sid_hash != register.logon_sid_hash
        || challenge.expected_windows_session_id != register.windows_session_id
    {
        return Err(AgentRuntimeError::InvalidChallenge);
    }
    Ok(())
}

enum InboundEvent {
    Message(ServiceToAgent),
    Failed(FrameError),
    Disconnected,
}

enum RegisteredLoopEvent {
    Inbound(Option<InboundEvent>),
    Consent(Option<BackendCompletion>),
    ConsentDeadline,
    DesktopChanged(Option<Result<(), tokio::sync::watch::error::RecvError>>),
    WriterTerminal,
    Heartbeat,
    Capture,
}

async fn read_loop<R>(mut reader: R, sender: mpsc::Sender<InboundEvent>)
where
    R: AsyncRead + Unpin,
{
    loop {
        let event = match read_service_frame(&mut reader, PARTIAL_FRAME_TIMEOUT).await {
            Ok(message) => InboundEvent::Message(message),
            Err(FrameError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                ) =>
            {
                InboundEvent::Disconnected
            }
            Err(error) => InboundEvent::Failed(error),
        };
        let terminal = matches!(event, InboundEvent::Failed(_) | InboundEvent::Disconnected);
        if sender.send(event).await.is_err() || terminal {
            break;
        }
    }
}

async fn read_service_frame<R>(
    reader: &mut R,
    partial_timeout: Duration,
) -> Result<ServiceToAgent, FrameError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; AGENT_IPC_FRAME_HEADER_BYTES];
    // An idle connection is allowed indefinitely. Once a frame starts, every
    // remaining byte must arrive within a bounded interval or the stream closes.
    reader.read_exact(&mut header[..1]).await?;
    tokio::time::timeout(partial_timeout, reader.read_exact(&mut header[1..]))
        .await
        .map_err(|_| partial_frame_timeout_error())??;

    let payload_len =
        u32::from_le_bytes(header[0..4].try_into().expect("fixed frame header")) as usize;
    let protocol_major = u16::from_le_bytes(header[4..6].try_into().expect("fixed frame header"));
    if protocol_major != AGENT_IPC_PROTOCOL_MAJOR {
        return Err(FrameError::UnsupportedMajor {
            received: protocol_major,
            supported: AGENT_IPC_PROTOCOL_MAJOR,
        });
    }
    if payload_len == 0 {
        return Err(FrameError::EmptyPayload);
    }
    if payload_len > AGENT_IPC_MAX_FRAME_BYTES {
        return Err(FrameError::FrameTooLarge {
            declared: payload_len,
            max: AGENT_IPC_MAX_FRAME_BYTES,
        });
    }

    let mut frame = Vec::with_capacity(AGENT_IPC_FRAME_HEADER_BYTES + payload_len);
    frame.extend_from_slice(&header);
    frame.resize(AGENT_IPC_FRAME_HEADER_BYTES + payload_len, 0);
    tokio::time::timeout(
        partial_timeout,
        reader.read_exact(&mut frame[AGENT_IPC_FRAME_HEADER_BYTES..]),
    )
    .await
    .map_err(|_| partial_frame_timeout_error())??;
    decode_frame::<ServiceToAgent>(&frame).map(|decoded| decoded.message)
}

fn partial_frame_timeout_error() -> FrameError {
    FrameError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "partial agent IPC frame timed out",
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticFingerprint {
    command_digest: [u8; 32],
    session_id: SessionId,
    peer: PeerBinding,
    policy_revision: u64,
    windows_session_id: u32,
    desktop_epoch: u64,
    desktop_kind: DesktopKind,
}

impl SemanticFingerprint {
    fn from_authorized(authorized: &AuthorizedCommand) -> Self {
        let claims = authorized.grant().claims();
        Self {
            command_digest: claims.command_digest,
            session_id: claims.session_id.clone(),
            peer: claims.peer.clone(),
            policy_revision: claims.policy_revision,
            windows_session_id: claims.windows_session_id,
            desktop_epoch: claims.desktop_epoch,
            desktop_kind: claims.desktop_kind,
        }
    }
}

#[derive(Debug, Clone)]
struct GrantReplayRecord {
    command_id: [u8; 16],
    fingerprint: SemanticFingerprint,
}

#[derive(Debug, Clone)]
struct CommandReplayRecord {
    fingerprint: SemanticFingerprint,
    outcome: Option<CommandOutcome>,
}

enum ReplayReservation {
    First,
    Cached(CommandOutcome),
}

struct ReplayLedger {
    capacity: usize,
    grants: HashMap<[u8; 32], GrantReplayRecord>,
    commands: HashMap<[u8; 16], CommandReplayRecord>,
}

impl ReplayLedger {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            grants: HashMap::new(),
            commands: HashMap::new(),
        }
    }

    fn reserve(
        &mut self,
        grant_id: [u8; 32],
        command_id: [u8; 16],
        fingerprint: SemanticFingerprint,
    ) -> Result<ReplayReservation, AgentRuntimeError> {
        if let Some(grant) = self.grants.get(&grant_id) {
            if grant.command_id != command_id || grant.fingerprint != fingerprint {
                return Err(AgentRuntimeError::ReplayConflict);
            }
            return self.cached_command(command_id, &fingerprint);
        }

        if let Some(command) = self.commands.get(&command_id) {
            if command.fingerprint != fingerprint {
                return Err(AgentRuntimeError::ReplayConflict);
            }
            if self.grants.len() >= self.capacity {
                return Err(AgentRuntimeError::ReplayCapacityExceeded);
            }
            let outcome = command.outcome.ok_or(AgentRuntimeError::ReplayConflict)?;
            self.grants.insert(
                grant_id,
                GrantReplayRecord {
                    command_id,
                    fingerprint,
                },
            );
            return Ok(ReplayReservation::Cached(outcome));
        }

        if self.grants.len() >= self.capacity || self.commands.len() >= self.capacity {
            return Err(AgentRuntimeError::ReplayCapacityExceeded);
        }
        self.grants.insert(
            grant_id,
            GrantReplayRecord {
                command_id,
                fingerprint: fingerprint.clone(),
            },
        );
        self.commands.insert(
            command_id,
            CommandReplayRecord {
                fingerprint,
                outcome: None,
            },
        );
        Ok(ReplayReservation::First)
    }

    fn cached_command(
        &self,
        command_id: [u8; 16],
        fingerprint: &SemanticFingerprint,
    ) -> Result<ReplayReservation, AgentRuntimeError> {
        let command = self
            .commands
            .get(&command_id)
            .ok_or(AgentRuntimeError::ReplayConflict)?;
        if &command.fingerprint != fingerprint {
            return Err(AgentRuntimeError::ReplayConflict);
        }
        command
            .outcome
            .map(ReplayReservation::Cached)
            .ok_or(AgentRuntimeError::ReplayConflict)
    }

    fn complete(&mut self, command_id: [u8; 16], outcome: CommandOutcome) {
        if let Some(command) = self.commands.get_mut(&command_id) {
            command.outcome = Some(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::DeviceId;
    use tokio::io::AsyncWriteExt;

    struct UnusedSigner;
    impl RegistrationSigner for UnusedSigner {
        fn key_id(&self) -> [u8; 32] {
            [1; 32]
        }
        fn sign(&self, _: &[u8]) -> Result<[u8; 64], RegistrationSigningError> {
            Err(RegistrationSigningError::Unavailable)
        }
    }

    fn cleanup_test_runtime() -> AgentRuntime {
        AgentRuntime::new(
            AgentRuntimeConfig {
                session: SessionDescriptor::new([1; 16], 1, 1, [1; 32], 7, [1; 32], 11).unwrap(),
                heartbeat_interval: Duration::from_secs(30),
                handshake_timeout: Duration::from_secs(1),
            },
            Arc::new(SystemClock),
            Arc::new(UnusedSigner),
        )
        .unwrap()
    }

    fn cleanup_test_binding() -> TrustedSessionBinding {
        TrustedSessionBinding {
            authority_generation: 1,
            consent_request_id: [1; 16],
            registration_id: [1; 16],
            registration_epoch: 1,
            session_id: SessionId("cleanup-bound-session".into()),
            peer: PeerBinding {
                device_id: DeviceId("cleanup-bound-peer".into()),
                key_id: [1; 32],
            },
            approved_scopes: [mrd_session::PermissionScope::InputKeyboard]
                .into_iter()
                .collect(),
            policy_revision: 1,
            windows_session_id: 7,
            desktop_epoch: 11,
            desktop_kind: DesktopKind::Default,
            authorization_expires_at_ms: 2_000,
            authorization_deadline: Instant::now() + Duration::from_secs(1),
            expected_issuer_key_id: [1; 32],
        }
    }

    #[test]
    fn receipt_expiry_keeps_unconfirmed_resources_and_permanent_resource_id_ownership() {
        let mut runtime = cleanup_test_runtime();
        let now = Instant::now();
        for (id, retired) in [([1; 16], true), ([2; 16], false)] {
            runtime.started_resource_ids.insert(id);
            runtime.cleanup_receipts.insert(
                id,
                CleanupReceipt {
                    binding: cleanup_test_binding(),
                    capability: mrd_agent_ipc::AgentCapability::Input,
                    grant_window: (1_000, 1_000, 2_000),
                    retained_until: now,
                    retired,
                },
            );
        }
        runtime.prune_cleanup_receipts(now - Duration::from_nanos(1));
        assert_eq!(runtime.cleanup_receipts.len(), 2);
        runtime.prune_cleanup_receipts(now);
        assert!(!runtime.cleanup_receipts.contains_key(&[1; 16]));
        assert!(runtime.cleanup_receipts.contains_key(&[2; 16]));
        assert!(runtime.started_resource_ids.contains(&[1; 16]));
    }

    #[test]
    fn cleanup_receipt_capacity_and_expired_used_ids_reject_start_before_native_execution() {
        struct AcceptVerifier;
        impl ExecuteGrantVerifier for AcceptVerifier {
            fn verify(&self, _: &[u8; 32], _: &[u8], _: &[u8; 64]) -> bool {
                true
            }
        }
        struct CountingInput(Arc<std::sync::atomic::AtomicUsize>);
        impl InputBackend for CountingInput {
            fn is_available(&self) -> bool {
                true
            }
            fn start(&mut self, _: AuthorizedCommand) -> Result<(), mrd_agent_ipc::InputRejection> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            fn handle(
                &mut self,
                _: &mrd_agent_ipc::InputEventEnvelope,
                _: &ExecutionContext,
            ) -> InputAckOutcome {
                panic!("capacity regression must never inject input")
            }
            fn stop(&mut self, _: &[u8; 16]) -> InputAckOutcome {
                panic!("capacity regression must never execute cleanup")
            }
            fn release_session(&mut self, _: &SessionId) -> Result<(), mrd_input::InputError> {
                Ok(())
            }
            fn release_all(&mut self) -> Result<(), mrd_input::InputError> {
                Ok(())
            }
        }
        fn start(
            binding: &TrustedSessionBinding,
            seed: u8,
            resource_id: [u8; 16],
        ) -> AuthorizedCommand {
            let command = mrd_agent_ipc::AgentCommand::StartInput {
                resource_id,
                input_scopes: binding.approved_scopes.clone(),
            };
            let mut execute = mrd_agent_ipc::ExecuteCommand {
                request_token: 1,
                command_id: [seed; 16],
                command,
                grant: mrd_agent_ipc::ExecuteGrant {
                    claims: mrd_agent_ipc::ExecuteGrantClaims {
                        grant_id: [seed; 32],
                        registration_id: binding.registration_id,
                        registration_epoch: binding.registration_epoch,
                        session_id: binding.session_id.clone(),
                        peer: binding.peer.clone(),
                        scopes: binding.approved_scopes.clone(),
                        policy_revision: binding.policy_revision,
                        windows_session_id: binding.windows_session_id,
                        desktop_epoch: binding.desktop_epoch,
                        desktop_kind: binding.desktop_kind,
                        issued_at_ms: 1_000,
                        not_before_ms: 1_000,
                        expires_at_ms: 2_000,
                        command_digest: [0; 32],
                        audience: mrd_agent_ipc::GrantAudience::SessionAgent,
                    },
                    issuer_key_id: binding.expected_issuer_key_id,
                    signature: [1; 64],
                },
            };
            execute.grant.claims.command_digest = execute.command_digest();
            validate_execute_command(
                &execute,
                &ExecutionContext {
                    registration_id: binding.registration_id,
                    registration_epoch: binding.registration_epoch,
                    session_id: binding.session_id.clone(),
                    peer: binding.peer.clone(),
                    policy_revision: binding.policy_revision,
                    windows_session_id: binding.windows_session_id,
                    desktop_epoch: binding.desktop_epoch,
                    desktop_kind: binding.desktop_kind,
                    now_ms: 1_500,
                    expected_issuer_key_id: binding.expected_issuer_key_id,
                    authorization_scopes: binding.approved_scopes.clone(),
                    authorization_expires_at_ms: binding.authorization_expires_at_ms,
                },
                &AcceptVerifier,
            )
            .unwrap()
        }
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runtime = cleanup_test_runtime();
        runtime.input = Some(Box::new(CountingInput(starts.clone())));
        let binding = cleanup_test_binding();
        let now = Instant::now();
        for seed in 0..MAX_CLEANUP_RECEIPTS {
            runtime.cleanup_receipts.insert(
                (seed as u128).to_le_bytes(),
                CleanupReceipt {
                    binding: binding.clone(),
                    capability: mrd_agent_ipc::AgentCapability::Input,
                    grant_window: (1_000, 1_000, 2_000),
                    retained_until: now,
                    retired: true,
                },
            );
        }
        assert_eq!(
            runtime
                .execute_once(start(&binding, 1, [255; 16]), binding.clone(), false, false)
                .unwrap(),
            CommandOutcome::Rejected
        );
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 0);
        runtime.prune_cleanup_receipts(now);
        assert!(runtime.cleanup_receipts.is_empty());
        runtime.started_resource_ids.insert([255; 16]);
        assert_eq!(
            runtime
                .execute_once(start(&binding, 2, [255; 16]), binding.clone(), false, false)
                .unwrap(),
            CommandOutcome::Rejected
        );
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 0);
        // New resource ownership is still allowed once retained receipts expire.
        assert_eq!(
            runtime
                .execute_once(start(&binding, 3, [254; 16]), binding, false, false)
                .unwrap(),
            CommandOutcome::Completed
        );
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    fn fingerprint(seed: u8) -> SemanticFingerprint {
        SemanticFingerprint {
            command_digest: [seed; 32],
            session_id: SessionId(format!("session-{seed}")),
            peer: PeerBinding {
                device_id: DeviceId(format!("peer-{seed}")),
                key_id: [seed; 32],
            },
            policy_revision: u64::from(seed),
            windows_session_id: u32::from(seed),
            desktop_epoch: u64::from(seed),
            desktop_kind: DesktopKind::Default,
        }
    }

    #[test]
    fn replay_ledger_caches_exact_replays_and_rejects_conflicts() {
        let mut ledger = ReplayLedger::new(4);
        let semantic = fingerprint(1);
        assert!(matches!(
            ledger.reserve([1; 32], [2; 16], semantic.clone()),
            Ok(ReplayReservation::First)
        ));
        ledger.complete([2; 16], CommandOutcome::Completed);
        assert!(matches!(
            ledger.reserve([1; 32], [2; 16], semantic.clone()),
            Ok(ReplayReservation::Cached(CommandOutcome::Completed))
        ));
        assert!(matches!(
            ledger.reserve([3; 32], [2; 16], semantic),
            Ok(ReplayReservation::Cached(CommandOutcome::Completed))
        ));
        assert!(matches!(
            ledger.reserve([1; 32], [9; 16], fingerprint(9)),
            Err(AgentRuntimeError::ReplayConflict)
        ));
    }

    #[test]
    fn replay_ledger_fails_closed_at_capacity() {
        let mut ledger = ReplayLedger::new(1);
        assert!(matches!(
            ledger.reserve([1; 32], [1; 16], fingerprint(1)),
            Ok(ReplayReservation::First)
        ));
        ledger.complete([1; 16], CommandOutcome::Rejected);
        assert!(matches!(
            ledger.reserve([2; 32], [2; 16], fingerprint(2)),
            Err(AgentRuntimeError::ReplayCapacityExceeded)
        ));
    }

    #[tokio::test]
    async fn partial_frame_times_out_without_timing_out_an_idle_stream() {
        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer.write_all(&[1]).await.unwrap();
        let error = read_service_frame(&mut reader, Duration::from_millis(10))
            .await
            .expect_err("partial header must time out");
        assert!(matches!(
            error,
            FrameError::Io(ref io_error) if io_error.kind() == std::io::ErrorKind::TimedOut
        ));

        let (_idle_writer, mut idle_reader) = tokio::io::duplex(16);
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            read_service_frame(&mut idle_reader, Duration::from_millis(5)),
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn outbound_writer_queue_full_fails_without_waiting_for_the_blocked_stream() {
        let (writer, _reader) = tokio::io::duplex(1);
        let mut outbound = OutboundWriter::spawn(writer);
        let mut failed = false;
        for sequence in 1..=(OUTBOUND_QUEUE_CAPACITY as u64 + 2) {
            let message = AgentToService::AgentHeartbeat(AgentHeartbeat {
                context: AgentEventContext {
                    registration_id: [1; 16],
                    registration_epoch: 1,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence,
                    observed_at_ms: 1,
                },
            });
            if outbound.enqueue(message).is_err() {
                failed = true;
                break;
            }
        }
        assert!(
            failed,
            "bounded output must fail instead of awaiting capacity"
        );
        outbound.abort_and_join().await;
    }

    #[tokio::test]
    async fn outbound_writer_failure_wakes_the_registered_control_loop() {
        let (writer, reader) = tokio::io::duplex(64);
        drop(reader);
        let mut outbound = OutboundWriter::spawn(writer);
        outbound
            .enqueue(AgentToService::AgentHeartbeat(AgentHeartbeat {
                context: AgentEventContext {
                    registration_id: [1; 16],
                    registration_epoch: 1,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence: 1,
                    observed_at_ms: 1,
                },
            }))
            .unwrap();

        assert_eq!(
            tokio::time::timeout(Duration::from_millis(100), outbound.terminal_changed())
                .await
                .expect("writer terminal wake"),
            WriterTerminal::Failed,
        );
        outbound.abort_and_join().await;
    }

    #[test]
    #[cfg(windows)]
    fn endpoint_parser_accepts_only_local_named_pipes_on_windows() {
        assert!(PrivateAgentEndpoint::parse(r"\\.\pipe\mrd-agent-7").is_ok());
        assert!(PrivateAgentEndpoint::parse("tcp://127.0.0.1:1234").is_err());
        assert!(PrivateAgentEndpoint::parse(r"\\server\pipe\mrd-agent-7").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn endpoint_parser_accepts_only_absolute_unix_socket_paths() {
        assert!(PrivateAgentEndpoint::parse("/tmp/mrd-agent-7.sock").is_ok());
        assert!(PrivateAgentEndpoint::parse("mrd-agent-7.sock").is_err());
        assert!(PrivateAgentEndpoint::parse("tcp://127.0.0.1:1234").is_err());
    }
}
