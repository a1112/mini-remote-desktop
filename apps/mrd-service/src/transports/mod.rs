use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
};

use anyhow::{bail, Result};
use mrd_application::ports::{
    TransportEnvelope, TransportLane, TransportRouteKind, TransportRouteSnapshot,
    TransportSendOutcome,
};
use mrd_proto::SessionId;
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};

pub mod memory;
pub mod quic;
pub mod webrtc;

/// Bounded queue configuration shared by concrete transport adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportMuxConfig {
    /// Maximum pending envelopes in each FIFO lane.
    pub lane_capacity: usize,
    /// Maximum accepted application payload size.
    pub max_payload_len: usize,
    /// Maximum queued payload bytes for video envelopes in each direction.
    pub video_byte_capacity: usize,
    /// Maximum queued payload bytes for reliable control in each direction.
    pub control_reliable_byte_capacity: usize,
    /// Maximum queued payload bytes for realtime control in each direction.
    pub control_realtime_byte_capacity: usize,
    /// Maximum queued payload bytes for bulk transfer in each direction.
    pub bulk_byte_capacity: usize,
}

impl Default for TransportMuxConfig {
    fn default() -> Self {
        Self {
            lane_capacity: 64,
            max_payload_len: 4 * 1024 * 1024,
            video_byte_capacity: 16 * 1024 * 1024,
            control_reliable_byte_capacity: 4 * 1024 * 1024,
            control_realtime_byte_capacity: 64 * 1024,
            bulk_byte_capacity: 16 * 1024 * 1024,
        }
    }
}

impl TransportMuxConfig {
    /// Small deterministic queues used by adapter conformance tests.
    pub fn test() -> Self {
        Self {
            lane_capacity: 4,
            max_payload_len: 1024 * 1024,
            video_byte_capacity: 2 * 1024 * 1024,
            control_reliable_byte_capacity: 256 * 1024,
            control_realtime_byte_capacity: 64 * 1024,
            bulk_byte_capacity: 256 * 1024,
        }
    }

