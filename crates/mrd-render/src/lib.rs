pub use mrd_pipeline_core::RuntimeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPixelFormat {
    Rgb24,
    Bgra32,
    /// D3D11 shared NV12 texture (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12,
}

/// Frame data for rendering
///
/// Supports both CPU data and D3D11 shared texture (zero-copy path)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderFrameData {
    /// CPU RGB24 data
    Rgb24(Vec<u8>),
    /// CPU BGRA32 data
    Bgra32(Vec<u8>),
    /// D3D11 shared texture handle (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12 {
        shared_handle: isize,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame {
    pub width: usize,
    pub height: usize,
    pub pixel_format: RenderPixelFormat,
    pub data: RenderFrameData,
}

impl RenderFrame {
    /// Create a frame from CPU RGB24 data
    pub fn from_rgb24(width: usize, height: usize, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::Rgb24,
            data: RenderFrameData::Rgb24(data),
        }
    }

    /// Create a frame from CPU BGRA32 data
    pub fn from_bgra32(width: usize, height: usize, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::Bgra32,
            data: RenderFrameData::Bgra32(data),
        }
    }

    /// Create a frame from D3D11 shared NV12 texture
    #[cfg(windows)]
    pub fn from_d3d11_shared_nv12(width: usize, height: usize, shared_handle: isize) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::D3D11SharedNv12,
            data: RenderFrameData::D3D11SharedNv12 {
                shared_handle,
                width: width as u32,
                height: height as u32,
            },
        }
    }

    /// Get the CPU RGB24 data if available
    pub fn as_rgb24(&self) -> Option<&[u8]> {
        match &self.data {
            RenderFrameData::Rgb24(data) => Some(data.as_slice()),
            _ => None,
        }
    }

    /// Get the CPU BGRA32 data if available
    pub fn as_bgra32(&self) -> Option<&[u8]> {
        match &self.data {
            RenderFrameData::Bgra32(data) => Some(data.as_slice()),
            _ => None,
        }
    }

    /// Get the shared texture handle if available
    #[cfg(windows)]
    pub fn shared_handle(&self) -> Option<isize> {
        match &self.data {
            RenderFrameData::D3D11SharedNv12 { shared_handle, .. } => Some(*shared_handle),
            _ => None,
        }
    }

    /// Check if this frame uses shared texture (zero-copy)
    pub fn is_shared_texture(&self) -> bool {
        match &self.data {
            #[cfg(windows)]
            RenderFrameData::D3D11SharedNv12 { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererDescriptor {
    pub id: &'static str,
    pub runtime_status: RuntimeStatus,
    pub supported_formats: &'static [RenderPixelFormat],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererSnapshot {
    pub attached_to_target: bool,
    pub uploaded_frame_count: u64,
    pub last_width: usize,
    pub last_height: usize,
    pub last_pixel_format: Option<RenderPixelFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    WindowHandle(isize),
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("{0}")]
    Message(String),
}

pub trait RendererInstance: Send {
    fn attach_target(&mut self, target: RenderTarget) -> Result<(), RenderError>;
    fn upload_frame(&mut self, frame: RenderFrame) -> Result<(), RenderError>;
    fn snapshot(&self) -> RendererSnapshot;
}

pub type BoxedRenderer = Box<dyn RendererInstance>;

pub trait RendererFactory: Send + Sync {
    fn descriptor(&self) -> RendererDescriptor;
    fn create(&self) -> Result<BoxedRenderer, RenderError>;
}

const RGB24_FORMATS: &[RenderPixelFormat] = &[RenderPixelFormat::Rgb24];
const SUPPORTED_FORMATS: &[RenderPixelFormat] = &[
    RenderPixelFormat::Rgb24,
    RenderPixelFormat::Bgra32,
    #[cfg(windows)]
    RenderPixelFormat::D3D11SharedNv12,
];

pub fn d3d11_descriptor() -> RendererDescriptor {
    RendererDescriptor {
        id: "d3d11",
        runtime_status: RuntimeStatus::RuntimeBacked,
        supported_formats: SUPPORTED_FORMATS,
    }
}

#[cfg(test)]
mod tests {
    use super::{d3d11_descriptor, RenderPixelFormat, RuntimeStatus};

    #[test]
    fn d3d11_descriptor_reports_runtime_backed_rgb24_support() {
        let descriptor = d3d11_descriptor();

        assert_eq!(descriptor.id, "d3d11");
        assert_eq!(descriptor.runtime_status, RuntimeStatus::RuntimeBacked);
        assert_eq!(descriptor.supported_formats, &[RenderPixelFormat::Rgb24]);
    }
}
