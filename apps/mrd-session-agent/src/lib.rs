//! Interactive-session agent application shell.

#![warn(missing_docs)]

/// Authenticated launcher bootstrap and OS-derived process identity.
pub mod bootstrap;
/// Truthful product capabilities implemented by this agent process.
pub mod capabilities;
/// Agent-local consent authority and trusted session bindings.
pub mod consent;
/// Fail-closed cached trusted desktop observations.
#[allow(dead_code)] // Consumed by the native watcher added in Task 24.B3.2.
pub(crate) mod desktop;
/// Authorized input-resource execution and pressed-state cleanup.
pub mod input;
/// Registration, control-loop, heartbeat, and shutdown runtime.
pub mod runtime;
