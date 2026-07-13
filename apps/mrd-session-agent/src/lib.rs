//! Interactive-session agent application shell.

#![warn(missing_docs)]

/// Authenticated launcher bootstrap and OS-derived process identity.
pub mod bootstrap;
/// Truthful product capabilities implemented by this agent process.
pub mod capabilities;
/// Grant-bound desktop capture adapter boundary.
pub mod capture;
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
/// Grant-bound desktop render adapter boundary.
pub mod render;
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
/// Windows hybrid decode and D3D11 presentation adapter.
#[cfg(windows)]
pub mod windows_render;
