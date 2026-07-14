use crate::paths::validate_relative_path;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum payload accepted in one file-bulk message.
pub const MAX_CHUNK_SIZE: u32 = 1024 * 1024;
/// Maximum number of bytes a manifest may describe.
pub const MAX_TRANSFER_SIZE: u64 = 1024 * 1024 * 1024 * 1024;

/// Direction relative to the target/session agent.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileDirection {
    /// Controller sends bytes to the target agent.
    Upload,
    /// Target agent sends bytes to the controller.
    Download,
}

/// Explicit provider selection.  Remote is the only provider implemented by
/// the file-bulk route; Local and External are never silently substituted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransferProvider {
    /// Authenticated peer/session-agent provider over the Bulk lane.
    #[default]
    Remote,
    /// Service-local administrative provider, never used for remote requests.
    Local,
    /// Reserved external provider (for example R-File).
    External,
}

/// Destination conflict policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Refuse to replace an existing destination.
    #[default]
    Reject,
    /// Replace only after final hash verification.
    Replace,
    /// Select a deterministic unique name.
    Rename,
}

/// An authenticated file transfer manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTransferManifest {
    /// Stable transfer identifier, scoped to one session.
    pub transfer_id: String,
    /// Authenticated session identifier.
    pub session_id: String,
    /// Upload or download direction.
    pub direction: FileDirection,
    /// Explicit data provider.
    #[serde(default)]
    pub provider: TransferProvider,
    /// Validated path relative to an approved root.
    pub relative_path: String,
    /// Total file size in bytes.
    pub size_bytes: u64,
    /// Bounded chunk size.
    pub chunk_size: u32,
    /// Lowercase SHA-256 digest of the complete file.
    pub file_sha256: String,
    /// Explicit destination conflict behavior.
    #[serde(default)]
    pub conflict_policy: ConflictPolicy,
}

/// Manifest validation failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    /// An identifier was empty or exceeded the wire limit.
    #[error("invalid transfer or session identifier")]
    InvalidIdentifier,
    /// The relative path failed secure path validation.
    #[error("invalid relative path: {0}")]
    InvalidPath(String),
    /// Chunk size is outside the bounded range.
    #[error("chunk size must be between 1 and {MAX_CHUNK_SIZE} bytes")]
    InvalidChunkSize,
    /// Transfer is larger than the configured safety ceiling.
    #[error("transfer exceeds {MAX_TRANSFER_SIZE} byte safety limit")]
    TransferTooLarge,
    /// Digest was not a lowercase hexadecimal SHA-256 value.
    #[error("file_sha256 must be a lowercase 64-character hexadecimal SHA-256")]
    InvalidHash,
}

impl FileTransferManifest {
    /// Construct and validate a remote manifest.
    pub fn new(
        transfer_id: impl Into<String>,
        session_id: impl Into<String>,
        direction: FileDirection,
        relative_path: impl AsRef<str>,
        size_bytes: u64,
        chunk_size: u32,
        file_sha256: impl Into<String>,
    ) -> Result<Self, ManifestError> {
        let transfer_id = transfer_id.into();
        let session_id = session_id.into();
        if transfer_id.trim().is_empty()
            || session_id.trim().is_empty()
            || transfer_id.len() > 128
            || session_id.len() > 128
        {
            return Err(ManifestError::InvalidIdentifier);
        }
        let relative_path = validate_relative_path(relative_path.as_ref())
            .map_err(|error| ManifestError::InvalidPath(error.to_string()))?;
        if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
            return Err(ManifestError::InvalidChunkSize);
        }
        if size_bytes > MAX_TRANSFER_SIZE {
            return Err(ManifestError::TransferTooLarge);
        }
        let file_sha256 = file_sha256.into();
        if file_sha256.len() != 64
            || !file_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ManifestError::InvalidHash);
        }
        Ok(Self {
            transfer_id,
            session_id,
            direction,
            provider: TransferProvider::Remote,
            relative_path,
            size_bytes,
            chunk_size,
            file_sha256,
            conflict_policy: ConflictPolicy::Reject,
        })
    }
}

/// One file-bulk protocol message.  The envelope is intended for the
/// transport-neutral Bulk lane; it must not be sent over interactive control.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileBulkMessage {
    /// Start or resume a transfer.
    Manifest(FileTransferManifest),
    /// One bounded, independently verifiable chunk.
    Chunk(crate::chunking::FileChunk),
    /// Receiver acknowledgement of a contiguous offset.
    Ack {
        /// Transfer identifier.
        transfer_id: String,
        /// Next byte expected by the receiver.
        next_offset: u64,
    },
    /// Cancellation or explicit disconnect notification.
    Cancel {
        /// Transfer identifier.
        transfer_id: String,
        /// Sanitized reason code.
        reason: String,
    },
}

impl FileBulkMessage {
    /// Return the transfer identifier without exposing file contents.
    pub fn transfer_id(&self) -> &str {
        match self {
            Self::Manifest(manifest) => &manifest.transfer_id,
            Self::Chunk(chunk) => &chunk.transfer_id,
            Self::Ack { transfer_id, .. } | Self::Cancel { transfer_id, .. } => transfer_id,
        }
    }
}
