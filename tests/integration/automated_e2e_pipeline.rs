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
use std::time::Duration;

#[test]
fn synthetic_capture_encode_quic_decode_render_pipeline() -> Result<()> {
    let width = 640;
    let height = 360;
    let fps = 30;
    let frame_count = 18;
    let mut capture = DeterministicCapture::new(width, height);
    let mut encoder = OpenH264Encoder::new_with_bitrate(width, height, fps, 4_000_000)?;
    let mut decoder = mrd_decode::create_decoder("h264_software")?;
    let mut renderer = create_renderer()?;

    let mut encoded_count = 0usize;
    let mut transported_count = 0usize;
    let mut decoded_count = 0usize;
    let mut rendered_count = 0usize;

    for frame_index in 0..frame_count {
        let captured = capture.capture_frame()?;
        let encoded_units = encoder.encode(&captured)?;
        encoded_count += encoded_units.len();

        let transported_units = transmit_quic_datagrams(frame_index as u32, encoded_units)?;
        transported_count += transported_units.len();

        for unit in transported_units {
            decoder.push_access_unit(&unit.bytes)?;
            for decoded in decoder.drain_decoded_frames() {
                let render_frame = decoded_frame_to_render_frame(&decoded);
                renderer.upload_frame(render_frame)?;
                decoded_count += 1;
                rendered_count += 1;
            }
        }
    }

    let snapshot = renderer.snapshot();
    assert!(encoded_count > 0, "encoder produced no access units");
    assert!(
        transported_count > 0,
        "QUIC transport loopback produced no access units"
    );
    assert!(decoded_count > 0, "decoder produced no frames");
    assert_eq!(
        snapshot.uploaded_frame_count as usize, rendered_count,
        "renderer upload count should match decoded frames"
    );
    assert_eq!(snapshot.last_width, width);
    assert_eq!(snapshot.last_height, height);
    assert!(
        matches!(
            snapshot.last_pixel_format,
            Some(RenderPixelFormat::Rgb24 | RenderPixelFormat::Bgra32)
        ),
        "renderer should receive a CPU renderable frame"
    );

    Ok(())
}

fn transmit_quic_datagrams(
    frame_id: u32,
    access_units: Vec<EncodedAccessUnit>,
) -> Result<Vec<EncodedAccessUnit>> {
    let mut reassembler = mrd_transport_quic_quinn::QuicAuReassembler::new(
        mrd_transport_quic_quinn::QuicAuReassemblerConfig {
            frame_timeout: Duration::from_millis(250),
            max_pending_frames: 64,
        },
    );
    let mut reassembled = Vec::new();

    for (unit_index, access_unit) in access_units.into_iter().enumerate() {
        let datagrams = mrd_transport_quic_quinn::fragment_access_unit(
            frame_id
                .checked_mul(16)
                .and_then(|base| base.checked_add(unit_index as u32))
                .ok_or_else(|| anyhow!("QUIC frame id overflow"))?,
            access_unit.timestamp_us,
            access_unit.is_keyframe,
            &access_unit.bytes,
            1200,
        )?;

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

    Ok(reassembled)
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
        DecodedFrameData::D3D11SharedNv12 { .. } => {
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
