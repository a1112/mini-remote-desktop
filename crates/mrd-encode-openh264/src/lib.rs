use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FramePixelFormat, PipelineError, VideoCodec, VideoEncoder,
};
use openh264::{
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode,
        UsageType,
    },
    formats::YUVSlices,
    OpenH264API,
};

pub struct OpenH264Encoder {
    encoder: Encoder,
    width: usize,
    height: usize,
    fps: u32,
    frame_index: u64,
    i420: Vec<u8>,
}

impl OpenH264Encoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_internal(width, height, fps, None)
    }

    pub fn new_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_internal(width, height, fps, Some(bitrate.max(1)))
    }

    fn new_internal(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: Option<u32>,
    ) -> Result<Self, PipelineError> {
        validate_even_dimensions(width, height)?;

        let rate_control_mode = if bitrate.is_some() {
            RateControlMode::Bitrate
        } else {
            RateControlMode::Off
        };

        let mut config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .max_frame_rate(FrameRate::from_hz(fps.max(1) as f32))
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.max(1)))
            .rate_control_mode(rate_control_mode)
            .complexity(Complexity::Low)
            .num_threads(openh264_thread_count())
            .scene_change_detect(true)
            .adaptive_quantization(false)
            .background_detection(false)
            .skip_frames(bitrate.is_some());

        if let Some(bitrate) = bitrate {
            config = config.bitrate(BitRate::from_bps(bitrate));
        }

        let api = OpenH264API::from_source();
        let encoder = Encoder::with_api_config(api, config).map_err(|error| {
            PipelineError::message(format!("create openh264 encoder failed: {error}"))
        })?;

        Ok(Self {
            encoder,
            width,
            height,
            fps: fps.max(1),
            frame_index: 0,
            i420: vec![0; i420_len(width, height)?],
        })
    }

    pub fn new_speed(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new(width, height, fps)
    }
}

impl VideoEncoder for OpenH264Encoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        validate_even_dimensions(frame.width, frame.height)?;

        if frame.width != self.width || frame.height != self.height {
            return Err(PipelineError::message(format!(
                "frame size mismatch: expected {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            )));
        }

        if self.frame_index == 0 || self.frame_index % self.fps as u64 == 0 {
            self.encoder.force_intra_frame();
        }

        write_i420(frame, &mut self.i420)?;
        let y_size = frame.width * frame.height;
        let uv_size = y_size / 4;
        let yuv = YUVSlices::new(
            (
                &self.i420[..y_size],
                &self.i420[y_size..y_size + uv_size],
                &self.i420[y_size + uv_size..],
            ),
            (frame.width, frame.height),
            (frame.width, frame.width / 2, frame.width / 2),
        );
        let bitstream = self
            .encoder
            .encode(&yuv)
            .map_err(|error| PipelineError::message(format!("openh264 encode failed: {error}")))?;
        self.frame_index += 1;

        Ok(vec![EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us: frame.timestamp_us,
            is_keyframe: true,
            bytes: normalize_h264_bitstream(bitstream.to_vec()),
        }])
    }
}

fn validate_even_dimensions(width: usize, height: usize) -> Result<(), PipelineError> {
    if width == 0 || height == 0 {
        return Err(PipelineError::message(format!(
            "openh264 frame dimensions must be non-zero, got {width}x{height}"
        )));
    }

    if width % 2 != 0 || height % 2 != 0 {
        return Err(PipelineError::message(format!(
            "openh264 requires even frame dimensions, got {width}x{height}"
        )));
    }

    Ok(())
}

fn openh264_thread_count() -> u16 {
    std::thread::available_parallelism()
        .map(|count| count.get().clamp(1, 8) as u16)
        .unwrap_or(1)
}

fn i420_len(width: usize, height: usize) -> Result<usize, PipelineError> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3))
        .map(|bytes| bytes / 2)
        .ok_or_else(|| PipelineError::message("openh264 i420 buffer size overflow"))
}

fn normalize_h264_bitstream(bytes: Vec<u8>) -> Vec<u8> {
    if looks_like_annex_b(&bytes) {
        return bytes;
    }

    if let Some(converted) = avcc_to_annex_b(&bytes) {
        return converted;
    }

    bytes
}

fn looks_like_annex_b(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window == [0, 0, 0, 1])
        || bytes.windows(3).any(|window| window == [0, 0, 1])
}

fn avcc_to_annex_b(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut offset = 0usize;
    let mut annex_b = Vec::with_capacity(bytes.len() + 16);

    while offset + 4 <= bytes.len() {
        let nal_len = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        if nal_len == 0 || offset + nal_len > bytes.len() {
            return None;
        }

        annex_b.extend_from_slice(&[0, 0, 0, 1]);
        annex_b.extend_from_slice(&bytes[offset..offset + nal_len]);
        offset += nal_len;
    }

    if offset == bytes.len() && !annex_b.is_empty() {
        Some(annex_b)
    } else {
        None
    }
}

