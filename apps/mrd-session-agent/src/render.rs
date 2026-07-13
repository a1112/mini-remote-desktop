//! Render-side adapter boundary for Task 25.

use crate::media::MediaResource;
use mrd_agent_ipc::RenderAccessUnit;
use mrd_proto::SessionId;

/// Cumulative counters returned by one live render adapter resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderAdapterMetrics {
    /// Exact resource identity.
    pub resource_id: [u8; 16],
    /// Logical product session.
    pub session_id: SessionId,
    /// Selected decoder backend.
    pub decoder_backend: String,
    /// Encoded units admitted at the IPC boundary.
    pub enqueued_units: u64,
    /// Disposable interframes replaced before decode.
    pub queue_replacements: u64,
    /// Frames emitted by decode.
    pub decoded_frames: u64,
    /// Frames accepted by presentation.
    pub presented_frames: u64,
}

/// Display/render implementation owned by the interactive-session agent.
pub trait RenderAdapter: Send {
    /// Whether this adapter has a viable production render implementation.
    fn is_available(&self) -> bool;
    /// Return cumulative metrics for all live resources.
    fn metrics(&self) -> Vec<RenderAdapterMetrics> {
        Vec::new()
    }
    /// Start rendering for one already-authorized resource.
    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool;
    /// Submit one validated encoded unit to the exact live render resource.
    fn push_access_unit(&mut self, resource: &MediaResource, unit: &RenderAccessUnit) -> bool;
    /// Stop rendering for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
}
