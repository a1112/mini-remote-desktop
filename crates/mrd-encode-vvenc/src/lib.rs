use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FrameMemoryKind, PipelineError, VideoEncoder,
};
#[cfg(feature = "software-vvenc")]
use mrd_pipeline_core::{FramePixelFormat, VideoCodec};

pub struct VvencSoftwareEncoder {
    inner: imp::Inner,
}

impl VvencSoftwareEncoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_with_bitrate(width, height, fps, 12_000_000)
    }

    pub fn new_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            inner: imp::Inner::new(width, height, fps, bitrate)?,
        })
    }
}

impl VideoEncoder for VvencSoftwareEncoder {
    fn input_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        self.inner.encode(frame)
    }
}

pub fn vvenc_software_compiled() -> bool {
    imp::compiled()
}

pub fn probe_vvenc_software_encoder_available() -> Result<(), PipelineError> {
    imp::probe_available()
}

#[cfg(not(feature = "software-vvenc"))]
mod imp {
    use super::*;

    pub struct Inner;

    impl Inner {
        pub fn new(
            _width: usize,
            _height: usize,
            _fps: u32,
            _bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Err(not_compiled_error())
        }

        pub fn encode(
            &mut self,
            _frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            Err(not_compiled_error())
        }
    }

    pub fn compiled() -> bool {
        false
    }

    pub fn probe_available() -> Result<(), PipelineError> {
        Err(not_compiled_error())
    }

    fn not_compiled_error() -> PipelineError {
        PipelineError::message(
            "H.266/VVC software encode requires mrd-encode-vvenc feature software-vvenc and libvvenc >= 1.13.0",
        )
    }
}

#[cfg(feature = "software-vvenc")]
mod imp {
    use super::*;
    use vvenc::{
        ChromaFormat, Config, DecodingRefreshType, Encoder, LogLevel, Preset, Rational, YUVBuffer,
        YUVComponent,
    };

    pub struct Inner {
        encoder: Encoder<u64>,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: u64,
        output: Vec<u8>,
    }

    impl Inner {
        pub fn new(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            validate_even_dimensions(width, height)?;

            let fps = fps.max(1);
            let mut config = Config::new();
            config
                .set_preset(Preset::Faster)
                .map_err(|error| PipelineError::message(format!("VVenC preset failed: {error}")))?;
            config
                .set_width(width as i32)
                .set_height(height as i32)
                .set_framerate(Rational {
                    num: fps as i32,
                    den: 1,
                })
                .set_ticks_per_second(1_000_000)
                .set_frames_to_be_encoded(0)
                .set_input_bit_depth([8, 8])
                .set_target_bitrate((bitrate / 1000).max(1) as i32)
                .set_num_threads(vvenc_thread_count())
                .set_intra_period(fps as i32)
                .set_gop_size(1)
                .set_decoding_refresh_type(DecodingRefreshType::Idr)
                .set_internal_chroma_format(ChromaFormat::Chroma420)
                .set_log_level(LogLevel::Error);

            let encoder = Encoder::with_config(config)
                .map_err(|error| PipelineError::message(format!("VVenC init failed: {error}")))?;

            Ok(Self {
                encoder,
                width,
                height,
                fps,
                frame_index: 0,
                output: vec![0; vvenc_output_len(width, height)?],
            })
        }

        pub fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "VVenC frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }
            validate_even_dimensions(frame.width, frame.height)?;

            let mut yuv = YUVBuffer::new(
                frame.width as i32,
                frame.height as i32,
                ChromaFormat::Chroma420,
            );
            write_yuv420_i16(frame, &mut yuv)?;
            yuv.set_sequence_number(self.frame_index);
            yuv.set_cts(frame.timestamp_us);
            yuv.set_opaque(frame.timestamp_us);

            let result = self
                .encoder
                .encode(&mut yuv, &mut self.output)
                .map_err(|error| PipelineError::message(format!("VVenC encode failed: {error}")))?;
            self.frame_index += 1;