fn write_i420(frame: &CapturedFrame, out: &mut [u8]) -> Result<(), PipelineError> {
    let expected_len = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels.checked_mul(4),
            FramePixelFormat::Rgb24 => pixels.checked_mul(3),
        })
        .ok_or_else(|| PipelineError::message("frame buffer size overflow"))?;

    if frame.data.len() != expected_len {
        return Err(PipelineError::message(format!(
            "frame bytes mismatch: expected {expected_len}, got {}",
            frame.data.len()
        )));
    }

    let expected_i420 = i420_len(frame.width, frame.height)?;
    if out.len() != expected_i420 {
        return Err(PipelineError::message(format!(
            "openh264 i420 scratch mismatch: expected {expected_i420}, got {}",
            out.len()
        )));
    }

    let y_size = frame.width * frame.height;
    let uv_size = y_size / 4;
    let (y_plane, uv_planes) = out.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);
    let bytes_per_pixel = match frame.pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
    };

    for block_y in (0..frame.height).step_by(2) {
        for block_x in (0..frame.width).step_by(2) {
            let p00 = read_rgb(frame, block_x, block_y, bytes_per_pixel);
            let p10 = read_rgb(frame, block_x + 1, block_y, bytes_per_pixel);
            let p01 = read_rgb(frame, block_x, block_y + 1, bytes_per_pixel);
            let p11 = read_rgb(frame, block_x + 1, block_y + 1, bytes_per_pixel);

            y_plane[block_y * frame.width + block_x] = rgb_to_y(p00);
            y_plane[block_y * frame.width + block_x + 1] = rgb_to_y(p10);
            y_plane[(block_y + 1) * frame.width + block_x] = rgb_to_y(p01);
            y_plane[(block_y + 1) * frame.width + block_x + 1] = rgb_to_y(p11);

            let avg = average_rgb([p00, p10, p01, p11]);
            let uv_index = (block_y / 2) * (frame.width / 2) + (block_x / 2);
            u_plane[uv_index] = rgb_to_u(avg);
            v_plane[uv_index] = rgb_to_v(avg);
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

#[allow(dead_code)]
fn to_rgba(frame: &CapturedFrame) -> Result<Vec<u8>, PipelineError> {
    let expected_len = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels.checked_mul(4),
            FramePixelFormat::Rgb24 => pixels.checked_mul(3),
        })
        .ok_or_else(|| PipelineError::message("frame buffer size overflow"))?;

    if frame.data.len() != expected_len {
        return Err(PipelineError::message(format!(
            "frame bytes mismatch: expected {expected_len}, got {}",
            frame.data.len()
        )));
    }

    match frame.pixel_format {
        FramePixelFormat::Rgba32 => Ok(frame.data.clone()),
        FramePixelFormat::Bgra32 => {
            // Optimized BGRA→RGBA conversion using swap_words
            // BGRA = [B, G, R, A], RGBA = [R, G, B, A]
            // We need to swap B and R in each 4-byte pixel
            let mut rgba = Vec::with_capacity(frame.data.len());
            for chunk in frame.data.chunks_exact(4) {
                // Swap R and B channels
                rgba.extend_from_slice(&[chunk[2], chunk[1], chunk[0], chunk[3]]);
            }
            Ok(rgba)
        }
        FramePixelFormat::Rgb24 => {
            let mut rgba = Vec::with_capacity(frame.width * frame.height * 4);
            for chunk in frame.data.chunks_exact(3) {
                rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            Ok(rgba)
        }
    }
}

#[cfg(test)]
mod conversion_tests {
    use super::*;

    #[test]
    fn bgra_to_i420_writes_expected_limited_range_planes() {
        let frame = CapturedFrame {
            width: 2,
            height: 2,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: 0,
            data: [0, 0, 255, 255]
                .into_iter()
                .cycle()
                .take(2 * 2 * 4)
                .collect(),
        };
        let mut i420 = vec![0; i420_len(2, 2).expect("i420 size")];

        write_i420(&frame, &mut i420).expect("convert bgra to i420");

        assert_eq!(&i420[..4], &[82, 82, 82, 82]);
        assert_eq!(&i420[4..5], &[90]);
        assert_eq!(&i420[5..], &[240]);
    }
}

#[cfg(test)]
mod tests {
    use super::{avcc_to_annex_b, looks_like_annex_b, normalize_h264_bitstream};

    #[test]
    fn avcc_bitstream_is_converted_to_annex_b() {
        let avcc = vec![0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 3, 0x68, 0xce, 0x06];
        let annex_b = avcc_to_annex_b(&avcc).expect("convert avcc");

        assert_eq!(
            annex_b,
            vec![0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xce, 0x06]
        );
        assert!(looks_like_annex_b(&annex_b));
        assert_eq!(normalize_h264_bitstream(avcc), annex_b);
    }
}
