pub use mrd_pipeline_core::RuntimeStatus;
use openh264::{decoder::Decoder as OpenH264Decoder, formats::YUVSource, Error as OpenH264Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    H264,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb24,
    D3d11Texture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderDescriptor {
    pub id: &'static str,
    pub codec: CodecKind,
    pub runtime_status: RuntimeStatus,
    pub output_formats: &'static [PixelFormat],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub data: Vec<u8>,
}

impl DecodedFrame {
    pub fn cpu_rgb24(width: usize, height: usize, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixel_format: PixelFormat::Rgb24,
            data,
        }
    }

    pub fn bytes_len(&self) -> usize {
        match self.pixel_format {
            PixelFormat::Rgb24 => self.data.len(),
            PixelFormat::D3d11Texture => 0,
        }
    }

    pub fn cpu_bytes(&self) -> Option<&[u8]> {
        match self.pixel_format {
            PixelFormat::Rgb24 => Some(self.data.as_slice()),
            PixelFormat::D3d11Texture => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrameInfo {
    pub width: usize,
    pub height: usize,
    pub pixel_format: PixelFormat,
    pub bytes: usize,
}

#[derive(Debug)]
pub struct DecoderError {
    message: String,
}

impl DecoderError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn from_message(message: impl Into<String>) -> Self {
        Self::new(message)
    }
}

impl std::fmt::Display for DecoderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DecoderError {}

impl From<OpenH264Error> for DecoderError {
    fn from(value: OpenH264Error) -> Self {
        Self::new(format!("openh264 解码失败: {value}"))
    }
}

pub trait VideoDecoder: Send {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), DecoderError>;
    fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame>;
}

const RGB24_OUTPUTS: &[PixelFormat] = &[PixelFormat::Rgb24];
const H264_SOFTWARE_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "h264_software",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};
const NVDEC_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec",
    codec: CodecKind::H264,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

pub fn available_decoder_descriptors() -> Vec<DecoderDescriptor> {
    vec![H264_SOFTWARE_DESCRIPTOR.clone(), NVDEC_DESCRIPTOR.clone()]
}

pub fn create_decoder(id: &str) -> Result<Box<dyn VideoDecoder>, DecoderError> {
    match id {
        "h264_software" => Ok(Box::new(H264SoftwareDecoder::new()?)),
        "nvdec" => Ok(Box::new(NvdecVideoDecoder::new()?)),
        other => Err(DecoderError::new(format!("未知 decoder backend: {other}"))),
    }
}

pub struct H264SoftwareDecoder {
    decoder: OpenH264Decoder,
    pending_frames: Vec<DecodedFrame>,
}

pub struct NvdecVideoDecoder {
    decoder: mrd_decode_nvdec::NvdecDecoder,
}

impl NvdecVideoDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new().map_err(DecoderError::from_message)?;
        Ok(Self { decoder })
    }
}

impl H264SoftwareDecoder {
    pub fn new() -> Result<Self, DecoderError> {
        Ok(Self {
            decoder: OpenH264Decoder::new()?,
            pending_frames: Vec::new(),
        })
    }
}

impl VideoDecoder for H264SoftwareDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), DecoderError> {
        match self.decoder.decode(access_unit)? {
            Some(yuv) => {
                let mut rgb = vec![0_u8; yuv.rgb8_len()];
                yuv.write_rgb8(&mut rgb);
                let (width, height) = yuv.dimensions();
                self.pending_frames
                    .push(DecodedFrame::cpu_rgb24(width, height, rgb));
                Ok(())
            }
            None => Err(DecoderError::new("访问单元未生成完整可解码帧")),
        }
    }

    fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
        std::mem::take(&mut self.pending_frames)
    }
}

impl VideoDecoder for NvdecVideoDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), DecoderError> {
        self.decoder
            .push_access_unit(access_unit)
            .map_err(DecoderError::from_message)
    }

    fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
        self.decoder
            .drain_decoded_frames()
            .into_iter()
            .map(|frame| DecodedFrame::cpu_rgb24(frame.width, frame.height, frame.data))
            .collect()
    }
}
