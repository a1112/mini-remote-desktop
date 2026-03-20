//! Rdesk Legacy Harness
//!
//! This package contains the legacy direct-control runtime that was previously
//! compiled into the Rdesk shell. It exists only for validation and reference
//! during the hard-cut migration to service-owned session state.
//!
//! # Purpose
//!
//! - Preserves old QUIC/WebRTC/realtime runtime code for regression testing
//! - Keeps benchmark helpers that still depend on the old direct mainline
//! - Maintains legacy integration tests until the new IPC architecture is complete
//!
//! # Future
//!
//! Once the migration is validated and complete, this entire package can be
//! removed as a single unit rather than file-by-file cleanup inside the shell.

pub mod app_settings;
pub mod benchmark;
pub mod frame_sink;
pub mod quic_host;
pub mod quic_session;
pub mod quic_transport_harness;
pub mod realtime_client;
pub mod realtime_management;
pub mod realtime_runtime;
pub mod render_host;
pub mod render_surface_catalog;
pub mod session_lifecycle;
pub mod session_runtime;
pub mod webrtc_host;
pub mod webrtc_media;
pub mod webrtc_session;

// Re-export commonly used types from legacy modules
pub use app_settings::*;
pub use benchmark::*;
pub use frame_sink::{DecodedFrameSink, DecodedFrameSnapshot, DEFAULT_SOURCE_ID};
pub use quic_host::{QuicHost, QuicHostSnapshot};
pub use quic_session::{QuicSessionCoordinator, QuicSessionSnapshot};
pub use realtime_runtime::{RealtimeRegistration, RealtimeRuntime};
pub use render_host::{RenderHost, RenderHostSnapshot};
pub use render_surface_catalog::{RenderSurfaceCatalog, RenderSurfaceDescriptor};
pub use session_lifecycle::{SessionLifecycleCoordinator, SessionLifecycleSnapshot, SurfaceSourceBinding};
pub use webrtc_host::{WebrtcHost, WebrtcHostSnapshot};
pub use webrtc_session::{WebrtcSessionCoordinator, WebrtcSessionSnapshot};
