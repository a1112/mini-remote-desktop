/// 渲染器配置
#[derive(Debug, Clone)]
pub struct RendererConfig {
    pub window_width: u32,
    pub window_height: u32,
    pub vsync: bool,
    pub low_latency_mode: bool,
    pub max_frame_latency: u32,
    pub sr_mode: String,
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            window_width: 1280,
            window_height: 720,
            vsync: false,
            low_latency_mode: true,
            max_frame_latency: 1,
            sr_mode: "off".to_string(),
        }
    }
}

#[cfg(windows)]
pub mod d3d11;

#[cfg(windows)]
pub use d3d11::{D3D11Renderer, OverlaySwitchField};

#[cfg(not(windows))]
pub mod stub {
    use crate::video::decoder::DecodedFrame;
    use anyhow::Result;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    /// 非 Windows 平台的存根实现
    pub struct D3D11Renderer;

    impl D3D11Renderer {
        pub fn new(_config: super::RendererConfig) -> Result<Self> {
            Err(anyhow::anyhow!("DirectX 11 is only available on Windows"))
        }

        pub fn new_with_stats(
            _config: super::RendererConfig,
            _video_frames_received: Arc<AtomicU64>,
        ) -> Result<Self> {
            Err(anyhow::anyhow!("DirectX 11 is only available on Windows"))
        }

        pub fn submit_decoded_frame(&self, _frame: DecodedFrame) {}

        pub fn window_handle(&self) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
            None
        }
    }
}

#[cfg(not(windows))]
pub use stub::D3D11Renderer;
