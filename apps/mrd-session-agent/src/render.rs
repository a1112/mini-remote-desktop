//! Render-side adapter boundary for Task 25.

use crate::media::MediaResource;
use mrd_proto::SessionId;

/// Display/render implementation owned by the interactive-session agent.
pub trait RenderAdapter: Send {
    /// Start rendering for one already-authorized resource.
    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool;
    /// Stop rendering for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
}
