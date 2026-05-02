pub use mrd_pipeline_core::RuntimeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPixelFormat {
    Rgb24,
    Bgra32,
    /// D3D11 shared BGRA texture (zero-copy direct capture-render path)
    #[cfg(windows)]
    D3D11SharedBgra,
    /// D3D11 shared NV12 texture (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12,
    /// D3D11 shared P010/P016 texture (zero-copy Main10 path)
    #[cfg(windows)]
    D3D11SharedP010,
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
    D3D11SharedBgra {
        shared_handle: isize,
        width: u32,
        height: u32,
        row_pitch: u32,
    },
    /// D3D11 shared texture handle (zero-copy path)
    #[cfg(windows)]
    D3D11SharedNv12 {
        shared_handle_y: isize,
        shared_handle_uv: isize,
        width: u32,
        height: u32,
    },
    /// D3D11 shared P010/P016 texture handle (zero-copy Main10 path)
    #[cfg(windows)]
    D3D11SharedP010 {
        shared_handle_y: isize,
        shared_handle_uv: isize,
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

    /// Create a frame from a D3D11 shared BGRA texture
    #[cfg(windows)]
    pub fn from_d3d11_shared_bgra(
        width: usize,
        height: usize,
        shared_handle: isize,
        row_pitch: u32,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::D3D11SharedBgra,
            data: RenderFrameData::D3D11SharedBgra {
                shared_handle,
                width: width as u32,
                height: height as u32,
                row_pitch,
            },
        }
    }

    /// Create a frame from D3D11 shared NV12 texture
    #[cfg(windows)]
    pub fn from_d3d11_shared_nv12(
        width: usize,
        height: usize,
        shared_handle_y: isize,
        shared_handle_uv: isize,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::D3D11SharedNv12,
            data: RenderFrameData::D3D11SharedNv12 {
                shared_handle_y,
                shared_handle_uv,
                width: width as u32,
                height: height as u32,
            },
        }
    }

    /// Create a frame from D3D11 shared P010/P016 texture
    #[cfg(windows)]
    pub fn from_d3d11_shared_p010(
        width: usize,
        height: usize,
        shared_handle_y: isize,
        shared_handle_uv: isize,
    ) -> Self {
        Self {
            width,
            height,
            pixel_format: RenderPixelFormat::D3D11SharedP010,
            data: RenderFrameData::D3D11SharedP010 {
                shared_handle_y,
                shared_handle_uv,
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
            RenderFrameData::D3D11SharedBgra { shared_handle, .. } => Some(*shared_handle),
            RenderFrameData::D3D11SharedNv12 {
                shared_handle_y, ..
            }
            | RenderFrameData::D3D11SharedP010 {
                shared_handle_y, ..
            } => Some(*shared_handle_y),
            _ => None,
        }
    }

    /// Get the shared BGRA texture handle if available.
    #[cfg(windows)]
    pub fn shared_bgra_handle(&self) -> Option<isize> {
        match &self.data {
            RenderFrameData::D3D11SharedBgra { shared_handle, .. } => Some(*shared_handle),
            _ => None,
        }
    }

    /// Get the shared BGRA texture row pitch if available.
    #[cfg(windows)]
    pub fn shared_bgra_row_pitch(&self) -> Option<u32> {
        match &self.data {
            RenderFrameData::D3D11SharedBgra { row_pitch, .. } => Some(*row_pitch),
            _ => None,
        }
    }

    /// Get the shared Y and UV texture handles if available.
    #[cfg(windows)]
    pub fn shared_handles(&self) -> Option<(isize, isize)> {
        match &self.data {
            RenderFrameData::D3D11SharedNv12 {
                shared_handle_y,
                shared_handle_uv,
                ..
            }
            | RenderFrameData::D3D11SharedP010 {
                shared_handle_y,
                shared_handle_uv,
                ..
            } => Some((*shared_handle_y, *shared_handle_uv)),
            _ => None,
        }
    }

    /// Check if this frame uses shared texture (zero-copy)
    pub fn is_shared_texture(&self) -> bool {
        match &self.data {
            #[cfg(windows)]
            RenderFrameData::D3D11SharedBgra { .. } => true,
            #[cfg(windows)]
            RenderFrameData::D3D11SharedNv12 { .. } | RenderFrameData::D3D11SharedP010 { .. } => {
                true
            }
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

const SUPPORTED_FORMATS: &[RenderPixelFormat] = &[
    RenderPixelFormat::Rgb24,
    RenderPixelFormat::Bgra32,
    #[cfg(windows)]
    RenderPixelFormat::D3D11SharedBgra,
    #[cfg(windows)]
    RenderPixelFormat::D3D11SharedNv12,
    #[cfg(windows)]
    RenderPixelFormat::D3D11SharedP010,
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
    fn d3d11_descriptor_reports_runtime_backed_formats() {
        let descriptor = d3d11_descriptor();

        assert_eq!(descriptor.id, "d3d11");
        assert_eq!(descriptor.runtime_status, RuntimeStatus::RuntimeBacked);
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::Rgb24));
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::Bgra32));
        #[cfg(windows)]
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::D3D11SharedNv12));
        #[cfg(windows)]
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::D3D11SharedBgra));
        #[cfg(windows)]
        assert!(descriptor
            .supported_formats
            .contains(&RenderPixelFormat::D3D11SharedP010));
    }

    #[cfg(windows)]
    #[test]
    fn d3d11_shared_bgra_frame_is_zero_copy_render_data() {
        let frame = super::RenderFrame::from_d3d11_shared_bgra(1280, 720, 42, 1280 * 4);

        assert_eq!(frame.pixel_format, RenderPixelFormat::D3D11SharedBgra);
        assert!(frame.is_shared_texture());
        assert_eq!(frame.shared_handle(), Some(42));
        assert_eq!(frame.shared_bgra_handle(), Some(42));
    }

    #[cfg(windows)]
    #[test]
    fn d3d11_shared_p010_frame_is_zero_copy_render_data() {
        let frame = super::RenderFrame::from_d3d11_shared_p010(1280, 720, 42, 43);

        assert_eq!(frame.pixel_format, RenderPixelFormat::D3D11SharedP010);
        assert!(frame.is_shared_texture());
        assert_eq!(frame.shared_handles(), Some((42, 43)));
    }
}
