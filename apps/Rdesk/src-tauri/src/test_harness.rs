//! Test harness for end-to-end pipeline visualization
//!
//! This module provides a test harness that runs the full capture→encode→decode
//! pipeline locally for visualization and testing purposes.

use anyhow::Result;
use mrd_capture_dxgi::DxgiDesktopCapture;
#[cfg(windows)]
use mrd_capture_dxgi::DxgiSharedTextureCapture;
use mrd_decode_nvdec::{NvdecDecoder, NvdecOutputMode};
use mrd_encode_nvenc::NvencH264Encoder;
#[cfg(windows)]
use mrd_encode_nvenc_av1::NvencAv1Encoder;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame, DecodedFrameData, EncodedAccessUnit, FrameCapture,
    FramePixelFormat, VideoCodec, VideoDecoder, VideoEncoder,
};
use mrd_render::{RenderFrame, RenderTarget, RendererFactory, RendererInstance};
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
    None,
    Nvdec,
    Software,
}

/// Available renderer types for live test display.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RendererType {
    D3d11,
}

/// Available transport test paths for encoded access units.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Loopback,
    WebrtcRtp,
    QuicDatagram,
}

/// Test configuration parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestConfig {
    pub resolution: Option<(usize, usize)>,
    pub fps: Option<u32>,
    pub bitrate: Option<u32>,
    pub renderer: Option<RendererType>,
    pub transport: Option<TransportKind>,
    pub zero_copy: Option<bool>,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            resolution: None,
            fps: None,
            bitrate: None,
            renderer: None,
            transport: None,
            zero_copy: None,
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
            Self::CaptureOnly | Self::NvencOnly | Self::OpenH264 => DecoderType::None,
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
    pub transport_latency_p50_ms: f64,
    pub transport_latency_p95_ms: f64,
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
            transport_latency_p50_ms: 0.0,
            transport_latency_p95_ms: 0.0,
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
    capture: Box<dyn FrameCapture>,
    encoder: Option<Box<dyn VideoEncoder>>,
    transport: PipelineTransport,
    decoder: Option<PipelineDecoder>,
    renderer: Option<PipelineRenderer>,
    use_decoder: bool,
    width: usize,
    height: usize,
    adapted_frame: Option<CapturedFrame>,
}

enum PipelineDecoder {
    Nvdec(NvdecDecoder),
    Software(Box<dyn VideoDecoder>),
}

enum PipelineTransport {
    Loopback,
    WebrtcRtp {
        sender: mrd_transport_webrtc::H264RtpSender,
        ingress: mrd_transport_webrtc::H264RtpIngress,
    },
    QuicDatagram {
        reassembler: mrd_transport_quic_quinn::QuicAuReassembler,
        next_frame_id: u32,
        max_datagram_size: usize,
    },
}

impl PipelineTransport {
    fn new(kind: Option<&TransportKind>, fps: u32) -> Self {
        match kind.unwrap_or(&TransportKind::Loopback) {
            TransportKind::Loopback => Self::Loopback,
            TransportKind::WebrtcRtp => Self::WebrtcRtp {
                sender: mrd_transport_webrtc::H264RtpSender::new(
                    "matrix-video",
                    "matrix-stream",
                    fps,
                    1200,
                ),
                ingress: mrd_transport_webrtc::H264RtpIngress::default(),
            },
            TransportKind::QuicDatagram => Self::QuicDatagram {
                reassembler: mrd_transport_quic_quinn::QuicAuReassembler::default(),
                next_frame_id: 0,
                max_datagram_size: 1200,
            },
        }
    }

