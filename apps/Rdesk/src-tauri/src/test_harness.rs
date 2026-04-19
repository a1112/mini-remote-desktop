//! Test harness for end-to-end pipeline visualization
//!
//! This module provides a test harness that runs the full capture→encode→decode
//! pipeline locally for visualization and testing purposes.

use anyhow::Result;
use mrd_capture_dxgi::DxgiDesktopCapture;
use mrd_encode_nvenc::NvencH264Encoder;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_decode_nvdec::NvdecDecoder;
use mrd_pipeline_core::{CapturedFrame, FrameCapture, VideoEncoder};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};

const DOWNSAMPLE_MAX_WIDTH: usize = 640;
const FRAME_UPDATE_INTERVAL: usize = 10;

/// Test chain configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestChain {
    /// DXGI capture + NVENC encode + NVDEC decode (fastest, full hardware)
    #[serde(rename = "nvenc_nvdec")]
    NvencNvdec,

    /// DXGI capture + NVENC encode (encode-only test)
    #[serde(rename = "nvenc_only")]
    NvencOnly,

    /// DXGI capture + OpenH264 encode (software encode test)
    #[serde(rename = "openh264")]
    OpenH264,

    /// Custom configuration with explicit parameters
    #[serde(rename = "custom")]
    Custom { capture: CaptureType, encoder: EncoderType, decoder: DecoderType },
}

/// Available capture types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureType {
    Dxgi,
    Winrt,
    Synthetic,
}

/// Available encoder types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncoderType {
    NvencH264,
    NvencAv1,
    OpenH264,
}

/// Available decoder types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecoderType {
    Nvdec,
    Software,
}

/// Test configuration parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    pub resolution: Option<(usize, usize)>,
    pub fps: Option<u32>,
    pub bitrate: Option<u32>,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            resolution: None,
            fps: None,
            bitrate: None,
        }
    }
}

impl TestChain {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::NvencNvdec => "NVENC H.264 + NVDEC (全硬件加速)",
            Self::NvencOnly => "NVENC H.264 编码器测试",
            Self::OpenH264 => "OpenH264 编码器测试 (软件)",
            Self::Custom { capture, encoder, decoder } => {
                // Build a descriptive name
                "自定义配置"
            }
        }
    }

    pub fn all() -> Vec<TestChain> {
        vec![
            Self::NvencNvdec,
            Self::NvencOnly,
            Self::OpenH264,
        ]
    }

    pub fn capture_type(&self) -> CaptureType {
        match self {
            Self::NvencNvdec | Self::NvencOnly | Self::OpenH264 => CaptureType::Dxgi,
            Self::Custom { capture, .. } => capture.clone(),
        }
    }

    pub fn encoder_type(&self) -> EncoderType {
        match self {
            Self::NvencNvdec | Self::NvencOnly => EncoderType::NvencH264,
            Self::OpenH264 => EncoderType::OpenH264,
            Self::Custom { encoder, .. } => encoder.clone(),
        }
    }

    pub fn decoder_type(&self) -> DecoderType {
        match self {
            Self::NvencNvdec => DecoderType::Nvdec,
            Self::NvencOnly | Self::OpenH264 => DecoderType::Software,
            Self::Custom { decoder, .. } => decoder.clone(),
        }
    }
}

