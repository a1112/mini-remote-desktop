use super::*;
use std::cell::Cell;

struct NoRender;
impl RenderAdapter for NoRender {
    fn is_available(&self) -> bool {
        false
    }
    fn start(&mut self, _: &MediaResource, _: &SessionId) -> bool {
        panic!("render is outside this fixture")
    }
    fn push_access_unit(&mut self, _: &MediaResource, _: &RenderAccessUnit) -> bool {
        panic!("render is outside this fixture")
    }
    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        panic!("render is outside this fixture")
    }
}

const ID: [u8; 16] = [81; 16];

fn unit(id: [u8; 16], owner: &SessionId, sequence: u64) -> EncodedMediaAccessUnit {
    EncodedMediaAccessUnit::new(
        id,
        owner.clone(),
        sequence,
        sequence * 8_333,
        sequence == 1,
        vec![0, 0, 0, 1, 0x65, sequence as u8],
    )
    .unwrap()
    .with_source_bounds(CaptureSourceBounds {
        left: 0,
        top: 0,
        width: 2560,
        height: 1440,
    })
    .unwrap()
}

struct QueueCapture {
    queues: HashMap<[u8; 16], MediaAccessUnitQueue>,
}

impl CaptureAdapter for QueueCapture {
    fn is_available(&self) -> bool {
        true
    }
    fn start(&mut self, _: &MediaResource, _: &SessionId) -> bool {
        true
    }
    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        true
    }
    fn poll_encoded(&mut self, resource: &[u8; 16]) -> Option<EncodedMediaAccessUnit> {
        self.queues.get_mut(resource)?.pop()
    }
}

fn queue_executor(owner: &SessionId, ids: &[[u8; 16]]) -> MediaExecutor<QueueCapture, NoRender> {
    let queues = ids
        .iter()
        .map(|id| {
            (
                *id,
                MediaAccessUnitQueue::new(*id, owner.clone(), 3, 128 * 1024).unwrap(),
            )
        })
        .collect();
    let mut executor = MediaExecutor::new(QueueCapture { queues }, NoRender);
    for id in ids {
        executor
            .registry
            .start(*id, owner.clone(), 0, MediaResourceKind::Capture, None);
    }
    executor
}

#[test]
fn capture_batch_drains_three_queued_units_fifo_and_then_empty() {
    let owner = SessionId("batch-owner".into());
    let mut executor = queue_executor(&owner, &[ID]);
    let expected: Vec<_> = (1..=3).map(|sequence| unit(ID, &owner, sequence)).collect();
    for value in &expected {
        assert!(executor
            .capture
            .queues
            .get_mut(&ID)
            .unwrap()
            .push(value.clone()));
    }
    let drained = executor.capture_access_units().unwrap();
    assert_eq!(
        drained, expected,
        "one pump must retain every queued unit and FIFO metadata"
    );
    assert!(executor.capture_access_units().unwrap().is_empty());
}

#[test]
fn capture_batch_is_bounded_per_owned_resource_and_never_polls_unowned_queue() {
    let owner = SessionId("batch-owner".into());
    let second = [82; 16];
    let unowned = [83; 16];
    let mut executor = queue_executor(&owner, &[ID, second]);
    // This adapter can hold more than the production queue to prove the pump's
    // bound independently. Real capture queues remain depth three.
    for id in [ID, second, unowned] {
        let mut queue = MediaAccessUnitQueue::new(id, owner.clone(), 4, 128 * 1024).unwrap();
        for sequence in 1..=4 {
            assert!(queue.push(unit(id, &owner, sequence)));
        }
        executor.capture.queues.insert(id, queue);
    }
    let drained = executor.capture_access_units().unwrap();
    assert_eq!(drained.len(), 6);
    for id in [ID, second] {
        let sequences: Vec<_> = drained
            .iter()
            .filter(|value| value.resource_id() == &id)
            .map(EncodedMediaAccessUnit::sequence)
            .collect();
        assert_eq!(sequences, [1, 2, 3]);
        assert_eq!(executor.capture.queues[&id].len(), 1);
    }
    assert_eq!(executor.capture.queues[&unowned].len(), 4);
}

