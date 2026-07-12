//! Interactive-session agent application shell.

#![warn(missing_docs)]

/// Authenticated launcher bootstrap and OS-derived process identity.
pub mod bootstrap;
/// Truthful product capabilities implemented by this agent process.
pub mod capabilities;
/// Agent-local consent authority and trusted session bindings.
pub mod consent;
/// Fail-closed cached trusted desktop observations.
#[allow(dead_code)] // Production bootstrap consumes this adapter in Task 24.B3.5.
pub(crate) mod desktop;
/// Authorized input-resource execution and pressed-state cleanup.
pub mod input;
/// Grant-bound desktop capture/render resource ownership.
pub mod media;
/// Native attended-consent adapter and sanitized surface model.
#[allow(dead_code)] // Production bootstrap consumes this adapter in Task 24.B3.5.
pub(crate) mod native_consent;
/// Registration, control-loop, heartbeat, and shutdown runtime.
pub mod runtime;
/// Native Windows attended-consent surface worker.
#[allow(dead_code)] // Production bootstrap consumes this adapter in Task 24.B3.5.
#[cfg(windows)]
pub(crate) mod windows_consent;
/// Native Windows trusted-desktop observation.
#[allow(dead_code)] // Production bootstrap consumes this adapter in Task 24.B3.5.
#[cfg(windows)]
pub(crate) mod windows_desktop;
