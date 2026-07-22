//! Fail-closed marker for platforms without the Task 23 Windows pipe boundary.

use thiserror::Error;

/// Platform agent-pipe availability failure.
#[derive(Debug, Error)]
#[error("authenticated interactive-session agent pipes are unsupported on this platform")]
pub struct UnsupportedAgentPipe;