struct ScriptedCapture {
    values: VecDeque<EncodedMediaAccessUnit>,
    polls: Cell<usize>,
    fail_after_polls: Option<usize>,
}

impl CaptureAdapter for ScriptedCapture {
    fn is_available(&self) -> bool {
        true
    }
    fn start(&mut self, _: &MediaResource, _: &SessionId) -> bool {
        true
    }
    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        true
    }
    fn is_resource_running(&self, _: &[u8; 16]) -> bool {
        self.fail_after_polls
            .is_none_or(|limit| self.polls.get() < limit)
    }
    fn poll_encoded(&mut self, _: &[u8; 16]) -> Option<EncodedMediaAccessUnit> {
        self.polls.set(self.polls.get() + 1);
        self.values.pop_front()
    }
}

fn scripted_executor(
    owner: &SessionId,
    values: Vec<EncodedMediaAccessUnit>,
    fail_after_polls: Option<usize>,
) -> MediaExecutor<ScriptedCapture, NoRender> {
    let capture = ScriptedCapture {
        values: values.into(),
        polls: Cell::new(0),
        fail_after_polls,
    };
    let mut executor = MediaExecutor::new(capture, NoRender);
    executor
        .registry
        .start(ID, owner.clone(), 0, MediaResourceKind::Capture, None);
    executor
}

#[test]
fn capture_batch_rejects_later_cross_resource_or_session_unit_without_partial_result() {
    let owner = SessionId("batch-owner".into());
    for wrong in [
        unit([82; 16], &owner, 2),
        unit(ID, &SessionId("other-owner".into()), 2),
    ] {
        let mut executor = scripted_executor(
            &owner,
            vec![unit(ID, &owner, 1), wrong, unit(ID, &owner, 3)],
            None,
        );
        assert!(executor.capture_access_units().is_err());
        assert_eq!(executor.capture.polls.get(), 2);
        assert_eq!(executor.capture.values.len(), 1);
    }
}

#[test]
fn capture_batch_rechecks_worker_health_before_the_next_unit() {
    let owner = SessionId("batch-owner".into());
    let mut executor = scripted_executor(
        &owner,
        vec![unit(ID, &owner, 1), unit(ID, &owner, 2)],
        Some(1),
    );
    assert!(executor.capture_access_units().is_err());
    assert_eq!(executor.capture.polls.get(), 1);
    assert_eq!(executor.capture.values.len(), 1);
}

#[test]
fn capture_batch_handles_120_units_at_64_pump_ticks_without_capacity_growth() {
    let owner = SessionId("batch-owner".into());
    let mut executor = queue_executor(&owner, &[ID]);
    let mut admitted = 0_u64;
    let mut pending = None;
    let mut capacity_waits = 0;
    let mut consumed = Vec::new();
    // Exactly one synthetic second: 120 producer deadlines, 64 consumer ticks.
    // A full result retains one pending AU, matching the bounded producer.
    for tick in 1..=64 {
        let due = tick * 120 / 64;
        while admitted < due {
            let candidate = pending
                .take()
                .unwrap_or_else(|| unit(ID, &owner, admitted + 1));
            match executor
                .capture
                .queues
                .get_mut(&ID)
                .unwrap()
                .try_push(candidate)
            {
                MediaAccessUnitPushResult::Accepted => admitted += 1,
                MediaAccessUnitPushResult::Full(value) => {
                    pending = Some(value);
                    capacity_waits += 1;
                    break;
                }
                MediaAccessUnitPushResult::Rejected(_) => panic!("valid producer unit rejected"),
            }
        }
        consumed.extend(
            executor
                .capture_access_units()
                .unwrap()
                .into_iter()
                .map(|value| value.sequence()),
        );
        assert!(executor.capture.queues[&ID].len() <= 3);
    }
    assert_eq!(
        consumed,
        (1..=120).collect::<Vec<_>>(),
        "one-unit pump throttles the nominal 120 producer"
    );
    assert_eq!(admitted, 120);
    assert_eq!(capacity_waits, 0);
    assert!(pending.is_none());
    assert!(executor.capture_access_units().unwrap().is_empty());
}
