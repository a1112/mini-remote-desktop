//! Fail-closed interactive-agent registration and event loop.

use crate::capabilities::AgentCapabilities;
use crate::consent::{
    BackendCompletion, ConsentAbortReason, ConsentBackend, ConsentManager, ConsentRegistryError,
    TrustedConsentContext,
};
pub use crate::consent::{TrustedSessionBinding, TrustedSessionBindingSource};
use crate::input::InputBackend;
use mrd_agent_ipc::{
    decode_frame, registration_proof_signing_bytes, validate_execute_command, write_frame,
    AgentCapabilitySnapshot, AgentChallenge, AgentEventContext, AgentHeartbeat, AgentRegister,
    AgentRegistered, AgentStopping, AgentToService, AuthorizedCommand, CancelConsent,
    CommandOutcome, CommandResult, ConsentDecision, ConsentRequest, ConsentResult, DesktopKind,
    ExecuteGrantVerifier, ExecutionContext, FrameError, InputAck, InputAckOutcome, PeerBinding,
    RegisteredAgentIdentity, ServiceToAgent, StoppingReason, AGENT_IPC_FRAME_HEADER_BYTES,
    AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR, AGENT_IPC_PROTOCOL_MINOR,
    AGENT_REGISTRATION_CHALLENGE_MAX_LIFETIME_MS,
};
use mrd_proto::SessionId;
#[cfg(unix)]
use std::path::{Component, Path};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::mpsc,
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};

const INBOUND_QUEUE_CAPACITY: usize = 32;
const REPLAY_LEDGER_CAPACITY: usize = 4_096;
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(15);

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
    /// Return current trusted state, or `None` when it cannot be established.
    fn current_state(&self) -> Option<TrustedDesktopState>;
}

/// Product backend boundary that can receive only validated commands.
pub trait AuthorizedCommandExecutor: Send {
    /// Capabilities implemented by this exact backend.
    fn capabilities(&self) -> AgentCapabilities;
    /// Execute one already-authorized command without blocking the event loop.
    fn execute(&mut self, command: AuthorizedCommand) -> CommandOutcome;
}

#[derive(Default)]
struct ShellBackend;