    fn transmit(&mut self, access_units: Vec<EncodedAccessUnit>) -> Result<Vec<EncodedAccessUnit>> {
        match self {
            Self::Loopback => Ok(access_units),
            Self::WebrtcRtp { sender, ingress } => {
                let mut reassembled = Vec::with_capacity(access_units.len());
                for access_unit in access_units {
                    if access_unit.codec != VideoCodec::H264 {
                        anyhow::bail!("WebRTC RTP matrix transport only supports H.264");
                    }
                    let packets = sender
                        .packetize_access_unit(&access_unit)
                        .map_err(|error| anyhow::anyhow!("WebRTC RTP packetize failed: {error}"))?;
                    for packet in packets {
                        if let Some(unit) = ingress.push_packet(
                            &packet.payload,
                            packet.header.marker,
                            packet.header.sequence_number,
                            access_unit.timestamp_us,
                        ) {
                            reassembled.push(unit);
                        }
                    }
                }
                Ok(reassembled)
            }
            Self::QuicDatagram {
                reassembler,
                next_frame_id,
                max_datagram_size,
            } => {
                let mut reassembled = Vec::with_capacity(access_units.len());
                for access_unit in access_units {
                    let frame_id = *next_frame_id;
                    *next_frame_id = next_frame_id.wrapping_add(1);
                    let datagrams = mrd_transport_quic_quinn::fragment_access_unit(
                        frame_id,
                        access_unit.timestamp_us,
                        access_unit.is_keyframe,
                        &access_unit.bytes,
                        *max_datagram_size,
                    )
                    .map_err(|error| anyhow::anyhow!("QUIC AU fragment failed: {error}"))?;

                    for datagram in datagrams {
                        if let Some(frame) =
                            reassembler.push_datagram(&datagram).map_err(|error| {
                                anyhow::anyhow!("QUIC AU reassemble failed: {error}")
                            })?
                        {
                            reassembled.push(EncodedAccessUnit {
                                codec: access_unit.codec,
                                timestamp_us: frame.timestamp_us,
                                is_keyframe: frame.is_keyframe,
                                bytes: frame.payload.to_vec(),
                            });
                        }
                    }
                }
                Ok(reassembled)
            }
        }
    }
}

impl PipelineDecoder {
    fn push_access_unit(&mut self, bytes: &[u8]) -> Result<()> {
        match self {
            Self::Nvdec(decoder) => decoder
                .push_access_unit(bytes)
                .map_err(|error| anyhow::anyhow!(error)),
            Self::Software(decoder) => decoder
                .push_access_unit(bytes)
                .map_err(|error| anyhow::anyhow!(error)),
        }
    }

    fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
        match self {
            Self::Nvdec(decoder) => decoder
                .drain_decoded_frames()
                .into_iter()
                .map(nvdec_frame_to_decoded_frame)
                .collect(),
            Self::Software(decoder) => decoder.drain_decoded_frames(),
        }
    }
}

enum RenderInput {
    Decoded(DecodedFrame),
    Captured(CapturedFrame),
}

enum RenderCommand {
    Frame(RenderInput),
    Stop,
}

struct PipelineRenderer {
    sender: mpsc::SyncSender<RenderCommand>,
    render_thread: Option<thread::JoinHandle<()>>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl PipelineRenderer {
    fn new(renderer_type: &RendererType, width: usize, height: usize) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let last_error = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&last_error);
        let renderer_type = renderer_type.clone();

        let render_thread = thread::Builder::new()
            .name("mrd-dx11-test-render".to_string())
            .spawn(move || {
                if let Err(error) = run_renderer_thread(renderer_type, width, height, receiver) {
                    if let Ok(mut last_error) = thread_error.lock() {
                        *last_error = Some(error.to_string());
                    }
                }
            })
            .map_err(|error| anyhow::anyhow!("spawn D3D11 render thread failed: {error}"))?;

        Ok(Self {
            sender,
            render_thread: Some(render_thread),
            last_error,
        })
    }

    fn submit_frame(&mut self, input: RenderInput) -> Result<()> {
        if let Some(error) = self.last_error.lock().unwrap().clone() {
            anyhow::bail!("D3D11 render thread failed: {error}");
        }

        match self.sender.try_send(RenderCommand::Frame(input)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(RenderCommand::Frame(_))) => Ok(()),
            Err(mpsc::TrySendError::Full(RenderCommand::Stop)) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                anyhow::bail!("D3D11 render thread stopped")
            }
        }
    }
}

impl Drop for PipelineRenderer {
    fn drop(&mut self) {
        let _ = self.sender.send(RenderCommand::Stop);
        if let Some(render_thread) = self.render_thread.take() {
            let _ = render_thread.join();
        }
    }
}

