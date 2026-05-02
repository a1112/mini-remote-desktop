use anyhow::{anyhow, Result};
use mrd_decode::DecodedFrameData;
use mrd_encode_openh264::OpenH264Encoder;
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame, EncodedAccessUnit, FrameCapture, FramePixelFormat, PipelineError,
    VideoCodec, VideoEncoder,
};
#[cfg(not(any(target_os = "macos", windows)))]
use mrd_render::{RenderError, RendererSnapshot};
use mrd_render::{RenderFrame, RenderPixelFormat, RendererFactory, RendererInstance};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct E2ePipelineCase {
    pub name: &'static str,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub frame_count: usize,
    pub bitrate_bps: u32,
    pub mtu: usize,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct E2ePipelineReport {
    pub name: &'static str,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub frame_count: usize,
    pub bitrate_bps: u32,
    pub mtu: usize,
    pub encoded_access_units: usize,
    pub encoded_bytes: usize,
    pub quic_datagrams: usize,
    pub transported_access_units: usize,
    pub decoded_frames: usize,
    pub rendered_frames: usize,
    pub elapsed_ms: f64,
    pub render_fps: f64,
    pub frame_avg_ms: f64,
    pub frame_p50_ms: f64,
    pub frame_p95_ms: f64,
    pub renderer: &'static str,
    pub last_pixel_format: Option<RenderPixelFormat>,
}

pub fn run_pipeline_case(case: &E2ePipelineCase) -> Result<E2ePipelineReport> {
    let mut capture = DeterministicCapture::new(case.width, case.height);
    let mut encoder =
        OpenH264Encoder::new_with_bitrate(case.width, case.height, case.fps, case.bitrate_bps)?;
    let mut decoder = mrd_decode::create_decoder("h264_software")?;
    let mut renderer = create_renderer()?;

    let mut encoded_access_units = 0usize;
    let mut encoded_bytes = 0usize;
    let mut quic_datagrams = 0usize;
    let mut transported_access_units = 0usize;
    let mut decoded_frames = 0usize;
    let mut rendered_frames = 0usize;
    let mut frame_latencies = Vec::with_capacity(case.frame_count);

    let case_start = Instant::now();
    for frame_index in 0..case.frame_count {
        let frame_start = Instant::now();
        let captured = capture.capture_frame()?;
        let encoded_units = encoder.encode(&captured)?;
        encoded_access_units += encoded_units.len();
        encoded_bytes += encoded_units
            .iter()
            .map(|unit| unit.bytes.len())
            .sum::<usize>();

        let transported = transmit_quic_datagrams(frame_index as u32, encoded_units, case.mtu)?;
        quic_datagrams += transported.datagram_count;
        transported_access_units += transported.access_units.len();

        for unit in transported.access_units {
            decoder.push_access_unit(&unit.bytes)?;
            for decoded in decoder.drain_decoded_frames() {
                let render_frame = decoded_frame_to_render_frame(&decoded);
                renderer.upload_frame(render_frame)?;
                decoded_frames += 1;
                rendered_frames += 1;
            }
        }
        frame_latencies.push(frame_start.elapsed());
    }

    let elapsed = case_start.elapsed();
    let snapshot = renderer.snapshot();
    validate_pipeline_report(
        case,
        &snapshot,
        encoded_access_units,
        transported_access_units,
        decoded_frames,
        rendered_frames,
    )?;

    Ok(E2ePipelineReport {
        name: case.name,
        width: case.width,
        height: case.height,
        fps: case.fps,
        frame_count: case.frame_count,
        bitrate_bps: case.bitrate_bps,
        mtu: case.mtu,
        encoded_access_units,
        encoded_bytes,
        quic_datagrams,
        transported_access_units,
        decoded_frames,
        rendered_frames,
        elapsed_ms: elapsed.as_secs_f64() * 1000.0,
        render_fps: rendered_frames as f64 / elapsed.as_secs_f64().max(f64::EPSILON),
        frame_avg_ms: average_ms(&frame_latencies),
        frame_p50_ms: percentile_ms(&frame_latencies, 0.50),
        frame_p95_ms: percentile_ms(&frame_latencies, 0.95),
        renderer: renderer_label(),
        last_pixel_format: snapshot.last_pixel_format,
    })
}

fn validate_pipeline_report(
    case: &E2ePipelineCase,
    snapshot: &mrd_render::RendererSnapshot,
    encoded_access_units: usize,
    transported_access_units: usize,
    decoded_frames: usize,
    rendered_frames: usize,
) -> Result<()> {
    if encoded_access_units == 0 {
        return Err(anyhow!("{}: encoder produced no access units", case.name));
    }
    if transported_access_units == 0 {
        return Err(anyhow!(
            "{}: QUIC transport loopback produced no access units",
            case.name
        ));
    }
    if decoded_frames == 0 {
        return Err(anyhow!("{}: decoder produced no frames", case.name));
    }
    if snapshot.uploaded_frame_count as usize != rendered_frames {
        return Err(anyhow!(
            "{}: renderer uploaded {} frames, expected {}",
            case.name,
            snapshot.uploaded_frame_count,
            rendered_frames
        ));
    }
    if snapshot.last_width != case.width || snapshot.last_height != case.height {
        return Err(anyhow!(
            "{}: renderer dimensions {}x{}, expected {}x{}",
            case.name,
            snapshot.last_width,
            snapshot.last_height,
            case.width,
            case.height
        ));
    }
    if !matches!(
        snapshot.last_pixel_format,
        Some(RenderPixelFormat::Rgb24 | RenderPixelFormat::Bgra32)
    ) {
        return Err(anyhow!(
            "{}: renderer received unsupported pixel format {:?}",
            case.name,
            snapshot.last_pixel_format
        ));
    }
    Ok(())
}

struct TransportedAccessUnits {
    access_units: Vec<EncodedAccessUnit>,
    datagram_count: usize,
}

fn transmit_quic_datagrams(
    frame_id: u32,
    access_units: Vec<EncodedAccessUnit>,
    mtu: usize,
) -> Result<TransportedAccessUnits> {
    let mut reassembler = mrd_transport_quic_quinn::QuicAuReassembler::new(
        mrd_transport_quic_quinn::QuicAuReassemblerConfig {
            frame_timeout: Duration::from_millis(250),
            max_pending_frames: 64,
        },
    );
    let mut reassembled = Vec::new();
    let mut datagram_count = 0usize;

    for (unit_index, access_unit) in access_units.into_iter().enumerate() {
        let datagrams = mrd_transport_quic_quinn::fragment_access_unit(
            frame_id
                .checked_mul(16)
                .and_then(|base| base.checked_add(unit_index as u32))
                .ok_or_else(|| anyhow!("QUIC frame id overflow"))?,
            access_unit.timestamp_us,
            access_unit.is_keyframe,
            &access_unit.bytes,
            mtu,
        )?;
        datagram_count += datagrams.len();

        for datagram in datagrams {
            if let Some(frame) = reassembler.push_datagram(&datagram)? {
                reassembled.push(EncodedAccessUnit {
                    codec: VideoCodec::H264,
                    timestamp_us: frame.timestamp_us,
                    is_keyframe: frame.is_keyframe,
                    bytes: frame.payload.to_vec(),
                });
            }
        }
    }

    Ok(TransportedAccessUnits {
        access_units: reassembled,
        datagram_count,
    })
}

fn create_renderer() -> Result<Box<dyn RendererInstance>> {
    #[cfg(target_os = "macos")]
    {
        let factory = mrd_render_macos::MacosRendererFactory;
        return factory.create().map_err(|error| anyhow!(error));
    }

    #[cfg(windows)]
    {
        use mrd_render::RenderTarget;

        let factory = mrd_render_d3d11::D3d11RendererFactory;
        let mut renderer = factory.create().map_err(|error| anyhow!(error))?;
        renderer
            .attach_target(RenderTarget::WindowHandle(0))
            .map_err(|error| anyhow!(error))?;
        return Ok(renderer);
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Ok(Box::<InMemoryRenderer>::default())
    }
}

fn renderer_label() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos-metal"
    }
    #[cfg(windows)]
    {
        "d3d11"
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        "memory"
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
        DecodedFrameData::CpuP010 { .. } => {
            unreachable!("automated OpenH264 software decode path should not emit P010 frames")
        }
        #[cfg(windows)]
        DecodedFrameData::D3D11SharedNv12 { .. } | DecodedFrameData::D3D11SharedP010 { .. } => {
            unreachable!("automated OpenH264 software decode path should not emit D3D11 frames")
        }
    }
}

