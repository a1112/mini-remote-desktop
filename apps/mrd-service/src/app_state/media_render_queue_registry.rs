use mrd_proto::SessionId;
use mrd_render::RenderFrame;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, PartialEq, Eq)]
pub enum MediaRenderQueueEnqueue {
    Start(MediaRenderFrame),
    Queued { replaced: bool, depth: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRenderFrame {
    Decoded(RenderFrame),
    #[cfg(target_os = "macos")]
    H264AccessUnit {
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    },
    #[cfg(target_os = "macos")]
    HevcAccessUnit {
        width: usize,
        height: usize,
        timestamp_us: u64,
        payload: bytes::Bytes,
    },
}

#[derive(Default)]
struct MediaRenderQueueState {
    running: bool,
    pending: VecDeque<MediaRenderFrame>,
    last_enqueue_at: Option<Instant>,
    last_present_at: Option<Instant>,
}

#[derive(Default)]
pub struct MediaRenderQueueRegistry {
    queues: HashMap<SessionId, MediaRenderQueueState>,
}

impl MediaRenderQueueRegistry {
    pub fn enqueue_latest(
        &mut self,
        session_id: SessionId,
        frame: MediaRenderFrame,
    ) -> MediaRenderQueueEnqueue {
        self.enqueue_bounded(session_id, frame, 1)
    }

    pub fn enqueue_bounded(
        &mut self,
        session_id: SessionId,
        frame: MediaRenderFrame,
        max_pending_frames: usize,
    ) -> MediaRenderQueueEnqueue {
        let state = self.queues.entry(session_id).or_default();
        if !state.running {
            state.running = true;
            return MediaRenderQueueEnqueue::Start(frame);
        }

        let max_pending_frames = max_pending_frames.max(1);
        let replaced = if state.pending.len() >= max_pending_frames {
            state.pending.pop_front();
            true
        } else {
            false
        };
        state.pending.push_back(frame);
        MediaRenderQueueEnqueue::Queued {
            replaced,
            depth: state.pending.len(),
        }
    }

    pub fn take_next_or_finish(&mut self, session_id: &SessionId) -> Option<MediaRenderFrame> {
        let state = self.queues.get_mut(session_id)?;

        if let Some(frame) = state.pending.pop_front() {
            return Some(frame);
        }

        state.running = false;
        None
    }

    pub fn take_latest_or_finish(
        &mut self,
        session_id: &SessionId,
    ) -> (Option<MediaRenderFrame>, usize) {
        let Some(state) = self.queues.get_mut(session_id) else {
            return (None, 0);
        };

        let Some(frame) = state.pending.pop_back() else {
            state.running = false;
            return (None, 0);
        };
        let dropped = state.pending.len();
        state.pending.clear();
        (Some(frame), dropped)
    }

    pub fn pending_depth(&self, session_id: &SessionId) -> usize {
        self.queues
            .get(session_id)
            .map_or(0, |state| state.pending.len())
    }

    pub fn pacing_delay(&self, session_id: &SessionId, fps: u32, now: Instant) -> Duration {
        let Some(last_present_at) = self
            .queues
            .get(session_id)
            .and_then(|state| state.last_present_at)
        else {
            return Duration::ZERO;
        };
        let Some(frame_interval) = render_frame_interval(fps) else {
            return Duration::ZERO;
        };
        let elapsed = now
            .checked_duration_since(last_present_at)
            .unwrap_or(Duration::ZERO);
        frame_interval.saturating_sub(elapsed)
    }

    pub fn record_enqueued(&mut self, session_id: &SessionId, at: Instant) -> Option<Duration> {
        let state = self.queues.entry(session_id.clone()).or_default();
        let gap = state
            .last_enqueue_at
            .and_then(|last| at.checked_duration_since(last));
        state.last_enqueue_at = Some(at);
        gap
    }

    pub fn record_presented(&mut self, session_id: &SessionId, at: Instant) -> Option<Duration> {
        let state = self.queues.entry(session_id.clone()).or_default();
        let gap = state
            .last_present_at
            .and_then(|last| at.checked_duration_since(last));
        state.last_present_at = Some(at);
        gap
    }

    pub fn remove(&mut self, session_id: &SessionId) {
        self.queues.remove(session_id);
    }
}

fn render_frame_interval(fps: u32) -> Option<Duration> {
    if fps == 0 {
        return None;
    }

    Some(Duration::from_secs_f64(1.0 / f64::from(fps)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;
    use mrd_render::RenderFrame;

    #[test]
    fn bounded_queue_clamps_zero_capacity_to_one_pending_frame() {
        let mut registry = MediaRenderQueueRegistry::default();
        let session_id = SessionId("render-queue-zero-capacity".to_string());
        let first = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![1, 2, 3]));
        let second = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![4, 5, 6]));
        let third = MediaRenderFrame::Decoded(RenderFrame::from_rgb24(1, 1, vec![7, 8, 9]));

        match registry.enqueue_bounded(session_id.clone(), first.clone(), 0) {
            MediaRenderQueueEnqueue::Start(frame) => assert_eq!(frame, first),
            other => panic!("expected render worker start, got {other:?}"),
        }
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), second, 0),
            MediaRenderQueueEnqueue::Queued {
                replaced: false,
                depth: 1
            }
        );
        assert_eq!(
            registry.enqueue_bounded(session_id.clone(), third.clone(), 0),
            MediaRenderQueueEnqueue::Queued {
                replaced: true,
                depth: 1
            }
        );

        assert_eq!(registry.take_next_or_finish(&session_id), Some(third));
        assert_eq!(registry.take_next_or_finish(&session_id), None);
    }
}