fn run_renderer_thread(
    renderer_type: RendererType,
    width: usize,
    height: usize,
    receiver: mpsc::Receiver<RenderCommand>,
) -> Result<()> {
    match renderer_type {
        RendererType::D3d11 => {
            #[cfg(windows)]
            {
                let window = D3d11TestWindow::new(width, height)?;
                let factory = mrd_render_d3d11::D3d11RendererFactory;
                let mut renderer = factory
                    .create()
                    .map_err(|error| anyhow::anyhow!("create D3D11 renderer failed: {error}"))?;
                renderer
                    .attach_target(RenderTarget::WindowHandle(window.hwnd_value()))
                    .map_err(|error| anyhow::anyhow!("attach D3D11 renderer failed: {error}"))?;

                run_d3d11_render_loop(window, renderer, receiver)
            }

            #[cfg(not(windows))]
            {
                let _ = (width, height, receiver);
                anyhow::bail!("D3D11 render display is only available on Windows");
            }
        }
    }
}

#[cfg(windows)]
fn run_d3d11_render_loop(
    window: D3d11TestWindow,
    mut renderer: Box<dyn RendererInstance>,
    receiver: mpsc::Receiver<RenderCommand>,
) -> Result<()> {
    loop {
        window.pump_messages();
        match receiver.recv_timeout(Duration::from_millis(8)) {
            Ok(RenderCommand::Frame(input)) => {
                let frame = render_input_to_frame(input);
                renderer.upload_frame(frame).map_err(|error| {
                    anyhow::anyhow!("upload frame to D3D11 renderer failed: {error}")
                })?;
            }
            Ok(RenderCommand::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    Ok(())
}

#[cfg(windows)]
struct D3d11TestWindow {
    hwnd: windows::Win32::Foundation::HWND,
}

#[cfg(windows)]
impl D3d11TestWindow {
    fn new(frame_width: usize, frame_height: usize) -> Result<Self> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, LoadCursorW, RegisterClassW, ShowWindow, CS_HREDRAW,
            CS_VREDRAW, CW_USEDEFAULT, HMENU, IDC_ARROW, SW_SHOW, WINDOW_EX_STYLE, WM_CLOSE,
            WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
        };

        unsafe extern "system" fn wnd_proc(
            hwnd: HWND,
            message: u32,
            wparam: WPARAM,
            lparam: LPARAM,
        ) -> LRESULT {
            if message == WM_CLOSE {
                ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_HIDE);
                return LRESULT(0);
            }

            DefWindowProcW(hwnd, message, wparam, lparam)
        }

        fn wide(value: &str) -> Vec<u16> {
            OsStr::new(value)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }

        let class_name = wide("RdeskD3D11TestWindow");
        let title = wide("Rdesk DX11 Render Test");
        let hmodule = unsafe { GetModuleHandleW(None) }
            .map_err(|error| anyhow::anyhow!("get module handle failed: {error}"))?;
        let hinstance = HINSTANCE(hmodule.0);
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|error| anyhow::anyhow!("load cursor failed: {error}"))?;

        let window_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };

        unsafe {
            RegisterClassW(&window_class);
        }

        let width = frame_width.clamp(640, 1280) as i32;
        let height = frame_height.clamp(360, 800) as i32;
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width,
                height,
                HWND(0),
                HMENU(0),
                hinstance,
                None,
            )
        };

        if hwnd.0 == 0 {
            anyhow::bail!("create D3D11 render test window failed");
        }

        unsafe {
            ShowWindow(hwnd, SW_SHOW);
        }

        Ok(Self { hwnd })
    }

    fn hwnd_value(&self) -> isize {
        self.hwnd.0
    }

    fn pump_messages(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
        };

        let mut message = MSG::default();
        unsafe {
            while PeekMessageW(&mut message, self.hwnd, 0, 0, PM_REMOVE).as_bool() {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for D3d11TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd);
        }
    }
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
        let use_shared_texture_decode = config.zero_copy.unwrap_or(false);
        let (capture, capture_width, capture_height): (Box<dyn FrameCapture>, usize, usize) =
            if use_shared_texture_decode {
                #[cfg(windows)]
                {
                    let mut capture = DxgiSharedTextureCapture::new_primary().map_err(|e| {
                        anyhow::anyhow!("DXGI shared texture capture init failed: {:?}", e)
                    })?;
                    let (width, height) =
                        select_pipeline_dimensions(capture.width(), capture.height(), config);
                    capture.set_target_dimensions(width, height);
                    (Box::new(capture) as Box<dyn FrameCapture>, width, height)
                }
                #[cfg(not(windows))]
                {
                    return Err(anyhow::anyhow!(
                        "D3D11 shared texture capture is only available on Windows"
                    ));
                }
            } else {
                let capture = DxgiDesktopCapture::new_primary()
                    .map_err(|e| anyhow::anyhow!("DXGI 捕获初始化失败: {:?}", e))?;
                let (width, height) =
                    select_pipeline_dimensions(capture.width(), capture.height(), config);
                (Box::new(capture) as Box<dyn FrameCapture>, width, height)
            };

        let (width, height) = (capture_width, capture_height);
        let fps = config.fps.unwrap_or(60).max(1);
        let low_latency_bitrate = config.bitrate.unwrap_or(12_000_000).max(1);
        let speed_bitrate = config.bitrate.unwrap_or(5_000_000).max(1);

        let (encoder, decoder, use_decoder) = match chain {
            TestChain::CaptureOnly => (None, None, false),
            TestChain::NvencNvdec => {
                let encoder =
                    NvencH264Encoder::new_with_bitrate(width, height, fps, low_latency_bitrate)
                        .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                let mut decoder = NvdecDecoder::new_with_output_mode(NvdecOutputMode::CpuNv12)
                    .map_err(|e| anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e))?;
                if use_shared_texture_decode {
                    decoder.enable_shared_texture(true);
                }
                (
                    Some(Box::new(encoder) as Box<dyn VideoEncoder>),
                    Some(PipelineDecoder::Nvdec(decoder)),
                    true,
                )
            }
            TestChain::NvencOnly => {
                let encoder =
                    NvencH264Encoder::new_max_speed_with_bitrate(width, height, fps, speed_bitrate)
                        .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                (
                    Some(Box::new(encoder) as Box<dyn VideoEncoder>),
                    None,
                    false,
                )
            }
            TestChain::OpenH264 => {
                let encoder = match config.bitrate {
                    Some(bitrate) => OpenH264Encoder::new_with_bitrate(width, height, fps, bitrate),
                    None => OpenH264Encoder::new(width, height, fps),
                }
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
                // For now, Custom configurations fall back to standard capture implementations.
                // TODO: Implement WinRT capture and AV1 encoding.
                match encoder {
                    EncoderType::NvencH264 => match decoder {
                        DecoderType::None => {
                            let enc = NvencH264Encoder::new_max_speed_with_bitrate(
                                width,
                                height,
                                fps,
                                speed_bitrate,
                            )
                            .map_err(|e| anyhow::anyhow!("NVENC encoder init failed: {:?}", e))?;
                            (Some(Box::new(enc) as Box<dyn VideoEncoder>), None, false)
                        }
                        DecoderType::Nvdec => {
                            let enc = NvencH264Encoder::new_with_bitrate(
                                width,
                                height,
                                fps,
                                low_latency_bitrate,
                            )
                            .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                            let mut dec =
                                NvdecDecoder::new_with_output_mode(NvdecOutputMode::CpuNv12)
                                    .map_err(|e| {
                                        anyhow::anyhow!("NVDEC 解码器初始化失败: {:?}", e)
                                    })?;
                            if use_shared_texture_decode {
                                dec.enable_shared_texture(true);
                            }
                            (
                                Some(Box::new(enc) as Box<dyn VideoEncoder>),
                                Some(PipelineDecoder::Nvdec(dec)),
                                true,
                            )
                        }
                        DecoderType::Software => {
                            let enc = NvencH264Encoder::new_max_speed_with_bitrate(
                                width,
                                height,
                                fps,
                                speed_bitrate,
                            )
                            .map_err(|e| anyhow::anyhow!("NVENC 编码器初始化失败: {:?}", e))?;
                            let dec = mrd_decode::create_decoder("h264_software").map_err(|e| {
                                anyhow::anyhow!("software decoder init failed: {:?}", e)
                            })?;
                            (
                                Some(Box::new(enc) as Box<dyn VideoEncoder>),
                                Some(PipelineDecoder::Software(dec)),
                                true,
                            )
                        }
                    },
                    EncoderType::OpenH264 => {
                        let enc = match config.bitrate {
                            Some(bitrate) => {
                                OpenH264Encoder::new_with_bitrate(width, height, fps, bitrate)
                            }
                            None => OpenH264Encoder::new(width, height, fps),
                        }
                        .map_err(|e| anyhow::anyhow!("OpenH264 编码器初始化失败: {:?}", e))?;
                        match decoder {
                            DecoderType::None => {
                                (Some(Box::new(enc) as Box<dyn VideoEncoder>), None, false)
                            }
                            DecoderType::Nvdec => {
                                let mut dec =
                                    NvdecDecoder::new_with_output_mode(NvdecOutputMode::CpuNv12)
                                        .map_err(|e| {
                                            anyhow::anyhow!("NVDEC decoder init failed: {:?}", e)
                                        })?;
                                if use_shared_texture_decode {
                                    dec.enable_shared_texture(true);
                                }
                                (
                                    Some(Box::new(enc) as Box<dyn VideoEncoder>),
                                    Some(PipelineDecoder::Nvdec(dec)),
                                    true,
                                )
                            }
                            DecoderType::Software => {
                                let dec =
                                    mrd_decode::create_decoder("h264_software").map_err(|e| {
                                        anyhow::anyhow!("software decoder init failed: {:?}", e)
                                    })?;
                                (
                                    Some(Box::new(enc) as Box<dyn VideoEncoder>),
                                    Some(PipelineDecoder::Software(dec)),
                                    true,
                                )
                            }
                        }
                    }
                    EncoderType::NvencAv1 => {
                        if !matches!(decoder, DecoderType::None) {
                            return Err(anyhow::anyhow!(
                                "AV1 decoder path is not implemented; choose decoder none"
                            ));
                        }
                        #[cfg(windows)]
                        {
                            let enc = NvencAv1Encoder::new_low_latency(width, height, fps)
                                .map_err(|e| {
                                    anyhow::anyhow!("NVENC AV1 encoder init failed: {:?}", e)
                                })?;
                            (Some(Box::new(enc) as Box<dyn VideoEncoder>), None, false)
                        }
                        #[cfg(not(windows))]
                        {
                            return Err(anyhow::anyhow!(
                                "NVENC AV1 encoder is only available on Windows"
                            ));
                        }
                    }
                }
            }
        };

        let transport = PipelineTransport::new(config.transport.as_ref(), fps);

        let renderer = config
            .renderer
            .as_ref()
            .map(|renderer_type| PipelineRenderer::new(renderer_type, width, height))
            .transpose()?;

        Ok(PipelineState {
            capture,
            encoder,
            transport,
            decoder,
            renderer,
            use_decoder,
            width,
            height,
            adapted_frame: None,
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
        let mut transport_latencies = Vec::with_capacity(1000);
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
                let frame_for_encode = prepare_frame_for_encode(
                    &captured_frame,
                    state.width,
                    state.height,
                    &mut state.adapted_frame,
                );
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

            let (transported_units, transport_latency) = if encoded_units.is_empty() {
                (encoded_units, None)
            } else {
                let transport_start = Instant::now();
                match state.transport.transmit(encoded_units) {
                    Ok(units) => (units, Some(transport_start.elapsed())),
                    Err(error) => {
                        let mut m = metrics.lock().unwrap();
                        m.error_message = Some(error.to_string());
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            };

            // Decode if needed
            let mut decoded_frames = Vec::new();
            let decode_latency = if state.use_decoder && !transported_units.is_empty() {
                if let Some(decoder) = state.decoder.as_mut() {
                    let decode_start = Instant::now();
                    let mut pushed_any = false;
                    let mut failed_units = 0_usize;
                    for unit in &transported_units {
                        match decoder.push_access_unit(&unit.bytes) {
                            Ok(()) => {
                                pushed_any = true;
                                decoded_frames = decoder.drain_decoded_frames();
                                if !decoded_frames.is_empty() {
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

            if let Some(renderer) = state.renderer.as_mut() {
                let input = decoded_frames.pop().map(RenderInput::Decoded).or_else(|| {
                    (!state.use_decoder).then(|| RenderInput::Captured(captured_frame.clone()))
                });
                if let Some(input) = input {
                    if let Err(error) = renderer.submit_frame(input) {
                        let mut m = metrics.lock().unwrap();
                        m.error_message = Some(error.to_string());
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }

            capture_latencies.push(capture_latency);
            if let Some(latency) = encode_latency {
                encode_latencies.push(latency);
            }
            if let Some(latency) = transport_latency {
                transport_latencies.push(latency);
            }
            if let Some(latency) = decode_latency {
                decode_latencies.push(latency);
            }
            total_latencies.push(pipeline_start.elapsed());

            Self::trim_latency_buffers(
                &mut capture_latencies,
                &mut encode_latencies,
                &mut transport_latencies,
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
                    &transport_latencies,
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
            &transport_latencies,
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
        transport_latencies: &[Duration],
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
        let (p50_transport, p95_transport) = Self::compute_percentiles(transport_latencies);
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
        m.transport_latency_p50_ms = p50_transport.as_secs_f64() * 1000.0;
        m.transport_latency_p95_ms = p95_transport.as_secs_f64() * 1000.0;
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
        transport_latencies: &mut Vec<Duration>,
        decode_latencies: &mut Vec<Duration>,
        total_latencies: &mut Vec<Duration>,
    ) {
        if capture_latencies.len() > 1000 {
            capture_latencies.remove(0);
        }
        if encode_latencies.len() > 1000 {
            encode_latencies.remove(0);
        }
        if transport_latencies.len() > 1000 {
            transport_latencies.remove(0);
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

fn nvdec_frame_to_decoded_frame(frame: mrd_decode_nvdec::NvdecDecodedFrame) -> DecodedFrame {
    match frame.data {
        mrd_decode_nvdec::NvdecDecodedFrameData::CpuRgb24(data) => {
            DecodedFrame::from_cpu_rgb24(frame.width, frame.height, 0, data)
        }
        mrd_decode_nvdec::NvdecDecodedFrameData::CpuNv12 { data, pitch } => {
            DecodedFrame::from_cpu_nv12(frame.width, frame.height, 0, pitch, data)
        }
        #[cfg(windows)]
        mrd_decode_nvdec::NvdecDecodedFrameData::D3D11SharedNv12 {
            shared_handle_y,
            shared_handle_uv,
            width: _,
            height: _,
        } => DecodedFrame::from_d3d11_shared_nv12(
            frame.width,
            frame.height,
            0,
            shared_handle_y,
            shared_handle_uv,
        ),
    }
}

fn render_input_to_frame(input: RenderInput) -> RenderFrame {
    match input {
        RenderInput::Decoded(frame) => decoded_frame_to_render_frame(&frame),
        RenderInput::Captured(frame) => captured_frame_to_render_frame(&frame),
    }
}

fn decoded_frame_to_render_frame(frame: &DecodedFrame) -> RenderFrame {
    match &frame.data {
        DecodedFrameData::CpuRgb24(data) => {
            RenderFrame::from_rgb24(frame.width, frame.height, data.clone())
        }
        DecodedFrameData::CpuBgra32(data) => {
            RenderFrame::from_bgra32(frame.width, frame.height, data.clone())
        }
        DecodedFrameData::CpuNv12 { data, pitch } => RenderFrame::from_rgb24(
            frame.width,
            frame.height,
            cpu_nv12_to_rgb24(data, frame.width, frame.height, *pitch),
        ),
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 {
            shared_handle_y,
            shared_handle_uv,
            ..
        } => RenderFrame::from_d3d11_shared_nv12(
            frame.width,
            frame.height,
            *shared_handle_y,
            *shared_handle_uv,
        ),
    }
}

fn captured_frame_to_render_frame(frame: &CapturedFrame) -> RenderFrame {
    match frame.pixel_format {
        FramePixelFormat::Bgra32 => {
            RenderFrame::from_bgra32(frame.width, frame.height, frame.data.clone())
        }
        FramePixelFormat::Rgb24 => {
            RenderFrame::from_rgb24(frame.width, frame.height, frame.data.clone())
        }
        FramePixelFormat::Rgba32 => RenderFrame::from_bgra32(
            frame.width,
            frame.height,
            rgba32_to_bgra32(&frame.data, frame.width, frame.height),
        ),
    }
}

fn rgba32_to_bgra32(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut bgra = vec![0_u8; width * height * 4];
    for (src, dst) in rgba.chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
    bgra
}

fn cpu_nv12_to_rgb24(nv12: &[u8], width: usize, height: usize, pitch: usize) -> Vec<u8> {
    let mut rgb = vec![0_u8; width * height * 3];
    let uv_base = pitch * height;
    let mut out_idx = 0;

    for y in 0..height {
        let y_row_start = y * pitch;
        let uv_row_start = uv_base + (y / 2) * pitch;
        for x in 0..width {
            let y_offset = y_row_start + x;
            let uv_offset = uv_row_start + (x / 2) * 2;
            if y_offset >= nv12.len() || uv_offset + 1 >= nv12.len() {
                out_idx += 3;
                continue;
            }

            let y_sample = nv12[y_offset] as i32 - 16;
            let u = nv12[uv_offset] as i32 - 128;
            let v = nv12[uv_offset + 1] as i32 - 128;

            let r = (298 * y_sample + 409 * v + 128) >> 8;
            let g = (298 * y_sample - 100 * u - 208 * v + 128) >> 8;
            let b = (298 * y_sample + 516 * u + 128) >> 8;

            rgb[out_idx] = r.clamp(0, 255) as u8;
            rgb[out_idx + 1] = g.clamp(0, 255) as u8;
            rgb[out_idx + 2] = b.clamp(0, 255) as u8;
            out_idx += 3;
        }
    }

    rgb
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

fn prepare_frame_for_encode<'a>(
    frame: &'a CapturedFrame,
    target_width: usize,
    target_height: usize,
    scratch: &'a mut Option<CapturedFrame>,
) -> &'a CapturedFrame {
    if frame.width == target_width && frame.height == target_height {
        return frame;
    }

    adapt_frame_dimensions_into(frame, target_width, target_height, scratch);
    scratch
        .as_ref()
        .expect("adapt_frame_dimensions_into must initialize scratch")
}

fn adapt_frame_dimensions_into(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
    scratch: &mut Option<CapturedFrame>,
) {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);
    let required_len = target_width * target_height * bytes_per_pixel;
    let output = scratch.get_or_insert_with(|| {
        CapturedFrame::from_cpu(
            target_width,
            target_height,
            frame.pixel_format,
            frame.timestamp_us,
            vec![0_u8; required_len],
        )
    });

    output.width = target_width;
    output.height = target_height;
    output.pixel_format = frame.pixel_format;
    output.timestamp_us = frame.timestamp_us;
    if output.data.len() != required_len {
        output.data.resize(required_len, 0);
    }

    if target_width <= frame.width && target_height <= frame.height {
        crop_frame_center_into(frame, target_width, target_height, &mut output.data);
    } else {
        resize_frame_nearest_into(frame, target_width, target_height, &mut output.data);
    }
}

#[cfg(test)]
fn resize_frame_nearest(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
) -> CapturedFrame {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);
    let mut data = vec![0_u8; target_width * target_height * bytes_per_pixel];

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        return CapturedFrame::from_cpu(
            target_width,
            target_height,
            frame.pixel_format,
            frame.timestamp_us,
            data,
        );
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

    CapturedFrame::from_cpu(
        target_width,
        target_height,
        frame.pixel_format,
        frame.timestamp_us,
        data,
    )
}

fn resize_frame_nearest_into(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
    data: &mut [u8],
) {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        data.fill(0);
        return;
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
}

#[cfg(test)]
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

fn crop_frame_center_into(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
    data: &mut [u8],
) {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        data.fill(0);
        return;
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
}

#[cfg(test)]
fn crop_frame_center(
    frame: &CapturedFrame,
    target_width: usize,
    target_height: usize,
) -> CapturedFrame {
    let bytes_per_pixel = bytes_per_pixel(frame.pixel_format);
    let mut data = vec![0_u8; target_width * target_height * bytes_per_pixel];

    if frame.width == 0 || frame.height == 0 || bytes_per_pixel == 0 {
        return CapturedFrame::from_cpu(
            target_width,
            target_height,
            frame.pixel_format,
            frame.timestamp_us,
            data,
        );
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

    CapturedFrame::from_cpu(
        target_width,
        target_height,
        frame.pixel_format,
        frame.timestamp_us,
        data,
    )
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
        let mut transport_latencies = Vec::new();
        let mut decode_latencies = Vec::new();
        let mut total_latencies = Vec::new();

        TestHarness::trim_latency_buffers(
            &mut capture_latencies,
            &mut encode_latencies,
            &mut transport_latencies,
            &mut decode_latencies,
            &mut total_latencies,
        );

        assert!(capture_latencies.is_empty());
        assert_eq!(encode_latencies.len(), 1000);
        assert_eq!(encode_latencies[0], Duration::from_millis(1));
        assert!(transport_latencies.is_empty());
        assert!(decode_latencies.is_empty());
        assert!(total_latencies.is_empty());
    }

    #[test]
    fn trim_latency_buffers_trims_each_populated_series_independently() {
        let mut capture_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut encode_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut transport_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut decode_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();
        let mut total_latencies = (0..=1000).map(Duration::from_millis).collect::<Vec<_>>();

        TestHarness::trim_latency_buffers(
            &mut capture_latencies,
            &mut encode_latencies,
            &mut transport_latencies,
            &mut decode_latencies,
            &mut total_latencies,
        );

        assert_eq!(capture_latencies.len(), 1000);
        assert_eq!(encode_latencies.len(), 1000);
        assert_eq!(transport_latencies.len(), 1000);
        assert_eq!(decode_latencies.len(), 1000);
        assert_eq!(total_latencies.len(), 1000);
        assert_eq!(capture_latencies[0], Duration::from_millis(1));
        assert_eq!(encode_latencies[0], Duration::from_millis(1));
        assert_eq!(transport_latencies[0], Duration::from_millis(1));
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
        let frame = CapturedFrame::from_cpu(
            3,
            2,
            FramePixelFormat::Bgra32,
            42,
            vec![
                1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
            ],
        );

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
        let frame = CapturedFrame::from_cpu(4, 3, FramePixelFormat::Bgra32, 99, pixels);

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

    fn env_capture_type() -> CaptureType {
        match std::env::var("MRD_HARNESS_CAPTURE").as_deref() {
            Ok("winrt") => CaptureType::Winrt,
            Ok("synthetic") => CaptureType::Synthetic,
            _ => CaptureType::Dxgi,
        }
    }

    fn env_encoder_type() -> EncoderType {
        match std::env::var("MRD_HARNESS_ENCODER").as_deref() {
            Ok("openh264") => EncoderType::OpenH264,
            Ok("nvenc_av1") => EncoderType::NvencAv1,
            _ => EncoderType::NvencH264,
        }
    }

    fn env_decoder_type() -> DecoderType {
        match std::env::var("MRD_HARNESS_DECODER").as_deref() {
            Ok("software") => DecoderType::Software,
            _ => DecoderType::Nvdec,
        }
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
            Ok("custom") | Ok("matrix") => TestChain::Custom {
                capture: env_capture_type(),
                encoder: env_encoder_type(),
                decoder: env_decoder_type(),
            },
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
            bitrate: std::env::var("MRD_HARNESS_BITRATE")
                .ok()
                .and_then(|value| value.parse::<u32>().ok()),
            renderer: match std::env::var("MRD_HARNESS_RENDERER").as_deref() {
                Ok("d3d11") => Some(RendererType::D3d11),
                _ => None,
            },
            transport: match std::env::var("MRD_HARNESS_TRANSPORT").as_deref() {
                Ok("webrtc") | Ok("webrtc_rtp") => Some(TransportKind::WebrtcRtp),
                Ok("quic") | Ok("quic_datagram") => Some(TransportKind::QuicDatagram),
                Ok("loopback") => Some(TransportKind::Loopback),
                _ => None,
            },
            zero_copy: match std::env::var("MRD_HARNESS_ZERO_COPY").as_deref() {
                Ok("1") | Ok("true") | Ok("d3d11_shared") => Some(true),
                Ok("0") | Ok("false") | Ok("cpu") => Some(false),
                _ => None,
            },
        });
        harness.start().expect("start harness");
        thread::sleep(Duration::from_secs(seconds));
        harness.stop().expect("stop harness");
        let metrics = harness.get_metrics();
        println!("{metrics:#?}");
        assert!(metrics.frame_count > 0);
    }
}
