//! Capture-side adapter boundary for Task 25.

use crate::media::MediaResource;
use mrd_proto::SessionId;

/// Desktop capture implementation owned by the interactive-session agent.
///
/// Implementations must not accept a resource that was not admitted by the
/// grant-bound media registry. They should return `false` on platform failure
/// without exposing native error text to the control plane.
pub trait CaptureAdapter: Send {
    /// Start capture for one already-authorized resource.
    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool;
    /// Stop capture for the exact resource identity.
    fn stop(&mut self, resource_id: &[u8; 16], session_id: &SessionId) -> bool;
}
