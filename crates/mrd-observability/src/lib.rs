use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use mrd_proto::SessionId;
use serde::{Deserialize, Serialize};

const DEFAULT_EVENT_CAPACITY: usize = 256;
const DEFAULT_WINDOW: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StageId {
    CaptureWait,
    CaptureCopy,
    EncodeTotal,
    SendPacketize,
    SendWrite,
    NetworkIngress,
    H264Assemble,
    DecodeTotal,
    FrameSinkIngest,
    RenderUpload,
    RenderSubmitWait,
    RenderExecute,
    RenderPrepareWait,
    RenderSharedResource,
    RenderDrawPresent,
    RenderPresent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaProbeEvent {
    pub session_id: SessionId,
    pub stream_id: String,
    pub stage: StageId,
    pub duration_us: u64,
    pub bytes: usize,
    pub is_keyframe: bool,
}

impl MediaProbeEvent {
    pub fn new(
        session_id: SessionId,
        stream_id: String,
        stage: StageId,
        duration_us: u64,
        bytes: usize,
        is_keyframe: bool,
    ) -> Self {
        Self {
            session_id,
            stream_id,
            stage,
            duration_us,
            bytes,
            is_keyframe,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StageStatsSnapshot {
    pub count: u64,
    pub bytes: u64,
    pub avg_ms: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
}

impl StageStatsSnapshot {
    pub fn from_durations_ms(durations_ms: &[f64], bytes: u64) -> Self {
        if durations_ms.is_empty() {
            return Self {
                count: 0,
                bytes,
                avg_ms: None,
                p50_ms: None,
                p95_ms: None,
                p99_ms: None,
                max_ms: None,
                jitter_ms: None,
            };
        }

        let count = durations_ms.len() as u64;
        let sum = durations_ms.iter().sum::<f64>();
        let avg = sum / durations_ms.len() as f64;
        let mut sorted = durations_ms.to_vec();
        sorted.sort_by(f64::total_cmp);

        let variance = durations_ms
            .iter()
            .map(|value| {
                let delta = value - avg;
                delta * delta
            })
            .sum::<f64>()
            / durations_ms.len() as f64;

        Self {
            count,
            bytes,
            avg_ms: Some(avg),
            p50_ms: Some(percentile(&sorted, 0.50)),
            p95_ms: Some(percentile(&sorted, 0.95)),
            p99_ms: Some(percentile(&sorted, 0.99)),
            max_ms: sorted.last().copied(),
            jitter_ms: Some(variance.sqrt()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineProbeSnapshot {
    pub session_id: SessionId,
    pub stream_id: String,
    pub backend: Option<String>,
    pub codec: Option<String>,
    pub transport: Option<String>,
    pub fps: f64,
    pub bitrate_kbps: f64,
    pub dropped_frames: u64,
    pub keyframes: u64,
    pub counters: Vec<(String, u64)>,
    pub stages: Vec<(StageId, StageStatsSnapshot)>,
}

impl PipelineProbeSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        session_id: SessionId,
        stream_id: String,
        backend: Option<String>,
        codec: Option<String>,
        transport: Option<String>,
        fps: f64,
        bitrate_kbps: f64,
        dropped_frames: u64,
        keyframes: u64,
        mut counters: Vec<(String, u64)>,
        mut stages: Vec<(StageId, StageStatsSnapshot)>,
    ) -> Self {
        counters.sort_by(|left, right| left.0.cmp(&right.0));
        stages.sort_by_key(|entry| entry.0);
        Self {
            session_id,
            stream_id,
            backend,
            codec,
            transport,
            fps,
            bitrate_kbps,
            dropped_frames,
            keyframes,
            counters,
            stages,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComponentKind {
    Capture,
    Encode,
    Decode,
    Transport,
    Render,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueStatsSnapshot {
    pub mean: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ComponentResult {
    pub component: ComponentKind,
    pub backend: String,
    pub case_name: String,
    pub sample_count: u64,
    pub duration_sec: f64,
    pub success_count: u64,
    pub failure_count: u64,
    pub throughput_fps: f64,
    pub latency_ms: StageStatsSnapshot,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_bytes: Option<usize>,
    pub success_ratio: Option<f64>,
    pub zero_copy_hit_ratio: Option<f64>,
    pub access_unit_bytes: Option<ValueStatsSnapshot>,
    pub written_bytes: Option<ValueStatsSnapshot>,
    pub packets_per_sample: Option<ValueStatsSnapshot>,
    pub keyframe_ratio: Option<f64>,
    pub decoded_frame_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PipelineComparisonResult {
    pub pipeline: String,
    pub codec: String,
    pub memory_path: String,
    #[serde(default = "default_pipeline_transport")]
    pub transport: String,
    pub frames: u64,
    pub encoded_units: u64,
    pub decoded_frames: u64,
    pub encode_failures: u64,
    pub decode_failures: u64,
    pub avg_capture_time_ms: Option<f64>,
    pub avg_encode_time_ms: Option<f64>,
    pub avg_decode_time_ms: Option<f64>,
    pub avg_render_time_ms: Option<f64>,
    pub avg_present_time_ms: Option<f64>,
    #[serde(default)]
    pub avg_transport_time_ms: Option<f64>,
    #[serde(default)]
    pub avg_total_time_ms: Option<f64>,
    #[serde(default)]
    pub avg_fps: Option<f64>,
    pub total_bitstream_bytes: u64,
}

fn default_pipeline_transport() -> String {
    "loopback".into()
}

impl PipelineComparisonResult {
    pub fn new(pipeline: impl Into<String>, codec: impl Into<String>) -> Self {
        Self {
            pipeline: pipeline.into(),
            codec: codec.into(),
            memory_path: "unknown".into(),
            transport: default_pipeline_transport(),
            frames: 0,
            encoded_units: 0,
            decoded_frames: 0,
            encode_failures: 0,
            decode_failures: 0,
            avg_capture_time_ms: None,
            avg_encode_time_ms: None,
            avg_decode_time_ms: None,
            avg_render_time_ms: None,
            avg_present_time_ms: None,
            avg_transport_time_ms: None,
            avg_total_time_ms: None,
            avg_fps: None,
            total_bitstream_bytes: 0,
        }
    }

    pub fn with_memory_path(mut self, memory_path: impl Into<String>) -> Self {
        self.memory_path = memory_path.into();
        self
    }

    pub fn with_transport(mut self, transport: impl Into<String>) -> Self {
        self.transport = transport.into();
        self
    }

    pub fn with_counts(
        mut self,
        frames: u64,
        encoded_units: u64,
        decoded_frames: u64,
        encode_failures: u64,
        decode_failures: u64,
    ) -> Self {
        self.frames = frames;
        self.encoded_units = encoded_units;
        self.decoded_frames = decoded_frames;
        self.encode_failures = encode_failures;
        self.decode_failures = decode_failures;
        self
    }

    pub fn with_average_stage_ms(
        mut self,
        capture: Option<f64>,
        encode: Option<f64>,
        decode: Option<f64>,
        render: Option<f64>,
        present: Option<f64>,
    ) -> Self {
        self.avg_capture_time_ms = capture;
        self.avg_encode_time_ms = encode;
        self.avg_decode_time_ms = decode;
        self.avg_render_time_ms = render;
        self.avg_present_time_ms = present;
        self
    }

    pub fn with_transport_stage_ms(mut self, transport: Option<f64>) -> Self {
        self.avg_transport_time_ms = transport;
        self
    }

    pub fn with_total_time_ms(mut self, total: Option<f64>) -> Self {
        self.avg_total_time_ms = total;
        self
    }

    pub fn with_avg_fps(mut self, fps: Option<f64>) -> Self {
        self.avg_fps = fps;
        self
    }

    pub fn with_total_bitstream_bytes(mut self, total_bitstream_bytes: u64) -> Self {
        self.total_bitstream_bytes = total_bitstream_bytes;
        self
    }
}

impl ValueStatsSnapshot {
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self {
                mean: None,
                p50: None,
                p95: None,
                p99: None,
                max: None,
            };
        }

        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        Self {
            mean: Some(mean),
            p50: Some(percentile(&sorted, 0.50)),
            p95: Some(percentile(&sorted, 0.95)),
            p99: Some(percentile(&sorted, 0.99)),
            max: sorted.last().copied(),
        }
    }
}

impl ComponentResult {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        component: ComponentKind,
        backend: impl Into<String>,
        case_name: impl Into<String>,
        duration_sec: f64,
        success_count: u64,
        failure_count: u64,
        latency_samples_ms: &[f64],
        width: Option<u32>,
        height: Option<u32>,
        frame_bytes: Option<usize>,
        zero_copy_hit_ratio: Option<f64>,
        access_unit_sizes: Option<&[usize]>,
        written_bytes: Option<&[usize]>,
        packets_per_sample: Option<&[usize]>,
        keyframe_ratio: Option<f64>,
        decoded_frame_bytes: Option<usize>,
    ) -> Self {
        let sample_count = (success_count + failure_count).max(latency_samples_ms.len() as u64);
        let throughput_fps = if duration_sec > 0.0 {
            success_count as f64 / duration_sec
        } else {
            0.0
        };
        let access_unit_bytes = access_unit_sizes.map(|sizes| {
            let values = sizes.iter().map(|value| *value as f64).collect::<Vec<_>>();
            ValueStatsSnapshot::from_values(&values)
        });
        let written_bytes = written_bytes.map(|sizes| {
            let values = sizes.iter().map(|value| *value as f64).collect::<Vec<_>>();
            ValueStatsSnapshot::from_values(&values)
        });
        let packets_per_sample = packets_per_sample.map(|sizes| {
            let values = sizes.iter().map(|value| *value as f64).collect::<Vec<_>>();
            ValueStatsSnapshot::from_values(&values)
        });
        let success_ratio = if sample_count > 0 {
            Some(success_count as f64 / sample_count as f64)
        } else {
            None
        };

        Self {
            component,
            backend: backend.into(),
            case_name: case_name.into(),
            sample_count,
            duration_sec,
            success_count,
            failure_count,
            throughput_fps,
            latency_ms: StageStatsSnapshot::from_durations_ms(latency_samples_ms, 0),
            width,
            height,
            frame_bytes,
            success_ratio,
            zero_copy_hit_ratio,
            access_unit_bytes,
            written_bytes,
            packets_per_sample,
            keyframe_ratio,
            decoded_frame_bytes,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProbeRegistry {
    sessions: ProbeSessionMap,
}

type ProbeSessionKey = (SessionId, String);
type ProbeStateHandle = Arc<Mutex<ProbeState>>;
type ProbeSessionMap = Arc<Mutex<HashMap<ProbeSessionKey, ProbeStateHandle>>>;

impl ProbeRegistry {
    pub fn session_handle(
        &self,
        session_id: SessionId,
        stream_id: impl Into<String>,
    ) -> ProbeSessionHandle {
        let stream_id = stream_id.into();
        let key = (session_id.clone(), stream_id.clone());
        let mut sessions = self.sessions.lock().expect("lock probe registry");
        let state = sessions
            .entry(key)
            .or_insert_with(|| {
                Arc::new(Mutex::new(ProbeState::new(
                    session_id.clone(),
                    stream_id.clone(),
                    DEFAULT_EVENT_CAPACITY,
                    DEFAULT_WINDOW,
                )))
            })
            .clone();
        ProbeSessionHandle { state }
    }

    pub fn snapshot(
        &self,
        session_id: &SessionId,
        stream_id: &str,
    ) -> Option<PipelineProbeSnapshot> {
        let sessions = self.sessions.lock().expect("lock probe registry");
        sessions
            .get(&(session_id.clone(), stream_id.to_string()))
            .map(|state| state.lock().expect("lock probe state").snapshot())
    }

    pub fn recent_events(
        &self,
        session_id: &SessionId,
        stream_id: &str,
        limit: usize,
    ) -> Vec<MediaProbeEvent> {
        let sessions = self.sessions.lock().expect("lock probe registry");
        sessions
            .get(&(session_id.clone(), stream_id.to_string()))
            .map(|state| state.lock().expect("lock probe state").recent_events(limit))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ProbeSessionHandle {
    state: Arc<Mutex<ProbeState>>,
}

impl ProbeSessionHandle {
    pub fn set_backend(&self, backend: impl Into<String>) {
        self.state.lock().expect("lock probe state").backend = Some(backend.into());
    }

    pub fn set_codec(&self, codec: impl Into<String>) {
        self.state.lock().expect("lock probe state").codec = Some(codec.into());
    }

    pub fn set_transport(&self, transport: impl Into<String>) {
        self.state.lock().expect("lock probe state").transport = Some(transport.into());
    }

    pub fn increment_dropped_frames(&self, count: u64) {
        self.state.lock().expect("lock probe state").dropped_frames += count;
    }

    pub fn increment_counter(&self, name: impl Into<String>, count: u64) {
        let mut state = self.state.lock().expect("lock probe state");
        let name = name.into();
        *state.counters.entry(name).or_default() += count;
    }

    pub fn set_counter(&self, name: impl Into<String>, value: u64) {
        self.state
            .lock()
            .expect("lock probe state")
            .counters
            .insert(name.into(), value);
    }

    pub fn record_stage(
        &self,
        stage: StageId,
        duration: Duration,
        bytes: usize,
        is_keyframe: bool,
    ) {
        self.state.lock().expect("lock probe state").record_stage(
            stage,
            duration,
            bytes,
            is_keyframe,
        );
    }

    pub fn snapshot(&self) -> PipelineProbeSnapshot {
        self.state.lock().expect("lock probe state").snapshot()
    }

    pub fn recent_events(&self, limit: usize) -> Vec<MediaProbeEvent> {
        self.state
            .lock()
            .expect("lock probe state")
            .recent_events(limit)
    }
}

#[derive(Debug, Clone)]
struct RecordedSample {
    recorded_at: Instant,
    event: MediaProbeEvent,
}

#[derive(Debug)]
struct ProbeState {
    session_id: SessionId,
    stream_id: String,
    backend: Option<String>,
    codec: Option<String>,
    transport: Option<String>,
    dropped_frames: u64,
    keyframes: u64,
    counters: HashMap<String, u64>,
    event_capacity: usize,
    window: Duration,
    events: VecDeque<RecordedSample>,
}

impl ProbeState {
    fn new(
        session_id: SessionId,
        stream_id: String,
        event_capacity: usize,
        window: Duration,
    ) -> Self {
        Self {
            session_id,
            stream_id,
            backend: None,
            codec: None,
            transport: None,
            dropped_frames: 0,
            keyframes: 0,
            counters: HashMap::new(),
            event_capacity,
            window,
            events: VecDeque::with_capacity(event_capacity),
        }
    }

    fn record_stage(
        &mut self,
        stage: StageId,
        duration: Duration,
        bytes: usize,
        is_keyframe: bool,
    ) {
        let now = Instant::now();
        self.prune(now);
        if self.events.len() == self.event_capacity {
            self.events.pop_front();
        }
        if is_keyframe {
            self.keyframes += 1;
        }
        self.events.push_back(RecordedSample {
            recorded_at: now,
            event: MediaProbeEvent::new(
                self.session_id.clone(),
                self.stream_id.clone(),
                stage,
                duration.as_micros().min(u64::MAX as u128) as u64,
                bytes,
                is_keyframe,
            ),
        });
    }

    fn snapshot(&mut self) -> PipelineProbeSnapshot {
        let now = Instant::now();
        self.prune(now);

        let mut per_stage: HashMap<StageId, Vec<f64>> = HashMap::new();
        let mut bytes_by_stage: HashMap<StageId, u64> = HashMap::new();
        let mut total_bytes = 0_u64;

        for sample in &self.events {
            total_bytes += sample.event.bytes as u64;
            per_stage
                .entry(sample.event.stage)
                .or_default()
                .push(sample.event.duration_us as f64 / 1000.0);
            *bytes_by_stage.entry(sample.event.stage).or_default() += sample.event.bytes as u64;
        }

        let seconds = self.window.as_secs_f64();
        let frame_count = self
            .events
            .iter()
            .filter(|sample| sample.event.stage == StageId::FrameSinkIngest)
            .count() as f64;
        let fps = if seconds > 0.0 {
            frame_count / seconds
        } else {
            0.0
        };
        let media_bytes = [
            StageId::EncodeTotal,
            StageId::NetworkIngress,
            StageId::SendWrite,
        ]
        .into_iter()
        .find_map(|stage| bytes_by_stage.get(&stage).copied())
        .unwrap_or(total_bytes);
        let bitrate_kbps = if seconds > 0.0 {
            (media_bytes as f64 * 8.0) / 1000.0 / seconds
        } else {
            0.0
        };

        let mut stages = per_stage
            .into_iter()
            .map(|(stage, durations_ms)| {
                let bytes = bytes_by_stage.get(&stage).copied().unwrap_or_default();
                (
                    stage,
                    StageStatsSnapshot::from_durations_ms(&durations_ms, bytes),
                )
            })
            .collect::<Vec<_>>();
        stages.sort_by_key(|entry| entry.0);

        PipelineProbeSnapshot::from_parts(
            self.session_id.clone(),
            self.stream_id.clone(),
            self.backend.clone(),
            self.codec.clone(),
            self.transport.clone(),
            fps,
            bitrate_kbps,
            self.dropped_frames,
            self.keyframes,
            self.counters.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            stages,
        )
    }

    fn recent_events(&mut self, limit: usize) -> Vec<MediaProbeEvent> {
        self.prune(Instant::now());
        self.events
            .iter()
            .rev()
            .take(limit)
            .map(|sample| sample.event.clone())
            .collect()
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.events.front() {
            if now.duration_since(front.recorded_at) <= self.window {
                break;
            }
            self.events.pop_front();
        }
    }
}

fn percentile(sorted: &[f64], ratio: f64) -> f64 {
    let last_index = sorted.len().saturating_sub(1);
    let index = ((last_index as f64) * ratio).round() as usize;
    sorted[index.min(last_index)]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mrd_proto::SessionId;

    use super::{
        ComponentKind, ComponentResult, MediaProbeEvent, PipelineComparisonResult,
        PipelineProbeSnapshot, ProbeRegistry, StageId, StageStatsSnapshot, ValueStatsSnapshot,
    };

    #[test]
    fn stage_stats_report_percentiles_for_recent_samples() {
        let samples = [2.0_f64, 4.0, 6.0, 8.0];
        let stats = StageStatsSnapshot::from_durations_ms(&samples, 4096);

        assert_eq!(stats.count, 4);
        assert_eq!(stats.bytes, 4096);
        assert_eq!(stats.avg_ms, Some(5.0));
        assert_eq!(stats.p50_ms, Some(6.0));
        assert_eq!(stats.p95_ms, Some(8.0));
        assert_eq!(stats.p99_ms, Some(8.0));
        assert_eq!(stats.max_ms, Some(8.0));
        assert_eq!(stats.jitter_ms, Some(2.23606797749979));
    }

    #[test]
    fn stage_stats_keep_empty_windows_unavailable() {
        let stats = StageStatsSnapshot::from_durations_ms(&[], 0);

        assert_eq!(stats.count, 0);
        assert_eq!(stats.bytes, 0);
        assert_eq!(stats.avg_ms, None);
        assert_eq!(stats.p50_ms, None);
        assert_eq!(stats.p95_ms, None);
        assert_eq!(stats.p99_ms, None);
        assert_eq!(stats.max_ms, None);
        assert_eq!(stats.jitter_ms, None);
    }

    #[test]
    fn pipeline_snapshot_collects_stage_rows_and_metadata() {
        let snapshot = PipelineProbeSnapshot::from_parts(
            SessionId("session-probe".into()),
            "video-main".into(),
            Some("dxgi".into()),
            Some("h264".into()),
            Some("webrtc".into()),
            30.0,
            2048.0,
            0,
            2,
            vec![],
            vec![(
                StageId::EncodeTotal,
                StageStatsSnapshot::from_durations_ms(&[1.0, 3.0], 1024),
            )],
        );

        assert_eq!(snapshot.session_id.0, "session-probe");
        assert_eq!(snapshot.stream_id, "video-main");
        assert_eq!(snapshot.backend.as_deref(), Some("dxgi"));
        assert_eq!(snapshot.codec.as_deref(), Some("h264"));
        assert_eq!(snapshot.transport.as_deref(), Some("webrtc"));
        assert_eq!(snapshot.fps, 30.0);
        assert_eq!(snapshot.bitrate_kbps, 2048.0);
        assert_eq!(snapshot.keyframes, 2);
        assert!(snapshot.counters.is_empty());
        assert_eq!(snapshot.stages.len(), 1);
        assert_eq!(snapshot.stages[0].0, StageId::EncodeTotal);
    }

    #[test]
    fn pipeline_comparison_result_serializes_captest_compatible_fields() {
        let result = PipelineComparisonResult::new("capture-encode-decode-render", "av1")
            .with_memory_path("d3d11-shared")
            .with_transport("quic-datagram")
            .with_counts(120, 120, 118, 0, 2)
            .with_average_stage_ms(Some(0.4), Some(2.1), Some(1.7), Some(0.8), Some(0.2))
            .with_transport_stage_ms(Some(0.05))
            .with_total_time_ms(Some(5.25))
            .with_avg_fps(Some(228.0))
            .with_total_bitstream_bytes(5_000_000);

        let value = serde_json::to_value(result).expect("serialize comparison result");

        assert_eq!(value["pipeline"], "capture-encode-decode-render");
        assert_eq!(value["codec"], "av1");
        assert_eq!(value["memory_path"], "d3d11-shared");
        assert_eq!(value["transport"], "quic-datagram");
        assert_eq!(value["frames"], 120);
        assert_eq!(value["encoded_units"], 120);
        assert_eq!(value["decoded_frames"], 118);
        assert_eq!(value["encode_failures"], 0);
        assert_eq!(value["decode_failures"], 2);
        assert_eq!(value["avg_capture_time_ms"], 0.4);
        assert_eq!(value["avg_encode_time_ms"], 2.1);
        assert_eq!(value["avg_decode_time_ms"], 1.7);
        assert_eq!(value["avg_render_time_ms"], 0.8);
        assert_eq!(value["avg_present_time_ms"], 0.2);
        assert_eq!(value["avg_transport_time_ms"], 0.05);
        assert_eq!(value["avg_total_time_ms"], 5.25);
        assert_eq!(value["avg_fps"], 228.0);
        assert_eq!(value["total_bitstream_bytes"], 5_000_000);
    }

    #[test]
    fn media_probe_events_capture_stage_and_bytes() {
        let event = MediaProbeEvent::new(
            SessionId("session-probe".into()),
            "video-main".into(),
            StageId::SendWrite,
            2_500,
            1500,
            true,
        );

        assert_eq!(event.stage, StageId::SendWrite);
        assert_eq!(event.duration_us, 2_500);
        assert_eq!(event.bytes, 1500);
        assert!(event.is_keyframe);
    }

    #[test]
    fn registry_snapshot_accumulates_recent_stage_samples() {
        let registry = ProbeRegistry::default();
        let handle = registry.session_handle(SessionId("session-probe".into()), "video-main");
        handle.set_backend("dxgi");
        handle.set_codec("h264");
        handle.set_transport("webrtc");
        handle.increment_counter("reassembly_expired", 2);
        handle.set_counter("pending_frames", 1);
        handle.record_stage(StageId::CaptureCopy, Duration::from_millis(2), 1024, false);
        handle.record_stage(StageId::EncodeTotal, Duration::from_millis(4), 2048, true);
        handle.record_stage(
            StageId::FrameSinkIngest,
            Duration::from_millis(1),
            512,
            false,
        );

        let snapshot = registry
            .snapshot(&SessionId("session-probe".into()), "video-main")
            .expect("probe snapshot");

        assert_eq!(snapshot.backend.as_deref(), Some("dxgi"));
        assert_eq!(snapshot.codec.as_deref(), Some("h264"));
        assert_eq!(snapshot.transport.as_deref(), Some("webrtc"));
        assert_eq!(snapshot.keyframes, 1);
        assert!(snapshot.fps > 0.0);
        assert!(snapshot
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::EncodeTotal && stats.count == 1));
        assert!(snapshot
            .counters
            .iter()
            .any(|(name, value)| name == "pending_frames" && *value == 1));
        assert!(snapshot
            .counters
            .iter()
            .any(|(name, value)| name == "reassembly_expired" && *value == 2));
    }

    #[test]
    fn pipeline_probe_bitrate_counts_encoded_bytes_once() {
        let registry = ProbeRegistry::default();
        let session_id = SessionId("session-bitrate".into());
        let handle = registry.session_handle(session_id.clone(), "video-main");
        handle.record_stage(StageId::CaptureCopy, Duration::ZERO, 2_000_000, false);
        handle.record_stage(StageId::EncodeTotal, Duration::ZERO, 1_000, false);
        handle.record_stage(StageId::SendWrite, Duration::ZERO, 1_000, false);

        let snapshot = registry
            .snapshot(&session_id, "video-main")
            .expect("probe snapshot");

        assert_eq!(snapshot.bitrate_kbps, 1.6);
    }

    #[test]
    fn pipeline_probe_bitrate_uses_network_bytes_for_receiver() {
        let registry = ProbeRegistry::default();
        let session_id = SessionId("session-receiver-bitrate".into());
        let handle = registry.session_handle(session_id.clone(), "video-main");
        handle.record_stage(StageId::NetworkIngress, Duration::ZERO, 2_000, false);
        handle.record_stage(StageId::DecodeTotal, Duration::ZERO, 2_000, false);
        handle.record_stage(StageId::FrameSinkIngest, Duration::ZERO, 8_000_000, false);

        let snapshot = registry
            .snapshot(&session_id, "video-main")
            .expect("probe snapshot");

        assert_eq!(snapshot.bitrate_kbps, 3.2);
    }

    #[test]
    fn component_result_computes_latency_and_throughput_fields() {
        let result = ComponentResult::new(
            ComponentKind::Encode,
            "openh264",
            "encode.openh264",
            2.0,
            60,
            0,
            &[1.0, 2.0, 4.0, 8.0],
            Some(1280),
            Some(720),
            None,
            None,
            Some(&[1000, 1200, 900, 1500]),
            None,
            None,
            Some(0.25),
            None,
        );

        assert_eq!(result.sample_count, 60);
        assert_eq!(result.throughput_fps, 30.0);
        assert_eq!(result.latency_ms.p50_ms, Some(4.0));
        assert_eq!(result.latency_ms.p95_ms, Some(8.0));
        assert_eq!(
            result
                .access_unit_bytes
                .as_ref()
                .and_then(|stats| stats.p95),
            Some(1500.0)
        );
        assert_eq!(result.written_bytes, None);
        assert_eq!(result.packets_per_sample, None);
        assert_eq!(result.keyframe_ratio, Some(0.25));
    }

    #[test]
    fn value_stats_snapshot_handles_empty_values() {
        let stats = ValueStatsSnapshot::from_values(&[]);

        assert_eq!(stats.mean, None);
        assert_eq!(stats.p50, None);
        assert_eq!(stats.p95, None);
        assert_eq!(stats.p99, None);
        assert_eq!(stats.max, None);
    }
}
