//! Grant-bound media resource ownership for the session agent.
//!
//! This module is deliberately independent of capture/render APIs.  It owns
//! the process-boundary invariant first: every desktop resource has one exact
//! resource id, session id, display id, and kind, and cleanup cannot silently
//! retarget another resource.  Platform adapters are added only behind this
//! registry in the next Task 25 slice.

use crate::capabilities::AgentCapabilities;
use crate::runtime::AuthorizedCommandExecutor;
use crate::{capture::CaptureAdapter, render::RenderAdapter};
use mrd_agent_ipc::{
    AgentCapability, AgentCommand, AgentEventContext, AuthorizedCommand, CommandOutcome,
    MediaAccessUnit, MediaCodec,
};
use mrd_proto::SessionId;
use std::collections::{HashMap, VecDeque};

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

/// Encoded media crossing the agent's bounded process boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedMediaAccessUnit {
    resource_id: [u8; 16],
    session_id: SessionId,
    sequence: u64,
    timestamp_us: u64,
    keyframe: bool,
    payload: Vec<u8>,
}

impl EncodedMediaAccessUnit {
    /// Creates an encoded unit for one live capture resource.
    pub fn new(
        resource_id: [u8; 16],
        session_id: SessionId,
        sequence: u64,
        timestamp_us: u64,
        keyframe: bool,
        payload: Vec<u8>,
    ) -> Option<Self> {
        (sequence != 0).then_some(Self {
            resource_id,
            session_id,
            sequence,
            timestamp_us,
            keyframe,
            payload,
        })
    }

    /// Exact source resource.
    pub fn resource_id(&self) -> &[u8; 16] {
        &self.resource_id
    }

    /// Product session bound to this unit.
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Monotonic resource-local sequence.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Presentation timestamp in microseconds.
    pub fn timestamp_us(&self) -> u64 {
        self.timestamp_us
    }

    /// Whether the unit is a random-access/keyframe unit.
    pub fn is_keyframe(&self) -> bool {
        self.keyframe
    }

    /// Encoded payload, never raw desktop pixels.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Converts the bounded agent-owned unit into the authenticated IPC form.
    pub fn into_ipc(
        self,
        context: AgentEventContext,
        codec: MediaCodec,
    ) -> Option<MediaAccessUnit> {
        let unit = MediaAccessUnit {
            context,
            resource_id: self.resource_id,
            sequence: self.sequence,
            timestamp_us: self.timestamp_us,
            codec,
            is_keyframe: self.keyframe,
            payload: self.payload,
        };
        unit.is_valid().then_some(unit)
    }
}

/// Bounded queue for encoded units from one exact capture resource.
#[derive(Debug)]
pub struct MediaAccessUnitQueue {
    resource_id: [u8; 16],
    session_id: SessionId,
    capacity: usize,
    max_payload_bytes: usize,
    last_sequence: u64,
    units: VecDeque<EncodedMediaAccessUnit>,
}

impl MediaAccessUnitQueue {
    /// Creates a queue with explicit depth and payload bounds.
    pub fn new(
        resource_id: [u8; 16],
        session_id: SessionId,
        capacity: usize,
        max_payload_bytes: usize,
    ) -> Option<Self> {
        (capacity != 0 && max_payload_bytes != 0).then_some(Self {
            resource_id,
            session_id,
            capacity,
            max_payload_bytes,
            last_sequence: 0,
            units: VecDeque::with_capacity(capacity),
        })
    }

    /// Enqueues a unit, rejecting mismatched identity, replayed sequence, or
    /// oversized payload. A full queue rejects the new unit so backpressure is
    /// explicit and no frame is silently retargeted or reordered.
    pub fn push(&mut self, unit: EncodedMediaAccessUnit) -> bool {
        if unit.resource_id != self.resource_id
            || unit.session_id != self.session_id
            || unit.payload.len() > self.max_payload_bytes
            || unit.sequence <= self.last_sequence
            || self.units.len() >= self.capacity
        {
            return false;
        }
        self.last_sequence = unit.sequence;
        self.units.push_back(unit);
        true
    }

    /// Removes the oldest queued unit.
    pub fn pop(&mut self) -> Option<EncodedMediaAccessUnit> {
        self.units.pop_front()
    }

