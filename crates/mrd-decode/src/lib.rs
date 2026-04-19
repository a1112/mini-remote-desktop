pub use mrd_pipeline_core::{
    RuntimeStatus, VideoDecoder, DecodedFrame as CoreDecodedFrame, DecodedFrameData,
    PipelineError,
};
use openh264::decoder::Decoder as OpenH264Decoder;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    H264,
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
const BGRA32_OUTPUTS: &[PixelFormat] = &[PixelFormat::Bgra32];

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
    vec![
        H264_SOFTWARE_DESCRIPTOR.clone(),
        NVDEC_DESCRIPTOR.clone(),
    ]
}

pub fn create_decoder(id: &str) -> Result<Box<dyn VideoDecoder>, PipelineError> {
    match id {
        "h264_software" => Ok(Box::new(H264SoftwareDecoder::new()?)),
        "nvdec" => Ok(Box::new(NvdecVideoDecoder::new()?)),
        other => Err(PipelineError::Message(format!("未知 decoder backend: {other}"))),
    }
}

pub struct H264SoftwareDecoder {
    decoder: OpenH264Decoder,
    pending_timestamp_us: Option<u64>,
}

pub struct NvdecVideoDecoder {
    decoder: mrd_decode_nvdec::NvdecDecoder,
}

impl NvdecVideoDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        let decoder = mrd_decode_nvdec::NvdecDecoder::new()
            .map_err(|e| PipelineError::Message(format!("nvdec 创建失败: {e}")))?;
        Ok(Self { decoder })
    }
}

impl H264SoftwareDecoder {
    pub fn new() -> Result<Self, PipelineError> {
        Ok(Self {
            decoder: OpenH264Decoder::new().map_err(|e| {
                PipelineError::Message(format!("openh264 初始化失败: {e}"))
            })?,
            pending_timestamp_us: None,
        })
    }
}

impl VideoDecoder for H264SoftwareDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        match self.decoder.decode(access_unit) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(PipelineError::Message(
                "访问单元未生成完整可解码帧".to_string(),
            )),
            Err(e) => Err(PipelineError::Message(format!("openh264 解码失败: {e}"))),
        }
    }

    fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
        let frames = Vec::new();
        // TODO: 实现帧提取
        frames
    }
}

impl VideoDecoder for NvdecVideoDecoder {
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
        self.decoder
            .push_access_unit(access_unit)
            .map_err(|e| PipelineError::Message(format!("nvdec 解码失败: {e}")))
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
                #[cfg(windows)]
                NvdecDecodedFrameData::D3D11SharedNv12 {
                    shared_handle,
                    width: _,
                    height: _,
                } => CoreDecodedFrame::from_d3d11_shared_nv12(
                    frame.width,
                    frame.height,
                    0,
                    shared_handle,
                ),
            })
            .collect()
    }
}
