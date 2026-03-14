use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FramePixelFormat, PipelineError, VideoCodec, VideoEncoder,
};
use openh264::{
    encoder::{Encoder, EncoderConfig, FrameRate, IntraFramePeriod, UsageType},
    formats::{RgbaSliceU8, YUVBuffer},
    OpenH264API,
};

pub struct OpenH264Encoder {
    encoder: Encoder,
    width: usize,
    height: usize,
    fps: u32,
    frame_index: u64,
}

impl OpenH264Encoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .max_frame_rate(FrameRate::from_hz(fps.max(1) as f32))
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.max(1)))
            .skip_frames(false);
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
        })
    }

    pub fn new_speed(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new(width, height, fps)
    }
}

impl VideoEncoder for OpenH264Encoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        if frame.width != self.width || frame.height != self.height {
            return Err(PipelineError::message(format!(
                "frame size mismatch: expected {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            )));
        }

        if self.frame_index == 0 || self.frame_index % self.fps as u64 == 0 {
            self.encoder.force_intra_frame();
        }

        let rgba = to_rgba(frame)?;
        let rgba_source = RgbaSliceU8::new(&rgba, (frame.width, frame.height));
        let yuv = YUVBuffer::from_rgb_source(rgba_source);
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
            let mut rgba = Vec::with_capacity(frame.data.len());
            for chunk in frame.data.chunks_exact(4) {
                rgba.push(chunk[2]);
                rgba.push(chunk[1]);
                rgba.push(chunk[0]);
                rgba.push(chunk[3]);
            }
            Ok(rgba)
        }
        FramePixelFormat::Rgb24 => {
            let mut rgba = Vec::with_capacity(frame.width * frame.height * 4);
            for chunk in frame.data.chunks_exact(3) {
                rgba.push(chunk[0]);
                rgba.push(chunk[1]);
                rgba.push(chunk[2]);
                rgba.push(255);
            }
            Ok(rgba)
        }
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
