pub use mrd_pipeline_core::{
    DecodedFrame as CoreDecodedFrame, DecodedFrameData, PipelineError, RuntimeStatus, VideoDecoder,
};
use openh264::{
    decoder::{DecodedYUV, Decoder as OpenH264Decoder},
    formats::YUVSource,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    H264,
    Hevc,
    HevcMain10,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgb24,
    Bgra32,
    D3d11Texture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderDescriptor {
    pub id: &'static str,
    pub codec: CodecKind,
    pub runtime_status: RuntimeStatus,
    pub output_formats: &'static [PixelFormat],
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

const NVDEC_AV1_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_av1",
    codec: CodecKind::Av1,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_HEVC_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc",
    codec: CodecKind::Hevc,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

const NVDEC_HEVC_MAIN10_DESCRIPTOR: DecoderDescriptor = DecoderDescriptor {
    id: "nvdec_hevc_main10",
    codec: CodecKind::HevcMain10,
    runtime_status: RuntimeStatus::RuntimeBacked,
    output_formats: RGB24_OUTPUTS,
};

pub fn available_decoder_descriptors() -> Vec<DecoderDescriptor> {
    vec![
        H264_SOFTWARE_DESCRIPTOR.clone(),
        NVDEC_DESCRIPTOR.clone(),
        NVDEC_HEVC_DESCRIPTOR.clone(),
        NVDEC_HEVC_MAIN10_DESCRIPTOR.clone(),
        NVDEC_AV1_DESCRIPTOR.clone(),
    ]
}

pub fn create_decoder(id: &str) -> Result<Box<dyn VideoDecoder>, PipelineError> {
    match id {
        "h264_software" => Ok(Box::new(H264SoftwareDecoder::new()?)),
        "nvdec" => Ok(Box::new(NvdecVideoDecoder::new()?)),
        "nvdec_hevc" => Ok(Box::new(NvdecVideoDecoder::new_hevc()?)),
        "nvdec_hevc_main10" => Ok(Box::new(NvdecVideoDecoder::new_hevc_main10()?)),
        "nvdec_av1" => Ok(Box::new(NvdecVideoDecoder::new_av1()?)),
        other => Err(PipelineError::Message(format!(
            "unknown decoder backend: {other}"
        ))),
    }
}

pub struct H264SoftwareDecoder {
    decoder: OpenH264Decoder,
    decoded_frames: Vec<CoreDecodedFrame>,
}

pub struct NvdecVideoDecoder {
    decoder: mrd_decode_nvdec::NvdecDecoder,
}

impl NvdecVideoDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new()
            .map_err(|e| PipelineError::Message(format!("nvdec create failed: {e}")))?;
        Ok(Self { decoder })
    }

    pub fn new_av1() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_av1_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuRgb24,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec av1 create failed: {e}")))?;
        Ok(Self { decoder })
    }

    pub fn new_hevc() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuRgb24,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec hevc create failed: {e}")))?;
        Ok(Self { decoder })
    }

    pub fn new_hevc_main10() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new_hevc_main10_with_output_mode(
            mrd_decode_nvdec::NvdecOutputMode::CpuRgb24,
        )
        .map_err(|e| PipelineError::Message(format!("nvdec hevc main10 create failed: {e}")))?;
        Ok(Self { decoder })
    }
}

impl H264SoftwareDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        Ok(Self {
            decoder: OpenH264Decoder::new()
                .map_err(|e| PipelineError::Message(format!("openh264 init failed: {e}")))?,
            decoded_frames: Vec::new(),
        })
    }
}

impl VideoDecoder for H264SoftwareDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        let decoded_frame = match self.decoder.decode(access_unit) {
            Ok(Some(decoded)) => Some(decoded_yuv_to_rgb_frame(&decoded, 0)),
            Ok(None) => None,
            Err(e) => {
                return Err(PipelineError::Message(format!(
                    "openh264 decode failed: {e}"
                )))
            }
        };

        if let Some(frame) = decoded_frame {
            self.decoded_frames.push(frame);
        }

        Ok(())
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        std::mem::take(&mut self.decoded_frames)
    }
}

fn decoded_yuv_to_rgb_frame(decoded: &DecodedYUV<'_>, timestamp_us: u64) -> CoreDecodedFrame {
    let (width, height) = decoded.dimensions();
    let mut rgb = vec![0_u8; width * height * 3];
    decoded.write_rgb8(&mut rgb);
    CoreDecodedFrame::from_cpu_rgb24(width, height, timestamp_us, rgb)
}

impl VideoDecoder for NvdecVideoDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        self.decoder
            .push_access_unit(access_unit)
            .map_err(|e| PipelineError::Message(format!("nvdec decode failed: {e}")))
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        use mrd_decode_nvdec::NvdecDecodedFrameData;
        self.decoder
            .drain_decoded_frames()
            .into_iter()
            .map(|frame| match frame.data {
                NvdecDecodedFrameData::CpuRgb24(data) => {
                    CoreDecodedFrame::from_cpu_rgb24(frame.width, frame.height, 0, data)
                }
                NvdecDecodedFrameData::CpuNv12 { data, pitch } => {
                    CoreDecodedFrame::from_cpu_nv12(frame.width, frame.height, 0, pitch, data)
                }
                NvdecDecodedFrameData::CpuP010 { data, pitch } => {
                    CoreDecodedFrame::from_cpu_p010(frame.width, frame.height, 0, pitch, data)
                }
                #[cfg(windows)]
                NvdecDecodedFrameData::D3D11SharedNv12 {
                    shared_handle_y,
                    shared_handle_uv,
                    width: _,
                    height: _,
                } => CoreDecodedFrame::from_d3d11_shared_nv12(
                    frame.width,
                    frame.height,
                    0,
                    shared_handle_y,
                    shared_handle_uv,
                ),
                #[cfg(windows)]
                NvdecDecodedFrameData::D3D11SharedP010 {
                    shared_handle_y,
                    shared_handle_uv,
                    width: _,
                    height: _,
                } => CoreDecodedFrame::from_d3d11_shared_p010(
                    frame.width,
                    frame.height,
                    0,
                    shared_handle_y,
                    shared_handle_uv,
                ),
            })
            .collect()
    }
}
