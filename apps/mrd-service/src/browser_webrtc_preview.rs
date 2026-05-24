use std::{
    collections::{HashMap, VecDeque},
    hint,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Once,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use crate::browser_preview_capture::open_browser_preview_dxgi_capture;
#[cfg(windows)]
use mrd_encode_nvenc::{NvencH264Encoder, NvencHevcEncoder};
use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
#[cfg(windows)]
use mrd_pipeline_core::{FrameCapture, VideoEncoder};
use mrd_transport_webrtc::{H264Profile, H264RtpSender, H264SampleSender, HevcRtpSender};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors, media_engine::MediaEngine,
        setting_engine::SettingEngine, APIBuilder,
    },
    data_channel::{data_channel_state::RTCDataChannelState, RTCDataChannel},
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription, RTCPeerConnection,
    },
    track::track_local::TrackLocal,
};

const DEFAULT_BROWSER_PREVIEW_FPS: u32 = 120;
const MAX_BROWSER_PREVIEW_FPS: u32 = 144;
const DEFAULT_BROWSER_PREVIEW_BITRATE_MBPS: u32 = 80;
const MAX_BROWSER_PREVIEW_BITRATE_MBPS: u32 = 160;
const BROWSER_PREVIEW_QUEUE_TARGET_LATENCY_MS: u32 = 96;
const BROWSER_FRAME_TIMING_CHANNEL_LABEL: &str = "mrd-frame-timing";

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserWebrtcPreviewStartRequest {
    pub session_id: String,
    pub offer_sdp: String,
    #[serde(default)]
    pub codec: Option<BrowserWebrtcPreviewCodec>,
    #[serde(default)]
    pub fps: Option<u32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub h264_profile: Option<String>,
    #[serde(default)]
    pub bitrate_mbps: Option<u32>,
    #[serde(default)]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserWebrtcPreviewCodec {
    #[serde(alias = "avc")]
    H264,
    #[serde(alias = "h265")]
    Hevc,
}

impl BrowserWebrtcPreviewStartRequest {
    pub fn selected_codec(&self) -> BrowserWebrtcPreviewCodec {
        self.codec.unwrap_or(BrowserWebrtcPreviewCodec::H264)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserWebrtcPreviewStopRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrowserWebrtcPreviewAnswer {
    pub session_id: String,
    pub answer_sdp: String,
}

#[derive(Default)]
pub struct BrowserWebrtcPreviewHost {
    sessions: HashMap<String, BrowserWebrtcPreviewSession>,
}

struct BrowserWebrtcPreviewSession {
    pc: Arc<RTCPeerConnection>,
    running: Arc<AtomicBool>,
    sender_task: JoinHandle<()>,
}

#[derive(Default)]
struct BrowserPreviewSendStats {
    enqueued: AtomicU64,
    sent: AtomicU64,
    bytes: AtomicU64,
    send_us: AtomicU64,
    max_send_us: AtomicU64,
    send_samples_us: Mutex<Vec<u64>>,
    pending: AtomicU64,
    max_pending: AtomicU64,
    dropped_full: AtomicU64,
    dropped_oldest: AtomicU64,
    timing_sent: AtomicU64,
    timing_no_channel: AtomicU64,
    timing_not_open: AtomicU64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct BrowserPreviewFrameTimingMessage {
    #[serde(rename = "type")]
    message_type: &'static str,
    sequence: u64,
    capture_unix_us: u64,
    sent_unix_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtp_timestamp: Option<u32>,
    keyframe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserPreviewTimingSendResult {
    Sent,
    NoChannel,
    NotOpen,
}

impl BrowserPreviewFrameTimingMessage {
    fn new(
        sequence: u64,
        access_unit: &EncodedAccessUnit,
        sent_unix_us: u64,
        rtp_timestamp: Option<u32>,
    ) -> Self {
        Self {
            message_type: "mrd.frame_timing.v1",
            sequence,
            capture_unix_us: access_unit.timestamp_us,
            sent_unix_us,
            rtp_timestamp,
            keyframe: access_unit.is_keyframe,
        }
    }
}

fn update_atomic_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

fn decrement_atomic_saturating(target: &AtomicU64) {
    let _ = target.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_sub(1))
    });
}

fn subtract_atomic_saturating(target: &AtomicU64, amount: u64) {
    let _ = target.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_sub(amount))
    });
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TimingSummaryUs {
    samples: usize,
    p50_us: u64,
    p95_us: u64,
}

