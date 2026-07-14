//! Authenticated, bounded remote file-transfer primitives.
//!
//! The crate intentionally contains no local-copy fallback.  A caller must
//! select [`protocol::TransferProvider::Remote`] and provide a session-bound
//! file-bulk transport; unsupported providers are represented as errors.

#![warn(missing_docs)]

pub mod chunking;
pub mod paths;
pub mod protocol;
pub mod resume;

pub use protocol::{FileBulkMessage, FileDirection, FileTransferManifest, TransferProvider};