impl Default for TestChain {
    fn default() -> Self {
        Self::NvencNvdec
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessMetrics {
    pub is_running: bool,
    pub capture_fps: f64,
    pub encode_latency_p50_ms: f64,
    pub encode_latency_p95_ms: f64,
    pub decode_latency_p50_ms: f64,
    pub decode_latency_p95_ms: f64,
    pub total_latency_p95_ms: f64,
    pub frame_count: usize,
    pub dropped_frames: usize,
    pub resolution: (usize, usize),
    pub error_message: Option<String>,
}

impl Default for HarnessMetrics {
    fn default() -> Self {
        Self {
            is_running: false,
            capture_fps: 0.0,
            encode_latency_p50_ms: 0.0,
            encode_latency_p95_ms: 0.0,
            decode_latency_p50_ms: 0.0,
            decode_latency_p95_ms: 0.0,
            total_latency_p95_ms: 0.0,
            frame_count: 0,
            dropped_frames: 0,
            resolution: (0, 0),
            error_message: None,
        }
    }
}

struct FrameBuffer {
    captured: Option<Vec<u8>>,
    width: usize,
    height: usize,
}

// Pipeline state - defined outside impl
struct PipelineState {
    capture: DxgiDesktopCapture,
    encoder: Box<dyn VideoEncoder>,
    decoder: Option<NvdecDecoder>,
    use_decoder: bool,
    width: usize,
    height: usize,
}

pub struct TestHarness {
    running: Arc<AtomicBool>,
    chain: TestChain,
    metrics: Arc<Mutex<HarnessMetrics>>,
    frame_buffer: Arc<Mutex<FrameBuffer>>,
    thread_handle: Option<thread::JoinHandle<()>>,
}

unsafe impl Send for TestHarness {}

impl TestHarness {
    pub fn new() -> Result<Self> {
        let frame_buffer = Arc::new(Mutex::new(FrameBuffer {
            captured: None,
            width: 0,
            height: 0,
        }));

        let metrics = Arc::new(Mutex::new(HarnessMetrics::default()));
        let running = Arc::new(AtomicBool::new(false));

        Ok(Self {
            running,
            chain: TestChain::default(),
            metrics,
            frame_buffer,
            thread_handle: None,
        })
    }

    pub fn set_chain(&mut self, chain: TestChain) {
        self.chain = chain;
    }

    pub fn get_chain(&self) -> TestChain {
        self.chain.clone()
    }

    pub fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            anyhow::bail!("test harness is already running");
        }

        let chain = self.chain.clone();
        let frame_buffer = self.frame_buffer.clone();
        let metrics = self.metrics.clone();
        let running = self.running.clone();
        let running_for_thread = running.clone();
        let (init_tx, init_rx) = mpsc::channel();

        running.store(true, Ordering::Relaxed);

        let handle = thread::spawn(move || {
            Self::run_pipeline(frame_buffer, metrics, running_for_thread, chain, init_tx);
        });

        match init_rx.recv() {
            Ok(Ok(())) => {
                self.thread_handle = Some(handle);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(error) => {
                running.store(false, Ordering::Relaxed);
                let _ = handle.join();
                anyhow::bail!("test harness initialization channel closed: {}", error);
            }
        }
    }

    fn run_pipeline(
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        metrics: Arc<Mutex<HarnessMetrics>>,
        running: Arc<AtomicBool>,
        chain: TestChain,
        init_tx: mpsc::Sender<Result<()>>,
    ) {
        let state = match Self::initialize_components(&chain) {
            Ok(s) => s,
            Err(e) => {
                let message = e.to_string();
                let mut m = metrics.lock().unwrap();
                m.is_running = false;
                m.error_message = Some(message.clone());
                running.store(false, Ordering::Relaxed);
                let _ = init_tx.send(Err(anyhow::anyhow!(message)));
                return;
            }
        };

        let (width, height) = (state.width, state.height);

        {
            let mut m = metrics.lock().unwrap();
            m.is_running = true;
            m.resolution = (width, height);
            m.error_message = None;
        }

        let _ = init_tx.send(Ok(()));

        Self::process_loop(state, frame_buffer, metrics, running);
    }

    fn initialize_components(chain: &TestChain) -> Result<PipelineState> {
        let capture = DxgiDesktopCapture::new_primary()
            .map_err(|e| anyhow::anyhow!("DXGI 捕获初始化失败: {:?}", e))?;
        let width = capture.width();
        let height = capture.height();

        let (encoder, decoder, use_decoder) = match chain {
            TestChain::NvencNvdec => {
                let encoder = NvencH264Encoder::new_max_speed(width, height, 60)
                    .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                let decoder = NvdecDecoder::new()
                    .map_err(|e| anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e))?;
                (Box::new(encoder) as Box<dyn VideoEncoder>, Some(decoder), true)
            }
            TestChain::NvencOnly => {
                let encoder = NvencH264Encoder::new_max_speed(width, height, 60)
                    .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                (Box::new(encoder) as Box<dyn VideoEncoder>, None, false)
            }
            TestChain::OpenH264 => {
                let encoder = OpenH264Encoder::new(width, height, 60)
                    .map_err(|e| anyhow::anyhow!("OpenH264 编码器初始化失败: {:?}", e))?;
                (Box::new(encoder) as Box<dyn VideoEncoder>, None, false)
            }
            TestChain::Custom { capture: _, encoder, decoder } => {
                // For now, Custom configurations fall back to standard implementations
                // TODO: Implement WinRT capture, AV1 encoding, software decoding
                match encoder {
                    EncoderType::NvencH264 => {
                        let enc = NvencH264Encoder::new_max_speed(width, height, 60)
                            .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                        match decoder {
                            DecoderType::Nvdec => {
                                let dec = NvdecDecoder::new()
                                    .map_err(|e| anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e))?;
                                (Box::new(enc) as Box<dyn VideoEncoder>, Some(dec), true)
                            }
                            DecoderType::Software => {
                                (Box::new(enc) as Box<dyn VideoEncoder>, None, false)
                            }
                        }
                    }
                    EncoderType::OpenH264 => {
                        let enc = OpenH264Encoder::new(width, height, 60)
                            .map_err(|e| anyhow::anyhow!("OpenH264 编码器初始化失败: {:?}", e))?;
                        (Box::new(enc) as Box<dyn VideoEncoder>, None, false)
                    }
                    EncoderType::NvencAv1 => {
                        return Err(anyhow::anyhow!("AV1 编码器尚未实现"));
                    }
                }
            }
        };

