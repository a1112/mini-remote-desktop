//! Grant-bound media resource ownership for the session agent.
//!
//! This module is deliberately independent of capture/render APIs.  It owns
//! the process-boundary invariant first: every desktop resource has one exact
//! resource id, session id, display id, and kind, and cleanup cannot silently
//! retarget another resource.  Platform adapters are added only behind this
//! registry in the next Task 25 slice.

use mrd_proto::SessionId;
use std::collections::HashMap;

/// Desktop-bound media operation represented by a live agent resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaResourceKind {
    /// Capture frames from one local display.
    Capture,
    /// Render frames to one local display surface.
    Render,
}

/// Immutable ownership record for one live media resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaResource {
    resource_id: [u8; 16],
    session_id: SessionId,
    display_id: u32,
    kind: MediaResourceKind,
}

impl MediaResource {
    /// Stable resource identity used for exact cleanup.
    pub fn resource_id(&self) -> &[u8; 16] {
        &self.resource_id
    }

    /// Product session that owns this resource.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Local display bound at creation time.
    pub fn display_id(&self) -> u32 {
        self.display_id
    }

    /// Capture or render role.
    pub fn kind(&self) -> MediaResourceKind {
        self.kind
    }
}

/// Fail-closed result of a resource mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaResourceMutation {
    /// Resource was created.
    Started,
    /// Resource was removed.
    Stopped,
    /// The exact resource already exists.
    Duplicate,
    /// No matching resource exists for cleanup.
    Missing,
    /// The requested cleanup belongs to another session or kind.
    Mismatch,
}

/// Bounded registry for desktop-bound media resources.
#[derive(Debug, Default)]
pub struct MediaResourceRegistry {
    resources: HashMap<[u8; 16], MediaResource>,
}

impl MediaResourceRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one resource only when its id is not already live.
    pub fn start(
        &mut self,
        resource_id: [u8; 16],
        session_id: SessionId,
        display_id: u32,
        kind: MediaResourceKind,
    ) -> MediaResourceMutation {
        if self.resources.contains_key(&resource_id) {
            return MediaResourceMutation::Duplicate;
        }
        self.resources.insert(
            resource_id,
            MediaResource {
                resource_id,
                session_id,
                display_id,
                kind,
            },
        );
        MediaResourceMutation::Started
    }

    /// Removes a resource only when its session and role match exactly.
    pub fn stop(
        &mut self,
        resource_id: &[u8; 16],
        session_id: &SessionId,
        kind: MediaResourceKind,
    ) -> MediaResourceMutation {
        let Some(resource) = self.resources.get(resource_id) else {
            return MediaResourceMutation::Missing;
        };
        if resource.session_id != *session_id || resource.kind != kind {
            return MediaResourceMutation::Mismatch;
        }
        self.resources.remove(resource_id);
        MediaResourceMutation::Stopped
    }

    /// Removes every resource owned by one invalidated session.
    pub fn stop_session(&mut self, session_id: &SessionId) -> usize {
        let before = self.resources.len();
        self.resources
            .retain(|_, resource| resource.session_id != *session_id);
        before - self.resources.len()
    }

    /// Returns the exact live resource, if present.
    pub fn get(&self, resource_id: &[u8; 16]) -> Option<&MediaResource> {
        self.resources.get(resource_id)
    }

    /// Number of live resources, used for boundedness assertions.
    pub fn len(&self) -> usize {
        self.resources.len()
    }

    /// Whether no media resource is live.
    pub fn is_empty(&self) -> bool {
        self.resources.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(name: &str) -> SessionId {
        SessionId(name.to_owned())
    }

    #[test]
    fn media_resources_are_bound_to_kind_session_and_display() {
        let mut registry = MediaResourceRegistry::new();
        let id = [1; 16];
        let owner = session("owner");
        assert_eq!(
            registry.start(id, owner.clone(), 7, MediaResourceKind::Capture),
            MediaResourceMutation::Started
        );
        assert_eq!(registry.get(&id).unwrap().display_id(), 7);
        assert_eq!(
            registry.get(&id).unwrap().kind(),
            MediaResourceKind::Capture
        );
        assert_eq!(registry.get(&id).unwrap().session_id(), &owner);
        assert_eq!(
            registry.start(id, owner.clone(), 7, MediaResourceKind::Capture),
            MediaResourceMutation::Duplicate
        );
        assert_eq!(
            registry.stop(&id, &owner, MediaResourceKind::Render),
            MediaResourceMutation::Mismatch
        );
    }

    #[test]
    fn cleanup_cannot_retarget_a_reused_resource_id() {
        let mut registry = MediaResourceRegistry::new();
        let id = [2; 16];
        let first = session("first");
        let second = session("second");
        assert_eq!(
            registry.start(id, first.clone(), 1, MediaResourceKind::Capture),
            MediaResourceMutation::Started
        );
        assert_eq!(
            registry.stop(&id, &second, MediaResourceKind::Capture),
            MediaResourceMutation::Mismatch
        );
        assert_eq!(registry.len(), 1);
        assert_eq!(
            registry.stop(&id, &first, MediaResourceKind::Capture),
            MediaResourceMutation::Stopped
        );
        assert!(registry.is_empty());
        assert_eq!(
            registry.start(id, second.clone(), 2, MediaResourceKind::Render),
            MediaResourceMutation::Started
        );
        assert_eq!(registry.stop_session(&first), 0);
        assert_eq!(registry.stop_session(&second), 1);
    }
}
