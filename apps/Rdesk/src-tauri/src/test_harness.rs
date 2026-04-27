//! Test harness for end-to-end pipeline visualization
//!
//! This module provides a test harness that runs the full capture→encode→decode
//! pipeline locally for visualization and testing purposes.

use anyhow::Result;
use mrd_capture_dxgi::DxgiDesktopCapture;
use mrd_decode_nvdec::NvdecDecoder;
use mrd_encode_nvenc::NvencH264Encoder;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{CapturedFrame, FrameCapture, FramePixelFormat, VideoEncoder};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DOWNSAMPLE_MAX_WIDTH: usize = 640;
const FRAME_UPDATE_INTERVAL: usize = 10;

/// Test chain configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestChain {
    /// DXGI capture only, no encode/decode
    #[serde(rename = "capture_only")]
    CaptureOnly,

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
    Custom {
        capture: CaptureType,
        encoder: EncoderType,
        decoder: DecoderType,
    },
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
            Self::CaptureOnly => "DXGI capture only",
            Self::NvencNvdec => "NVENC H.264 + NVDEC (全硬件加速)",
            Self::NvencOnly => "NVENC H.264 编码器测试",
            Self::OpenH264 => "OpenH264 编码器测试 (软件)",
            Self::Custom { .. } => {
                // Build a descriptive name
                "自定义配置"
            }
        }
    }

    pub fn all() -> Vec<TestChain> {
        vec![
            Self::CaptureOnly,
            Self::NvencNvdec,
            Self::NvencOnly,
            Self::OpenH264,
        ]
    }

    pub fn capture_type(&self) -> CaptureType {
        match self {
            Self::CaptureOnly | Self::NvencNvdec | Self::NvencOnly | Self::OpenH264 => {
                CaptureType::Dxgi
            }
            Self::Custom { capture, .. } => capture.clone(),
        }
    }

    pub fn encoder_type(&self) -> EncoderType {
        match self {
            Self::CaptureOnly | Self::NvencNvdec | Self::NvencOnly => EncoderType::NvencH264,
            Self::OpenH264 => EncoderType::OpenH264,
            Self::Custom { encoder, .. } => encoder.clone(),
        }
    }

    pub fn decoder_type(&self) -> DecoderType {
        match self {
            Self::NvencNvdec => DecoderType::Nvdec,
            Self::CaptureOnly | Self::NvencOnly | Self::OpenH264 => DecoderType::Software,
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
    pub capture_latency_p50_ms: f64,
    pub capture_latency_p95_ms: f64,
    pub encode_latency_p50_ms: f64,
    pub encode_latency_p95_ms: f64,
    pub decode_latency_p50_ms: f64,
    pub decode_latency_p95_ms: f64,
    pub total_latency_p50_ms: f64,
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
            capture_latency_p50_ms: 0.0,
            capture_latency_p95_ms: 0.0,
            encode_latency_p50_ms: 0.0,
            encode_latency_p95_ms: 0.0,
            decode_latency_p50_ms: 0.0,
            decode_latency_p95_ms: 0.0,
            total_latency_p50_ms: 0.0,
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
    encoder: Option<Box<dyn VideoEncoder>>,
    decoder: Option<NvdecDecoder>,
    use_decoder: bool,
    width: usize,
    height: usize,
}

pub struct TestHarness {
    running: Arc<AtomicBool>,
    chain: TestChain,
    config: TestConfig,
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
            config: TestConfig::default(),
            metrics,
            frame_buffer,
            thread_handle: None,
        })
    }

    pub fn set_chain(&mut self, chain: TestChain) {
        self.chain = chain;
    }

    pub fn set_config(&mut self, config: TestConfig) {
        self.config = config;
    }

    pub fn get_chain(&self) -> TestChain {
        self.chain.clone()
    }

    pub fn start(&mut self) -> Result<()> {
        if self.running.load(Ordering::Relaxed) {
            anyhow::bail!("test harness is already running");
        }

        let chain = self.chain.clone();
        let config = self.config.clone();
        let frame_buffer = self.frame_buffer.clone();
        let metrics = self.metrics.clone();
        let running = self.running.clone();
        let running_for_thread = running.clone();
        let (init_tx, init_rx) = mpsc::channel();

        running.store(true, Ordering::Relaxed);

        let handle = thread::spawn(move || {
            Self::run_pipeline(
                frame_buffer,
                metrics,
                running_for_thread,
                chain,
                config,
                init_tx,
            );
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
        config: TestConfig,
        init_tx: mpsc::Sender<Result<()>>,
    ) {
        let state = match Self::initialize_components(&chain, &config) {
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

    fn initialize_components(chain: &TestChain, config: &TestConfig) -> Result<PipelineState> {
        let capture = DxgiDesktopCapture::new_primary()
            .map_err(|e| anyhow::anyhow!("DXGI 捕获初始化失败: {:?}", e))?;
        let (width, height) = select_pipeline_dimensions(capture.width(), capture.height(), config);
        let fps = config.fps.unwrap_or(60).max(1);

        let (encoder, decoder, use_decoder) = match chain {
            TestChain::CaptureOnly => (None, None, false),
            TestChain::NvencNvdec => {
                let encoder = NvencH264Encoder::new(width, height, fps)
                    .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                let decoder = NvdecDecoder::new()
                    .map_err(|e| anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e))?;
                (
                    Some(Box::new(encoder) as Box<dyn VideoEncoder>),
                    Some(decoder),
                    true,
                )
            }
            TestChain::NvencOnly => {
                let encoder = NvencH264Encoder::new_max_speed(width, height, fps)
                    .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                (
                    Some(Box::new(encoder) as Box<dyn VideoEncoder>),
                    None,
                    false,
                )
            }
            TestChain::OpenH264 => {
                let encoder = OpenH264Encoder::new(width, height, fps)
                    .map_err(|e| anyhow::anyhow!("OpenH264 编码器初始化失败: {:?}", e))?;
                (
                    Some(Box::new(encoder) as Box<dyn VideoEncoder>),
                    None,
                    false,
                )
            }
            TestChain::Custom {
                capture: _,
                encoder,
                decoder,
            } => {
                // For now, Custom configurations fall back to standard implementations
                // TODO: Implement WinRT capture, AV1 encoding, software decoding
                match encoder {
                    EncoderType::NvencH264 => match decoder {
                        DecoderType::Nvdec => {
                            let enc = NvencH264Encoder::new(width, height, fps)
                                .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                            let dec = NvdecDecoder::new()
                                .map_err(|e| anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e))?;
                            (
                                Some(Box::new(enc) as Box<dyn VideoEncoder>),
                                Some(dec),
                                true,
                            )
                        }
                        DecoderType::Software => {
                            let enc = NvencH264Encoder::new_max_speed(width, height, fps)
                                .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                            (Some(Box::new(enc) as Box<dyn VideoEncoder>), None, false)
                        }
                    },
                    EncoderType::OpenH264 => {
                        let enc = OpenH264Encoder::new(width, height, fps)
                            .map_err(|e| anyhow::anyhow!("OpenH264 编码器初始化失败: {:?}", e))?;
                        (Some(Box::new(enc) as Box<dyn VideoEncoder>), None, false)
                    }
                    EncoderType::NvencAv1 => {
                        return Err(anyhow::anyhow!("AV1 编码器尚未实现"));
                    }
                }
            }
        };

        Ok(PipelineState {
            capture,
            encoder,
            decoder,
            use_decoder,
            width,
            height,
        })
    }

    fn process_loop(
        mut state: PipelineState,
        frame_buffer: Arc<Mutex<FrameBuffer>>,
        metrics: Arc<Mutex<HarnessMetrics>>,
        running: Arc<AtomicBool>,
    ) {
        let start_time = Instant::now();
        let mut capture_latencies = Vec::with_capacity(1000);
        let mut encode_latencies = Vec::with_capacity(1000);
        let mut decode_latencies = Vec::with_capacity(1000);
        let mut total_latencies = Vec::with_capacity(1000);
        let mut frame_count = 0_usize;
        let mut dropped_frames = 0_usize;

        while running.load(Ordering::Relaxed) {
            let pipeline_start = Instant::now();

            let capture_start = Instant::now();
            let captured_frame = match state.capture.capture_frame() {
                Ok(frame) => frame,
                Err(_) => {
                    dropped_frames += 1;
                    thread::sleep(Duration::from_millis(1));
                    continue;
                }
            };
            let capture_latency = capture_start.elapsed();

            let (encoded_units, encode_latency) = if let Some(encoder) = state.encoder.as_mut() {
                let prepared_frame;
                let frame_for_encode = if captured_frame.width == state.width
                    && captured_frame.height == state.height
                {
                    &captured_frame
                } else {
                    prepared_frame =
                        adapt_frame_dimensions(&captured_frame, state.width, state.height);
                    &prepared_frame
                };
                let encode_start = Instant::now();
                let encoded_units = match encoder.encode(frame_for_encode) {
                    Ok(units) => units,
                    Err(_) => {
                        dropped_frames += 1;
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                };
                (encoded_units, Some(encode_start.elapsed()))
            } else {
                (Vec::new(), None)
            };

            // Decode if needed
            let decode_latency = if state.use_decoder && !encoded_units.is_empty() {
                if let Some(decoder) = state.decoder.as_mut() {
                    let decode_start = Instant::now();
                    let mut pushed_any = false;
                    let mut failed_units = 0_usize;
                    for unit in &encoded_units {
                        match decoder.push_access_unit(&unit.bytes) {
                            Ok(()) => {
                                pushed_any = true;
                                if !decoder.drain_decoded_frames().is_empty() {
                                    break;
                                }
                            }
                            Err(_) => {
                                failed_units += 1;
                            }
                        }
                    }

                    if !pushed_any {
                        if failed_units > 0 {
                            dropped_frames += 1;
                            Some(decode_start.elapsed())
                        } else {
                            None
                        }
                    } else {
                        Some(decode_start.elapsed())
                    }
                } else {
                    None
                }
            } else {
                None
            };

            capture_latencies.push(capture_latency);
            if let Some(latency) = encode_latency {
                encode_latencies.push(latency);
            }
            if let Some(latency) = decode_latency {
                decode_latencies.push(latency);
            }
            total_latencies.push(pipeline_start.elapsed());

            Self::trim_latency_buffers(
                &mut capture_latencies,
                &mut encode_latencies,
                &mut decode_latencies,
                &mut total_latencies,
            );

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
                    &capture_latencies,
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
            &capture_latencies,
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
        capture_latencies: &[Duration],
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

        let (p50_cap, p95_cap) = Self::compute_percentiles(capture_latencies);
        let (p50_enc, p95_enc) = Self::compute_percentiles(encode_latencies);
        let (p50_dec, p95_dec) = Self::compute_percentiles(decode_latencies);
        let (p50_total, p95_total) = Self::compute_percentiles(total_latencies);

        let mut m = metrics.lock().unwrap();
        m.capture_fps = fps;
        m.frame_count = frame_count;
        m.dropped_frames = dropped_frames;
        m.capture_latency_p50_ms = p50_cap.as_secs_f64() * 1000.0;
        m.capture_latency_p95_ms = p95_cap.as_secs_f64() * 1000.0;
        m.encode_latency_p50_ms = p50_enc.as_secs_f64() * 1000.0;
        m.encode_latency_p95_ms = p95_enc.as_secs_f64() * 1000.0;
        m.decode_latency_p50_ms = p50_dec.as_secs_f64() * 1000.0;
        m.decode_latency_p95_ms = p95_dec.as_secs_f64() * 1000.0;
        m.total_latency_p50_ms = p50_total.as_secs_f64() * 1000.0;
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

    fn trim_latency_buffers(
        capture_latencies: &mut Vec<Duration>,
        encode_latencies: &mut Vec<Duration>,
        decode_latencies: &mut Vec<Duration>,
        total_latencies: &mut Vec<Duration>,
    ) {
        if capture_latencies.len() > 1000 {
            capture_latencies.remove(0);
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
    }

    pub fn stop(&mut self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);

        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        {
            let mut m = self.metrics.lock().unwrap();
            m.is_running = false;
        }

        Ok(())
    }

    pub fn get_metrics(&self) -> HarnessMetrics {
        self.metrics.lock().unwrap().clone()
    }

    pub fn get_latest_frames(
        &self,
    ) -> (
        Option<(Vec<u8>, usize, usize)>,
        Option<(Vec<u8>, usize, usize)>,
    ) {
        let buf = self.frame_buffer.lock().unwrap();
        let captured = buf
            .captured
            .as_ref()
            .map(|data| (data.clone(), buf.width, buf.height));
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

fn select_pipeline_dimensions(
    capture_width: usize,
    capture_height: usize,
    config: &TestConfig,
) -> (usize, usize) {
    let (width, height) = config.resolution.unwrap_or((capture_width, capture_height));

    (even_dimension(width), even_dimension(height))
}

fn even_dimension(value: usize) -> usize {
    let value = value.max(2);
    if value % 2 == 0 {
        value
    } else {
        value - 1
    }
}

fn resize_frame_nearest(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
) -> CapturedFrame {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);
    let mut data = vec![0_u8; target_width * target_height * bytes_per_pixel];

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        return CapturedFrame {
            width: target_width,
            height: target_height,
            pixel_format: frame.pixel_format,
            timestamp_us: frame.timestamp_us,
            data,
        };
    }

    for y in 0..target_height {
        let src_y = (y * frame.height / target_height).min(frame.height.saturating_sub(1));
        for x in 0..target_width {
            let src_x = (x * frame.width / target_width).min(frame.width.saturating_sub(1));
            let src_idx = (src_y * frame.width + src_x) * bytes_per_pixel;
            let dst_idx = (y * target_width + x) * bytes_per_pixel;
            if src_idx + bytes_per_pixel <= frame.data.len()
                && dst_idx + bytes_per_pixel <= data.len()
            {
                data[dst_idx..dst_idx + bytes_per_pixel]
                    .copy_from_slice(&frame.data[src_idx..src_idx + bytes_per_pixel]);
            }
        }
    }

    CapturedFrame {
        width: target_width,
        height: target_height,
        pixel_format: frame.pixel_format,
        timestamp_us: frame.timestamp_us,
        data,
    }
}

fn adapt_frame_dimensions(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
) -> CapturedFrame {
    if target_width <= frame.width && target_height <= frame.height {
        crop_frame_center(frame, target_width, target_height)
    } else {
        resize_frame_nearest(frame, target_width, target_height)
    }
}

fn crop_frame_center(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
) -> CapturedFrame {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);
    let mut data = vec![0_u8; target_width * target_height * bytes_per_pixel];

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        return CapturedFrame {
            width: target_width,
            height: target_height,
            pixel_format: frame.pixel_format,
            timestamp_us: frame.timestamp_us,
            data,
        };
    }

    let src_x = frame.width.saturating_sub(target_width) / 2;
    let src_y = frame.height.saturating_sub(target_height) / 2;
    let row_bytes = target_width * bytes_per_pixel;

    for y in 0..target_height {
        let src_idx = ((src_y + y) * frame.width + src_x) * bytes_per_pixel;
        let dst_idx = y * row_bytes;
        if src_idx + row_bytes <= frame.data.len() && dst_idx + row_bytes <= data.len() {
            data[dst_idx..dst_idx + row_bytes]
                .copy_from_slice(&frame.data[src_idx..src_idx + row_bytes]);
        }
    }

    CapturedFrame {
        width: target_width,
        height: target_height,
        pixel_format: frame.pixel_format,
        timestamp_us: frame.timestamp_us,
        data,
    }
}

fn bytes_per_pixel(format: FramePixelFormat) -> usize {
    match format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_latency_buffers_handles_encode_only_samples() {
        let mut capture_latencies = Vec::new();
        let mut encode_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut decode_latencies = Vec::new();
        let mut total_latencies = Vec::new();

        TestHarness::trim_latency_buffers(
            &mut capture_latencies,
            &mut encode_latencies,
            &mut decode_latencies,
            &mut total_latencies,
        );

        assert!(capture_latencies.is_empty());
        assert_eq!(encode_latencies.len(), 1000);
        assert_eq!(encode_latencies[0], Duration::from_millis(1));
        assert!(decode_latencies.is_empty());
        assert!(total_latencies.is_empty());
    }

    #[test]
    fn trim_latency_buffers_trims_each_populated_series_independently() {
        let mut capture_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut encode_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut decode_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut total_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();

        TestHarness::trim_latency_buffers(
            &mut capture_latencies,
            &mut encode_latencies,
            &mut decode_latencies,
            &mut total_latencies,
        );

        assert_eq!(capture_latencies.len(), 1000);
        assert_eq!(encode_latencies.len(), 1000);
        assert_eq!(decode_latencies.len(), 1000);
        assert_eq!(total_latencies.len(), 1000);
        assert_eq!(capture_latencies[0], Duration::from_millis(1));
        assert_eq!(encode_latencies[0], Duration::from_millis(1));
        assert_eq!(decode_latencies[0], Duration::from_millis(1));
        assert_eq!(total_latencies[0], Duration::from_millis(1));
    }

    #[test]
    fn stop_preserves_last_metrics_snapshot() {
        let mut harness = TestHarness::new().expect("create harness");
        {
            let mut metrics = harness.metrics.lock().unwrap();
            metrics.is_running = true;
            metrics.capture_fps = 12.5;
            metrics.frame_count = 7;
        }

        harness.stop().expect("stop harness");

        let metrics = harness.get_metrics();
        assert!(!metrics.is_running);
        assert_eq!(metrics.capture_fps, 12.5);
        assert_eq!(metrics.frame_count, 7);
    }

    #[test]
    fn select_pipeline_dimensions_rounds_to_even_values() {
        let config = TestConfig::default();
        assert_eq!(
            select_pipeline_dimensions(1707, 1067, &config),
            (1706, 1066)
        );

        let config = TestConfig {
            resolution: Some((1921, 1081)),
            ..Default::default()
        };
        assert_eq!(
            select_pipeline_dimensions(1707, 1067, &config),
            (1920, 1080)
        );
    }

    #[test]
    fn resize_frame_nearest_outputs_requested_shape() {
        let frame = CapturedFrame {
            width: 3,
            height: 2,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: 42,
            data: vec![
                1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
            ],
        };

        let resized = resize_frame_nearest(&frame, 2, 2);

        assert_eq!(resized.width, 2);
        assert_eq!(resized.height, 2);
        assert_eq!(resized.timestamp_us, 42);
        assert_eq!(resized.data.len(), 2 * 2 * 4);
        assert_eq!(resized.data[0], 1);
        assert_eq!(resized.data[4], 2);
        assert_eq!(resized.data[8], 4);
        assert_eq!(resized.data[12], 5);
    }

    #[test]
    fn adapt_frame_dimensions_crops_when_target_fits_source() {
        let pixels = (1_u8..=12)
            .flat_map(|value| [value, 0, 0, 255])
            .collect::<Vec<_>>();
        let frame = CapturedFrame {
            width: 4,
            height: 3,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: 99,
            data: pixels,
        };

        let cropped = adapt_frame_dimensions(&frame, 2, 2);

        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        assert_eq!(cropped.timestamp_us, 99);
        assert_eq!(cropped.data.len(), 2 * 2 * 4);
        assert_eq!(cropped.data[0], 2);
        assert_eq!(cropped.data[4], 3);
        assert_eq!(cropped.data[8], 6);
        assert_eq!(cropped.data[12], 7);
    }

    #[test]
    #[ignore = "manual perf probe: requires DXGI, NVENC, and NVDEC on the host"]
    fn nvenc_nvdec_harness_prints_stage_metrics() {
        let seconds = std::env::var("MRD_HARNESS_PROBE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5);
        let chain = match std::env::var("MRD_HARNESS_CHAIN").as_deref() {
            Ok("capture_only") => TestChain::CaptureOnly,
            Ok("nvenc_only") => TestChain::NvencOnly,
            Ok("openh264") => TestChain::OpenH264,
            _ => TestChain::NvencNvdec,
        };
        let mut harness = TestHarness::new().expect("create harness");
        harness.set_chain(chain);
        harness.set_config(TestConfig {
            resolution: match (
                std::env::var("MRD_HARNESS_WIDTH")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok()),
                std::env::var("MRD_HARNESS_HEIGHT")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok()),
            ) {
                (Some(width), Some(height)) => Some((width, height)),
                _ => None,
            },
            fps: std::env::var("MRD_HARNESS_FPS")
                .ok()
                .and_then(|value| value.parse::<u32>().ok()),
            bitrate: None,
        });
        harness.start().expect("start harness");
        thread::sleep(Duration::from_secs(seconds));
        harness.stop().expect("stop harness");
        let metrics = harness.get_metrics();
        println!("{metrics:#?}");
        assert!(metrics.frame_count > 0);
    }
}
