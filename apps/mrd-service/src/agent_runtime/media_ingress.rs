use mrd_agent_ipc::MediaAccessUnit;
use std::collections::{HashMap, VecDeque};

/// Bounded hand-off from authenticated agent IPC to the service media loop.
#[derive(Debug)]
pub struct AgentMediaIngress {
    capacity: usize,
    queue: VecDeque<MediaAccessUnit>,
    dropped: u64,
    last_sequences: HashMap<String, u64>,
    admitted_resources: HashMap<String, AdmittedMediaResource>,
    require_admitted_resource: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct AdmittedMediaResource {
    pub resource_id: [u8; 16],
    pub registration_id: [u8; 16],
    pub registration_epoch: u64,
    pub windows_session_id: u32,
    pub desktop_epoch: u64,
}

impl AdmittedMediaResource {
    fn admits(&self, unit: &MediaAccessUnit) -> bool {
        unit.resource_id == self.resource_id
            && unit.context.registration_id == self.registration_id
            && unit.context.registration_epoch == self.registration_epoch
            && unit.context.windows_session_id == self.windows_session_id
            && unit.context.desktop_epoch == self.desktop_epoch
    }
}

impl AgentMediaIngress {
    /// Creates a queue with an explicit backpressure limit.
    pub fn new(capacity: usize) -> Option<Self> {
        (capacity > 0).then_some(Self {
            capacity,
            queue: VecDeque::with_capacity(capacity),
            dropped: 0,
            last_sequences: HashMap::new(),
            admitted_resources: HashMap::new(),
            require_admitted_resource: false,
        })
    }

    /// Enqueues a validated unit, rejecting invalid or over-capacity input.
    pub fn push(&mut self, unit: MediaAccessUnit) -> bool {
        if !unit.is_valid()
            || (self.require_admitted_resource
                && !self
                    .admitted_resources
                    .get(&unit.session_id)
                    .is_some_and(|admission| admission.admits(&unit)))
            || unit.sequence
                <= self
                    .last_sequences
                    .get(&unit.session_id)
                    .copied()
                    .unwrap_or(0)
            || self.queue.len() >= self.capacity
        {
            self.dropped = self.dropped.saturating_add(1);
            return false;
        }
        self.last_sequences
            .insert(unit.session_id.clone(), unit.sequence);
        self.queue.push_back(unit);
        true
    }

    /// Removes the oldest unit for the LAN sender.
    pub fn pop(&mut self) -> Option<MediaAccessUnit> {
        self.queue.pop_front()
    }

    /// Drains at most `limit` units for one sender scheduling turn.
    pub fn drain(&mut self, limit: usize) -> Vec<MediaAccessUnit> {
        let count = limit.min(self.queue.len());
        self.queue.drain(..count).collect()
    }

    /// Drains only units belonging to one logical session.
    pub fn drain_session(&mut self, session_id: &str, limit: usize) -> Vec<MediaAccessUnit> {
        let mut selected = Vec::new();
        let mut retained = VecDeque::with_capacity(self.queue.len());
        while let Some(unit) = self.queue.pop_front() {
            if unit.session_id == session_id && selected.len() < limit {
                selected.push(unit);
            } else {
                retained.push_back(unit);
            }
        }
        self.queue = retained;
        selected
    }

    /// Current queue depth.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue contains no media access units.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Number of queued units for one logical session.
    pub fn session_len(&self, session_id: &str) -> usize {
        self.queue
            .iter()
            .filter(|unit| unit.session_id == session_id)
            .count()
    }

    /// Whether authenticated media ownership has been established for a session.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.last_sequences.contains_key(session_id)
    }

    /// Pins the Agent path before its first encoded frame arrives.
    pub fn reserve_session(&mut self, session_id: &str) {
        self.last_sequences
            .entry(session_id.to_owned())
            .or_insert(0);
    }

    /// Production product capture admits one exact signed resource before any
    /// frame. Late frames after stop and frames from another desktop are denied.
    pub fn reserve_resource(&mut self, session_id: &str, resource: AdmittedMediaResource) {
        self.require_admitted_resource = true;
        self.admitted_resources
            .insert(session_id.to_owned(), resource);
        self.queue.retain(|unit| {
            self.admitted_resources
                .get(&unit.session_id)
                .is_some_and(|admission| admission.admits(unit))
        });
        self.last_sequences
            .retain(|session, _| self.admitted_resources.contains_key(session));
        self.last_sequences.insert(session_id.to_owned(), 0);
    }

    /// Removes both buffered media and sequence ownership on session cleanup.
    pub fn remove_session(&mut self, session_id: &str) {
        self.queue.retain(|unit| unit.session_id != session_id);
        self.last_sequences.remove(session_id);
        self.admitted_resources.remove(session_id);
    }

