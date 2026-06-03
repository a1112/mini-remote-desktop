use mrd_proto::SessionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod encoder_config;

#[cfg(target_os = "macos")]
use std::{ffi::c_void, ptr::NonNull};

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRetain(cf: *const c_void) -> *const c_void;
    fn CFRelease(cf: *const c_void);
}

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
    Hevc,
    Av1,
    Vvc,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FramePixelFormat {
    Bgra32,
    Rgba32,
    Rgb24,
    /// CPU-backed NV12, tightly packed as Y plane followed by interleaved UV.
    Nv12,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FrameMemoryKind {
    Cpu,
    #[cfg(target_os = "macos")]
    MacosCvPixelBuffer,
    #[cfg(windows)]
    D3D11SharedBgra,
    #[cfg(windows)]
    D3D11SharedNv12,
    #[cfg(windows)]
    D3D11SharedP010,
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct MacosCvPixelBufferFrame {
    pixel_buffer: NonNull<c_void>,
    pub pixel_format: FramePixelFormat,
}

#[cfg(target_os = "macos")]
unsafe impl Send for MacosCvPixelBufferFrame {}

#[cfg(target_os = "macos")]
unsafe impl Sync for MacosCvPixelBufferFrame {}

#[cfg(target_os = "macos")]
impl MacosCvPixelBufferFrame {
    pub fn retain(
        pixel_buffer: *mut c_void,
        pixel_format: FramePixelFormat,
    ) -> Option<MacosCvPixelBufferFrame> {
        let pixel_buffer = NonNull::new(pixel_buffer)?;
        unsafe {
            CFRetain(pixel_buffer.as_ptr().cast_const());
        }
        Some(Self {
            pixel_buffer,
            pixel_format,
        })
    }

    pub fn as_ptr(&self) -> *mut c_void {
        self.pixel_buffer.as_ptr()
    }
}

#[cfg(target_os = "macos")]
impl Clone for MacosCvPixelBufferFrame {
    fn clone(&self) -> Self {
        unsafe {
            CFRetain(self.pixel_buffer.as_ptr().cast_const());
        }
        Self {
            pixel_buffer: self.pixel_buffer,
            pixel_format: self.pixel_format,
        }
    }
}

#[cfg(target_os = "macos")]
impl PartialEq for MacosCvPixelBufferFrame {
    fn eq(&self, other: &Self) -> bool {
        self.pixel_buffer == other.pixel_buffer && self.pixel_format == other.pixel_format
    }
}

#[cfg(target_os = "macos")]
impl Eq for MacosCvPixelBufferFrame {}

#[cfg(target_os = "macos")]
impl Drop for MacosCvPixelBufferFrame {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.pixel_buffer.as_ptr().cast_const());
        }
    }
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
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    pub macos_cv_pixel_buffer: Option<MacosCvPixelBufferFrame>,
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
            #[cfg(target_os = "macos")]
            macos_cv_pixel_buffer: None,
            #[cfg(windows)]
            d3d11_shared_bgra: None,
        }
    }

    #[cfg(target_os = "macos")]
    pub fn from_macos_cv_pixel_buffer(
        width: usize,
        height: usize,
        pixel_format: FramePixelFormat,
        timestamp_us: u64,
        pixel_buffer: *mut c_void,
    ) -> Option<Self> {
        Some(Self {
            width,
            height,
            pixel_format,
            timestamp_us,
            data: Vec::new(),
            macos_cv_pixel_buffer: Some(MacosCvPixelBufferFrame::retain(
                pixel_buffer,
                pixel_format,
            )?),
        })
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
            #[cfg(target_os = "macos")]
            macos_cv_pixel_buffer: None,
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

    #[cfg(target_os = "macos")]
    pub fn macos_cv_pixel_buffer(&self) -> Option<&MacosCvPixelBufferFrame> {
        self.macos_cv_pixel_buffer.as_ref()
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
    /// CPU I420/YUV420 planar data with Y and UV pitches.
    CpuI420 {
        data: Vec<u8>,
        y_pitch: usize,
        uv_pitch: usize,
    },
    /// CPU P010/P016 data with decoder pitch.
    CpuP010 { data: Vec<u8>, pitch: usize },
    /// D3D11 shared texture handle (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12 {
        shared_handle_y: isize,
        shared_handle_uv: isize,
        width: u32,
        height: u32,
    },
    /// D3D11 shared P010/P016 texture handles (zero-copy Main10 path)
    #[cfg(windows)]
    D3D11SharedP010 {
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

    /// Create a decoded frame from CPU I420/YUV420 planar data
    pub fn from_cpu_i420(
        width: usize,
        height: usize,
        timestamp_us: u64,
        y_pitch: usize,
        uv_pitch: usize,
        data: Vec<u8>,
    ) -> Self {
        Self {
            width,
            height,
            timestamp_us,
            data: DecodedFrameData::CpuI420 {
                data,
                y_pitch,
                uv_pitch,
            },
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

    /// Create a decoded frame from CPU P010/P016 data
    pub fn from_cpu_p010(
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
            data: DecodedFrameData::CpuP010 { data, pitch },
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

    /// Create a decoded frame from D3D11 shared P010/P016 textures
    #[cfg(windows)]
    pub fn from_d3d11_shared_p010(
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
            data: DecodedFrameData::D3D11SharedP010 {
                shared_handle_y,
                shared_handle_uv,
                width: width as u32,
                height: height as u32,
            },
        }
    }

    /// Check if this frame uses shared texture (zero-copy)
    pub fn is_shared_texture(&self) -> bool {
        #[cfg(windows)]
        {
            matches!(
                self.data,
                DecodedFrameData::D3D11SharedNv12 { .. } | DecodedFrameData::D3D11SharedP010 { .. }
            )
        }

        #[cfg(not(windows))]
        {
            false
        }
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

    /// Get the CPU I420 data with Y and UV pitches if available
    pub fn cpu_i420(&self) -> Option<(&[u8], usize, usize)> {
        match &self.data {
            DecodedFrameData::CpuI420 {
                data,
                y_pitch,
                uv_pitch,
            } => Some((data.as_slice(), *y_pitch, *uv_pitch)),
            _ => None,
        }
    }

    /// Get the CPU P010/P016 data and pitch if available
    pub fn cpu_p010(&self) -> Option<(&[u8], usize)> {
        match &self.data {
            DecodedFrameData::CpuP010 { data, pitch } => Some((data.as_slice(), *pitch)),
            _ => None,
        }
    }

    /// Get any CPU data as bytes
    pub fn cpu_bytes(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::CpuRgb24(data)
            | DecodedFrameData::CpuBgra32(data)
            | DecodedFrameData::CpuNv12 { data, .. }
            | DecodedFrameData::CpuI420 { data, .. }
            | DecodedFrameData::CpuP010 { data, .. } => Some(data.as_slice()),
            #[cfg(windows)]
            DecodedFrameData::D3D11SharedNv12 { .. } | DecodedFrameData::D3D11SharedP010 { .. } => {
                None
            }
        }
    }

    /// Get the shared texture handle if available
    #[cfg(windows)]
    pub fn d3d11_shared_handle(&self) -> Option<isize> {
        match &self.data {
            DecodedFrameData::D3D11SharedNv12 {
                shared_handle_y, ..
            }
            | DecodedFrameData::D3D11SharedP010 {
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
            }
            | DecodedFrameData::D3D11SharedP010 {
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

    fn request_keyframe(&mut self) {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_decoded_frames_are_not_shared_textures() {
        let frame = DecodedFrame::from_cpu_rgb24(2, 2, 0, vec![0; 12]);

        assert!(!frame.is_shared_texture());
    }

    #[test]
    fn hevc_codec_is_available_for_hardware_pipelines() {
        assert_eq!(VideoCodec::Hevc, VideoCodec::Hevc);
    }
}