fn summarize_timing_us(samples: &[u64]) -> TimingSummaryUs {
    if samples.is_empty() {
        return TimingSummaryUs::default();
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |value: f64| {
        let index = ((sorted.len() as f64 * value).ceil() as usize).saturating_sub(1);
        sorted[index.min(sorted.len() - 1)]
    };

    TimingSummaryUs {
        samples: sorted.len(),
        p50_us: percentile(0.50),
        p95_us: percentile(0.95),
    }
}

fn take_timing_samples(samples: &Mutex<Vec<u64>>) -> Vec<u64> {
    let mut samples = samples.lock().expect("lock browser preview timing samples");
    std::mem::take(&mut *samples)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserPreviewSenderTrackKind {
    Rtp,
    Sample,
}

fn browser_preview_sender_track_kind(bitrate_bps: u32) -> BrowserPreviewSenderTrackKind {
    if bitrate_bps <= 20_000_000 {
        BrowserPreviewSenderTrackKind::Rtp
    } else {
        BrowserPreviewSenderTrackKind::Sample
    }
}

struct BrowserPreviewMediaSendReport {
    bytes_written: usize,
    rtp_timestamp: Option<u32>,
}

enum BrowserPreviewMediaSender {
    H264Rtp(H264RtpSender),
    H264Sample(H264SampleSender),
    HevcRtp(HevcRtpSender),
}

impl BrowserPreviewMediaSender {
    fn new(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        codec: BrowserWebrtcPreviewCodec,
        profile: H264Profile,
        profile_level_id: impl Into<String>,
        bitrate_bps: u32,
    ) -> Self {
        let track_id = track_id.into();
        let stream_id = stream_id.into();
        let profile_level_id = profile_level_id.into();
        match codec {
            BrowserWebrtcPreviewCodec::H264 => match browser_preview_sender_track_kind(bitrate_bps)
            {
                BrowserPreviewSenderTrackKind::Rtp => {
                    Self::H264Rtp(H264RtpSender::new_with_profile_level_id(
                        track_id,
                        stream_id,
                        fps,
                        1200,
                        profile,
                        profile_level_id,
                    ))
                }
                BrowserPreviewSenderTrackKind::Sample => {
                    Self::H264Sample(H264SampleSender::new_with_profile_level_id(
                        track_id,
                        stream_id,
                        fps,
                        profile_level_id,
                    ))
                }
            },
            BrowserWebrtcPreviewCodec::Hevc => {
                Self::HevcRtp(HevcRtpSender::new(track_id, stream_id, fps, 1200))
            }
        }
    }

    fn track_kind(&self) -> BrowserPreviewSenderTrackKind {
        match self {
            Self::H264Rtp(_) | Self::HevcRtp(_) => BrowserPreviewSenderTrackKind::Rtp,
            Self::H264Sample(_) => BrowserPreviewSenderTrackKind::Sample,
        }
    }

    fn track(&self) -> Arc<dyn TrackLocal + Send + Sync> {
        match self {
            Self::H264Rtp(sender) => sender.track(),
            Self::H264Sample(sender) => sender.track(),
            Self::HevcRtp(sender) => sender.track(),
        }
    }

    async fn send_access_unit_with_report(
        &mut self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<BrowserPreviewMediaSendReport, mrd_transport_webrtc::TransportError> {
        match self {
            Self::H264Rtp(sender) => {
                let report = sender.send_access_unit_with_report(access_unit).await?;
                Ok(BrowserPreviewMediaSendReport {
                    bytes_written: report.bytes_written,
                    rtp_timestamp: Some(report.rtp_timestamp),
                })
            }
            Self::H264Sample(sender) => {
                let report = sender.send_access_unit_with_report(access_unit).await?;
                Ok(BrowserPreviewMediaSendReport {
                    bytes_written: report.bytes_written,
                    rtp_timestamp: None,
                })
            }
            Self::HevcRtp(sender) => {
                let report = sender.send_access_unit_with_report(access_unit).await?;
                Ok(BrowserPreviewMediaSendReport {
                    bytes_written: report.bytes_written,
                    rtp_timestamp: Some(report.rtp_timestamp),
                })
            }
        }
    }
}

#[cfg(windows)]
enum BrowserPreviewEncoder {
    H264(NvencH264Encoder),
    Hevc(NvencHevcEncoder),
}

#[cfg(windows)]
impl BrowserPreviewEncoder {
    fn new(
        codec: BrowserWebrtcPreviewCodec,
        width: usize,
        height: usize,
        fps: u32,
        bitrate_bps: u32,
    ) -> Result<Self, mrd_pipeline_core::PipelineError> {
        match codec {
            BrowserWebrtcPreviewCodec::H264 => {
                NvencH264Encoder::new_max_speed_with_bitrate(width, height, fps, bitrate_bps)
                    .map(Self::H264)
            }
            BrowserWebrtcPreviewCodec::Hevc => {
                NvencHevcEncoder::new_max_speed_with_bitrate(width, height, fps, bitrate_bps)
                    .map(Self::Hevc)
            }
        }
    }

    fn codec(&self) -> VideoCodec {
        match self {
            Self::H264(_) => VideoCodec::H264,
            Self::Hevc(_) => VideoCodec::Hevc,
        }
    }

    fn request_keyframe(&mut self) {
        if let Self::H264(encoder) = self {
            encoder.request_keyframe();
        }
    }

    fn encode(
        &mut self,
        frame: &mrd_pipeline_core::CapturedFrame,
    ) -> Result<Vec<EncodedAccessUnit>, mrd_pipeline_core::PipelineError> {
        match self {
            Self::H264(encoder) => encoder.encode(frame),
            Self::Hevc(encoder) => encoder.encode(frame),
        }
    }
}

fn browser_webrtc_preview_codec_label(codec: BrowserWebrtcPreviewCodec) -> &'static str {
    match codec {
        BrowserWebrtcPreviewCodec::H264 => "H.264",
        BrowserWebrtcPreviewCodec::Hevc => "HEVC",
    }
}

struct BrowserPreviewFrameQueue {
    capacity: usize,
    frames: Mutex<VecDeque<EncodedAccessUnit>>,
    notify: tokio::sync::Notify,
    closed: AtomicBool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BrowserPreviewQueuePushResult {
    accepted: bool,
    queued: bool,
    dropped_oldest: bool,
    needs_keyframe: bool,
}

impl BrowserPreviewFrameQueue {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            frames: Mutex::new(VecDeque::with_capacity(capacity.max(1))),
            notify: tokio::sync::Notify::new(),
            closed: AtomicBool::new(false),
        }
    }

    fn push_latest(
        &self,
        access_unit: EncodedAccessUnit,
        stats: &BrowserPreviewSendStats,
    ) -> BrowserPreviewQueuePushResult {
        if self.closed.load(Ordering::Relaxed) {
            return BrowserPreviewQueuePushResult::default();
        }

        let mut result = BrowserPreviewQueuePushResult {
            accepted: true,
            ..BrowserPreviewQueuePushResult::default()
        };
        let pending = {
            let mut frames = self
                .frames
                .lock()
                .expect("lock browser preview frame queue");
            if self.closed.load(Ordering::Relaxed) {
                return BrowserPreviewQueuePushResult::default();
            }
            if frames.len() >= self.capacity {
                let dropped = frames.len() as u64;
                frames.clear();
                stats.dropped_oldest.fetch_add(dropped, Ordering::Relaxed);
                subtract_atomic_saturating(&stats.pending, dropped);
                result.dropped_oldest = dropped > 0;
                if !access_unit.is_keyframe {
                    result.needs_keyframe = true;
                    Some(stats.pending.load(Ordering::Relaxed))
                } else {
                    let pending = stats.pending.fetch_add(1, Ordering::Relaxed) + 1;
                    frames.push_back(access_unit);
                    stats.enqueued.fetch_add(1, Ordering::Relaxed);
                    result.queued = true;
                    Some(pending)
                }
            } else {
                let pending = stats.pending.fetch_add(1, Ordering::Relaxed) + 1;
                frames.push_back(access_unit);
                stats.enqueued.fetch_add(1, Ordering::Relaxed);
                result.queued = true;
                Some(pending)
            }
        };
        if let Some(pending) = pending {
            update_atomic_max(&stats.max_pending, pending);
        }
        if result.queued {
            self.notify.notify_one();
        }
        result
    }

    async fn pop(&self) -> Option<EncodedAccessUnit> {
        loop {
            if let Some(frame) = self
                .frames
                .lock()
                .expect("lock browser preview frame queue")
                .pop_front()
            {
                return Some(frame);
            }

            if self.closed.load(Ordering::Relaxed) {
                return None;
            }

            self.notify.notified().await;
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }
}

impl BrowserWebrtcPreviewHost {
    pub async fn start(
        &mut self,
        request: BrowserWebrtcPreviewStartRequest,
    ) -> Result<BrowserWebrtcPreviewAnswer, String> {
        ensure_rustls_crypto_provider();

        let session_id = request.session_id.trim().to_string();
        if session_id.is_empty() {
            return Err("browser WebRTC preview session_id is empty".to_string());
        }
        if request.offer_sdp.trim().is_empty() {
            return Err("browser WebRTC preview offer SDP is empty".to_string());
        }
        if self.sessions.contains_key(&session_id) {
            self.stop(&session_id).await?;
        }

        let fps = sanitize_browser_preview_fps(request.fps);
        let bitrate_bps =
            sanitize_browser_preview_bitrate_mbps(request.bitrate_mbps).saturating_mul(1_000_000);
        let codec = request.selected_codec();
        let profile = h264_profile_from_label(request.h264_profile.as_deref());
        let profile_level_id =
            select_browser_offer_h264_profile_level_id(&request.offer_sdp, profile);
        validate_browser_preview_sender(
            request.source_id.as_deref(),
            codec,
            fps,
            bitrate_bps,
            request.width,
            request.height,
        )?;
        let pc = build_peer_connection().await?;
        let running = Arc::new(AtomicBool::new(true));
        let frame_timing_channel = Arc::new(Mutex::new(None::<Arc<RTCDataChannel>>));
        let frame_timing_channel_for_handler = frame_timing_channel.clone();
        let data_channel_session_id = session_id.clone();
        pc.on_data_channel(Box::new(move |channel: Arc<RTCDataChannel>| {
            let frame_timing_channel = frame_timing_channel_for_handler.clone();
            let session_id = data_channel_session_id.clone();
            Box::pin(async move {
                info!(
                    "browser WebRTC preview received data channel '{}' for {}",
                    channel.label(),
                    session_id
                );
                let _ = frame_timing_channel;
            })
        }));
        let state_running = running.clone();
        let state_session_id = session_id.clone();
        pc.on_peer_connection_state_change(Box::new(move |state: RTCPeerConnectionState| {
            let running = state_running.clone();
            let session_id = state_session_id.clone();
            Box::pin(async move {
                if matches!(
                    state,
                    RTCPeerConnectionState::Disconnected
                        | RTCPeerConnectionState::Failed
                        | RTCPeerConnectionState::Closed
                ) {
                    warn!("browser WebRTC preview peer {session_id} changed to {state}; stopping sender");
                    running.store(false, Ordering::Relaxed);
                }
            })
        }));
        info!(
            "browser WebRTC preview {} offer selected profile-level-id={} for {}",
            browser_webrtc_preview_codec_label(codec),
            profile_level_id,
            session_id
        );
        let media_sender = BrowserPreviewMediaSender::new(
            "video",
            format!("{session_id}-browser-web"),
            fps,
            codec,
            profile,
            profile_level_id,
            bitrate_bps,
        );
        let track_kind = media_sender.track_kind();
        info!(
            "browser WebRTC preview selected {:?} track pacing for {}",
            track_kind, session_id
        );
        let track = media_sender.track();
        let rtp_sender = pc
            .add_track(track)
            .await
            .map_err(|error| format!("add browser WebRTC video track failed: {error}"))?;
        tokio::spawn(async move { while rtp_sender.read_rtcp().await.is_ok() {} });

        let description = RTCSessionDescription::offer(request.offer_sdp.clone())
            .map_err(|error| format!("build browser WebRTC offer failed: {error}"))?;
        pc.set_remote_description(description)
            .await
            .map_err(|error| format!("set browser WebRTC remote offer failed: {error}"))?;

        match pc
            .create_data_channel(BROWSER_FRAME_TIMING_CHANNEL_LABEL, None)
            .await
        {
            Ok(channel) => {
                let open_session_id = session_id.clone();
                channel.on_open(Box::new(move || {
                    let session_id = open_session_id.clone();
                    Box::pin(async move {
                        info!(
                            "browser WebRTC preview frame timing data channel open for {}",
                            session_id
                        );
                    })
                }));
                *frame_timing_channel
                    .lock()
                    .expect("lock browser frame timing data channel") = Some(channel);
                info!(
                    "browser WebRTC preview frame timing data channel created for {}",
                    session_id
                );
            }
            Err(error) => {
                warn!(
                    "browser WebRTC preview frame timing data channel unavailable for {}: {}",
                    session_id, error
                );
            }
        }

        let answer = pc
            .create_answer(None)
            .await
            .map_err(|error| format!("create browser WebRTC answer failed: {error}"))?;
        let mut gather_complete = pc.gathering_complete_promise().await;
        pc.set_local_description(answer)
            .await
            .map_err(|error| format!("set browser WebRTC local answer failed: {error}"))?;
        let _ = gather_complete.recv().await;
        let local = pc
            .local_description()
            .await
            .ok_or_else(|| "browser WebRTC local answer is missing".to_string())?;

        let sender_task = spawn_local_capture_sender(
            session_id.clone(),
            fps,
            bitrate_bps,
            request.width,
            request.height,
            request.source_id.clone(),
            codec,
            media_sender,
            frame_timing_channel,
            running.clone(),
        );
        self.sessions.insert(
            session_id.clone(),
            BrowserWebrtcPreviewSession {
                pc,
                running,
                sender_task,
            },
        );

        Ok(BrowserWebrtcPreviewAnswer {
            session_id,
            answer_sdp: local.sdp,
        })
    }

    pub async fn stop(&mut self, session_id: &str) -> Result<(), String> {
        let Some(session) = self.sessions.remove(session_id) else {
            return Ok(());
        };
        session.running.store(false, Ordering::Relaxed);
        session.sender_task.abort();
        let _ = session.sender_task.await;
        let _ = tokio::time::timeout(Duration::from_secs(2), session.pc.close()).await;
        Ok(())
    }
}

#[cfg(windows)]
fn validate_browser_preview_sender(
    source_id: Option<&str>,
    codec: BrowserWebrtcPreviewCodec,
    fps: u32,
    bitrate_bps: u32,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<(), String> {
    let mut capture = open_browser_preview_dxgi_capture(source_id)
        .map_err(|error| format!("browser WebRTC preview DXGI capture unavailable: {error}"))?;
    let (target_width, target_height) = sanitize_browser_preview_target_dimensions(
        width,
        height,
        capture.width(),
        capture.height(),
    );
    capture.set_target_dimensions(target_width, target_height);
    let _encoder =
        BrowserPreviewEncoder::new(codec, capture.width(), capture.height(), fps, bitrate_bps)
            .map_err(|error| {
                format!(
                    "browser WebRTC preview NVENC {} unavailable: {error}",
                    browser_webrtc_preview_codec_label(codec)
                )
            })?;
    Ok(())
}

#[cfg(not(windows))]
fn validate_browser_preview_sender(
    _source_id: Option<&str>,
    _codec: BrowserWebrtcPreviewCodec,
    _fps: u32,
    _bitrate_bps: u32,
    _width: Option<u32>,
    _height: Option<u32>,
) -> Result<(), String> {
    Err("browser WebRTC preview currently requires Windows DXGI + NVENC H.264/HEVC".to_string())
}

pub fn sanitize_browser_preview_fps(fps: Option<u32>) -> u32 {
    fps.unwrap_or(DEFAULT_BROWSER_PREVIEW_FPS)
        .clamp(1, MAX_BROWSER_PREVIEW_FPS)
}

pub fn sanitize_browser_preview_bitrate_mbps(bitrate_mbps: Option<u32>) -> u32 {
    bitrate_mbps
        .unwrap_or(DEFAULT_BROWSER_PREVIEW_BITRATE_MBPS)
        .clamp(1, MAX_BROWSER_PREVIEW_BITRATE_MBPS)
}

fn sanitize_browser_preview_target_dimensions(
    width: Option<u32>,
    height: Option<u32>,
    source_width: usize,
    source_height: usize,
) -> (usize, usize) {
    let source_width = source_width.max(2);
    let source_height = source_height.max(2);
    let (Some(width), Some(height)) = (width, height) else {
        return (source_width & !1, source_height & !1);
    };

    let target_width = (width as usize).clamp(2, source_width) & !1;
    let target_height = (height as usize).clamp(2, source_height) & !1;
    (target_width.max(2), target_height.max(2))
}

fn browser_preview_queue_capacity(fps: u32) -> usize {
    let frames_for_latency =
        (fps.max(1) as u64 * BROWSER_PREVIEW_QUEUE_TARGET_LATENCY_MS as u64).div_ceil(1000);
    frames_for_latency.clamp(4, 12) as usize
}

fn sleep_until_frame_deadline(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }

        let remaining = deadline - now;
        if remaining > Duration::from_millis(2) {
            thread::sleep(remaining - Duration::from_millis(1));
        } else if remaining > Duration::from_micros(350) {
            thread::yield_now();
        } else {
            hint::spin_loop();
        }
    }
}

fn h264_profile_from_label(label: Option<&str>) -> H264Profile {
    match label {
        Some("high" | "nvenc" | "nvenc_h264") => H264Profile::High,
        _ => H264Profile::Baseline,
    }
}

fn select_browser_offer_h264_profile_level_id(offer_sdp: &str, profile: H264Profile) -> String {
    let mut offered = Vec::<String>::new();
    for line in offer_sdp.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("a=fmtp:") || !lower.contains("profile-level-id=") {
            continue;
        }
        if !lower.contains("packetization-mode=1") {
            continue;
        }
        if let Some(value) = lower
            .split("profile-level-id=")
            .nth(1)
            .and_then(|tail| tail.split([';', ' ', '\r']).next())
            .filter(|value| value.len() == 6)
        {
            offered.push(value.to_string());
        }
    }

    let preferred_prefixes: &[&str] = match profile {
        H264Profile::High => &["64", "4d", "42"],
        H264Profile::Baseline => &["42", "4d", "64"],
    };
    for prefix in preferred_prefixes {
        if let Some(value) = offered.iter().find(|value| value.starts_with(prefix)) {
            return h264_profile_level_id_with_minimum_level(value, "34");
        }
    }

    offered
        .into_iter()
        .next()
        .map(|value| h264_profile_level_id_with_minimum_level(&value, "34"))
        .unwrap_or_else(|| match profile {
            H264Profile::High => "640034".to_string(),
            H264Profile::Baseline => "42e034".to_string(),
        })
}