        Ok(PipelineState { capture, encoder, decoder, use_decoder, width, height })
    }

    fn process_loop(
        mut state: PipelineState,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        metrics: Arc<Mutex<HarnessMetrics>>,
        running: Arc<AtomicBool>,
    ) {
        let start_time = Instant::now();
        let mut encode_latencies = Vec::with_capacity(1000);
        let mut decode_latencies = Vec::with_capacity(1000);
        let mut total_latencies = Vec::with_capacity(1000);
        let mut frame_count = 0_usize;
        let mut dropped_frames = 0_usize;

        while running.load(Ordering::Relaxed) {
            let pipeline_start = Instant::now();

            let captured_frame = match state.capture.capture_frame() {
                Ok(frame) => frame,
                Err(_) => {
                    dropped_frames += 1;
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };

            let encode_start = Instant::now();
            let encoded_units = match state.encoder.encode(&captured_frame) {
                Ok(units) => units,
                Err(_) => {
                    dropped_frames += 1;
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };
            let encode_latency = encode_start.elapsed();

            // Decode if needed
            let decode_latency = if state.use_decoder {
                if let Some(ref mut decoder) = state.decoder.as_mut() {
                    let decode_start = Instant::now();
                    let mut got_frame = false;
                    for unit in &encoded_units {
                        if decoder.push_access_unit(&unit.bytes).is_ok() {
                            if !decoder.drain_decoded_frames().is_empty() {
                                got_frame = true;
                                break;
                            }
                        }
                    }
                    if got_frame {
                        Some(decode_start.elapsed())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            encode_latencies.push(encode_latency);
            if let Some(latency) = decode_latency {
                decode_latencies.push(latency);
                total_latencies.push(pipeline_start.elapsed());
            }

            if encode_latencies.len() > 1000 {
                encode_latencies.remove(0);
            }
            if decode_latencies.len() > 1000 {
                decode_latencies.remove(0);
            }
            if total_latencies.len() > 1000 {
                total_latencies.remove(0);
            }

            frame_count += 1;

            if frame_count % FRAME_UPDATE_INTERVAL == 0 {
                if let Ok((captured_ds, ds_width, ds_height)) =
                    downsample_frame(&captured_frame, DOWNSAMPLE_MAX_WIDTH)
                {
                    let mut buf = frame_buffer.lock().unwrap();
                    buf.captured = Some(captured_ds);
                    buf.width = ds_width;
                    buf.height = ds_height;
                }
            }

            if frame_count % 30 == 0 {
                Self::update_metrics(
                    &metrics,
                    frame_count,
                    dropped_frames,
                    &start_time,
                    &encode_latencies,
                    &decode_latencies,
                    &total_latencies,
                );
            }
        }

        Self::update_metrics(
            &metrics,
            frame_count,
            dropped_frames,
            &start_time,
            &encode_latencies,
            &decode_latencies,
            &total_latencies,
        );

        let mut m = metrics.lock().unwrap();
        m.is_running = false;
    }

    fn update_metrics(
        metrics: &Arc<Mutex<HarnessMetrics>>,
        frame_count: usize,
        dropped_frames: usize,
        start_time: &Instant,
        encode_latencies: &[Duration],
        decode_latencies: &[Duration],
        total_latencies: &[Duration],
    ) {
        let elapsed = start_time.elapsed().as_secs_f64();
        let fps = if elapsed > 0.0 {
            frame_count as f64 / elapsed
        } else {
            0.0
        };

        let (p50_enc, p95_enc) = Self::compute_percentiles(encode_latencies);
        let (p50_dec, p95_dec) = Self::compute_percentiles(decode_latencies);
        let (_, p95_total) = Self::compute_percentiles(total_latencies);

        let mut m = metrics.lock().unwrap();
        m.capture_fps = fps;
        m.frame_count = frame_count;
        m.dropped_frames = dropped_frames;
        m.encode_latency_p50_ms = p50_enc.as_secs_f64() * 1000.0;
        m.encode_latency_p95_ms = p95_enc.as_secs_f64() * 1000.0;
        m.decode_latency_p50_ms = p50_dec.as_secs_f64() * 1000.0;
        m.decode_latency_p95_ms = p95_dec.as_secs_f64() * 1000.0;
        m.total_latency_p95_ms = p95_total.as_secs_f64() * 1000.0;
    }

    fn compute_percentiles(latencies: &[Duration]) -> (Duration, Duration) {
        if latencies.is_empty() {
            return (Duration::ZERO, Duration::ZERO);
        }
        let mut sorted = latencies.to_vec();
        sorted.sort_by_key(|d| d.as_nanos());
        let p50_idx = sorted.len() / 2;
        let p95_idx = ((sorted.len() * 95) / 100).min(sorted.len().saturating_sub(1));
        (sorted[p50_idx], sorted[p95_idx])
    }

    pub fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        {
            let mut m = self.metrics.lock().unwrap();
            m.is_running = false;
            m.capture_fps = 0.0;
        }

        Ok(())
    }

    pub fn get_metrics(&self) -> HarnessMetrics {
        self.metrics.lock().unwrap().clone()
    }

    pub fn get_latest_frames(&self) -> (Option<(Vec<u8>, usize, usize)>, Option<(Vec<u8>, usize, usize)>) {
        let buf = self.frame_buffer.lock().unwrap();
        let captured = buf.captured.as_ref().map(|data| (data.clone(), buf.width, buf.height));
        (captured, None)
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn downsample_frame(frame: &CapturedFrame, max_width: usize) -> Result<(Vec<u8>, usize, usize)> {
    let (width, height) = (frame.width, frame.height);
    let scale = if width > max_width {
        max_width as f32 / width as f32
    } else {
        1.0_f32
    };

    if scale >= 1.0 {
        return Ok((frame.data.clone(), width, height));
    }

    let new_width = (width as f32 * scale) as usize;
    let new_height = (height as f32 * scale) as usize;

    let mut result = vec![0u8; new_width * new_height * 4];

    for y in 0..new_height {
        for x in 0..new_width {
            let src_x = ((x as f32) / scale) as usize;
            let src_y = ((y as f32) / scale) as usize;
            let src_idx = (src_y * width + src_x) * 4;
            let dst_idx = (y * new_width + x) * 4;

            if src_idx + 3 < frame.data.len() && dst_idx + 3 < result.len() {
                result[dst_idx..dst_idx + 4].copy_from_slice(&frame.data[src_idx..src_idx + 4]);
            }
        }
    }

    Ok((result, new_width, new_height))
}
