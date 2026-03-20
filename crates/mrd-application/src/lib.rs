// mrd-application: Application use case layer
//
// Orchestrates session lifecycle, signaling, transport, and media
// through well-defined use cases. Depends on abstract ports rather
// than concrete implementations.

#![warn(missing_docs)]

/// Application use cases
pub mod usecases {
    pub use super::usecases_start_session::*;
    pub use super::usecases_accept_session::*;
    pub use super::usecases_sync_runtime::*;
}

// Placeholder modules - will be implemented in later tasks
mod usecases_start_session {
    use anyhow::Result;

    /// Start a new controller session
    pub fn start_session() -> Result<()> {
        Ok(())
    }
}

mod usecases_accept_session {
    use anyhow::Result;

    /// Accept an incoming agent session
    pub fn accept_session() -> Result<()> {
        Ok(())
    }
}

mod usecases_sync_runtime {
    use anyhow::Result;

    /// Synchronize runtime state
    pub fn sync_runtime() -> Result<()> {
        Ok(())
    }
}
