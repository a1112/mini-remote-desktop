use crate::protocol::MAX_CHUNK_SIZE;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// One bounded file chunk with an independently checked SHA-256 digest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChunk {
    /// Transfer identifier.
    pub transfer_id: String,
    /// Absolute byte offset in the manifest's file.
    pub offset: u64,
    /// Chunk bytes.
    pub payload: Vec<u8>,
    /// Lowercase SHA-256 digest of `payload`.
    pub sha256: String,
}

/// Chunking and verification failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    /// A zero or oversized chunk size was requested.
    #[error("chunk size must be between 1 and {MAX_CHUNK_SIZE} bytes")]
    InvalidSize,
    /// A payload exceeded the wire bound.
    #[error("chunk payload exceeds {MAX_CHUNK_SIZE} bytes")]
    Oversized,
    /// A digest did not match the payload.
    #[error("chunk SHA-256 mismatch")]
    HashMismatch,
}

/// Compute lowercase hexadecimal SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Split bytes into bounded, sequential chunks.
pub fn chunk_bytes(bytes: &[u8], chunk_size: u32) -> Result<Vec<FileChunk>, ChunkError> {
    if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
        return Err(ChunkError::InvalidSize);
    }
    let chunk_size = chunk_size as usize;
    Ok(bytes
        .chunks(chunk_size)
        .enumerate()
        .map(|(index, payload)| FileChunk {
            transfer_id: String::new(),
            offset: (index * chunk_size) as u64,
            payload: payload.to_vec(),
            sha256: sha256_hex(payload),
        })
        .collect())
}

/// Verify size bound and digest for one received chunk.
pub fn verify_chunk(chunk: &FileChunk) -> Result<(), ChunkError> {
    if chunk.payload.len() > MAX_CHUNK_SIZE as usize {
        return Err(ChunkError::Oversized);
    }
    if sha256_hex(&chunk.payload) != chunk.sha256 {
        return Err(ChunkError::HashMismatch);
    }
    Ok(())
}
