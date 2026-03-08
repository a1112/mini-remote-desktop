#![cfg(test)]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use bytes::{BufMut, Bytes, BytesMut};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_observability::{PipelineProbeSnapshot, ProbeRegistry, StageId};
use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
use mrd_proto::SessionId;
use mrd_transport_quic_quinn::QuinnDatagramPair;

use crate::frame_sink::{DecodedFrameSink, DecodedFrameSnapshot, DEFAULT_SOURCE_ID};

mod tests {
    use super::*;

    #[tokio::test]
    async fn quic_single_process_pipeline_delivers_remote_frames() {
        let mut harness = QuicHostedPairHarness::new("session-quic-frames")
            .await
            .expect("create quic harness");

        harness.start().await.expect("start quic harness");
        harness
            .wait_for_first_frame(Duration::from_secs(5))
            .await
            .expect("remote frame");

        let sink_snapshot = harness.sink_snapshot().expect("sink snapshot");
        let sender_probe = harness.sender_probe();
        let receiver_probe = harness.receiver_probe();

        assert!(sink_snapshot.frame_count > 0);
        assert!(sender_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::SendWrite && stats.count > 0));
        assert!(receiver_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::DecodeTotal && stats.count > 0));
    }

    #[tokio::test]
    async fn quic_single_process_pipeline_exposes_probe_stages() {
        let mut harness = QuicHostedPairHarness::new("session-quic-probe")
            .await
            .expect("create quic harness");

        harness.start().await.expect("start quic harness");
        harness
            .wait_for_first_frame(Duration::from_secs(5))
            .await
            .expect("remote frame");

        let sender_probe = harness.sender_probe();
        let receiver_probe = harness.receiver_probe();

        assert!(sender_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::CaptureCopy && stats.count > 0));
        assert!(sender_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::EncodeTotal && stats.count > 0));
        assert!(sender_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::SendWrite && stats.count > 0));
        assert!(receiver_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::NetworkIngress && stats.count > 0));
        assert!(receiver_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::DecodeTotal && stats.count > 0));
        assert!(receiver_probe
            .stages
            .iter()
            .any(|(stage, stats)| *stage == StageId::FrameSinkIngest && stats.count > 0));
    }

    #[tokio::test]
    async fn quic_single_process_pipeline_runs_for_fixed_duration_without_stalling() {
        let mut harness = QuicHostedPairHarness::new("session-quic-stable")
            .await
            .expect("create quic harness");

        harness.start().await.expect("start quic harness");
        harness
            .wait_for_first_frame(Duration::from_secs(5))
            .await
            .expect("remote frame");

        let progress = harness
            .sample_frame_progress(Duration::from_secs(2), Duration::from_millis(250))
            .await;

        assert!(progress.start_frame_count > 0);
        assert!(progress.end_frame_count > progress.start_frame_count);
        assert!(progress.observed_samples > 0);
    }
}

struct FakeCapture {
    tick: u8,
}

impl FrameCapture for FakeCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, mrd_pipeline_core::PipelineError> {
        self.tick = self.tick.wrapping_add(1);
        let mut data = vec![0_u8; 16 * 16 * 4];
        for chunk in data.chunks_exact_mut(4) {
            chunk[0] = self.tick;
            chunk[1] = 64;
            chunk[2] = 192;
            chunk[3] = 255;
        }

        Ok(CapturedFrame {
            width: 16,
            height: 16,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: self.tick as u64 * 33_000,
            data,
        })
    }
}

struct FrameProgressSample {
    start_frame_count: u64,
    end_frame_count: u64,
    observed_samples: usize,
}

pub(crate) struct QuicBenchmarkOutcome {
    pub sender_probe: PipelineProbeSnapshot,
    pub receiver_probe: PipelineProbeSnapshot,
    pub sink_snapshot: DecodedFrameSnapshot,
    pub first_frame_time_ms: f64,
}

struct QuicHostedPairHarness {
    pair: QuinnDatagramPair,
    sink: Arc<Mutex<DecodedFrameSink>>,
    probe_registry: ProbeRegistry,
    session_id: SessionId,
    running: Arc<AtomicBool>,
    sender_task: Option<tokio::task::JoinHandle<()>>,
    receiver_task: Option<tokio::task::JoinHandle<()>>,
}

impl QuicHostedPairHarness {
    async fn new(session_id: &str) -> Result<Self, String> {
        let pair = QuinnDatagramPair::loopback()
            .await
            .map_err(|error| format!("create quic loopback pair failed: {error}"))?;
        Ok(Self {
            pair,
            sink: Arc::new(Mutex::new(DecodedFrameSink::default())),
            probe_registry: ProbeRegistry::default(),
            session_id: SessionId(session_id.into()),
            running: Arc::new(AtomicBool::new(false)),
            sender_task: None,
            receiver_task: None,
        })
    }

