//! Bounded worker-to-agent capture publication.

use super::{EncodedMediaAccessUnit, MediaAccessUnitPushResult, MediaAccessUnitQueue};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, TryLockError,
};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureQueueOutcome {
    Accepted,
    Stopped,
    Rejected,
}

/// Retain at most one pending AU while waiting for the bounded queue to drain.
/// Cancellation or terminal rejection ends this publication; no AU is replaced.
pub(crate) fn enqueue_capture_access_unit(
    queues: &Mutex<HashMap<[u8; 16], MediaAccessUnitQueue>>,
    resource_id: &[u8; 16],
    mut pending: EncodedMediaAccessUnit,
    stop: &AtomicBool,
) -> CaptureQueueOutcome {
    loop {
        if stop.load(Ordering::Acquire) {
            return CaptureQueueOutcome::Stopped;
        }
        let mut all = match queues.try_lock() {
            Ok(all) => all,
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            Err(TryLockError::Poisoned(_)) => return CaptureQueueOutcome::Rejected,
        };
        if stop.load(Ordering::Acquire) {
            return CaptureQueueOutcome::Stopped;
        }
        let Some(queue) = all.get_mut(resource_id) else {
            return CaptureQueueOutcome::Rejected;
        };
        let outcome = queue.try_push(pending);
        drop(all);
        match outcome {
            MediaAccessUnitPushResult::Accepted => return CaptureQueueOutcome::Accepted,
            MediaAccessUnitPushResult::Rejected(_) => return CaptureQueueOutcome::Rejected,
            MediaAccessUnitPushResult::Full(unit) => pending = unit,
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    const RESOURCE: [u8; 16] = [3; 16];
    const MAX_BYTES: usize = 128 * 1024;

    fn unit(sequence: u64) -> EncodedMediaAccessUnit {
        EncodedMediaAccessUnit::new(
            RESOURCE,
            SessionId("owner".into()),
            sequence,
            sequence * 10,
            sequence == 1,
            vec![sequence as u8],
        )
        .unwrap()
    }

    fn full_queues() -> Arc<Mutex<HashMap<[u8; 16], MediaAccessUnitQueue>>> {
        let mut queue =
            MediaAccessUnitQueue::new(RESOURCE, SessionId("owner".into()), 3, MAX_BYTES).unwrap();
        for sequence in 1..=3 {
            assert!(queue.push(unit(sequence)));
        }
        Arc::new(Mutex::new(HashMap::from([(RESOURCE, queue)])))
    }

    #[test]
    fn full_queue_waits_for_slow_consumer_without_losing_or_reordering_units() {
        let queues = full_queues();
        let stop = Arc::new(AtomicBool::new(false));
        let (completed, completion) = mpsc::channel();
        let producer_queues = Arc::clone(&queues);
        let producer_stop = Arc::clone(&stop);
        let producer = std::thread::spawn(move || {
            for sequence in 4..=12 {
                let outcome = enqueue_capture_access_unit(
                    &producer_queues,
                    &RESOURCE,
                    unit(sequence),
                    &producer_stop,
                );
                if outcome != CaptureQueueOutcome::Accepted {
                    completed.send(outcome).unwrap();
                    return;
                }
            }
            completed.send(CaptureQueueOutcome::Accepted).unwrap();
        });
        std::thread::sleep(Duration::from_millis(20));
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        let mut result = None;
        while received.len() < 12 && Instant::now() < deadline {
            {
                let mut all = queues.lock().unwrap();
                let queue = all.get_mut(&RESOURCE).unwrap();
                assert!(queue.len() <= 3);
                if let Some(unit) = queue.pop() {
                    received.push(unit.sequence());
                }
            }
            if let Ok(outcome) = completion.try_recv() {
                result = Some(outcome);
                if outcome != CaptureQueueOutcome::Accepted {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        if received.len() != 12 {
            stop.store(true, Ordering::Release);
        }
        producer.join().unwrap();
        let outcome =
            result.unwrap_or_else(|| completion.recv_timeout(Duration::from_secs(1)).unwrap());
        assert_eq!(outcome, CaptureQueueOutcome::Accepted);
        assert_eq!(received, (1..=12).collect::<Vec<_>>());
    }

    #[test]
    fn stop_cancels_a_full_queue_before_any_pending_unit_is_admitted() {
        let queues = full_queues();
        let stop = Arc::new(AtomicBool::new(false));
        let producer_queues = Arc::clone(&queues);
        let producer_stop = Arc::clone(&stop);
        let (completed, completion) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            completed
                .send(enqueue_capture_access_unit(
                    &producer_queues,
                    &RESOURCE,
                    unit(4),
                    &producer_stop,
                ))
                .unwrap();
        });
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, Ordering::Release);
        let outcome = completion.recv_timeout(Duration::from_millis(500));
        producer.join().unwrap();
        assert_eq!(outcome.unwrap(), CaptureQueueOutcome::Stopped);
        assert_eq!(queues.lock().unwrap().get(&RESOURCE).unwrap().len(), 3);
    }

    #[test]
    fn stop_cancels_even_when_another_thread_holds_the_queue_mutex() {
        let queues = full_queues();
        let held = queues.lock().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let producer_queues = Arc::clone(&queues);
        let producer_stop = Arc::clone(&stop);
        let (completed, completion) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            completed
                .send(enqueue_capture_access_unit(
                    &producer_queues,
                    &RESOURCE,
                    unit(4),
                    &producer_stop,
                ))
                .unwrap();
        });
        std::thread::sleep(Duration::from_millis(20));
        stop.store(true, Ordering::Release);
        let outcome = completion.recv_timeout(Duration::from_millis(500));
        drop(held);
        producer.join().unwrap();
        assert_eq!(outcome.unwrap(), CaptureQueueOutcome::Stopped);
    }

    #[test]
    fn absent_or_poisoned_queue_is_a_terminal_rejection() {
        let stop = AtomicBool::new(false);
        let absent = Mutex::new(HashMap::new());
        assert_eq!(
            enqueue_capture_access_unit(&absent, &RESOURCE, unit(1), &stop),
            CaptureQueueOutcome::Rejected,
        );
        let poisoned = full_queues();
        let poison_owner = Arc::clone(&poisoned);
        let _ = std::thread::spawn(move || {
            let _guard = poison_owner.lock().unwrap();
            panic!("synthetic queue poison");
        })
        .join();
        assert_eq!(
            enqueue_capture_access_unit(&poisoned, &RESOURCE, unit(4), &stop),
            CaptureQueueOutcome::Rejected,
        );
    }

    #[test]
    fn typed_full_returns_original_allocation_without_advancing_sequence() {
        let queues = full_queues();
        let mut all = queues.lock().unwrap();
        let queue = all.get_mut(&RESOURCE).unwrap();
        let pending = EncodedMediaAccessUnit::new(
            RESOURCE,
            SessionId("owner".into()),
            4,
            40,
            false,
            vec![0x5a; MAX_BYTES],
        )
        .unwrap();
        let pointer = pending.payload().as_ptr();
        let pending = match queue.try_push(pending) {
            MediaAccessUnitPushResult::Full(unit) => unit,
            _ => panic!("full queue must retain the exact valid unit"),
        };
        assert_eq!(pending.payload().as_ptr(), pointer);
        assert_eq!(pending.payload().len(), MAX_BYTES);
        assert_eq!(pending.sequence(), 4);
        assert_eq!(queue.last_sequence, 3);
        assert_eq!(queue.len(), 3);
        assert_eq!(queue.pop().unwrap().sequence(), 1);
        assert!(matches!(
            queue.try_push(pending),
            MediaAccessUnitPushResult::Accepted
        ));
        assert_eq!(queue.last_sequence, 4);
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn guards_precede_full_and_rejected_units_do_not_advance_sequence() {
        let queues = full_queues();
        let mut all = queues.lock().unwrap();
        let queue = all.get_mut(&RESOURCE).unwrap();
        let wrong_resource = EncodedMediaAccessUnit::new(
            [9; 16],
            SessionId("owner".into()),
            99,
            990,
            false,
            vec![1],
        )
        .unwrap();
        let wrong_session = EncodedMediaAccessUnit::new(
            RESOURCE,
            SessionId("other".into()),
            99,
            990,
            false,
            vec![1],
        )
        .unwrap();
        let oversized = EncodedMediaAccessUnit::new(
            RESOURCE,
            SessionId("owner".into()),
            99,
            990,
            false,
            vec![1; MAX_BYTES + 1],
        )
        .unwrap();
        for rejected in [wrong_resource, wrong_session, oversized, unit(3)] {
            let pointer = rejected.payload().as_ptr();
            let returned = match queue.try_push(rejected) {
                MediaAccessUnitPushResult::Rejected(unit) => unit,
                _ => panic!("invalid unit cannot be retried as full"),
            };
            assert_eq!(returned.payload().as_ptr(), pointer);
            assert_eq!(queue.last_sequence, 3);
            assert_eq!(queue.len(), 3);
        }
    }

    #[test]
    fn rejected_identity_on_full_queue_never_waits_as_capacity_pressure() {
        let queues = full_queues();
        let stop = Arc::new(AtomicBool::new(false));
        let producer_stop = Arc::clone(&stop);
        let (completed, completion) = mpsc::channel();
        let producer = std::thread::spawn(move || {
            let wrong = EncodedMediaAccessUnit::new(
                RESOURCE,
                SessionId("other".into()),
                4,
                40,
                false,
                vec![1],
            )
            .unwrap();
            completed
                .send(enqueue_capture_access_unit(
                    &queues,
                    &RESOURCE,
                    wrong,
                    &producer_stop,
                ))
                .unwrap();
        });
        let outcome = completion.recv_timeout(Duration::from_millis(500));
        stop.store(true, Ordering::Release);
        producer.join().unwrap();
        assert_eq!(outcome.unwrap(), CaptureQueueOutcome::Rejected);
    }

    #[test]
    fn already_stopped_producer_does_not_enqueue() {
        let mut queue =
            MediaAccessUnitQueue::new(RESOURCE, SessionId("owner".into()), 3, MAX_BYTES).unwrap();
        assert!(queue.push(unit(1)));
        let queues = Mutex::new(HashMap::from([(RESOURCE, queue)]));
        let stop = AtomicBool::new(true);
        assert_eq!(
            enqueue_capture_access_unit(&queues, &RESOURCE, unit(2), &stop),
            CaptureQueueOutcome::Stopped,
        );
        assert_eq!(
            queues.lock().unwrap().get(&RESOURCE).unwrap().last_sequence,
            1
        );
        assert_eq!(queues.lock().unwrap().get(&RESOURCE).unwrap().len(), 1);
    }
}
