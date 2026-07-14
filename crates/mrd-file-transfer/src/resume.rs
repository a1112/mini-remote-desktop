use crate::{chunking::{sha256_hex, FileChunk}, protocol::FileTransferManifest};
use std::path::Path;
use thiserror::Error;

/// Resume-state validation failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResumeError {
    /// The chunk did not start at the next contiguous offset.
    #[error("chunk offset is not contiguous")]
    NonContiguous,
    /// The chunk extends beyond the manifest size or exceeds its chunk bound.
    #[error("chunk exceeds manifest bounds")]
    OutOfBounds,
    /// The transfer identifier or file hash differs from the state.
    #[error("resume manifest does not match transfer state")]
    ManifestMismatch,
    /// Final file hash did not match.
    #[error("final file SHA-256 mismatch")]
    FinalHashMismatch,
    /// Atomic commit failed.
    #[error("atomic commit failed: {0}")]
    Commit(String),
}

/// Contiguous resume cursor bound to one manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeState {
    transfer_id: String,
    file_sha256: String,
    expected_offset: u64,
    size_bytes: u64,
}

impl ResumeState {
    /// Start a new empty cursor.
    pub fn new(manifest: &FileTransferManifest) -> Self {
        Self {
            transfer_id: manifest.transfer_id.clone(),
            file_sha256: manifest.file_sha256.clone(),
            expected_offset: 0,
            size_bytes: manifest.size_bytes,
        }
    }

    /// Return the next byte offset required by the receiver.
    pub fn next_offset(&self) -> u64 { self.expected_offset }

    /// Return bytes accepted so far.
    pub fn completed_bytes(&self) -> u64 { self.expected_offset }

    /// Accept one verified, contiguous chunk.
    pub fn accept_chunk(&mut self, chunk: &FileChunk, manifest: &FileTransferManifest) -> Result<u64, ResumeError> {
        if self.transfer_id != manifest.transfer_id
            || self.file_sha256 != manifest.file_sha256
            || chunk.transfer_id != manifest.transfer_id
        {
            return Err(ResumeError::ManifestMismatch);
        }
        crate::chunking::verify_chunk(chunk).map_err(|_| ResumeError::OutOfBounds)?;
        self.accept_offset(chunk.offset, chunk.payload.len() as u64, manifest)
    }

    /// Accept a contiguous offset/length pair after transport-level hash check.
    pub fn accept_offset(&mut self, offset: u64, len: u64, manifest: &FileTransferManifest) -> Result<u64, ResumeError> {
        if self.transfer_id != manifest.transfer_id || self.file_sha256 != manifest.file_sha256 {
            return Err(ResumeError::ManifestMismatch);
        }
        if offset != self.expected_offset
            || len > manifest.chunk_size as u64
            || offset.saturating_add(len) > self.size_bytes
        {
            return Err(if offset != self.expected_offset {
                ResumeError::NonContiguous
            } else {
                ResumeError::OutOfBounds
            });
        }
        self.expected_offset = self.expected_offset.saturating_add(len);
        Ok(self.expected_offset)
    }
}

/// Verify a temporary file and atomically move it to the destination.
pub fn atomic_commit(temp: &Path, destination: &Path, expected_sha256: &str) -> Result<(), ResumeError> {
    let bytes = std::fs::read(temp).map_err(|error| ResumeError::Commit(error.to_string()))?;
    if sha256_hex(&bytes) != expected_sha256 {
        return Err(ResumeError::FinalHashMismatch);
    }
    let parent = destination
        .parent()
        .ok_or_else(|| ResumeError::Commit("destination has no parent".into()))?;
    std::fs::create_dir_all(parent).map_err(|error| ResumeError::Commit(error.to_string()))?;
    #[cfg(windows)]
    if destination.exists() {
        std::fs::remove_file(destination).map_err(|error| ResumeError::Commit(error.to_string()))?;
    }
    std::fs::rename(temp, destination).map_err(|error| ResumeError::Commit(error.to_string()))
}
