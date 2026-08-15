//! Secret-bearing launcher bootstrap codec and bound registration verifier.

use crate::{ExecuteGrantVerifier, RegistrationProofVerifier, AGENT_IPC_MAX_IDENTIFIER_BYTES};
use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use ring::digest::{Context as DigestContext, SHA256};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

const BOOTSTRAP_MAGIC: &[u8; 8] = b"MRDABT2\0";
const BOOTSTRAP_VERSION: u16 = 2;
const BOOTSTRAP_HEADER_BYTES: usize = 168;
const BOOTSTRAP_MAX_BYTES: usize = 1_024;
const REGISTRATION_KEY_ID_CONTEXT: &[u8] = b"mrd-agent-registration-key-v1\0";

/// Non-secret public half of a bootstrap-derived registration key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistrationPublicKey {
    /// Domain-separated identifier pinned into launcher admission state.
    pub key_id: [u8; 32],
    /// Ed25519 public verification key.
    pub public_key: [u8; 32],
}

/// One-shot bootstrap record written by the trusted launcher.
///
/// This type deliberately owns the registration seed and implements neither
/// `Debug` nor `Clone`. Passing it to [`write_agent_bootstrap`] consumes the
/// record so the owned zeroizing copy is not accidentally reused.
pub struct AgentBootstrap<'a> {
    /// Random private control-pipe endpoint.
    pub control_endpoint: &'a str,
    /// Machine-service process identifier.
    pub service_process_id: u32,
    /// Machine-service process creation time protecting against PID reuse.
    pub service_process_creation_time: u64,
    /// Agent heartbeat cadence.
    pub heartbeat_interval_ms: u32,
    /// Registration handshake deadline.
    pub handshake_timeout_ms: u32,
    /// Per-launch Ed25519 seed; never serialized through serde.
    pub registration_seed: Zeroizing<[u8; 32]>,
    /// Key identifier already pinned in the service admission.
    pub expected_agent_key_id: [u8; 32],
    /// SHA-256 identifier of the sole trusted execute-grant issuer public key.
    pub execute_grant_issuer_key_id: [u8; 32],
    /// Sole trusted Ed25519 execute-grant issuer public key.
    pub execute_grant_public_key: [u8; 32],
}

/// Validated bootstrap record received through a protected launcher channel.
///
/// This type deliberately implements neither `Debug` nor `Clone`.
pub struct ReceivedAgentBootstrap {
    control_endpoint: String,
    service_process_id: u32,
    service_process_creation_time: u64,
    heartbeat_interval_ms: u32,
    handshake_timeout_ms: u32,
    registration_seed: Zeroizing<[u8; 32]>,
    expected_agent_key_id: [u8; 32],
    execute_grant_issuer_key_id: [u8; 32],
    execute_grant_public_key: [u8; 32],
}

impl ReceivedAgentBootstrap {
    /// Random local endpoint provisioned by the launcher.
    pub fn control_endpoint(&self) -> &str {
        &self.control_endpoint
    }

    /// Expected machine-service PID.
    pub fn service_process_id(&self) -> u32 {
        self.service_process_id
    }

    /// Expected machine-service process creation time.
    pub fn service_process_creation_time(&self) -> u64 {
        self.service_process_creation_time
    }

    /// Configured heartbeat cadence.
    pub fn heartbeat_interval_ms(&self) -> u32 {
        self.heartbeat_interval_ms
    }

    /// Configured handshake timeout.
    pub fn handshake_timeout_ms(&self) -> u32 {
        self.handshake_timeout_ms
    }

    /// Bootstrap-pinned registration key identifier.
    pub fn expected_agent_key_id(&self) -> &[u8; 32] {
        &self.expected_agent_key_id
    }

    /// Bootstrap-pinned SHA-256 identifier of the execute-grant issuer.
    pub fn execute_grant_issuer_key_id(&self) -> &[u8; 32] {
        &self.execute_grant_issuer_key_id
    }

    /// Bootstrap-pinned Ed25519 execute-grant issuer public key.
    pub fn execute_grant_public_key(&self) -> &[u8; 32] {
        &self.execute_grant_public_key
    }

    /// Consume the record and transfer the zeroizing registration seed.
    pub fn into_registration_seed(self) -> Zeroizing<[u8; 32]> {
        self.registration_seed
    }

    /// Split configuration fields from the zeroizing registration seed.
    #[allow(clippy::type_complexity)]
    pub fn into_parts(
        self,
    ) -> (
        String,
        u32,
        u64,
        u32,
        u32,
        [u8; 32],
        Zeroizing<[u8; 32]>,
        [u8; 32],
        [u8; 32],
    ) {
        (
            self.control_endpoint,
            self.service_process_id,
            self.service_process_creation_time,
            self.heartbeat_interval_ms,
            self.handshake_timeout_ms,
            self.expected_agent_key_id,
            self.registration_seed,
            self.execute_grant_issuer_key_id,
            self.execute_grant_public_key,
        )
    }
}