    fn byte_capacity(self, lane: TransportLane) -> usize {
        match lane {
            TransportLane::Video => self.video_byte_capacity,
            TransportLane::ControlReliable => self.control_reliable_byte_capacity,
            TransportLane::ControlRealtime => self.control_realtime_byte_capacity,
            TransportLane::Bulk => self.bulk_byte_capacity,
        }
        .max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueuePush {
    Enqueued,
    ReplacedStale,
    DroppedOldest,
    Backpressured,
    DiscardedStale,
}

#[derive(Debug)]
struct LaneQueueState {
    values: VecDeque<TransportEnvelope>,
    newest_realtime_sequence: Option<u64>,
    buffered_bytes: usize,
}

#[derive(Debug)]
struct LaneQueue {
    state: Mutex<LaneQueueState>,
    available: Notify,
    space: Notify,
    capacity: usize,
    byte_capacity: usize,
}

impl LaneQueue {
    fn new(capacity: usize, byte_capacity: usize) -> Self {
        Self {
            state: Mutex::new(LaneQueueState {
                values: VecDeque::with_capacity(capacity),
                newest_realtime_sequence: None,
                buffered_bytes: 0,
            }),
            available: Notify::new(),
            space: Notify::new(),
            capacity: capacity.max(1),
            byte_capacity: byte_capacity.max(1),
        }
    }

    async fn try_push(&self, envelope: TransportEnvelope) -> QueuePush {
        let mut state = self.state.lock().await;
        let payload_len = envelope.payload.len();
        if payload_len > self.byte_capacity {
            return QueuePush::Backpressured;
        }
        let result = match envelope.lane {
            TransportLane::ControlRealtime if !state.values.is_empty() => {
                state.values.clear();
                state.buffered_bytes = payload_len;
                state.values.push_back(envelope);
                QueuePush::ReplacedStale
            }
            TransportLane::Video
                if state.values.len() >= self.capacity
                    || state.buffered_bytes.saturating_add(payload_len) > self.byte_capacity =>
            {
                while !state.values.is_empty()
                    && (state.values.len() >= self.capacity
                        || state.buffered_bytes.saturating_add(payload_len) > self.byte_capacity)
                {
                    if let Some(dropped) = state.values.pop_front() {
                        state.buffered_bytes =
                            state.buffered_bytes.saturating_sub(dropped.payload.len());
                    }
                }
                state.buffered_bytes = state.buffered_bytes.saturating_add(payload_len);
                state.values.push_back(envelope);
                QueuePush::DroppedOldest
            }
            TransportLane::ControlReliable | TransportLane::Bulk
                if state.values.len() >= self.capacity
                    || state.buffered_bytes.saturating_add(payload_len) > self.byte_capacity =>
            {
                QueuePush::Backpressured
            }
            _ => {
                state.buffered_bytes = state.buffered_bytes.saturating_add(payload_len);
                state.values.push_back(envelope);
                QueuePush::Enqueued
            }
        };
        drop(state);
        if result != QueuePush::Backpressured {
            self.available.notify_one();
        }
        result
    }

    async fn push_wait(
        &self,
        envelope: TransportEnvelope,
        closed: &AtomicBool,
        reject_stale_realtime: bool,
    ) -> QueuePush {
        let lane = envelope.lane;
        let mut envelope = Some(envelope);
        loop {
            if closed.load(Ordering::Acquire) {
                return QueuePush::Backpressured;
            }
            let space = self.space.notified();
            tokio::pin!(space);
            space.as_mut().enable();
            let mut state = self.state.lock().await;
            let sequence = envelope.as_ref().expect("pending lane envelope").sequence;
            let payload_len = envelope
                .as_ref()
                .expect("pending lane envelope")
                .payload
                .len();
            if payload_len > self.byte_capacity {
                return QueuePush::Backpressured;
            }
            let result = match lane {
                TransportLane::ControlRealtime
                    if reject_stale_realtime
                        && state
                            .newest_realtime_sequence
                            .is_some_and(|newest| sequence <= newest) =>
                {
                    QueuePush::DiscardedStale
                }
                TransportLane::ControlRealtime if !state.values.is_empty() => {
                    state.values.clear();
                    state.buffered_bytes = payload_len;
                    state
                        .values
                        .push_back(envelope.take().expect("pending lane envelope"));
                    state.newest_realtime_sequence = Some(sequence);
                    QueuePush::ReplacedStale
                }
                TransportLane::Video
                    if state.values.len() >= self.capacity
                        || state.buffered_bytes.saturating_add(payload_len)
                            > self.byte_capacity =>
                {
                    while !state.values.is_empty()
                        && (state.values.len() >= self.capacity
                            || state.buffered_bytes.saturating_add(payload_len)
                                > self.byte_capacity)
                    {
                        if let Some(dropped) = state.values.pop_front() {
                            state.buffered_bytes =
                                state.buffered_bytes.saturating_sub(dropped.payload.len());
                        }
                    }
                    state.buffered_bytes = state.buffered_bytes.saturating_add(payload_len);
                    state
                        .values
                        .push_back(envelope.take().expect("pending lane envelope"));
                    QueuePush::DroppedOldest
                }
                TransportLane::ControlReliable | TransportLane::Bulk
                    if state.values.len() >= self.capacity
                        || state.buffered_bytes.saturating_add(payload_len)
                            > self.byte_capacity =>
                {
                    QueuePush::Backpressured
                }
                _ => {
                    state.buffered_bytes = state.buffered_bytes.saturating_add(payload_len);
                    state
                        .values
                        .push_back(envelope.take().expect("pending lane envelope"));
                    if lane == TransportLane::ControlRealtime {
                        state.newest_realtime_sequence = Some(sequence);
                    }
                    QueuePush::Enqueued
                }
            };
            drop(state);
            if result != QueuePush::Backpressured {
                if result != QueuePush::DiscardedStale {
                    self.available.notify_one();
                }
                return result;
            }
            space.as_mut().await;
        }
    }

    async fn pop(&self, closed: &AtomicBool) -> Option<TransportEnvelope> {
        loop {
            let available = self.available.notified();
            tokio::pin!(available);
            available.as_mut().enable();
            let mut state = self.state.lock().await;
            if let Some(envelope) = state.values.pop_front() {
                state.buffered_bytes = state.buffered_bytes.saturating_sub(envelope.payload.len());
                drop(state);
                self.space.notify_one();
                return Some(envelope);
            }
            drop(state);
            if closed.load(Ordering::Acquire) {
                return None;
            }
            available.as_mut().await;
        }
    }

    fn wake_all(&self) {
        self.available.notify_waiters();
        self.space.notify_waiters();
    }
}

#[derive(Debug)]
struct LaneQueues {
    video: LaneQueue,
    control_reliable: LaneQueue,
    control_realtime: LaneQueue,
    bulk: LaneQueue,
}

impl LaneQueues {
    fn new(config: TransportMuxConfig) -> Self {
        Self {
            video: LaneQueue::new(
                config.lane_capacity,
                config.byte_capacity(TransportLane::Video),
            ),
            control_reliable: LaneQueue::new(
                config.lane_capacity,
                config.byte_capacity(TransportLane::ControlReliable),
            ),
            control_realtime: LaneQueue::new(
                1,
                config.byte_capacity(TransportLane::ControlRealtime),
            ),
            bulk: LaneQueue::new(
                config.lane_capacity,
                config.byte_capacity(TransportLane::Bulk),
            ),
        }
    }

    fn get(&self, lane: TransportLane) -> &LaneQueue {
        match lane {
            TransportLane::Video => &self.video,
            TransportLane::ControlReliable => &self.control_reliable,
            TransportLane::ControlRealtime => &self.control_realtime,
            TransportLane::Bulk => &self.bulk,
        }
    }

    fn wake_all(&self) {
        for lane in TransportLane::ALL {
            self.get(lane).wake_all();
        }
    }
}

#[derive(Debug)]
pub(crate) struct SessionMuxCore {
    session_id: SessionId,
    config: TransportMuxConfig,
    outbound: LaneQueues,
    inbound: LaneQueues,
    snapshot: StdMutex<TransportRouteSnapshot>,
    closed: AtomicBool,
    tasks: StdMutex<Vec<JoinHandle<()>>>,
}

impl SessionMuxCore {
    pub(crate) fn new(
        session_id: SessionId,
        config: TransportMuxConfig,
        kind: TransportRouteKind,
        local_endpoint: impl Into<String>,
        peer_endpoint: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            session_id: session_id.clone(),
            config,
            outbound: LaneQueues::new(config),
            inbound: LaneQueues::new(config),
            snapshot: StdMutex::new(TransportRouteSnapshot::new(
                session_id,
                kind,
                local_endpoint,
                peer_endpoint,
            )),
            closed: AtomicBool::new(false),
            tasks: StdMutex::new(Vec::new()),
        })
    }

