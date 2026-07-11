//! Bounded per-connection registration and lifecycle server.

use super::{AgentConnectionId, AgentRegistry, AgentRegistryError, ObservedAgentIdentity};
use mrd_agent_ipc::{
    decode_frame, write_frame, AgentHeartbeat, AgentStopping, AgentToService, FrameError,
    RegisteredAgentIdentity, ServiceToAgent, StopAgent, StopReason, AGENT_IPC_FRAME_HEADER_BYTES,
    AGENT_IPC_MAX_FRAME_BYTES, AGENT_IPC_PROTOCOL_MAJOR,
};
use std::{
    collections::{hash_map::Entry, HashMap},
    future::Future,
    io::ErrorKind,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::mpsc,
    task::JoinHandle,
};

const INBOUND_QUEUE_CAPACITY: usize = 32;
const OUTBOUND_QUEUE_CAPACITY: usize = 32;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const PARTIAL_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const REPLACED_STOP_GRACE_MS: u64 = 5_000;

/// Service clock boundary used by deterministic protocol tests.
pub trait AgentServerClock: Send + Sync {
    /// Current service time in Unix milliseconds.
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Default)]
struct SystemAgentServerClock;

impl AgentServerClock for SystemAgentServerClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }
}

/// Normal terminal state for one private agent connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentConnectionExit {
    /// Peer closed the private stream without a graceful stopping event.
    Disconnected,
    /// A bound `AgentStopping` event completed graceful shutdown.
    Stopped,
}