    /// Number of rejected units since creation.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Clears queued units when the owning agent/session is invalidated.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.last_sequences.clear();
        self.admitted_resources.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_agent_ipc::{AgentEventContext, MediaCodec};

    fn unit(sequence: u64) -> MediaAccessUnit {
        MediaAccessUnit {
            context: AgentEventContext {
                registration_id: [1; 16],
                registration_epoch: 1,
                windows_session_id: 1,
                desktop_epoch: 1,
                sequence,
                observed_at_ms: sequence,
            },
            resource_id: [2; 16],
            session_id: "session-1".to_string(),
            sequence,
            timestamp_us: sequence,
            codec: MediaCodec::H264,
            is_keyframe: sequence == 1,
            source_bounds: None,
            payload: vec![1, 2],
        }
    }

    #[test]
    fn ingress_applies_validation_and_backpressure() {
        let mut ingress = AgentMediaIngress::new(1).unwrap();
        assert!(ingress.push(unit(1)));
        assert!(!ingress.push(unit(2)));
        assert_eq!(ingress.dropped(), 1);
        assert_eq!(ingress.pop().unwrap().sequence, 1);
        assert!(!ingress.push(unit(1)));
        assert_eq!(ingress.dropped(), 2);
        assert!(!ingress.push({
            let mut invalid = unit(3);
            invalid.payload.clear();
            invalid
        }));
        assert_eq!(ingress.dropped(), 3);
        ingress.clear();
        assert_eq!(ingress.len(), 0);
        assert!(ingress.push(unit(1)));
    }

    #[test]
    fn session_drain_does_not_cross_route_units() {
        let mut ingress = AgentMediaIngress::new(4).unwrap();
        let mut second = unit(2);
        second.session_id = "session-2".to_string();
        second.sequence = 1;
        assert!(ingress.push(unit(1)));
        assert!(ingress.push(second));
        let first = ingress.drain_session("session-1", 8);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].session_id, "session-1");
        assert_eq!(ingress.pop().unwrap().session_id, "session-2");
    }

    #[test]
    fn session_len_counts_only_target_session() {
        let mut ingress = AgentMediaIngress::new(4).unwrap();
        let mut second = unit(2);
        second.session_id = "session-2".to_string();
        assert!(ingress.push(unit(1)));
        assert!(ingress.push(second));
        assert_eq!(ingress.session_len("session-1"), 1);
        assert_eq!(ingress.session_len("session-2"), 1);
    }

    #[test]
    fn reserved_agent_session_never_requires_a_first_frame_to_establish_ownership() {
        let mut ingress = AgentMediaIngress::new(4).unwrap();
        ingress.reserve_session("session-1");
        assert!(ingress.has_session("session-1"));
        assert!(ingress.drain_session("session-1", 8).is_empty());
        assert!(ingress.push(unit(1)));
        let mut other = unit(1);
        other.session_id = "session-2".into();
        assert!(ingress.push(other));
        ingress.remove_session("session-1");
        assert!(!ingress.has_session("session-1"));
        assert_eq!(ingress.session_len("session-1"), 0);
        assert_eq!(ingress.session_len("session-2"), 1);
        ingress.reserve_session("session-1");
        assert!(ingress.push(unit(1)));
    }

    #[test]
    fn approved_resource_rejects_other_desktops_and_late_frames_after_stop() {
        let mut ingress = AgentMediaIngress::new(4).unwrap();
        let first = unit(1);
        ingress.reserve_resource(
            "session-1",
            AdmittedMediaResource {
                resource_id: first.resource_id,
                registration_id: first.context.registration_id,
                registration_epoch: first.context.registration_epoch,
                windows_session_id: first.context.windows_session_id,
                desktop_epoch: first.context.desktop_epoch,
            },
        );
        let mutations: [fn(&mut MediaAccessUnit); 5] = [
            |unit: &mut MediaAccessUnit| unit.resource_id = [9; 16],
            |unit: &mut MediaAccessUnit| unit.context.registration_id = [9; 16],
            |unit: &mut MediaAccessUnit| unit.context.registration_epoch += 1,
            |unit: &mut MediaAccessUnit| unit.context.windows_session_id += 1,
            |unit: &mut MediaAccessUnit| unit.context.desktop_epoch += 1,
        ];
        for mutate in mutations {
            let mut forged = first.clone();
            mutate(&mut forged);
            assert!(!ingress.push(forged));
        }
        assert!(ingress.push(first.clone()));
        ingress.remove_session("session-1");
        assert!(!ingress.push(first));
        assert!(ingress.is_empty());
        assert!(!ingress.has_session("session-1"));
    }
}
