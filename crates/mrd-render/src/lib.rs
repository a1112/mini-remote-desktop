pub use mrd_pipeline_core::RuntimeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPixelFormat {
    Rgb24,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFrame {
    pub width: usize,
    pub height: usize,
    pub pixel_format: RenderPixelFormat,
    pub data: Vec<u8>,
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

pub fn d3d11_descriptor() -> RendererDescriptor {
    RendererDescriptor {
        id: "d3d11",
        runtime_status: RuntimeStatus::RuntimeBacked,
        supported_formats: RGB24_FORMATS,
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
