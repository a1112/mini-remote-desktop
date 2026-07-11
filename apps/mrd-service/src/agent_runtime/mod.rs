//! Authenticated interactive-session agent registry and private server.

#![allow(dead_code, unused_imports)]

mod registry;
mod server;
#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows_pipe;

pub use registry::*;
pub use server::*;
#[cfg(not(windows))]
pub use unsupported::*;
#[cfg(windows)]
pub use windows_pipe::*;