/// Bootstrap validation and framing failures.
#[derive(Debug, Error)]
pub enum AgentBootstrapError {
    /// Magic, version, lengths, timing, or identity fields are invalid.
    #[error("agent bootstrap record is invalid")]
    InvalidRecord,
    /// Registration seed cannot produce the expected Ed25519 key.
    #[error("agent bootstrap registration key is invalid")]
    InvalidRegistrationKey,
    /// Execute-grant issuer id or Ed25519 public key is invalid.
    #[error("agent bootstrap execute-grant issuer key is invalid")]
    InvalidExecuteGrantIssuerKey,
    /// Bootstrap channel ended or failed.
    #[error("agent bootstrap channel failed")]
    Io(#[from] std::io::Error),
}

/// Derive the public registration key and its domain-separated identifier.
pub fn derive_registration_public_key(
    seed: &[u8; 32],
) -> Result<RegistrationPublicKey, AgentBootstrapError> {
    if seed.iter().all(|byte| *byte == 0) {
        return Err(AgentBootstrapError::InvalidRegistrationKey);
    }
    let public_key = SigningKey::from_bytes(seed).verifying_key().to_bytes();
    let mut context = DigestContext::new(&SHA256);
    context.update(REGISTRATION_KEY_ID_CONTEXT);
    context.update(&public_key);
    let key_id = digest_to_array(context.finish().as_ref())?;
    Ok(RegistrationPublicKey { key_id, public_key })
}

/// Derive the raw SHA-256 execute-grant issuer key id used by `mrd-identity`.
///
/// `mrd-identity` renders these same 32 bytes as lowercase hexadecimal. Keeping
/// the bootstrap representation raw avoids text parsing at this trust boundary.
pub fn derive_execute_grant_issuer_key_id(public_key: &[u8; 32]) -> [u8; 32] {
    ring::digest::digest(&SHA256, public_key)
        .as_ref()
        .try_into()
        .expect("SHA-256 always returns 32 bytes")
}

/// Verifier pinned to the public key provisioned by one launcher bootstrap.
#[derive(Debug, Clone)]
pub struct BoundEd25519RegistrationVerifier {
    expected_key_id: [u8; 32],
    public_key: VerifyingKey,
}

impl BoundEd25519RegistrationVerifier {
    /// Validate and bind a public key to its expected key identifier.
    pub fn new(
        expected_key_id: [u8; 32],
        public_key: [u8; 32],
    ) -> Result<Self, AgentBootstrapError> {
        let mut context = DigestContext::new(&SHA256);
        context.update(REGISTRATION_KEY_ID_CONTEXT);
        context.update(&public_key);
        if digest_to_array(context.finish().as_ref())? != expected_key_id {
            return Err(AgentBootstrapError::InvalidRegistrationKey);
        }
        let public_key = VerifyingKey::from_bytes(&public_key)
            .map_err(|_| AgentBootstrapError::InvalidRegistrationKey)?;
        Ok(Self {
            expected_key_id,
            public_key,
        })
    }
}

impl RegistrationProofVerifier for BoundEd25519RegistrationVerifier {
    fn verify(&self, agent_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool {
        agent_key_id == &self.expected_key_id
            && self
                .public_key
                .verify_strict(signing_bytes, &Signature::from_bytes(signature))
                .is_ok()
    }
}

/// Execute-grant verifier pinned to the issuer provisioned by one bootstrap.
#[derive(Debug, Clone)]
pub struct BoundEd25519ExecuteGrantVerifier {
    expected_key_id: [u8; 32],
    public_key: VerifyingKey,
}

impl BoundEd25519ExecuteGrantVerifier {
    /// Validate and bind one raw SHA-256 key id to an Ed25519 public key.
    pub fn new(
        expected_key_id: [u8; 32],
        public_key: [u8; 32],
    ) -> Result<Self, AgentBootstrapError> {
        if expected_key_id.iter().all(|byte| *byte == 0)
            || public_key.iter().all(|byte| *byte == 0)
            || derive_execute_grant_issuer_key_id(&public_key) != expected_key_id
        {
            return Err(AgentBootstrapError::InvalidExecuteGrantIssuerKey);
        }
        let point = CompressedEdwardsY(public_key)
            .decompress()
            .ok_or(AgentBootstrapError::InvalidExecuteGrantIssuerKey)?;
        if point.compress().to_bytes() != public_key || !point.is_torsion_free() {
            return Err(AgentBootstrapError::InvalidExecuteGrantIssuerKey);
        }
        let public_key = VerifyingKey::from_bytes(&public_key)
            .map_err(|_| AgentBootstrapError::InvalidExecuteGrantIssuerKey)?;
        if public_key.is_weak() {
            return Err(AgentBootstrapError::InvalidExecuteGrantIssuerKey);
        }
        Ok(Self {
            expected_key_id,
            public_key,
        })
    }
}

impl ExecuteGrantVerifier for BoundEd25519ExecuteGrantVerifier {
    fn verify(&self, issuer_key_id: &[u8; 32], signing_bytes: &[u8], signature: &[u8; 64]) -> bool {
        issuer_key_id == &self.expected_key_id
            && self
                .public_key
                .verify_strict(signing_bytes, &Signature::from_bytes(signature))
                .is_ok()
    }
}

/// Write one bounded non-serde bootstrap record.
pub async fn write_agent_bootstrap<W>(
    writer: &mut W,
    bootstrap: AgentBootstrap<'_>,
) -> Result<(), AgentBootstrapError>
where
    W: AsyncWrite + Unpin,
{
    validate_bootstrap_fields(BootstrapFields {
        control_endpoint: bootstrap.control_endpoint,
        service_process_id: bootstrap.service_process_id,
        service_process_creation_time: bootstrap.service_process_creation_time,
        heartbeat_interval_ms: bootstrap.heartbeat_interval_ms,
        handshake_timeout_ms: bootstrap.handshake_timeout_ms,
        registration_seed: &bootstrap.registration_seed,
        expected_agent_key_id: &bootstrap.expected_agent_key_id,
        execute_grant_issuer_key_id: &bootstrap.execute_grant_issuer_key_id,
        execute_grant_public_key: &bootstrap.execute_grant_public_key,
    })?;
    let endpoint = bootstrap.control_endpoint.as_bytes();
    let total_len = BOOTSTRAP_HEADER_BYTES
        .checked_add(endpoint.len())
        .filter(|length| *length <= BOOTSTRAP_MAX_BYTES)
        .ok_or(AgentBootstrapError::InvalidRecord)?;
    let endpoint_len =
        u16::try_from(endpoint.len()).map_err(|_| AgentBootstrapError::InvalidRecord)?;
    let mut frame = Zeroizing::new(Vec::with_capacity(total_len));
    frame.extend_from_slice(BOOTSTRAP_MAGIC);
    frame.extend_from_slice(&BOOTSTRAP_VERSION.to_le_bytes());
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&(total_len as u32).to_le_bytes());
    frame.extend_from_slice(&endpoint_len.to_le_bytes());
    frame.extend_from_slice(&0_u16.to_le_bytes());
    frame.extend_from_slice(&bootstrap.service_process_id.to_le_bytes());
    frame.extend_from_slice(&bootstrap.service_process_creation_time.to_le_bytes());
    frame.extend_from_slice(&bootstrap.heartbeat_interval_ms.to_le_bytes());
    frame.extend_from_slice(&bootstrap.handshake_timeout_ms.to_le_bytes());
    frame.extend_from_slice(&bootstrap.expected_agent_key_id);
    frame.extend_from_slice(&bootstrap.registration_seed[..]);
    frame.extend_from_slice(&bootstrap.execute_grant_issuer_key_id);
    frame.extend_from_slice(&bootstrap.execute_grant_public_key);
    frame.extend_from_slice(endpoint);
    debug_assert_eq!(frame.len(), total_len);
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Read and validate one bounded non-serde bootstrap record.
pub async fn read_agent_bootstrap<R>(
    reader: &mut R,
) -> Result<ReceivedAgentBootstrap, AgentBootstrapError>
where
    R: AsyncRead + Unpin,
{
    let mut header = Zeroizing::new([0_u8; BOOTSTRAP_HEADER_BYTES]);
    reader.read_exact(&mut *header).await?;
    if &header[0..8] != BOOTSTRAP_MAGIC
        || u16_at(&*header, 8)? != BOOTSTRAP_VERSION
        || u16_at(&*header, 10)? != 0
        || u16_at(&*header, 18)? != 0
    {
        return Err(AgentBootstrapError::InvalidRecord);
    }
    let total_len =
        usize::try_from(u32_at(&*header, 12)?).map_err(|_| AgentBootstrapError::InvalidRecord)?;
    let endpoint_len = usize::from(u16_at(&*header, 16)?);
    if total_len != BOOTSTRAP_HEADER_BYTES + endpoint_len
        || total_len > BOOTSTRAP_MAX_BYTES
        || endpoint_len == 0
        || endpoint_len > AGENT_IPC_MAX_IDENTIFIER_BYTES * 2
    {
        return Err(AgentBootstrapError::InvalidRecord);
    }

    let service_process_id = u32_at(&*header, 20)?;
    let service_process_creation_time = u64_at(&*header, 24)?;
    let heartbeat_interval_ms = u32_at(&*header, 32)?;
    let handshake_timeout_ms = u32_at(&*header, 36)?;
    let expected_agent_key_id: [u8; 32] = header[40..72]
        .try_into()
        .map_err(|_| AgentBootstrapError::InvalidRecord)?;
    let registration_seed = Zeroizing::new(
        header[72..104]
            .try_into()
            .map_err(|_| AgentBootstrapError::InvalidRecord)?,
    );
    let execute_grant_issuer_key_id: [u8; 32] = header[104..136]
        .try_into()
        .map_err(|_| AgentBootstrapError::InvalidRecord)?;
    let execute_grant_public_key: [u8; 32] = header[136..168]
        .try_into()
        .map_err(|_| AgentBootstrapError::InvalidRecord)?;
    let mut endpoint = Zeroizing::new(vec![0_u8; endpoint_len]);
    reader.read_exact(&mut endpoint).await?;
    let control_endpoint = std::str::from_utf8(&endpoint)
        .map_err(|_| AgentBootstrapError::InvalidRecord)?
        .to_owned();
    let received = ReceivedAgentBootstrap {
        control_endpoint,
        service_process_id,
        service_process_creation_time,
        heartbeat_interval_ms,
        handshake_timeout_ms,
        registration_seed,
        expected_agent_key_id,
        execute_grant_issuer_key_id,
        execute_grant_public_key,
    };
    validate_received_bootstrap(&received)?;
    Ok(received)
}

/// Derive the Windows bootstrap pipe name from immutable OS process identity.
pub fn windows_agent_bootstrap_pipe_name(
    windows_session_id: u32,
    process_id: u32,
    process_creation_time: u64,
) -> String {
    format!(
        r"\\.\pipe\mrd-agent-bootstrap-v2-s{windows_session_id}-p{process_id}-c{process_creation_time:016x}"
    )
}

struct BootstrapFields<'a> {
    control_endpoint: &'a str,
    service_process_id: u32,
    service_process_creation_time: u64,
    heartbeat_interval_ms: u32,
    handshake_timeout_ms: u32,
    registration_seed: &'a [u8; 32],
    expected_agent_key_id: &'a [u8; 32],
    execute_grant_issuer_key_id: &'a [u8; 32],
    execute_grant_public_key: &'a [u8; 32],
}

fn validate_bootstrap_fields(fields: BootstrapFields<'_>) -> Result<(), AgentBootstrapError> {
    if fields.control_endpoint.is_empty()
        || fields.control_endpoint.contains('\0')
        || fields.control_endpoint.len() > AGENT_IPC_MAX_IDENTIFIER_BYTES * 2
        || fields.service_process_id == 0
        || fields.service_process_creation_time == 0
        || fields.heartbeat_interval_ms == 0
        || fields.handshake_timeout_ms == 0
        || fields.expected_agent_key_id.iter().all(|byte| *byte == 0)
    {
        return Err(AgentBootstrapError::InvalidRecord);
    }
    let derived = derive_registration_public_key(fields.registration_seed)?;
    if &derived.key_id != fields.expected_agent_key_id {
        return Err(AgentBootstrapError::InvalidRegistrationKey);
    }
    BoundEd25519ExecuteGrantVerifier::new(
        *fields.execute_grant_issuer_key_id,
        *fields.execute_grant_public_key,
    )?;
    Ok(())
}

fn validate_received_bootstrap(
    bootstrap: &ReceivedAgentBootstrap,
) -> Result<(), AgentBootstrapError> {
    validate_bootstrap_fields(BootstrapFields {
        control_endpoint: &bootstrap.control_endpoint,
        service_process_id: bootstrap.service_process_id,
        service_process_creation_time: bootstrap.service_process_creation_time,
        heartbeat_interval_ms: bootstrap.heartbeat_interval_ms,
        handshake_timeout_ms: bootstrap.handshake_timeout_ms,
        registration_seed: &bootstrap.registration_seed,
        expected_agent_key_id: &bootstrap.expected_agent_key_id,
        execute_grant_issuer_key_id: &bootstrap.execute_grant_issuer_key_id,
        execute_grant_public_key: &bootstrap.execute_grant_public_key,
    })
}

fn digest_to_array(bytes: &[u8]) -> Result<[u8; 32], AgentBootstrapError> {
    bytes
        .try_into()
        .map_err(|_| AgentBootstrapError::InvalidRegistrationKey)
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, AgentBootstrapError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or(AgentBootstrapError::InvalidRecord)
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, AgentBootstrapError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or(AgentBootstrapError::InvalidRecord)
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, AgentBootstrapError> {
    bytes
        .get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or(AgentBootstrapError::InvalidRecord)
}