fn cpu_nv12_to_rgb24(nv12: &[u8], width: usize, height: usize, pitch: usize) -> Vec<u8> {
    let mut rgb = vec![0_u8; width * height * 3];
    let uv_base = pitch * height;
    let mut out_idx = 0usize;

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

fn average_ms(samples: &[Duration]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1000.0)
        .sum::<f64>()
        / samples.len() as f64
}

fn percentile_ms(samples: &[Duration], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut values = samples.to_vec();
    values.sort_unstable();
    let index = ((values.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[index].as_secs_f64() * 1000.0
}

struct DeterministicCapture {
    tick: u64,
    width: usize,
    height: usize,
}

impl DeterministicCapture {
    fn new(width: usize, height: usize) -> Self {
        Self {
            tick: 0,
            width,
            height,
        }
    }
}

impl FrameCapture for DeterministicCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        self.tick = self.tick.saturating_add(1);
        let mut data = vec![0_u8; self.width * self.height * 4];
        let phase = (self.tick & 0xff) as u8;

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = (y * self.width + x) * 4;
                data[idx] = (x as u8).wrapping_add(phase);
                data[idx + 1] = (y as u8).wrapping_add(phase / 2);
                data[idx + 2] = 255_u8.wrapping_sub(phase);
                data[idx + 3] = 255;
            }
        }

        Ok(CapturedFrame::from_cpu(
            self.width,
            self.height,
            FramePixelFormat::Bgra32,
            self.tick.saturating_mul(33_333),
            data,
        ))
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
#[derive(Default)]
struct InMemoryRenderer {
    uploaded_frame_count: u64,
    last_width: usize,
    last_height: usize,
    last_pixel_format: Option<RenderPixelFormat>,
}

#[cfg(not(any(target_os = "macos", windows)))]
impl RendererInstance for InMemoryRenderer {
    fn attach_target(&mut self, _target: mrd_render::RenderTarget) -> Result<(), RenderError> {
        Ok(())
    }

    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError> {
        self.uploaded_frame_count = self.uploaded_frame_count.saturating_add(1);
        self.last_width = frame.width;
        self.last_height = frame.height;
        self.last_pixel_format = Some(frame.pixel_format);
        Ok(())
    }

    fn snapshot(&self) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: false,
            uploaded_frame_count: self.uploaded_frame_count,
            last_width: self.last_width,
            last_height: self.last_height,
            last_pixel_format: self.last_pixel_format,
        }
    }
}