    pub(crate) async fn submit(&self, envelope: TransportEnvelope) -> Result<TransportSendOutcome> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(TransportSendOutcome::Closed);
        }
        if envelope.session_id != self.session_id {
            bail!(
                "transport envelope session {:?} does not match mux session {:?}",
                envelope.session_id,
                self.session_id
            );
        }
        if envelope.payload.len() > self.config.max_payload_len {
            bail!(
                "transport payload exceeds {} byte limit",
                self.config.max_payload_len
            );
        }
        if envelope.session_id.0.len() > u16::MAX as usize {
            bail!("transport session identifier exceeds wire limit");
        }
        if (envelope.lane == TransportLane::Video) != envelope.video.is_some() {
            bail!("video metadata must be present exactly for the video lane");
        }
        if envelope.video.as_ref().is_some_and(|metadata| {
            metadata.codec.is_empty() || metadata.codec.len() > u8::MAX as usize
        }) {
            bail!("transport video codec identifier is empty or exceeds wire limit");
        }

        let lane = envelope.lane;
        let result = self.outbound.get(lane).try_push(envelope).await;
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("transport route snapshot lock poisoned");
        let stats = snapshot.lane_mut(lane);
        let outcome = match result {
            QueuePush::Enqueued => {
                stats.sent += 1;
                TransportSendOutcome::Enqueued
            }
            QueuePush::ReplacedStale => {
                stats.sent += 1;
                stats.stale_replaced += 1;
                TransportSendOutcome::ReplacedStale
            }
            QueuePush::DroppedOldest => {
                stats.sent += 1;
                stats.dropped += 1;
                TransportSendOutcome::Enqueued
            }
            QueuePush::Backpressured => {
                stats.backpressured += 1;
                TransportSendOutcome::Backpressured
            }
            QueuePush::DiscardedStale => {
                unreachable!("outbound submission does not reject realtime sequence regressions")
            }
        };
        Ok(outcome)
    }

    pub(crate) fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub(crate) fn configured_lane_capacity(&self) -> usize {
        self.config.lane_capacity.max(1)
    }

    pub(crate) async fn next_outbound(&self, lane: TransportLane) -> Option<TransportEnvelope> {
        self.outbound.get(lane).pop(&self.closed).await
    }

    pub(crate) async fn deliver(&self, envelope: TransportEnvelope) -> Result<()> {
        if envelope.session_id != self.session_id {
            bail!("received envelope for a different session");
        }
        let lane = envelope.lane;
        let result = self
            .inbound
            .get(lane)
            .push_wait(envelope, &self.closed, true)
            .await;
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("transport route snapshot lock poisoned");
        let stats = snapshot.lane_mut(lane);
        stats.received += 1;
        match result {
            QueuePush::ReplacedStale => stats.stale_replaced += 1,
            QueuePush::DroppedOldest => stats.dropped += 1,
            QueuePush::Backpressured => stats.backpressured += 1,
            QueuePush::DiscardedStale => {
                stats.dropped += 1;
                stats.stale_discarded += 1;
            }
            QueuePush::Enqueued => {}
        }
        drop(snapshot);
        if result == QueuePush::Backpressured {
            bail!("received {lane:?} envelope exceeds the configured lane byte budget");
        }
        Ok(())
    }

    pub(crate) async fn recv(&self, lane: TransportLane) -> Result<Option<TransportEnvelope>> {
        Ok(self.inbound.get(lane).pop(&self.closed).await)
    }

    pub(crate) async fn snapshot(&self) -> TransportRouteSnapshot {
        self.snapshot
            .lock()
            .expect("transport route snapshot lock poisoned")
            .clone()
    }

    pub(crate) fn record_adapter_drops(&self, lane: TransportLane, count: u64) {
        if count == 0 {
            return;
        }
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("transport route snapshot lock poisoned");
        let stats = snapshot.lane_mut(lane);
        stats.dropped = stats.dropped.saturating_add(count);
    }

    pub(crate) async fn update_route(
        &self,
        kind: TransportRouteKind,
        local_endpoint: String,
        peer_endpoint: String,
        local_candidate_kind: Option<String>,
        remote_candidate_kind: Option<String>,
    ) {
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("transport route snapshot lock poisoned");
        snapshot.kind = kind;
        snapshot.local_endpoint = local_endpoint;
        snapshot.peer_endpoint = peer_endpoint;
        snapshot.local_candidate_kind = local_candidate_kind;
        snapshot.remote_candidate_kind = remote_candidate_kind;
    }

    pub(crate) fn register_task(&self, task: JoinHandle<()>) {
        let mut tasks = self
            .tasks
            .lock()
            .expect("transport task registry lock poisoned");
        if self.closed.load(Ordering::Acquire) {
            task.abort();
            return;
        }
        tasks.push(task);
    }

    fn mark_closed(&self, last_error: Option<String>) {
        let newly_closed = !self.closed.swap(true, Ordering::AcqRel);
        if newly_closed {
            self.outbound.wake_all();
            self.inbound.wake_all();
        }
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("transport route snapshot lock poisoned");
        snapshot.closed = true;
        if newly_closed && snapshot.last_error.is_none() {
            snapshot.last_error = last_error;
        }
    }

    fn abort_tasks(&self) {
        let mut tasks = self
            .tasks
            .lock()
            .expect("transport task registry lock poisoned");
        for task in tasks.iter() {
            task.abort();
        }
        tasks.clear();
    }

    pub(crate) fn terminate_now(&self, last_error: Option<String>) {
        self.mark_closed(last_error);
        self.abort_tasks();
    }

    pub(crate) async fn fail(&self, reason: impl Into<String>) {
        self.terminate_now(Some(reason.into()));
    }

    pub(crate) async fn close(&self) {
        self.terminate_now(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn realtime(session_id: &SessionId, sequence: u64) -> TransportEnvelope {
        TransportEnvelope {
            session_id: session_id.clone(),
            lane: TransportLane::ControlRealtime,
            sequence,
            payload: vec![sequence as u8],
            video: None,
        }
    }

    #[tokio::test]
    async fn inbound_realtime_never_regresses_to_an_older_sequence() {
        let session_id = SessionId("realtime-order".into());
        let core = SessionMuxCore::new(
            session_id.clone(),
            TransportMuxConfig::test(),
            TransportRouteKind::TestMemory,
            "memory:left",
            "memory:right",
        );

        core.deliver(realtime(&session_id, 12))
            .await
            .expect("deliver newest realtime value");
        core.deliver(realtime(&session_id, 11))
            .await
            .expect("discard reordered stale realtime value");

        assert_eq!(
            core.recv(TransportLane::ControlRealtime)
                .await
                .expect("receive latest")
                .expect("latest value"),
            realtime(&session_id, 12)
        );
        assert_eq!(
            core.snapshot()
                .await
                .lane(TransportLane::ControlRealtime)
                .stale_discarded,
            1
        );
        core.close().await;
    }

    #[tokio::test]
    async fn transport_failure_closes_route_and_wakes_blocked_receivers() {
        let session_id = SessionId("remote-failure".into());
        let core = SessionMuxCore::new(
            session_id.clone(),
            TransportMuxConfig::test(),
            TransportRouteKind::TestMemory,
            "memory:left",
            "memory:right",
        );
        let receiver_core = Arc::clone(&core);
        let receiver = tokio::spawn(async move {
            receiver_core
                .recv(TransportLane::Video)
                .await
                .expect("blocked receive")
        });
        tokio::task::yield_now().await;

        core.fail("peer closed abruptly").await;

        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), receiver)
                .await
                .expect("failure did not wake receiver")
                .expect("receiver task panicked")
                .is_none()
        );
        let snapshot = core.snapshot().await;
        assert!(snapshot.closed);
        assert_eq!(snapshot.last_error.as_deref(), Some("peer closed abruptly"));
        assert_eq!(
            core.submit(TransportEnvelope {
                session_id,
                lane: TransportLane::ControlReliable,
                sequence: 1,
                payload: vec![1],
                video: None,
            })
            .await
            .expect("closed send outcome"),
            TransportSendOutcome::Closed
        );
        core.close().await;
    }

    #[tokio::test]
    async fn bulk_queue_backpressures_on_bytes_before_envelope_count() {
        let session_id = SessionId("bulk-byte-budget".into());
        let core = SessionMuxCore::new(
            session_id.clone(),
            TransportMuxConfig::test(),
            TransportRouteKind::TestMemory,
            "memory:left",
            "memory:right",
        );
        let bulk = |sequence| TransportEnvelope {
            session_id: session_id.clone(),
            lane: TransportLane::Bulk,
            sequence,
            payload: vec![0x5a; 160 * 1024],
            video: None,
        };

        assert_eq!(
            core.submit(bulk(1)).await.expect("first bulk submission"),
            TransportSendOutcome::Enqueued
        );
        assert_eq!(
            core.submit(bulk(2)).await.expect("second bulk submission"),
            TransportSendOutcome::Backpressured
        );
        assert_eq!(
            core.snapshot()
                .await
                .lane(TransportLane::Bulk)
                .backpressured,
            1
        );
        core.close().await;
    }

    #[tokio::test]
    async fn adapter_drop_evidence_is_monotonic() {
        let core = SessionMuxCore::new(
            SessionId("adapter-drops".into()),
            TransportMuxConfig::test(),
            TransportRouteKind::WebRtcDirect,
            "webrtc:local",
            "webrtc:remote",
        );

        core.record_adapter_drops(TransportLane::Video, 2);
        core.record_adapter_drops(TransportLane::Video, 3);
        core.close().await;

        assert_eq!(core.snapshot().await.lane(TransportLane::Video).dropped, 5);
    }
}