fn h264_profile_level_id_with_minimum_level(profile_level_id: &str, minimum_level: &str) -> String {
    if profile_level_id.len() != 6 || minimum_level.len() != 2 {
        return profile_level_id.to_string();
    }
    let current = u8::from_str_radix(&profile_level_id[4..6], 16).unwrap_or(0);
    let minimum = u8::from_str_radix(minimum_level, 16).unwrap_or(current);
    if current >= minimum {
        profile_level_id.to_string()
    } else {
        format!("{}{minimum_level}", &profile_level_id[0..4])
    }
}

fn ensure_rustls_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn now_unix_us_lossy() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

fn send_browser_frame_timing_message(
    channel: &Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    message: &BrowserPreviewFrameTimingMessage,
) -> BrowserPreviewTimingSendResult {
    let Some(channel) = channel
        .lock()
        .expect("lock browser frame timing data channel")
        .clone()
    else {
        return BrowserPreviewTimingSendResult::NoChannel;
    };
    if channel.ready_state() != RTCDataChannelState::Open {
        return BrowserPreviewTimingSendResult::NotOpen;
    }
    let Ok(json) = serde_json::to_string(message) else {
        return BrowserPreviewTimingSendResult::NoChannel;
    };
    tokio::spawn(async move {
        let _ = channel.send_text(json).await;
    });
    BrowserPreviewTimingSendResult::Sent
}