impl AuthorizedCommandExecutor for ShellBackend {
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::empty()
    }

    fn execute(&mut self, _command: AuthorizedCommand) -> CommandOutcome {
        CommandOutcome::Rejected
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
    /// A StopAgent message carried an invalid request identity.
    #[error("StopAgent request id is invalid")]
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

struct ExecutionSecurity {
    bindings: Arc<dyn TrustedSessionBindingSource>,
    verifier: Arc<dyn ExecuteGrantVerifier + Send + Sync>,
    desktop_state: Arc<dyn TrustedDesktopStateSource>,
}

struct ConsentRuntime {
    manager: ConsentManager,
    desktop_state: Arc<dyn TrustedDesktopStateSource>,
    expected_issuer_key_id: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
struct ResolvedDesktopState {
    runtime: TrustedDesktopState,
    consent: Option<TrustedDesktopState>,
}

/// One connected session-agent runtime.
pub struct AgentRuntime {
    config: AgentRuntimeConfig,
    clock: Arc<dyn AgentClock>,
    signer: Arc<dyn RegistrationSigner>,
    security: Option<ExecutionSecurity>,
    consent: Option<ConsentRuntime>,
    executor: Box<dyn AuthorizedCommandExecutor>,
    input: Option<Box<dyn InputBackend>>,
    last_desktop_state: Option<TrustedDesktopState>,
    replay: ReplayLedger,
    event_sequence: u64,
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
            security: None,
            consent: None,
            executor: Box::new(ShellBackend),
            input: None,
            last_desktop_state: None,
            replay: ReplayLedger::new(REPLAY_LEDGER_CAPACITY),
            event_sequence: 0,
        })
    }

    /// Replace the empty security/backend ports with trusted product adapters.
    ///
    /// The binding source must resolve service-owned state independently from
    /// the grant. The executor receives only an [`AuthorizedCommand`].
    pub fn with_execution_security(
        mut self,
        bindings: Arc<dyn TrustedSessionBindingSource>,
        verifier: Arc<dyn ExecuteGrantVerifier + Send + Sync>,
        desktop_state: Arc<dyn TrustedDesktopStateSource>,
        executor: Box<dyn AuthorizedCommandExecutor>,
    ) -> Self {
        self.security = Some(ExecutionSecurity {
            bindings,
            verifier,
            desktop_state,
        });
        self.executor = executor;
        self
    }

    /// Install an asynchronous attended-consent backend and trusted context sources.
    ///
    /// The runtime constructs and exclusively owns the corresponding authority
    /// registry; the backend receives only display-safe prompt data.
    pub fn with_consent_backend(
        mut self,
        backend: Arc<dyn ConsentBackend>,
        desktop_state: Arc<dyn TrustedDesktopStateSource>,
        expected_issuer_key_id: [u8; 32],
    ) -> Result<Self, AgentRuntimeError> {
        if expected_issuer_key_id.iter().all(|byte| *byte == 0) {
            return Err(AgentRuntimeError::InvalidConfiguration);
        }
        self.consent = Some(ConsentRuntime {
            manager: ConsentManager::new(backend),
            desktop_state,
            expected_issuer_key_id,
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
        let (reader, mut writer) = tokio::io::split(stream);
        let (inbound_tx, mut inbound_rx) = mpsc::channel(INBOUND_QUEUE_CAPACITY);
        let reader_task = tokio::spawn(read_loop(reader, inbound_tx));

        let result = self.run_connected(&mut writer, &mut inbound_rx).await;
        let shutdown_reason = match &result {
            Ok(AgentExit::StoppedByService) => ConsentAbortReason::RuntimeStopping,
            Ok(AgentExit::ServiceDisconnected) | Err(_) => ConsentAbortReason::ServiceDisconnected,
        };
        let _ = self.release_input();
        let consent_shutdown = self.shutdown_consent(shutdown_reason).await;
        stop_reader(reader_task).await;
        match (result, consent_shutdown) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(exit), Ok(())) => Ok(exit),
        }
    }

    async fn run_connected<W>(
        &mut self,
        writer: &mut W,
        inbound: &mut mpsc::Receiver<InboundEvent>,
    ) -> Result<AgentExit, AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let identity = self.register(writer, inbound).await?;
        self.send_capabilities(writer, &identity).await?;

        let mut heartbeat = tokio::time::interval_at(
            Instant::now() + self.config.heartbeat_interval,
            self.config.heartbeat_interval,
        );
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            let consent_deadline = self
                .consent
                .as_ref()
                .and_then(|consent| consent.manager.next_deadline());
            let event = {
                let consent_completion = async {
                    match self.consent.as_mut() {
                        Some(consent) => consent.manager.next_completion().await,
                        None => std::future::pending().await,
                    }
                };
                let consent_deadline_wait = async move {
                    match consent_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    inbound_event = inbound.recv() => RegisteredLoopEvent::Inbound(inbound_event),
                    completion = consent_completion => RegisteredLoopEvent::Consent(completion),
                    _ = consent_deadline_wait => RegisteredLoopEvent::ConsentDeadline,
                    _ = heartbeat.tick() => RegisteredLoopEvent::Heartbeat,
                }
            };
            match event {
                RegisteredLoopEvent::Inbound(inbound_event) => match inbound_event {
                    Some(InboundEvent::Message(ServiceToAgent::StopAgent(stop))) => {
                        if stop.request_id.iter().all(|byte| *byte == 0) {
                            return Err(AgentRuntimeError::InvalidStopRequest);
                        }
                        let _ = self.release_input();
                        self.shutdown_consent(ConsentAbortReason::RuntimeStopping)
                            .await?;
                        let desktop = self.current_desktop_state()?;
                        let context = self.next_event_context(&identity, desktop)?;
                        write_frame(
                            writer,
                            &AgentToService::AgentStopping(AgentStopping {
                                context,
                                reason: StoppingReason::ServiceRequest,
                            }),
                        )
                        .await?;
                        return Ok(AgentExit::StoppedByService);
                    }
                    Some(InboundEvent::Message(ServiceToAgent::Execute(execute))) => {
                        self.handle_execute(writer, &identity, &execute).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::ConsentRequest(request))) => {
                        self.handle_managed_consent(writer, &identity, request)
                            .await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::CancelConsent(cancel))) => {
                        self.handle_managed_cancel(writer, cancel).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::InputEvent(envelope))) => {
                        self.handle_input(writer, &identity, envelope).await?;
                    }
                    Some(InboundEvent::Message(ServiceToAgent::AgentChallenge(_))) => {
                        return Err(AgentRuntimeError::UnsupportedMessage);
                    }
                    Some(InboundEvent::Failed(error)) => return Err(error.into()),
                    Some(InboundEvent::Disconnected) => {
                        let _ = self.release_input();
                        self.shutdown_consent(ConsentAbortReason::ServiceDisconnected)
                            .await?;
                        return Ok(AgentExit::ServiceDisconnected);
                    }
                    None => {
                        let _ = self.release_input();
                        self.shutdown_consent(ConsentAbortReason::ServiceDisconnected)
                            .await?;
                        return Ok(AgentExit::ServiceDisconnected);
                    }
                },
                RegisteredLoopEvent::Consent(Some(completion)) => {
                    self.handle_consent_completion(writer, &identity, completion)
                        .await?;
                }
                RegisteredLoopEvent::Consent(None) => {
                    return Err(AgentRuntimeError::ConsentStateUnavailable);
                }
                RegisteredLoopEvent::ConsentDeadline => {
                    self.handle_consent_deadline(writer).await?;
                }
                RegisteredLoopEvent::Heartbeat => {
                    let desktop = self.current_desktop_state()?;
                    self.refresh_input_desktop(desktop);
                    let context = self.next_event_context(&identity, desktop)?;
                    write_frame(
                        writer,
                        &AgentToService::AgentHeartbeat(AgentHeartbeat { context }),
                    )
                    .await?;
                }
            }
        }
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

    async fn send_capabilities<W>(
        &mut self,
        writer: &mut W,
        identity: &RegisteredAgentIdentity,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let resolved_desktop = self.resolve_desktop_state()?;
        let state = resolved_desktop.runtime;
        self.last_desktop_state = Some(state);
        let desktop_epoch = state.desktop_epoch;
        let mut capabilities =
            if self.security.is_some() && state.desktop_kind == DesktopKind::Default {
                self.executor.capabilities()
            } else {
                AgentCapabilities::empty()
            };
        if self.security.is_some()
            && self
                .last_desktop_state
                .is_some_and(|state| state.desktop_kind == DesktopKind::Default)
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
        if self.consent.as_ref().is_some_and(|consent| {
            consent.manager.is_available()
                && resolved_desktop
                    .consent
                    .is_some_and(|desktop| desktop.desktop_kind == DesktopKind::Default)
        }) {
            let mut advertised = capabilities.as_set().clone();
            advertised.insert(mrd_agent_ipc::AgentCapability::Consent);
            capabilities = AgentCapabilities::from_implemented(advertised);
        }
        write_frame(
            writer,
            &AgentToService::AgentCapabilitySnapshot(AgentCapabilitySnapshot {
                agent_instance_id: identity.agent_instance_id,
                registration_id: identity.registration_id,
                windows_session_id: identity.windows_session_id,
                revision: 1,
                desktop_epoch,
                observed_at_ms: self.clock.now_ms(),
                capabilities: capabilities.as_set().clone(),
            }),
        )
        .await?;
        Ok(())
    }

    async fn handle_execute<W>(
        &mut self,
        writer: &mut W,
        identity: &RegisteredAgentIdentity,
        execute: &mrd_agent_ipc::ExecuteCommand,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let now_ms = self.clock.now_ms();
        let authorized = if let Some(security) = &self.security {
            if let Some(binding) = security
                .bindings
                .resolve(&execute.grant.claims.session_id, now_ms)
            {
                let desktop = validated_desktop_state(security.desktop_state.as_ref())?;
                if binding_matches_runtime(&binding, identity, desktop) {
                    let context = ExecutionContext {
                        registration_id: binding.registration_id,
                        registration_epoch: binding.registration_epoch,
                        session_id: binding.session_id,
                        peer: binding.peer,
                        policy_revision: binding.policy_revision,
                        windows_session_id: binding.windows_session_id,
                        desktop_epoch: binding.desktop_epoch,
                        desktop_kind: binding.desktop_kind,
                        now_ms,
                        expected_issuer_key_id: binding.expected_issuer_key_id,
                        authorization_scopes: binding.approved_scopes,
                        authorization_expires_at_ms: binding.authorization_expires_at_ms,
                    };
                    validate_execute_command(execute, &context, security.verifier.as_ref()).ok()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let outcome = match authorized {
            Some(authorized) => self.execute_once(authorized)?,
            None => CommandOutcome::Rejected,
        };

        write_frame(
            writer,
            &AgentToService::CommandResult(CommandResult {
                request_token: execute.request_token,
                registration_id: identity.registration_id,
                command_id: execute.command_id,
                outcome,
                completed_at_ms: self.clock.now_ms(),
            }),
        )
        .await?;
        Ok(())
    }

    async fn handle_input<W>(
        &mut self,
        writer: &mut W,
        identity: &RegisteredAgentIdentity,
        envelope: mrd_agent_ipc::InputEventEnvelope,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let now_ms = self.clock.now_ms();
        let outcome = if let (Some(input), Some(security)) = (&mut self.input, &self.security) {
            if let Some(binding) = security.bindings.resolve(&envelope.session_id, now_ms) {
                let desktop = validated_desktop_state(security.desktop_state.as_ref())?;
                if binding_matches_registration(&binding, identity)
                    && binding.desktop_kind == DesktopKind::Default
                {
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
                    let outcome = input.handle(&envelope, &context);
                    if matches!(
                        outcome,
                        InputAckOutcome::Rejected {
                            reason: mrd_agent_ipc::InputRejection::StaleDesktop
                        }
                    ) {
                        let _ = input.release_all();
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
        } else {
            InputAckOutcome::Rejected {
                reason: mrd_agent_ipc::InputRejection::Unsupported,
            }
        };
        write_frame(
            writer,
            &AgentToService::InputAck(InputAck {
                request_token: envelope.request_token,
                registration_id: identity.registration_id,
                registration_epoch: identity.registration_epoch,
                session_id: envelope.session_id.clone(),
                resource_id: envelope.resource_id,
                start_grant_id: envelope.start_grant_id,
                sequence: envelope.sequence,
                event_commitment: envelope.commitment().unwrap_or([0; 32]),
                outcome,
            }),
        )
        .await?;
        Ok(())
    }

    async fn handle_managed_consent<W>(
        &mut self,
        writer: &mut W,
        identity: &RegisteredAgentIdentity,
        request: ConsentRequest,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        if self.consent.is_none() {
            return self.handle_consent_without_manager(writer, request).await;
        }
        let context = self.trusted_consent_context(identity)?;
        let consent = self
            .consent
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?;
        let due = consent
            .manager
            .expire_due(Instant::now(), context.now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in due {
            write_frame(writer, &AgentToService::ConsentResult(result)).await?;
        }
        let immediate = match consent.manager.begin(request.clone(), context) {
            Ok(results) => results,
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
            write_frame(writer, &AgentToService::ConsentResult(result)).await?;
        }
        Ok(())
    }

    async fn handle_consent_completion<W>(
        &mut self,
        writer: &mut W,
        identity: &RegisteredAgentIdentity,
        completion: BackendCompletion,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let context = self.trusted_consent_context(identity)?;
        let results = self
            .consent
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
            .manager
            .complete(completion, context)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in results {
            write_frame(writer, &AgentToService::ConsentResult(result)).await?;
        }
        Ok(())
    }

    async fn handle_consent_deadline<W>(&mut self, writer: &mut W) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let now_ms = self.clock.now_ms();
        let results = self
            .consent
            .as_mut()
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?
            .manager
            .expire_due(Instant::now(), now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in results {
            write_frame(writer, &AgentToService::ConsentResult(result)).await?;
        }
        Ok(())
    }

    async fn handle_managed_cancel<W>(
        &mut self,
        writer: &mut W,
        cancel: CancelConsent,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let now_ms = self.clock.now_ms();
        let Some(consent) = &mut self.consent else {
            // Cleanup is deliberately safe to consume when no manager exists.
            return Ok(());
        };
        let results = consent
            .manager
            .cancel(&cancel, Instant::now(), now_ms)
            .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        for result in results {
            write_frame(writer, &AgentToService::ConsentResult(result)).await?;
        }
        Ok(())
    }

    fn trusted_consent_context(
        &self,
        identity: &RegisteredAgentIdentity,
    ) -> Result<TrustedConsentContext, AgentRuntimeError> {
        let expected_issuer_key_id = self
            .consent
            .as_ref()
            .map(|consent| consent.expected_issuer_key_id)
            .ok_or(AgentRuntimeError::ConsentStateUnavailable)?;
        let desktop = self
            .resolve_desktop_state()?
            .consent
            .unwrap_or(TrustedDesktopState {
                desktop_epoch: 0,
                desktop_kind: DesktopKind::Unknown,
            });
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
        if let Some(consent) = &mut self.consent {
            consent
                .manager
                .shutdown(reason, now_ms)
                .await
                .map_err(|_| AgentRuntimeError::ConsentStateUnavailable)?;
        }
        Ok(())
    }

    async fn handle_consent_without_manager<W>(
        &mut self,
        writer: &mut W,
        request: ConsentRequest,
    ) -> Result<(), AgentRuntimeError>
    where
        W: AsyncWrite + Unpin,
    {
        let now_ms = self.clock.now_ms();
        let decision = if now_ms >= request.issued_at_ms && now_ms < request.expires_at_ms {
            ConsentDecision::Dismissed
        } else {
            ConsentDecision::Expired
        };
        write_frame(
            writer,
            &AgentToService::ConsentResult(coarse_consent_result(&request, decision, now_ms)),
        )
        .await?;
        Ok(())
    }

    fn execute_once(
        &mut self,
        authorized: AuthorizedCommand,
    ) -> Result<CommandOutcome, AgentRuntimeError> {
        let grant_id = *authorized.grant_id();
        let command_id = *authorized.command_id();
        let fingerprint = SemanticFingerprint::from_authorized(&authorized);
        match self.replay.reserve(grant_id, command_id, fingerprint)? {
            ReplayReservation::First => {
                let capabilities = self.executor.capabilities();
                let outcome = if let Some(input) = &mut self.input {
                    match authorized.command().clone() {
                        mrd_agent_ipc::AgentCommand::StartInput { .. } => input
                            .start(authorized)
                            .map(|()| CommandOutcome::Completed)
                            .unwrap_or(CommandOutcome::Rejected),
                        mrd_agent_ipc::AgentCommand::StopInput { resource_id } => {
                            command_outcome(input.stop(&resource_id))
                        }
                        _ if capabilities.supports_command(authorized.command()) => {
                            self.executor.execute(authorized)
                        }
                        _ => CommandOutcome::Rejected,
                    }
                } else if capabilities.supports_command(authorized.command()) {
                    self.executor.execute(authorized)
                } else {
                    CommandOutcome::Rejected
                };
                self.replay.complete(command_id, outcome);
                Ok(outcome)
            }
            ReplayReservation::Cached(outcome) => Ok(outcome),
        }
    }

    fn release_input(&mut self) -> Result<(), mrd_input::InputError> {
        self.input
            .as_mut()
            .map_or(Ok(()), |input| input.release_all())
    }

    fn refresh_input_desktop(&mut self, current: TrustedDesktopState) {
        if self
            .last_desktop_state
            .is_some_and(|previous| previous != current)
        {
            let _ = self.release_input();
        }
        self.last_desktop_state = Some(current);
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

    fn current_desktop_state(&self) -> Result<TrustedDesktopState, AgentRuntimeError> {
        self.resolve_desktop_state()
            .map(|resolved| resolved.runtime)
    }

    fn resolve_desktop_state(&self) -> Result<ResolvedDesktopState, AgentRuntimeError> {
        let consent = match self.consent.as_ref() {
            Some(consent) => optional_desktop_state(consent.desktop_state.as_ref())?,
            None => None,
        };
        let execution = self
            .security
            .as_ref()
            .map(|security| validated_desktop_state(security.desktop_state.as_ref()))
            .transpose()?;
        let runtime = match (consent, execution) {
            (Some(consent), Some(execution)) if consent == execution => Ok(consent),
            (Some(_), Some(_)) => Err(AgentRuntimeError::DesktopStateUnavailable),
            (Some(consent), None) => Ok(consent),
            (None, Some(execution)) => Ok(execution),
            (None, None) => Ok(TrustedDesktopState {
                desktop_epoch: self.config.session.desktop_epoch,
                desktop_kind: DesktopKind::Default,
            }),
        }?;
        Ok(ResolvedDesktopState { runtime, consent })
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

fn optional_desktop_state(
    source: &dyn TrustedDesktopStateSource,
) -> Result<Option<TrustedDesktopState>, AgentRuntimeError> {
    match source.current_state() {
        Some(state) if state.desktop_epoch != 0 => Ok(Some(state)),
        Some(_) => Err(AgentRuntimeError::DesktopStateUnavailable),
        None => Ok(None),
    }
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
    Heartbeat,
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

async fn stop_reader(reader_task: JoinHandle<()>) {
    reader_task.abort();
    let _ = reader_task.await;
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
