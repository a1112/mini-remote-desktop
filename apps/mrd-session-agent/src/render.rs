//! Render-side adapter boundary for Task 25.

use crate::media::MediaResource;
use mrd_agent_ipc::RenderAccessUnit;
use mrd_proto::SessionId;

/// Display/render implementation owned by the interactive-session agent.
pub trait RenderAdapter: Send {
    /// Whether this adapter has a viable production render implementation.
    fn is_available(&self) -> bool;
    /// Start rendering for one already-authorized resource.
    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool;
    /// Submit one validated encoded unit to the exact live render resource.
    fn push_access_unit(&mut self, resource: &MediaResource, unit: &RenderAccessUnit) -> bool;
    /// Stop rendering for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
}
