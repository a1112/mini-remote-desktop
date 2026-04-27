use mrd_proto::SessionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod encoder_config;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeStatus {
    ProfileOnly,
    RuntimeBacked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeBinding {
    pub session_id: SessionId,
    pub runtime_status: RuntimeStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Av1,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FramePixelFormat {
    Bgra32,
    Rgba32,
    Rgb24,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FrameMemoryKind {
    Cpu,
    #[cfg(windows)]
    D3D11SharedBgra,
    #[cfg(windows)]
    D3D11SharedNv12,
}

#[cfg(windows)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct D3D11SharedBgraFrame {
    pub shared_handle: isize,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: usize,
    pub height: usize,
    pub pixel_format: FramePixelFormat,
    pub timestamp_us: u64,
    pub data: Vec<u8>,
    #[cfg(windows)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub d3d11_shared_bgra: Option<D3D11SharedBgraFrame>,
}

impl CapturedFrame {
    pub fn from_cpu(
        width: usize,
        height: usize,
        pixel_format: FramePixelFormat,
        timestamp_us: u64,
        data: Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format,
            timestamp_us,
            data,
            #[cfg(windows)]
            d3d11_shared_bgra: None,
        }
    }

    #[cfg(windows)]
    pub fn from_d3d11_shared_bgra(
        width: usize,
        height: usize,
        timestamp_us: u64,
        shared_handle: isize,
        row_pitch: u32,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us,
            data: Vec::new(),
            d3d11_shared_bgra: Some(D3D11SharedBgraFrame {
                shared_handle,
                width: width as u32,
                height: height as u32,
                row_pitch,
            }),
        }
    }

    pub fn is_cpu_backed(&self) -> bool {
        !self.data.is_empty()
    }

    #[cfg(windows)]
    pub fn d3d11_shared_bgra(&self) -> Option<&D3D11SharedBgraFrame> {
        self.d3d11_shared_bgra.as_ref()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodedAccessUnit {
    pub codec: VideoCodec,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub bytes: Vec<u8>,
}

/// Decoded frame data type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedFrameData {
    /// CPU RGB24 data (existing path)
    CpuRgb24(Vec<u8>),
    /// CPU BGRA32 data (optimized path for D3D11 rendering)
    CpuBgra32(Vec<u8>),
    /// CPU NV12 data with decoder pitch.
    CpuNv12 { data: Vec<u8>, pitch: usize },
    /// D3D11 shared texture handle (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12 {
        shared_handle_y: isize,
        shared_handle_uv: isize,
        width: u32,
        height: u32,
    },
}

/// Decoded video frame
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub width: usize,
    pub height: usize,
    pub timestamp_us: u64,
    pub data: DecodedFrameData,
}

impl DecodedFrame {
    /// Create a decoded frame from CPU RGB24 data
    pub fn from_cpu_rgb24(width: usize, height: usize, timestamp_us: u64, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            timestamp_us,
            data: DecodedFrameData::CpuRgb24(data),
        }
    }

    /// Create a decoded frame from CPU BGRA32 data
    pub fn from_cpu_bgra32(width: usize, height: usize, timestamp_us: u64, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            timestamp_us,
            data: DecodedFrameData::CpuBgra32(data),
        }
    }

    /// Create a decoded frame from CPU NV12 data
    pub fn from_cpu_nv12(
        width: usize,
        height: usize,
        timestamp_us: u64,
        pitch: usize,
        data: Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            timestamp_us,
            data: DecodedFrameData::CpuNv12 { data, pitch },
        }
    }

    /// Create a decoded frame from D3D11 shared texture
    #[cfg(windows)]
    pub fn from_d3d11_shared_nv12(
        width: usize,
        height: usize,
        timestamp_us: u64,
        shared_handle_y: isize,
        shared_handle_uv: isize,
    ) -> Self {
        Self {
            width,
            height,
            timestamp_us,
            data: DecodedFrameData::D3D11SharedNv12 {
                shared_handle_y,
                shared_handle_uv,
                width: width as u32,
                height: height as u32,
            },
        }
    }

    /// Check if this frame uses shared texture (zero-copy)
    pub fn is_shared_texture(&self) -> bool {
        matches!(self.data, DecodedFrameData::D3D11SharedNv12 { .. })
    }

    /// Get the CPU RGB24 data if available
    pub fn cpu_rgb24(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::CpuRgb24(data) => Some(data.as_slice()),
            _ => None,
        }
    }

    /// Get the CPU BGRA32 data if available
    pub fn cpu_bgra32(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::CpuBgra32(data) => Some(data.as_slice()),
            _ => None,
        }
    }

    /// Get the CPU NV12 data and pitch if available
    pub fn cpu_nv12(&self) -> Option<(&[u8], usize)> {
        match &self.data {
            DecodedFrameData::CpuNv12 { data, pitch } => Some((data.as_slice(), *pitch)),
            _ => None,
        }
    }

    /// Get any CPU data as bytes
    pub fn cpu_bytes(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::CpuRgb24(data)
            | DecodedFrameData::CpuBgra32(data)
            | DecodedFrameData::CpuNv12 { data, .. } => Some(data.as_slice()),
            #[cfg(windows)]
            DecodedFrameData::D3D11SharedNv12 { .. } => None,
        }
    }

    /// Get the shared texture handle if available
    #[cfg(windows)]
    pub fn d3d11_shared_handle(&self) -> Option<isize> {
        match &self.data {
            DecodedFrameData::D3D11SharedNv12 {
                shared_handle_y, ..
            } => Some(*shared_handle_y),
            _ => None,
        }
    }

    /// Get the shared Y and UV texture handles if available.
    #[cfg(windows)]
    pub fn d3d11_shared_handles(&self) -> Option<(isize, isize)> {
        match &self.data {
            DecodedFrameData::D3D11SharedNv12 {
                shared_handle_y,
                shared_handle_uv,
                ..
            } => Some((*shared_handle_y, *shared_handle_uv)),
            _ => None,
        }
    }
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("{0}")]
    Message(String),
}

impl PipelineError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

pub trait FrameCapture {
    fn output_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError>;
}

pub trait VideoEncoder {
    fn input_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError>;
}

pub trait VideoDecoder: Send {
    fn output_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    /// Push an encoded access unit to the decoder
    fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError>;

    /// Drain all decoded frames
    fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame>;
}