    async fn start(&mut self) -> Result<(), String> {
        self.start_with_capture(FakeCapture { tick: 0 }, 16, 16, 30)
            .await
    }

    async fn start_with_capture<C>(
        &mut self,
        mut capture: C,
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<(), String>
    where
        C: FrameCapture + Send + 'static,
    {
        if self.running.swap(true, Ordering::Relaxed) {
            return Ok(());
        }

        let mut encoder = OpenH264Encoder::new(width, height, fps)
            .map_err(|error| format!("create encoder failed: {error}"))?;
        let sender_probe = self
            .probe_registry
            .session_handle(self.session_id.clone(), format!("{DEFAULT_SOURCE_ID}-sender"));
        sender_probe.set_backend("synthetic");
        sender_probe.set_codec("h264");
        sender_probe.set_transport("quic_quinn");

        let receiver_probe = self
            .probe_registry
            .session_handle(self.session_id.clone(), DEFAULT_SOURCE_ID);
        receiver_probe.set_codec("h264");
        receiver_probe.set_transport("quic_quinn");

        let running = self.running.clone();
        let client = self.pair.client.clone();
        let sender_task = tokio::spawn(async move {
            let mut last_tick = tokio::time::Instant::now();
            while running.load(Ordering::Relaxed) {
                sender_probe.record_stage(StageId::CaptureWait, last_tick.elapsed(), 0, false);
                last_tick = tokio::time::Instant::now();

                let capture_started_at = std::time::Instant::now();
                let frame = match capture.capture_frame() {
                    Ok(frame) => frame,
                    Err(_) => break,
                };
                sender_probe.record_stage(
                    StageId::CaptureCopy,
                    capture_started_at.elapsed(),
                    frame.data.len(),
                    false,
                );

                let encode_started_at = std::time::Instant::now();
                let access_units = match encoder.encode(&frame) {
                    Ok(access_units) => access_units,
                    Err(_) => break,
                };
                for access_unit in access_units {
                    sender_probe.record_stage(
                        StageId::EncodeTotal,
                        encode_started_at.elapsed(),
                        access_unit.bytes.len(),
                        access_unit.is_keyframe,
                    );
                    let datagram = encode_datagram(&access_unit);
                    let send_started_at = std::time::Instant::now();
                    if client.send_datagram(datagram).is_err() {
                        break;
                    }
                    sender_probe.record_stage(
                        StageId::SendWrite,
                        send_started_at.elapsed(),
                        access_unit.bytes.len(),
                        access_unit.is_keyframe,
                    );
                }

                tokio::time::sleep(Duration::from_millis((1000 / fps.max(1)) as u64)).await;
            }
        });

        let running = self.running.clone();
        let sink = self.sink.clone();
        let session_id = self.session_id.clone();
        let server = self.pair.server.clone();
        let receiver_task = tokio::spawn(async move {
            let mut decoder = mrd_decode::create_decoder("h264_software").expect("decoder");
            while running.load(Ordering::Relaxed) {
                let receive_started_at = std::time::Instant::now();
                let payload = match server.read_datagram().await {
                    Ok(payload) => payload,
                    Err(_) => break,
                };
                receiver_probe.record_stage(
                    StageId::NetworkIngress,
                    receive_started_at.elapsed(),
                    payload.len(),
                    false,
                );

                let (timestamp_us, is_keyframe, access_unit) = match decode_datagram(&payload) {
                    Ok(parts) => parts,
                    Err(_) => break,
                };
                let decode_started_at = std::time::Instant::now();
                if decoder.push_access_unit(&access_unit).is_err() {
                    continue;
                }
                let frames = decoder.drain_decoded_frames();
                receiver_probe.record_stage(
                    StageId::DecodeTotal,
                    decode_started_at.elapsed(),
                    access_unit.len(),
                    is_keyframe,
                );
                for frame in frames {
                    let bytes = frame.data.len();
                    sink.lock().expect("lock sink").ingest_frame_for_source(
                        session_id.clone(),
                        DEFAULT_SOURCE_ID.to_string(),
                        frame,
                    );
                    receiver_probe.record_stage(
                        StageId::FrameSinkIngest,
                        Duration::from_millis(0),
                        bytes,
                        false,
                    );
                }

                let _ = timestamp_us;
            }
        });

        self.sender_task = Some(sender_task);
        self.receiver_task = Some(receiver_task);
        Ok(())
    }

    async fn wait_for_first_frame(&self, timeout: Duration) -> Result<(), String> {
        tokio::time::timeout(timeout, async {
            loop {
                if self
                    .sink
                    .lock()
                    .expect("lock sink")
                    .snapshot(&self.session_id)
                    .map(|snapshot| snapshot.frame_count > 0)
                    .unwrap_or(false)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| format!("timed out waiting for first QUIC frame for {}", self.session_id.0))
    }

    async fn sample_frame_progress(&self, duration: Duration, step: Duration) -> FrameProgressSample {
        let start_frame_count = self
            .sink_snapshot()
            .map(|snapshot| snapshot.frame_count)
            .unwrap_or(0);
        let started_at = tokio::time::Instant::now();
        let mut observed_samples = 0usize;
        while started_at.elapsed() < duration {
            tokio::time::sleep(step).await;
            observed_samples += 1;
        }
        let end_frame_count = self
            .sink_snapshot()
            .map(|snapshot| snapshot.frame_count)
            .unwrap_or(0);

        FrameProgressSample {
            start_frame_count,
            end_frame_count,
            observed_samples,
        }
    }

    fn sender_probe(&self) -> PipelineProbeSnapshot {
        self.probe_registry
            .snapshot(&self.session_id, &format!("{DEFAULT_SOURCE_ID}-sender"))
            .expect("sender probe snapshot")
    }

    fn receiver_probe(&self) -> PipelineProbeSnapshot {
        self.probe_registry
            .snapshot(&self.session_id, DEFAULT_SOURCE_ID)
            .expect("receiver probe snapshot")
    }

    fn sink_snapshot(&self) -> Option<DecodedFrameSnapshot> {
        self.sink
            .lock()
            .expect("lock sink")
            .snapshot(&self.session_id)
            .cloned()
    }
}

pub(crate) async fn run_quic_benchmark_pipeline(
    session_id: SessionId,
    width: usize,
    height: usize,
    fps: u32,
    duration_secs: u64,
) -> Result<QuicBenchmarkOutcome, String> {
    let mut harness = QuicHostedPairHarness::new(&session_id.0).await?;
    harness
        .start_with_capture(BenchmarkCapture { tick: 0, width, height }, width, height, fps)
        .await?;
    let started_at = std::time::Instant::now();
    harness.wait_for_first_frame(Duration::from_secs(8)).await?;
    let first_frame_time_ms = started_at.elapsed().as_secs_f64() * 1000.0;
    tokio::time::sleep(Duration::from_secs(duration_secs)).await;
    Ok(QuicBenchmarkOutcome {
        sender_probe: harness.sender_probe(),
        receiver_probe: harness.receiver_probe(),
        sink_snapshot: harness.sink_snapshot().expect("sink snapshot"),
        first_frame_time_ms,
    })
}

struct BenchmarkCapture {
    tick: u8,
    width: usize,
    height: usize,
}

impl FrameCapture for BenchmarkCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, mrd_pipeline_core::PipelineError> {
        self.tick = self.tick.wrapping_add(1);
        let mut data = vec![0_u8; self.width * self.height * 4];
        for chunk in data.chunks_exact_mut(4) {
            chunk[0] = self.tick;
            chunk[1] = 64;
            chunk[2] = 192;
            chunk[3] = 255;
        }
        Ok(CapturedFrame {
            width: self.width,
            height: self.height,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: self.tick as u64 * 33_000,
            data,
        })
    }
}

impl Drop for QuicHostedPairHarness {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(task) = self.sender_task.take() {
            task.abort();
        }
        if let Some(task) = self.receiver_task.take() {
            task.abort();
        }
    }
}

fn encode_datagram(access_unit: &mrd_pipeline_core::EncodedAccessUnit) -> Bytes {
    let mut buffer = BytesMut::with_capacity(access_unit.bytes.len() + 9);
    buffer.put_u64_le(access_unit.timestamp_us);
    buffer.put_u8(u8::from(access_unit.is_keyframe));
    buffer.extend_from_slice(&access_unit.bytes);
    buffer.freeze()
}

fn decode_datagram(payload: &[u8]) -> Result<(u64, bool, Vec<u8>), String> {
    if payload.len() < 9 {
        return Err("payload too small".into());
    }
    let mut timestamp = [0_u8; 8];
    timestamp.copy_from_slice(&payload[..8]);
    Ok((
        u64::from_le_bytes(timestamp),
        payload[8] != 0,
        payload[9..].to_vec(),
    ))
}