    /// Current bounded depth.
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether no unit is queued.
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// Bounded registry for desktop-bound media resources.
#[derive(Debug, Default)]
pub struct MediaResourceRegistry {
    resources: HashMap<[u8; 16], MediaResource>,
}

/// Generic authorized media executor that keeps platform work behind ports.
///
/// The executor is intentionally generic so capture/render implementations can
/// be tested with synthetic adapters before binding to DXGI, D3D11, or a
/// process-bound transport. It never creates a resource before the runtime has
/// validated the signed [`AuthorizedCommand`].
pub struct MediaExecutor<C, R> {
    capture: C,
    render: R,
    registry: MediaResourceRegistry,
}

impl<C, R> MediaExecutor<C, R> {
    /// Creates an executor with no live resources.
    pub fn new(capture: C, render: R) -> Self {
        Self {
            capture,
            render,
            registry: MediaResourceRegistry::new(),
        }
    }

    /// Returns the live-resource registry for diagnostics and shutdown tests.
    pub fn registry(&self) -> &MediaResourceRegistry {
        &self.registry
    }
}

impl<C, R> MediaExecutor<C, R>
where
    C: CaptureAdapter,
    R: RenderAdapter,
{
    /// Stops every resource owned by one invalidated session.
    ///
    /// Adapter failure leaves that exact resource registered so a later
    /// shutdown/retry can attempt cleanup again; successful resources are
    /// removed only after the platform adapter acknowledges the stop.
    pub fn stop_session(&mut self, session_id: &SessionId) -> usize {
        let resources = self.registry.resources_for_session(session_id);
        let mut stopped = 0;
        for resource in resources {
            let acknowledged = match resource.kind {
                MediaResourceKind::Capture => self.capture.stop(&resource.resource_id, session_id),
                MediaResourceKind::Render => self.render.stop(&resource.resource_id, session_id),
            };
            if acknowledged
                && self
                    .registry
                    .stop(&resource.resource_id, session_id, resource.kind)
                    == MediaResourceMutation::Stopped
            {
                stopped += 1;
            }
        }
        stopped
    }
}

impl<C, R> AuthorizedCommandExecutor for MediaExecutor<C, R>
where
    C: CaptureAdapter,
    R: RenderAdapter,
{
    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities::from_implemented([AgentCapability::Capture, AgentCapability::Render])
    }

    fn execute(&mut self, authorized: AuthorizedCommand) -> CommandOutcome {
        let session_id = &authorized.grant().claims().session_id;
        match authorized.command() {
            AgentCommand::StartCapture {
                resource_id,
                display_id,
            } => {
                let result = self.registry.start(
                    *resource_id,
                    session_id.clone(),
                    *display_id,
                    MediaResourceKind::Capture,
                );
                if result != MediaResourceMutation::Started {
                    return CommandOutcome::Rejected;
                }
                let resource = self.registry.get(resource_id).expect("started resource");
                if self.capture.start(resource, session_id) {
                    CommandOutcome::Completed
                } else {
                    let _ = self
                        .registry
                        .stop(resource_id, session_id, MediaResourceKind::Capture);
                    CommandOutcome::Failed
                }
            }
            AgentCommand::StartRender {
                resource_id,
                display_id,
            } => {
                let result = self.registry.start(
                    *resource_id,
                    session_id.clone(),
                    *display_id,
                    MediaResourceKind::Render,
                );
                if result != MediaResourceMutation::Started {
                    return CommandOutcome::Rejected;
                }
                let resource = self.registry.get(resource_id).expect("started resource");
                if self.render.start(resource, session_id) {
                    CommandOutcome::Completed
                } else {
                    let _ = self
                        .registry
                        .stop(resource_id, session_id, MediaResourceKind::Render);
                    CommandOutcome::Failed
                }
            }
            AgentCommand::StopCapture { resource_id } => {
                let Some(resource) = self.registry.get(resource_id) else {
                    return CommandOutcome::Rejected;
                };
                if resource.session_id() != session_id
                    || resource.kind() != MediaResourceKind::Capture
                {
                    return CommandOutcome::Rejected;
                }
                if !self.capture.stop(resource_id, session_id) {
                    return CommandOutcome::Failed;
                }
                let _ = self
                    .registry
                    .stop(resource_id, session_id, MediaResourceKind::Capture);
                CommandOutcome::Completed
            }
            AgentCommand::StopRender { resource_id } => {
                let Some(resource) = self.registry.get(resource_id) else {
                    return CommandOutcome::Rejected;
                };
                if resource.session_id() != session_id
                    || resource.kind() != MediaResourceKind::Render
                {
                    return CommandOutcome::Rejected;
                }
                if !self.render.stop(resource_id, session_id) {
                    return CommandOutcome::Failed;
                }
                let _ = self
                    .registry
                    .stop(resource_id, session_id, MediaResourceKind::Render);
                CommandOutcome::Completed
            }
            _ => CommandOutcome::Rejected,
        }
    }
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