async fn build_peer_connection() -> Result<Arc<RTCPeerConnection>, String> {
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|error| format!("register browser WebRTC codecs failed: {error}"))?;
    let mut interceptor_registry = Registry::new();
    interceptor_registry =
        register_default_interceptors(interceptor_registry, &mut media_engine)
            .map_err(|error| format!("register browser WebRTC interceptors failed: {error}"))?;

    let mut setting_engine = SettingEngine::default();
    setting_engine.set_include_loopback_candidate(true);

    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(interceptor_registry)
        .with_setting_engine(setting_engine)
        .build();

    api.new_peer_connection(RTCConfiguration::default())
        .await
        .map(Arc::new)
        .map_err(|error| format!("create browser WebRTC peer connection failed: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn spawn_local_capture_sender(
    session_id: String,
    fps: u32,
    bitrate_bps: u32,
    width: Option<u32>,
    height: Option<u32>,
    source_id: Option<String>,
    codec: BrowserWebrtcPreviewCodec,
    media_sender: BrowserPreviewMediaSender,
    frame_timing_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    running: Arc<AtomicBool>,
) -> JoinHandle<()> {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        run_local_capture_sender(
            session_id,
            fps,
            bitrate_bps,
            width,
            height,
            source_id,
            codec,
            media_sender,
            frame_timing_channel,
            running,
            handle,
        );
    })
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn run_local_capture_sender(
    session_id: String,
    fps: u32,
    bitrate_bps: u32,
    width: Option<u32>,
    height: Option<u32>,
    source_id: Option<String>,
    codec: BrowserWebrtcPreviewCodec,
    media_sender: BrowserPreviewMediaSender,
    frame_timing_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    running: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
) {
    let mut capture = match open_browser_preview_dxgi_capture(source_id.as_deref()) {
        Ok(capture) => capture,
        Err(error) => {
            warn!("browser WebRTC preview DXGI capture failed for {session_id}: {error}");
            running.store(false, Ordering::Relaxed);
            return;
        }
    };
    let source_width = capture.width();
    let source_height = capture.height();
    let (target_width, target_height) =
        sanitize_browser_preview_target_dimensions(width, height, source_width, source_height);
    capture.set_target_dimensions(target_width, target_height);
    let mut encoder = match BrowserPreviewEncoder::new(
        codec,
        capture.width(),
        capture.height(),
        fps,
        bitrate_bps,
    ) {
        Ok(encoder) => encoder,
        Err(error) => {
            warn!(
                "browser WebRTC preview NVENC {} failed for {session_id}: {error}",
                browser_webrtc_preview_codec_label(codec)
            );
            running.store(false, Ordering::Relaxed);
            return;
        }
    };

    info!(
        "browser WebRTC preview {} sender started for {} at {}x{} @ {} fps / {} Mbps (source_id {}, source {}x{}, track {:?})",
        browser_webrtc_preview_codec_label(codec),
        session_id,
        capture.width(),
        capture.height(),
        fps,
        bitrate_bps / 1_000_000,
        source_id.as_deref().unwrap_or("<primary>"),
        source_width,
        source_height,
        media_sender.track_kind()
    );

    let access_unit_queue = Arc::new(BrowserPreviewFrameQueue::new(
        browser_preview_queue_capacity(fps),
    ));
    let send_running = running.clone();
    let send_session_id = session_id.clone();
    let send_stats = Arc::new(BrowserPreviewSendStats::default());
    let send_stats_task = send_stats.clone();
    let send_queue = access_unit_queue.clone();
    let send_task = handle.spawn(async move {
        let mut media_sender = media_sender;
        let mut frame_sequence = 0u64;
        while send_running.load(Ordering::Relaxed) {
            let Some(access_unit) = send_queue.pop().await else {
                break;
            };
            frame_sequence = frame_sequence.saturating_add(1);
            let send_started = Instant::now();
            match media_sender
                .send_access_unit_with_report(&access_unit)
                .await
            {
                Ok(report) => {
                    let timing_message = BrowserPreviewFrameTimingMessage::new(
                        frame_sequence,
                        &access_unit,
                        now_unix_us_lossy(),
                        report.rtp_timestamp,
                    );
                    match send_browser_frame_timing_message(&frame_timing_channel, &timing_message)
                    {
                        BrowserPreviewTimingSendResult::Sent => {
                            send_stats_task.timing_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        BrowserPreviewTimingSendResult::NoChannel => {
                            send_stats_task
                                .timing_no_channel
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        BrowserPreviewTimingSendResult::NotOpen => {
                            send_stats_task
                                .timing_not_open
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    send_stats_task.sent.fetch_add(1, Ordering::Relaxed);
                    send_stats_task
                        .bytes
                        .fetch_add(report.bytes_written as u64, Ordering::Relaxed);
                    let elapsed = send_started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                    send_stats_task
                        .send_us
                        .fetch_add(elapsed, Ordering::Relaxed);
                    update_atomic_max(&send_stats_task.max_send_us, elapsed);
                    send_stats_task
                        .send_samples_us
                        .lock()
                        .expect("lock browser preview send timing samples")
                        .push(elapsed);
                    decrement_atomic_saturating(&send_stats_task.pending);
                }
                Err(error) => {
                    decrement_atomic_saturating(&send_stats_task.pending);
                    warn!("browser WebRTC preview send failed for {send_session_id}: {error}");
                    send_running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        }
    });

    let frame_interval = Duration::from_nanos(1_000_000_000u64 / fps.max(1) as u64);
    let mut next_frame_at = Instant::now();
    let mut last_report_at = Instant::now();
    let mut frames_encoded = 0u64;
    let mut capture_us = 0u128;
    let mut encode_us = 0u128;
    let mut loop_us = 0u128;
    let mut max_capture_us = 0u128;
    let mut max_encode_us = 0u128;
    let mut max_loop_us = 0u128;
    let mut capture_samples_us = Vec::<u64>::with_capacity((fps.max(1) * 2) as usize);
    let mut encode_samples_us = Vec::<u64>::with_capacity((fps.max(1) * 2) as usize);
    let mut loop_samples_us = Vec::<u64>::with_capacity((fps.max(1) * 2) as usize);
    let mut request_next_keyframe = false;
    while running.load(Ordering::Relaxed) {
        let loop_started = Instant::now();
        let now = Instant::now();
        if now < next_frame_at {
            sleep_until_frame_deadline(next_frame_at);
        } else if now.duration_since(next_frame_at) > frame_interval {
            next_frame_at = now;
        }
        next_frame_at += frame_interval;

        let capture_started = Instant::now();
        let frame = match capture.capture_frame() {
            Ok(frame) => frame,
            Err(error) => {
                warn!("browser WebRTC preview capture failed for {session_id}: {error}");
                running.store(false, Ordering::Relaxed);
                break;
            }
        };
        let capture_elapsed = capture_started.elapsed().as_micros();
        capture_us += capture_elapsed;
        max_capture_us = max_capture_us.max(capture_elapsed);
        capture_samples_us.push(capture_elapsed.min(u64::MAX as u128) as u64);
        if request_next_keyframe {
            encoder.request_keyframe();
            request_next_keyframe = false;
        }
        let encode_started = Instant::now();
        let access_units = match encoder.encode(&frame) {
            Ok(access_units) => access_units,
            Err(error) => {
                warn!("browser WebRTC preview encode failed for {session_id}: {error}");
                running.store(false, Ordering::Relaxed);
                break;
            }
        };
        let encode_elapsed = encode_started.elapsed().as_micros();
        encode_us += encode_elapsed;
        max_encode_us = max_encode_us.max(encode_elapsed);
        encode_samples_us.push(encode_elapsed.min(u64::MAX as u128) as u64);
        frames_encoded += 1;
        for access_unit in access_units {
            if access_unit.codec != encoder.codec() {
                continue;
            }
            let push_result = access_unit_queue.push_latest(access_unit, &send_stats);
            if !push_result.accepted {
                running.store(false, Ordering::Relaxed);
                break;
            }
            if push_result.needs_keyframe {
                request_next_keyframe = true;
            }
        }
        let loop_elapsed = loop_started.elapsed().as_micros();
        loop_us += loop_elapsed;
        max_loop_us = max_loop_us.max(loop_elapsed);
        loop_samples_us.push(loop_elapsed.min(u64::MAX as u128) as u64);
        if last_report_at.elapsed() >= Duration::from_secs(2) {
            let frames = frames_encoded.max(1) as u128;
            let enqueued = send_stats.enqueued.swap(0, Ordering::Relaxed);
            let sent = send_stats.sent.swap(0, Ordering::Relaxed);
            let bytes_sent = send_stats.bytes.swap(0, Ordering::Relaxed);
            let send_us = send_stats.send_us.swap(0, Ordering::Relaxed);
            let max_send_us = send_stats.max_send_us.swap(0, Ordering::Relaxed);
            let dropped_full = send_stats.dropped_full.swap(0, Ordering::Relaxed);
            let dropped_oldest = send_stats.dropped_oldest.swap(0, Ordering::Relaxed);
            let timing_sent = send_stats.timing_sent.swap(0, Ordering::Relaxed);
            let timing_no_channel = send_stats.timing_no_channel.swap(0, Ordering::Relaxed);
            let timing_not_open = send_stats.timing_not_open.swap(0, Ordering::Relaxed);
            let pending = send_stats.pending.load(Ordering::Relaxed);
            let max_pending = send_stats.max_pending.swap(pending, Ordering::Relaxed);
            let capture_summary = summarize_timing_us(&capture_samples_us);
            let encode_summary = summarize_timing_us(&encode_samples_us);
            let send_summary =
                summarize_timing_us(&take_timing_samples(&send_stats.send_samples_us));
            let loop_summary = summarize_timing_us(&loop_samples_us);
            info!(
                "browser WebRTC preview sender progress for {}: encoded={} enqueued={} sent={} pending={} max_pending={} dropped_full={} dropped_oldest={} timing_sent={} timing_no_channel={} timing_not_open={} bytes={} avg_us capture={} encode={} send={} loop={} p50_us capture={} encode={} send={} loop={} p95_us capture={} encode={} send={} loop={} max_us capture={} encode={} send={} loop={} samples capture={} encode={} send={} loop={}",
                session_id,
                frames_encoded,
                enqueued,
                sent,
                pending,
                max_pending,
                dropped_full,
                dropped_oldest,
                timing_sent,
                timing_no_channel,
                timing_not_open,
                bytes_sent,
                capture_us / frames,
                encode_us / frames,
                send_us / sent.max(1),
                loop_us / frames,
                capture_summary.p50_us,
                encode_summary.p50_us,
                send_summary.p50_us,
                loop_summary.p50_us,
                capture_summary.p95_us,
                encode_summary.p95_us,
                send_summary.p95_us,
                loop_summary.p95_us,
                max_capture_us,
                max_encode_us,
                max_send_us,
                max_loop_us,
                capture_summary.samples,
                encode_summary.samples,
                send_summary.samples,
                loop_summary.samples
            );
            last_report_at = Instant::now();
            frames_encoded = 0;
            capture_us = 0;
            encode_us = 0;
            loop_us = 0;
            max_capture_us = 0;
            max_encode_us = 0;
            max_loop_us = 0;
            capture_samples_us.clear();
            encode_samples_us.clear();
            loop_samples_us.clear();
        }
        if !running.load(Ordering::Relaxed) {
            break;
        }
    }
    access_unit_queue.close();
    let _ =
        handle.block_on(async { tokio::time::timeout(Duration::from_secs(1), send_task).await });
}

#[cfg(not(windows))]
fn run_local_capture_sender(
    session_id: String,
    _fps: u32,
    _bitrate_bps: u32,
    _width: Option<u32>,
    _height: Option<u32>,
    _source_id: Option<String>,
    _codec: BrowserWebrtcPreviewCodec,
    _media_sender: BrowserPreviewMediaSender,
    _frame_timing_channel: Arc<Mutex<Option<Arc<RTCDataChannel>>>>,
    running: Arc<AtomicBool>,
    _handle: tokio::runtime::Handle,
) {
    warn!("browser WebRTC preview is currently Windows-only: {session_id}");
    running.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_access_unit(timestamp_us: u64) -> EncodedAccessUnit {
        EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us,
            is_keyframe: timestamp_us == 0,
            bytes: vec![timestamp_us as u8],
        }
    }

    #[test]
    fn browser_preview_frame_timing_message_uses_capture_timestamp() {
        let access_unit = EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us: 1_779_371_954_345_123,
            is_keyframe: true,
            bytes: vec![1, 2, 3],
        };

        let message = BrowserPreviewFrameTimingMessage::new(
            42,
            &access_unit,
            1_779_371_954_350_999,
            Some(123_456),
        );
        let json = serde_json::to_string(&message).expect("timing json");

        assert!(json.contains("\"type\":\"mrd.frame_timing.v1\""));
        assert!(json.contains("\"sequence\":42"));
        assert!(json.contains("\"capture_unix_us\":1779371954345123"));
        assert!(json.contains("\"sent_unix_us\":1779371954350999"));
        assert!(json.contains("\"rtp_timestamp\":123456"));
        assert!(json.contains("\"keyframe\":true"));
    }

    #[test]
    fn browser_webrtc_preview_start_deserializes_source_id() {
        let request: BrowserWebrtcPreviewStartRequest = serde_json::from_str(
            r#"{"session_id":"s1","offer_sdp":"v=0","source_id":"windows:display-shared:1"}"#,
        )
        .unwrap();

        assert_eq!(
            request.source_id.as_deref(),
            Some("windows:display-shared:1")
        );
    }

    #[test]
    fn browser_webrtc_preview_start_deserializes_hevc_codec() {
        let request: BrowserWebrtcPreviewStartRequest =
            serde_json::from_str(r#"{"session_id":"s1","offer_sdp":"v=0","codec":"hevc"}"#)
                .unwrap();

        assert_eq!(request.selected_codec(), BrowserWebrtcPreviewCodec::Hevc);
    }

    #[test]
    fn browser_preview_uses_rtp_track_for_low_latency_browser_video() {
        assert_eq!(
            browser_preview_sender_track_kind(20_000_000),
            BrowserPreviewSenderTrackKind::Rtp
        );
    }

    #[test]
    fn browser_preview_uses_sample_track_for_high_bitrate_browser_video() {
        assert_eq!(
            browser_preview_sender_track_kind(50_000_000),
            BrowserPreviewSenderTrackKind::Sample
        );
    }

    #[test]
    fn browser_preview_hevc_media_sender_uses_rtp_even_at_high_bitrate() {
        let sender = BrowserPreviewMediaSender::new(
            "video",
            "stream",
            120,
            BrowserWebrtcPreviewCodec::Hevc,
            H264Profile::High,
            "640034",
            80_000_000,
        );

        assert!(matches!(sender, BrowserPreviewMediaSender::HevcRtp(_)));
    }

    #[test]
    fn browser_offer_profile_level_id_prefers_matching_h264_profile() {
        let offer = "\
a=fmtp:102 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=fmtp:123 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c1f\r\n";

        assert_eq!(
            select_browser_offer_h264_profile_level_id(offer, H264Profile::Baseline),
            "42e034"
        );
        assert_eq!(
            select_browser_offer_h264_profile_level_id(offer, H264Profile::High),
            "640c34"
        );
    }

    #[test]
    fn browser_preview_target_dimensions_follow_requested_profile() {
        assert_eq!(
            sanitize_browser_preview_target_dimensions(Some(1920), Some(1080), 2560, 1440),
            (1920, 1080)
        );
        assert_eq!(
            sanitize_browser_preview_target_dimensions(Some(4000), Some(3000), 2560, 1440),
            (2560, 1440)
        );
        assert_eq!(
            sanitize_browser_preview_target_dimensions(Some(1919), Some(1079), 2560, 1440),
            (1918, 1078)
        );
        assert_eq!(
            sanitize_browser_preview_target_dimensions(None, Some(1080), 2560, 1440),
            (2560, 1440)
        );
    }

    #[test]
    fn browser_preview_queue_capacity_absorbs_short_send_stalls() {
        assert_eq!(browser_preview_queue_capacity(30), 4);
        assert_eq!(browser_preview_queue_capacity(60), 6);
        assert_eq!(browser_preview_queue_capacity(120), 12);
        assert_eq!(browser_preview_queue_capacity(144), 12);

        let max_buffered_latency_ms = browser_preview_queue_capacity(144) as u32 * 1000 / 144;
        assert!(max_buffered_latency_ms <= 100);
    }

    #[test]
    fn browser_preview_frame_queue_drops_oldest_and_keeps_latest() {
        let queue = BrowserPreviewFrameQueue::new(2);
        let stats = BrowserPreviewSendStats::default();

        assert!(queue.push_latest(test_access_unit(1), &stats).accepted);
        assert!(queue.push_latest(test_access_unit(2), &stats).accepted);
        assert!(queue.push_latest(test_access_unit(0), &stats).accepted);

        assert_eq!(stats.enqueued.load(Ordering::Relaxed), 3);
        assert_eq!(stats.dropped_oldest.load(Ordering::Relaxed), 2);
        assert_eq!(stats.pending.load(Ordering::Relaxed), 1);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let first = runtime.block_on(queue.pop()).expect("first frame");

        assert_eq!(first.timestamp_us, 0);
    }

    #[test]
    fn browser_preview_frame_queue_requests_keyframe_after_delta_overflow() {
        let queue = BrowserPreviewFrameQueue::new(2);
        let stats = BrowserPreviewSendStats::default();

        assert!(queue.push_latest(test_access_unit(1), &stats).queued);
        assert!(queue.push_latest(test_access_unit(2), &stats).queued);
        let result = queue.push_latest(test_access_unit(3), &stats);

        assert!(result.accepted);
        assert!(!result.queued);
        assert!(result.dropped_oldest);
        assert!(result.needs_keyframe);
        assert_eq!(stats.dropped_oldest.load(Ordering::Relaxed), 2);
        assert_eq!(stats.pending.load(Ordering::Relaxed), 0);
        assert_eq!(queue.frames.lock().expect("lock test frame queue").len(), 0);
    }

    #[test]
    fn browser_preview_timing_summary_reports_p50_and_p95() {
        let summary = summarize_timing_us(&[100, 200, 300, 400, 500, 600, 700, 800, 900, 1000]);

        assert_eq!(summary.samples, 10);
        assert_eq!(summary.p50_us, 500);
        assert_eq!(summary.p95_us, 1000);
    }
}