            let Some(access_unit) = result else {
                return Ok(Vec::new());
            };
            let bytes = access_unit.payload().to_vec();
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Vvc,
                timestamp_us: access_unit.cts().unwrap_or(frame.timestamp_us),
                is_keyframe: access_unit.rap() || annex_b_contains_vvc_keyframe(&bytes),
                bytes,
            }])
        }
    }

    pub fn compiled() -> bool {
        true
    }

    pub fn probe_available() -> Result<(), PipelineError> {
        Inner::new(16, 16, 30, 1_000_000).map(|_| ())
    }

    fn validate_even_dimensions(width: usize, height: usize) -> Result<(), PipelineError> {
        if width == 0 || height == 0 {
            return Err(PipelineError::message(format!(
                "VVenC frame dimensions must be non-zero, got {width}x{height}"
            )));
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(PipelineError::message(format!(
                "VVenC requires even frame dimensions, got {width}x{height}"
            )));
        }
        Ok(())
    }

    fn vvenc_thread_count() -> i32 {
        std::thread::available_parallelism()
            .map(|count| count.get().clamp(1, 8) as i32)
            .unwrap_or(1)
    }

    fn vvenc_output_len(width: usize, height: usize) -> Result<usize, PipelineError> {
        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(6))
            .map(|bytes| bytes.max(1_048_576))
            .ok_or_else(|| PipelineError::message("VVenC output buffer size overflow"))
    }

    fn frame_input_len(frame: &CapturedFrame) -> Result<usize, PipelineError> {
        let pixels = frame
            .width
            .checked_mul(frame.height)
            .ok_or_else(|| PipelineError::message("frame pixel count overflow"))?;
        match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels
                .checked_mul(4)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
            FramePixelFormat::Rgb24 => pixels
                .checked_mul(3)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
            FramePixelFormat::Nv12 => pixels
                .checked_mul(3)
                .map(|bytes| bytes / 2)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
        }
    }

    fn write_yuv420_i16(
        frame: &CapturedFrame,
        yuv: &mut YUVBuffer<u64>,
    ) -> Result<(), PipelineError> {
        let expected_len = frame_input_len(frame)?;
        if frame.data.len() != expected_len {
            return Err(PipelineError::message(format!(
                "VVenC frame bytes mismatch: expected {expected_len}, got {}",
                frame.data.len()
            )));
        }

        let mut y_plane = yuv.plane_mut(YUVComponent::Y);
        let mut u_plane = yuv.plane_mut(YUVComponent::U);
        let mut v_plane = yuv.plane_mut(YUVComponent::V);
        let y_stride = y_plane.stride() as usize;
        let u_stride = u_plane.stride() as usize;
        let v_stride = v_plane.stride() as usize;
        let y_data = y_plane.data_mut();
        let u_data = u_plane.data_mut();
        let v_data = v_plane.data_mut();

        if frame.pixel_format == FramePixelFormat::Nv12 {
            let y_size = frame.width * frame.height;
            for row in 0..frame.height {
                let src_row = row * frame.width;
                let dst_row = row * y_stride;
                for col in 0..frame.width {
                    y_data[dst_row + col] = i16::from(frame.data[src_row + col]);
                }
            }

            let uv_plane = &frame.data[y_size..expected_len];
            for row in 0..frame.height / 2 {
                for col in 0..frame.width / 2 {
                    let nv12_index = row * frame.width + col * 2;
                    u_data[row * u_stride + col] = i16::from(uv_plane[nv12_index]);
                    v_data[row * v_stride + col] = i16::from(uv_plane[nv12_index + 1]);
                }
            }
            return Ok(());
        }

        let bytes_per_pixel = match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
            FramePixelFormat::Rgb24 => 3,
            FramePixelFormat::Nv12 => unreachable!("NV12 was copied above"),
        };

        for block_y in (0..frame.height).step_by(2) {
            for block_x in (0..frame.width).step_by(2) {
                let p00 = read_rgb(frame, block_x, block_y, bytes_per_pixel);
                let p10 = read_rgb(frame, block_x + 1, block_y, bytes_per_pixel);
                let p01 = read_rgb(frame, block_x, block_y + 1, bytes_per_pixel);
                let p11 = read_rgb(frame, block_x + 1, block_y + 1, bytes_per_pixel);

                y_data[block_y * y_stride + block_x] = i16::from(rgb_to_y(p00));
                y_data[block_y * y_stride + block_x + 1] = i16::from(rgb_to_y(p10));
                y_data[(block_y + 1) * y_stride + block_x] = i16::from(rgb_to_y(p01));
                y_data[(block_y + 1) * y_stride + block_x + 1] = i16::from(rgb_to_y(p11));

                let avg = average_rgb([p00, p10, p01, p11]);
                let uv_x = block_x / 2;
                let uv_y = block_y / 2;
                u_data[uv_y * u_stride + uv_x] = i16::from(rgb_to_u(avg));
                v_data[uv_y * v_stride + uv_x] = i16::from(rgb_to_v(avg));
            }
        }

        Ok(())
    }

    fn read_rgb(frame: &CapturedFrame, x: usize, y: usize, bytes_per_pixel: usize) -> (u8, u8, u8) {
        let index = (y * frame.width + x) * bytes_per_pixel;
        match frame.pixel_format {
            FramePixelFormat::Bgra32 => (
                frame.data[index + 2],
                frame.data[index + 1],
                frame.data[index],
            ),
            FramePixelFormat::Rgba32 | FramePixelFormat::Rgb24 => (
                frame.data[index],
                frame.data[index + 1],
                frame.data[index + 2],
            ),
            FramePixelFormat::Nv12 => unreachable!("NV12 is not read as packed RGB"),
        }
    }

    fn average_rgb(pixels: [(u8, u8, u8); 4]) -> (u8, u8, u8) {
        let (mut r, mut g, mut b) = (0_u32, 0_u32, 0_u32);
        for (pr, pg, pb) in pixels {
            r += u32::from(pr);
            g += u32::from(pg);
            b += u32::from(pb);
        }
        ((r / 4) as u8, (g / 4) as u8, (b / 4) as u8)
    }

    fn rgb_to_y((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((66 * i32::from(r) + 129 * i32::from(g) + 25 * i32::from(b) + 128) >> 8) + 16)
    }

    fn rgb_to_u((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((-38 * i32::from(r) - 74 * i32::from(g) + 112 * i32::from(b) + 128) >> 8) + 128)
    }

    fn rgb_to_v((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((112 * i32::from(r) - 94 * i32::from(g) - 18 * i32::from(b) + 128) >> 8) + 128)
    }

    fn clamp_u8(value: i32) -> u8 {
        value.clamp(0, 255) as u8
    }

    fn annex_b_contains_vvc_keyframe(access_unit: &[u8]) -> bool {
        let mut offset = 0usize;
        while offset + 5 < access_unit.len() {
            if access_unit[offset..].starts_with(&[0, 0, 0, 1]) {
                if vvc_nal_is_keyframe(&access_unit[offset + 4..]) {
                    return true;
                }
                offset += 4;
            } else if access_unit[offset..].starts_with(&[0, 0, 1]) {
                if vvc_nal_is_keyframe(&access_unit[offset + 3..]) {
                    return true;
                }
                offset += 3;
            } else {
                offset += 1;
            }
        }
        false
    }

    fn vvc_nal_is_keyframe(nal: &[u8]) -> bool {
        if nal.len() < 2 {
            return false;
        }
        matches!((nal[1] >> 3) & 0x1f, 7 | 8 | 9 | 10 | 14 | 15 | 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "software-vvenc"))]
    fn default_build_reports_vvenc_not_compiled() {
        let error = match VvencSoftwareEncoder::new(16, 16, 30) {
            Ok(_) => panic!("VVenC should be gated"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("software-vvenc"));
        assert!(!vvenc_software_compiled());
    }
}
