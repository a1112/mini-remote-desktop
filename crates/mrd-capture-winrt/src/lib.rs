#[cfg(windows)]
mod windows_impl;

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(not(windows))]
use mrd_pipeline_core::{
    CapturedFrame, FrameCapture, FrameMemoryKind, FramePixelFormat, PipelineError,
};

#[cfg(not(windows))]
use std::time::Duration;

#[cfg(not(windows))]
fn unsupported() -> PipelineError {
    PipelineError::message("Windows.Graphics.Capture is only available on Windows")
}

#[cfg(not(windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureTarget {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
    pub process_id: u32,
}

#[cfg(not(windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureItemProbe {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
}

#[cfg(not(windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureFrameProbe {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
    pub pixel_format: FramePixelFormat,
    pub frame: CapturedFrame,
}

#[cfg(not(windows))]
#[derive(Debug, Clone)]
pub struct WinrtCaptureInfo {
    pub monitor_count: usize,
    pub window_capture_supported: bool,
    pub monitor_capture_supported: bool,
}

#[cfg(not(windows))]
pub struct WinrtCapture;

#[cfg(not(windows))]
impl WinrtCapture {
    pub fn from_monitor_index(_monitor_index: u32) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn from_monitor_index_shared_texture(_monitor_index: u32) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn from_monitor_device_name(_device_name: &str) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn from_monitor_device_name_shared_texture(
        _device_name: &str,
    ) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn from_window_handle(_hwnd: isize) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn from_window_handle_shared_texture(_hwnd: isize) -> Result<Self, PipelineError> {
        Err(unsupported())
    }

    pub fn with_shared_texture_output(self) -> Self {
        self
    }

    pub fn get_monitor_count() -> Result<usize, anyhow::Error> {
        Ok(0)
    }

    pub fn width(&self) -> usize {
        0
    }

    pub fn height(&self) -> usize {
        0
    }

    pub fn set_target_dimensions(&mut self, _width: usize, _height: usize) {}

    pub fn start(&mut self) -> Result<(), PipelineError> {
        Err(unsupported())
    }

    pub fn stop(&mut self) -> Result<(), PipelineError> {
        Ok(())
    }

    pub fn try_get_frame(&mut self) -> Option<CapturedFrame> {
        None
    }

    pub fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        Err(unsupported())
    }

    pub fn capture_frame_with_timeout(
        &mut self,
        _timeout: Duration,
    ) -> Result<CapturedFrame, PipelineError> {
        Err(unsupported())
    }

    pub fn probe_available() -> Result<(), PipelineError> {
        Err(unsupported())
    }
}

#[cfg(not(windows))]
impl FrameCapture for WinrtCapture {
    fn output_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        WinrtCapture::capture_frame(self)
    }
}

#[cfg(not(windows))]
pub fn probe_winrt_available() -> Result<(), PipelineError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn get_monitor_count() -> Result<usize, PipelineError> {
    Ok(0)
}

#[cfg(not(windows))]
pub fn get_capture_info() -> Result<WinrtCaptureInfo, PipelineError> {
    Ok(WinrtCaptureInfo {
        monitor_count: 0,
        window_capture_supported: false,
        monitor_capture_supported: false,
    })
}

#[cfg(not(windows))]
pub fn enumerate_window_capture_targets() -> Result<Vec<WindowCaptureTarget>, PipelineError> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub fn probe_window_capture_item(
    _hwnd_value: isize,
) -> Result<WindowCaptureItemProbe, PipelineError> {
    Err(unsupported())
}

#[cfg(not(windows))]
pub fn probe_window_first_frame(
    _hwnd_value: isize,
    _timeout: Duration,
) -> Result<WindowCaptureFrameProbe, PipelineError> {
    Err(unsupported())
}
