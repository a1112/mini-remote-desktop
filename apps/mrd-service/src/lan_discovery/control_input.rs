use mrd_ipc::{ControlInputEvent, ControlInputLane};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

const REALTIME_ATTEMPTS: usize = 1;
const RELIABLE_ATTEMPTS: usize = 3;
const DEDUPE_WINDOW_MS: u64 = 10_000;
const DEDUPE_CACHE_LIMIT: usize = 4096;

static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct DedupeKey {
    pub(super) source_device_id: String,
    pub(super) session_id: String,
    pub(super) event_id: u64,
}

#[derive(Debug, Clone)]
pub(super) struct AckState {
    pub(super) accepted: bool,
    pub(super) message: Option<String>,
    pub(super) lane: Option<ControlInputLane>,
    pub(super) event_count: u32,
    pub(super) timestamp_ms: u64,
}

pub(super) fn next_event_id() -> u64 {
    EVENT_COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1)
}

pub(super) fn request_attempts(event: &ControlInputEvent) -> usize {
    match event {
        ControlInputEvent::MouseMove { .. } | ControlInputEvent::MouseWheel { .. } => {
            REALTIME_ATTEMPTS
        }
        ControlInputEvent::MouseButton { .. }
        | ControlInputEvent::Key { .. }
        | ControlInputEvent::ReleaseAll => RELIABLE_ATTEMPTS,
    }
}

pub(super) fn prune_recent(cache: &mut HashMap<DedupeKey, AckState>, now: u64) {
    let cutoff = now.saturating_sub(DEDUPE_WINDOW_MS);
    cache.retain(|_, ack| ack.timestamp_ms >= cutoff);
    if cache.len() <= DEDUPE_CACHE_LIMIT {
        return;
    }

    let remove_count = cache.len() - DEDUPE_CACHE_LIMIT;
    let mut oldest = cache
        .iter()
        .map(|(key, ack)| (key.clone(), ack.timestamp_ms))
        .collect::<Vec<_>>();
    oldest.sort_by_key(|(_, timestamp_ms)| *timestamp_ms);
    for (key, _) in oldest.into_iter().take(remove_count) {
        cache.remove(&key);
    }
}