/// Registration server failures. Any failure closes and revokes the connection.
#[derive(Debug, Error)]
pub enum AgentServerError {
    /// Registry rejected identity, proof, capability, or lifecycle state.
    #[error(transparent)]
    Registry(#[from] AgentRegistryError),
    /// Bounded control framing failed.
    #[error(transparent)]
    Frame(#[from] FrameError),
    /// The peer did not complete the registration sequence in time.
    #[error("agent registration handshake timed out")]
    HandshakeTimeout,
    /// The stream ended during registration.
    #[error("agent disconnected during registration")]
    DisconnectedDuringHandshake,
    /// Registration messages arrived out of order.
    #[error("agent registration message sequence is invalid")]
    UnexpectedHandshakeMessage,
    /// A post-registration message is not yet supported by the service shell.
    #[error("agent sent an unsupported registered message")]
    UnsupportedRegisteredMessage,
    /// The connection id is already installed in the live server directory.
    #[error("agent connection id is already served")]
    DuplicateConnection,
    /// No live connection owns the requested outbound queue.
    #[error("agent connection is unavailable")]
    ConnectionUnavailable,
    /// The bounded outbound queue is full or closed.
    #[error("agent outbound queue is unavailable")]
    OutboundUnavailable,
    /// Peer stopped reading before a bounded control write completed.
    #[error("agent control write timed out")]
    WriteTimeout,
}

enum InboundEvent {
    Message(AgentToService),
    Failed(FrameError),
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevocableWriteOutcome {
    Written,
    Revoked,
}

/// Shared server for authenticated agent connections.
pub struct AgentServer {
    registry: Arc<AgentRegistry>,
    clock: Arc<dyn AgentServerClock>,
    controls: Arc<Mutex<HashMap<AgentConnectionId, mpsc::Sender<ServiceToAgent>>>>,
}

impl AgentServer {
    /// Construct a server using the production wall clock.
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self::with_clock(registry, Arc::new(SystemAgentServerClock))
    }

    /// Construct a server with an injected trusted clock.
    pub fn with_clock(registry: Arc<AgentRegistry>, clock: Arc<dyn AgentServerClock>) -> Self {
        Self {
            registry,
            clock,
            controls: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Queue a bounded service command for one exact private connection.
    pub fn send_to_connection(
        &self,
        connection_id: AgentConnectionId,
        message: ServiceToAgent,
    ) -> Result<(), AgentServerError> {
        if !self.registry.is_connection_active(connection_id) {
            return Err(AgentServerError::ConnectionUnavailable);
        }
        let sender = self
            .controls
            .lock()
            .map_err(|_| AgentServerError::ConnectionUnavailable)?
            .get(&connection_id)
            .cloned()
            .ok_or(AgentServerError::ConnectionUnavailable)?;
        sender
            .try_send(message)
            .map_err(|_| AgentServerError::OutboundUnavailable)
    }

    /// Serve one stream whose OS identity was independently verified.
    pub async fn serve_connection<S>(
        &self,
        stream: S,
        connection_id: AgentConnectionId,
        observed: ObservedAgentIdentity,
    ) -> Result<AgentConnectionExit, AgentServerError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound_tx, outbound_rx) = mpsc::channel(OUTBOUND_QUEUE_CAPACITY);
        {
            let mut controls = self
                .controls
                .lock()
                .map_err(|_| AgentServerError::ConnectionUnavailable)?;
            match controls.entry(connection_id) {
                Entry::Occupied(_) => return Err(AgentServerError::DuplicateConnection),
                Entry::Vacant(entry) => {
                    entry.insert(outbound_tx);
                }
            }
        }
        let cleanup = ConnectionCleanup {
            registry: Arc::clone(&self.registry),
            controls: Arc::clone(&self.controls),
            connection_id,
        };
        let (reader, mut writer) = tokio::io::split(stream);
        let (inbound_tx, mut inbound_rx) = mpsc::channel(INBOUND_QUEUE_CAPACITY);
        let reader_task = tokio::spawn(read_loop(reader, inbound_tx));

        let result = self
            .run_connection(
                &mut writer,
                &mut inbound_rx,
                outbound_rx,
                connection_id,
                observed,
            )
            .await;
        stop_reader(reader_task).await;
        drop(cleanup);
        result
    }

    async fn run_connection<W>(
        &self,
        writer: &mut W,
        inbound: &mut mpsc::Receiver<InboundEvent>,
        mut outbound: mpsc::Receiver<ServiceToAgent>,
        connection_id: AgentConnectionId,
        observed: ObservedAgentIdentity,
    ) -> Result<AgentConnectionExit, AgentServerError>
    where
        W: AsyncWrite + Unpin,
    {
        let register = match next_handshake_message(inbound).await? {
            AgentToService::AgentRegister(register) => register,
            _ => return Err(AgentServerError::UnexpectedHandshakeMessage),
        };
        let challenge = self.registry.begin_registration(
            connection_id,
            register,
            observed,
            self.clock.now_ms(),
        )?;
        write_service_frame(writer, &ServiceToAgent::AgentChallenge(challenge)).await?;

        let proof = match next_handshake_message(inbound).await? {
            AgentToService::AgentRegistered(proof) => proof,
            _ => return Err(AgentServerError::UnexpectedHandshakeMessage),
        };
        let identity =
            self.registry
                .complete_registration(connection_id, proof, self.clock.now_ms())?;

        let capabilities = match next_handshake_message(inbound).await? {
            AgentToService::AgentCapabilitySnapshot(snapshot) => snapshot,
            _ => return Err(AgentServerError::UnexpectedHandshakeMessage),
        };
        self.registry
            .activate_registration(connection_id, capabilities, self.clock.now_ms())?;
        let mut lease = self
            .registry
            .lease_for_session(identity.windows_session_id)
            .filter(|lease| {
                lease.registration_id() == &identity.registration_id
                    && lease.registration_epoch() == identity.registration_epoch
            })
            .ok_or(AgentRegistryError::NotActive)?;

        loop {
            tokio::select! {
                biased;
                _ = lease.wait_revoked() => {
                    return self
                        .stop_revoked_connection(writer, inbound, &identity)
                        .await;
                }
                outbound_message = outbound.recv() => {
                    match outbound_message {
                        Some(message) => {
                            if lease.is_revoked() {
                                return self
                                    .stop_revoked_connection(writer, inbound, &identity)
                                    .await;
                            }
                            let revoked = lease.wait_revoked();
                            if write_service_frame_until_revoked(writer, &message, revoked).await?
                                == RevocableWriteOutcome::Revoked
                            {
                                // A frame may now be partial, so hard-close instead of
                                // appending a StopAgent frame to a corrupt stream.
                                return Ok(AgentConnectionExit::Disconnected);
                            }
                        }
                        None => return Err(AgentServerError::OutboundUnavailable),
                    }
                }
                inbound_event = inbound.recv() => {
                    match inbound_event {
                        Some(InboundEvent::Message(AgentToService::AgentHeartbeat(heartbeat))) => {
                            self.registry.record_heartbeat(
                                connection_id,
                                heartbeat,
                                self.clock.now_ms(),
                            )?;
                        }
                        Some(InboundEvent::Message(AgentToService::AgentCapabilitySnapshot(snapshot))) => {
                            self.registry.record_capabilities(
                                connection_id,
                                snapshot,
                                self.clock.now_ms(),
                            )?;
                        }
                        Some(InboundEvent::Message(AgentToService::AgentStopping(stopping))) => {
                            let heartbeat = AgentHeartbeat { context: stopping.context };
                            match self.registry.record_heartbeat(
                                connection_id,
                                heartbeat,
                                self.clock.now_ms(),
                            ) {
                                Ok(()) | Err(AgentRegistryError::NotActive) => {
                                    return Ok(AgentConnectionExit::Stopped);
                                }
                                Err(error) => return Err(error.into()),
                            }
                        }
                        Some(InboundEvent::Message(_)) => {
                            return Err(AgentServerError::UnsupportedRegisteredMessage);
                        }
                        Some(InboundEvent::Failed(error)) => return Err(error.into()),
                        Some(InboundEvent::Disconnected) | None => {
                            return Ok(AgentConnectionExit::Disconnected);
                        }
                    }
                }
            }
        }
    }

    async fn stop_revoked_connection<W>(
        &self,
        writer: &mut W,
        inbound: &mut mpsc::Receiver<InboundEvent>,
        identity: &RegisteredAgentIdentity,
    ) -> Result<AgentConnectionExit, AgentServerError>
    where
        W: AsyncWrite + Unpin,
    {
        let budget = Duration::from_millis(REPLACED_STOP_GRACE_MS);
        let stop = ServiceToAgent::StopAgent(StopAgent {
            request_id: identity.registration_id,
            deadline_ms: self.clock.now_ms().saturating_add(REPLACED_STOP_GRACE_MS),
            reason: StopReason::PolicyChange,
        });
        match tokio::time::timeout(budget, async {
            write_service_frame(writer, &stop).await?;
            wait_for_revoked_stop(inbound, identity).await
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Ok(AgentConnectionExit::Disconnected),
        }
    }
}

struct ConnectionCleanup {
    registry: Arc<AgentRegistry>,
    controls: Arc<Mutex<HashMap<AgentConnectionId, mpsc::Sender<ServiceToAgent>>>>,
    connection_id: AgentConnectionId,
}

impl Drop for ConnectionCleanup {
    fn drop(&mut self) {
        self.registry.disconnect(self.connection_id);
        if let Ok(mut controls) = self.controls.lock() {
            controls.remove(&self.connection_id);
        }
    }
}

async fn next_handshake_message(
    inbound: &mut mpsc::Receiver<InboundEvent>,
) -> Result<AgentToService, AgentServerError> {
    let event = tokio::time::timeout(HANDSHAKE_TIMEOUT, inbound.recv())
        .await
        .map_err(|_| AgentServerError::HandshakeTimeout)?
        .ok_or(AgentServerError::DisconnectedDuringHandshake)?;
    match event {
        InboundEvent::Message(message) => Ok(message),
        InboundEvent::Failed(error) => Err(error.into()),
        InboundEvent::Disconnected => Err(AgentServerError::DisconnectedDuringHandshake),
    }
}

async fn write_service_frame<W>(
    writer: &mut W,
    message: &ServiceToAgent,
) -> Result<(), AgentServerError>
where
    W: AsyncWrite + Unpin,
{
    tokio::time::timeout(WRITE_TIMEOUT, write_frame(writer, message))
        .await
        .map_err(|_| AgentServerError::WriteTimeout)??;
    Ok(())
}

async fn write_service_frame_until_revoked<W, F>(
    writer: &mut W,
    message: &ServiceToAgent,
    revocation: F,
) -> Result<RevocableWriteOutcome, AgentServerError>
where
    W: AsyncWrite + Unpin,
    F: Future<Output = ()>,
{
    tokio::pin!(revocation);
    tokio::select! {
        biased;
        _ = &mut revocation => Ok(RevocableWriteOutcome::Revoked),
        result = write_service_frame(writer, message) => {
            result?;
            Ok(RevocableWriteOutcome::Written)
        }
    }
}

async fn wait_for_revoked_stop(
    inbound: &mut mpsc::Receiver<InboundEvent>,
    identity: &RegisteredAgentIdentity,
) -> Result<AgentConnectionExit, AgentServerError> {
    loop {
        match inbound.recv().await {
            Some(InboundEvent::Message(AgentToService::AgentStopping(stopping))) => {
                validate_stopping_binding(&stopping, identity)?;
                return Ok(AgentConnectionExit::Stopped);
            }
            Some(InboundEvent::Message(_)) => {}
            Some(InboundEvent::Failed(error)) => return Err(error.into()),
            Some(InboundEvent::Disconnected) | None => {
                return Ok(AgentConnectionExit::Disconnected)
            }
        }
    }
}

fn validate_stopping_binding(
    stopping: &AgentStopping,
    identity: &RegisteredAgentIdentity,
) -> Result<(), AgentServerError> {
    let context = &stopping.context;
    if context.registration_id != identity.registration_id
        || context.registration_epoch != identity.registration_epoch
        || context.windows_session_id != identity.windows_session_id
        || context.desktop_epoch == 0
        || context.sequence == 0
        || context.observed_at_ms == 0
    {
        return Err(AgentRegistryError::MessageBindingMismatch.into());
    }
    Ok(())
}

async fn read_loop<R>(mut reader: R, sender: mpsc::Sender<InboundEvent>)
where
    R: AsyncRead + Unpin,
{
    loop {
        match read_agent_frame(&mut reader, PARTIAL_FRAME_TIMEOUT).await {
            Ok(message) => {
                if sender.send(InboundEvent::Message(message)).await.is_err() {
                    return;
                }
            }
            Err(FrameError::Io(error))
                if matches!(
                    error.kind(),
                    ErrorKind::UnexpectedEof
                        | ErrorKind::BrokenPipe
                        | ErrorKind::ConnectionAborted
                        | ErrorKind::ConnectionReset
                ) =>
            {
                let _ = sender.send(InboundEvent::Disconnected).await;
                return;
            }
            Err(error) => {
                let _ = sender.send(InboundEvent::Failed(error)).await;
                return;
            }
        }
    }
}

async fn read_agent_frame<R>(
    reader: &mut R,
    partial_timeout: Duration,
) -> Result<AgentToService, FrameError>
where
    R: AsyncRead + Unpin,
{
    let mut header = [0_u8; AGENT_IPC_FRAME_HEADER_BYTES];
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
    decode_frame::<AgentToService>(&frame).map(|decoded| decoded.message)
}

fn partial_frame_timeout_error() -> FrameError {
    FrameError::Io(std::io::Error::new(
        ErrorKind::TimedOut,
        "partial agent IPC frame timed out",
    ))
}

async fn stop_reader(reader_task: JoinHandle<()>) {
    reader_task.abort();
    let _ = reader_task.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_agent_ipc::encode_frame;
    use tokio::{
        io::{duplex, AsyncReadExt},
        sync::oneshot,
    };

    #[tokio::test]
    async fn revocation_cancels_a_stalled_outbound_frame() {
        let message = ServiceToAgent::StopAgent(StopAgent {
            request_id: [7; 16],
            deadline_ms: 5_000,
            reason: StopReason::ServiceShutdown,
        });
        let complete_frame_len = encode_frame(&message).unwrap().len();
        let (mut writer, mut reader) = duplex(1);
        let (revoke_tx, revoke_rx) = oneshot::channel();
        let writing = tokio::spawn(async move {
            write_service_frame_until_revoked(&mut writer, &message, async {
                let _ = revoke_rx.await;
            })
            .await
        });

        tokio::task::yield_now().await;
        revoke_tx.send(()).unwrap();
        assert_eq!(
            writing.await.unwrap().unwrap(),
            RevocableWriteOutcome::Revoked
        );
        let mut delivered = Vec::new();
        reader.read_to_end(&mut delivered).await.unwrap();
        assert!(
            delivered.len() < complete_frame_len,
            "a revoked peer must not receive a complete stalled frame"
        );
    }
}