    fn resources_for_session(&self, session_id: &SessionId) -> Vec<MediaResource> {
        self.resources
            .values()
            .filter(|resource| resource.session_id == *session_id)
            .cloned()
            .collect()
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

    #[derive(Default)]
    struct FakeCapture {
        stopped: Vec<[u8; 16]>,
        acknowledge_stop: bool,
    }

    impl CaptureAdapter for FakeCapture {
        fn start(&mut self, _resource: &MediaResource, _session_id: &SessionId) -> bool {
            true
        }

        fn stop(&mut self, resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
            self.stopped.push(*resource_id);
            self.acknowledge_stop
        }
    }

    #[derive(Default)]
    struct FakeRender {
        stopped: Vec<[u8; 16]>,
        acknowledge_stop: bool,
    }

    impl RenderAdapter for FakeRender {
        fn start(&mut self, _resource: &MediaResource, _session_id: &SessionId) -> bool {
            true
        }

        fn stop(&mut self, resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
            self.stopped.push(*resource_id);
            self.acknowledge_stop
        }
    }

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

    #[test]
    fn encoded_queue_rejects_cross_session_replay_and_unbounded_growth() {
        let id = [3; 16];
        let owner = session("owner");
        let other = session("other");
        let mut queue = MediaAccessUnitQueue::new(id, owner.clone(), 2, 4).unwrap();
        let unit = |session_id, sequence, payload| {
            EncodedMediaAccessUnit::new(
                id,
                session_id,
                sequence,
                sequence * 10,
                sequence == 1,
                payload,
            )
            .unwrap()
        };
        assert!(queue.push(unit(owner.clone(), 1, vec![1, 2])));
        assert!(!queue.push(unit(owner.clone(), 1, vec![3])));
        assert!(!queue.push(unit(other, 2, vec![4])));
        assert!(queue.push(unit(owner.clone(), 2, vec![5, 6, 7, 8])));
        assert!(!queue.push(unit(owner, 3, vec![9])));
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.pop().unwrap().sequence(), 1);
        assert_eq!(queue.pop().unwrap().sequence(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn encoded_unit_maps_to_authenticated_ipc_without_raw_frame_copy() {
        let unit =
            EncodedMediaAccessUnit::new([3; 16], session("owner"), 1, 42, true, vec![0x01, 0x02])
                .unwrap();
        let ipc = unit
            .into_ipc(
                AgentEventContext {
                    registration_id: [8; 16],
                    registration_epoch: 1,
                    windows_session_id: 7,
                    desktop_epoch: 1,
                    sequence: 1,
                    observed_at_ms: 99,
                },
                MediaCodec::H264,
            )
            .unwrap();
        assert!(ipc.is_valid());
        assert_eq!(ipc.payload, vec![0x01, 0x02]);
        assert_eq!(ipc.resource_id, [3; 16]);
    }

    #[test]
    fn session_invalidation_calls_each_adapter_and_retains_failed_cleanup() {
        let owner = session("owner");
        let capture_id = [4; 16];
        let render_id = [5; 16];
        let mut executor = MediaExecutor::new(
            FakeCapture {
                acknowledge_stop: true,
                ..FakeCapture::default()
            },
            FakeRender {
                acknowledge_stop: false,
                ..FakeRender::default()
            },
        );
        assert_eq!(
            executor
                .registry
                .start(capture_id, owner.clone(), 1, MediaResourceKind::Capture),
            MediaResourceMutation::Started
        );
        assert_eq!(
            executor
                .registry
                .start(render_id, owner.clone(), 2, MediaResourceKind::Render),
            MediaResourceMutation::Started
        );
        assert_eq!(executor.stop_session(&owner), 1);
        assert!(executor.registry.get(&capture_id).is_none());
        assert!(executor.registry.get(&render_id).is_some());
        assert_eq!(executor.capture.stopped, vec![capture_id]);
        assert_eq!(executor.render.stopped, vec![render_id]);
    }
}
