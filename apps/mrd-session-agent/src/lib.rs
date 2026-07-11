//! Interactive-session agent application shell.

#![warn(missing_docs)]

/// Authenticated launcher bootstrap and OS-derived process identity.
pub mod bootstrap;
/// Truthful product capabilities implemented by this agent process.
pub mod capabilities;
/// Authorized input-resource execution and pressed-state cleanup.
pub mod input;
/// Registration, control-loop, heartbeat, and shutdown runtime.
pub mod runtime;
