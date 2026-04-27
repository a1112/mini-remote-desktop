//! Windows.Graphics.Capture screen capture implementation
//!
//! This module provides screen capture using the Windows.Graphics.Capture API,
//! which supports both monitor and item-level (window) capture.
//!
//! # Advantages over DXGI Desktop Duplication
//! - Per-window capture capability
//! - Better multi-monitor support
//! - User consent prompts for privacy
//! - Works on Windows 10 1803+
//!
//! # Example
//! ```ignore
//! use mrd_capture_winrt::WinrtCapture;
//!
//! // Capture from monitor
//! let capture = WinrtCapture::from_monitor_index(0)?;
//! ```

#[cfg(not(windows))]
compile_error!("mrd-capture-winrt is only supported on Windows");

use anyhow::{anyhow, Context, Result};
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, PipelineError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::*;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, IsWindowVisible,
};

// WinRT imports - use Graphics namespace not Win32
use windows::Graphics::Capture::*;
use windows::Graphics::DirectX::Direct3D11::*;
use windows::Graphics::DirectX::*;

/// WinRT screen capture using Windows.Graphics.Capture API
pub struct WinrtCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    item: Option<GraphicsCaptureItem>,
    frame_pool: Option<Direct3D11CaptureFramePool>,
    session: Option<GraphicsCaptureSession>,
    width: usize,
    height: usize,
}

unsafe impl Send for WinrtCapture {}

/// Visible top-level window that can be used as a WinRT capture target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureTarget {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
    pub process_id: u32,
}

/// Result of creating a WinRT capture item for a selected window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCaptureItemProbe {
    pub hwnd: isize,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
}

/// Result of starting a WinRT window capture session and pulling one CPU frame.
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

impl WinrtCapture {
    /// Create a new capture from a monitor index
    pub fn from_monitor_index(monitor_index: u32) -> Result<Self, PipelineError> {
        let item = Self::get_monitor_item(monitor_index)
            .map_err(|e| PipelineError::message(format!("get monitor item failed: {e}")))?;
        Self::from_item(item)
    }

    /// Create a new capture from a window handle (HWND)
    pub fn from_window(hwnd: HWND) -> Result<Self, PipelineError> {
        let item = Self::create_item_for_window(hwnd)
            .map_err(|e| PipelineError::message(format!("create window item failed: {e}")))?;
        Self::from_item(item)
    }

    /// Create a new capture from a capture item
    fn from_item(item: GraphicsCaptureItem) -> Result<Self, PipelineError> {
        // Initialize D3D11 device
        let (device, context) = create_d3d11_device()
            .map_err(|e| PipelineError::message(format!("create d3d11 device failed: {e}")))?;

        let size = item
            .Size()
            .map_err(|e| PipelineError::message(format!("get capture item size failed: {e:?}")))?;

        let width = size.Width as usize;
        let height = size.Height as usize;

        Ok(Self {
            device,
            context,
            item: Some(item),
            frame_pool: None,
            session: None,
            width,
            height,
        })
    }

    /// Get a monitor capture item by index
    fn get_monitor_item(monitor_index: u32) -> Result<GraphicsCaptureItem> {
        // Create DXGI factory
        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.context("CreateDXGIFactory1 failed")?;

        // Enumerate adapters and outputs
        let mut current_index = 0u32;

        for adapter_i in 0..10 {
            let adapter = unsafe { factory.EnumAdapters1(adapter_i) };
            let Ok(adapter) = adapter else { continue };

            for output_i in 0..10 {
                let output = unsafe { adapter.EnumOutputs(output_i) };
                let Ok(output) = output else { continue };

                // Get output description
                let desc = unsafe {
                    match output.GetDesc() {
                        Ok(d) => d,
                        Err(_) => continue,
                    }
                };

                if current_index == monitor_index {
                    // Create capture item for this monitor
                    return Self::create_item_for_monitor(desc.Monitor);
                }

                current_index += 1;
            }
        }

        Err(anyhow!("Monitor {} not found", monitor_index))
    }

    /// Get monitor count
    pub fn get_monitor_count() -> Result<usize> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }?;
        let mut count = 0usize;

        for adapter_i in 0..10 {
            let adapter = unsafe { factory.EnumAdapters1(adapter_i) };
            let Ok(adapter) = adapter else { continue };

            for output_i in 0..10 {
                let output = unsafe { adapter.EnumOutputs(output_i) };
                if output.is_ok() {
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Create a GraphicsCaptureItem from a window handle
    fn create_item_for_window(hwnd: HWND) -> Result<GraphicsCaptureItem> {
        // Initialize COM
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        // Get the interop interface
        let factory: IGraphicsCaptureItemInterop = unsafe {
            RoGetActivationFactory(&HSTRING::from(
                "Windows.Graphics.Capture.GraphicsCaptureItem",
            ))
            .context("RoGetActivationFactory failed")?
        };

        let item: GraphicsCaptureItem = unsafe {
            factory
                .CreateForWindow(hwnd)
                .context("CreateForWindow failed")?
        };

        Ok(item)
    }

    /// Create a GraphicsCaptureItem from a monitor handle
    fn create_item_for_monitor(hmonitor: HMONITOR) -> Result<GraphicsCaptureItem> {
        // Initialize COM
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        // Get the interop interface
        let factory: IGraphicsCaptureItemInterop = unsafe {
            RoGetActivationFactory(&HSTRING::from(
                "Windows.Graphics.Capture.GraphicsCaptureItem",
            ))
            .context("RoGetActivationFactory failed")?
        };

        let item: GraphicsCaptureItem = unsafe {
            factory
                .CreateForMonitor(hmonitor)
                .context("CreateForMonitor failed")?
        };

        Ok(item)
    }

    /// Get the capture width
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the capture height
    pub fn height(&self) -> usize {
        self.height
    }

    /// Start the capture session
    pub fn start(&mut self) -> Result<(), PipelineError> {
        let item = self.item.take().ok_or_else(|| {
            PipelineError::message("Cannot start capture: no capture item available")
        })?;

        // Create Direct3D device for WinRT
        let dxgi_device: IDXGIDevice = self
            .device
            .cast()
            .map_err(|e| PipelineError::message(format!("cast to IDXGIDevice failed: {e}")))?;
        let d3d_device: IDirect3DDevice = unsafe {
            CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)
                .and_then(|inspectable| inspectable.cast())
        }
        .map_err(|e| {
            PipelineError::message(format!(
                "CreateDirect3D11DeviceFromDXGIDevice failed: {e:?}"
            ))
        })?;

        // Create frame pool with BGRA8 format
        let size = item
            .Size()
            .map_err(|e| PipelineError::message(format!("get item size failed: {e:?}")))?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2, // Number of buffers
            size,
        )
        .map_err(|e| PipelineError::message(format!("Create frame pool failed: {e:?}")))?;

        // Create capture session
        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(|e| PipelineError::message(format!("CreateCaptureSession failed: {e:?}")))?;

        self.frame_pool = Some(frame_pool);
        self.session = Some(session);

        // Start capture
        if let Some(session) = &self.session {
            session
                .StartCapture()
                .map_err(|e| PipelineError::message(format!("StartCapture failed: {e:?}")))?;
        }

        Ok(())
    }

    /// Stop the capture session
    pub fn stop(&mut self) -> Result<(), PipelineError> {
        if let Some(_session) = self.session.take() {
            // Session will be dropped, which stops capture
        }
        self.frame_pool = None;
        Ok(())
    }

    /// Try to get the latest captured frame
    pub fn try_get_frame(&mut self) -> Option<CapturedFrame> {
        let frame_pool = self.frame_pool.as_ref()?;
        let frame = frame_pool.TryGetNextFrame().ok()?;
        self.frame_to_captured_frame(&frame).ok()
    }

    /// Capture a frame (synchronous approximation)
    pub fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        self.capture_frame_with_timeout(Duration::from_secs(1))
    }

    /// Capture one frame, waiting up to the provided timeout for WinRT to deliver it.
    pub fn capture_frame_with_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<CapturedFrame, PipelineError> {
        let frame_pool = self
            .frame_pool
            .as_ref()
            .ok_or_else(|| PipelineError::message("Capture not started - call start() first"))?;

        let deadline = Instant::now() + timeout;
        let mut last_error: Option<String>;

        loop {
            match frame_pool.TryGetNextFrame() {
                Ok(frame) => return self.frame_to_captured_frame(&frame),
                Err(error) => last_error = Some(format!("{error:?}")),
            }

            if Instant::now() >= deadline {
                let suffix = last_error
                    .map(|error| format!("; last TryGetNextFrame error: {error}"))
                    .unwrap_or_default();
                return Err(PipelineError::message(format!(
                    "WinRT capture produced no frame within {} ms{}",
                    timeout.as_millis(),
                    suffix
                )));
            }

            thread::sleep(Duration::from_millis(10));
        }
    }

    fn frame_to_captured_frame(
        &self,
        frame: &Direct3D11CaptureFrame,
    ) -> Result<CapturedFrame, PipelineError> {
        let content_size = frame
            .ContentSize()
            .map_err(|e| PipelineError::message(format!("get WinRT frame size failed: {e:?}")))?;
        let width = usize::try_from(content_size.Width)
            .map_err(|_| PipelineError::message("WinRT frame width is negative"))?;
        let height = usize::try_from(content_size.Height)
            .map_err(|_| PipelineError::message("WinRT frame height is negative"))?;

        if width == 0 || height == 0 {
            return Err(PipelineError::message("WinRT frame size is empty"));
        }

        let surface = frame.Surface().map_err(|e| {
            PipelineError::message(format!("get WinRT frame surface failed: {e:?}"))
        })?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(|e| {
            PipelineError::message(format!("cast frame surface to DXGI access failed: {e:?}"))
        })?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(|e| {
            PipelineError::message(format!(
                "get D3D11 texture from frame surface failed: {e:?}"
            ))
        })?;

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }

        if desc.SampleDesc.Count > 1 {
            return Err(PipelineError::message(format!(
                "unsupported multisampled WinRT frame texture: {} samples",
                desc.SampleDesc.Count
            )));
        }

        let mut staging_desc = desc;
        staging_desc.Usage = D3D11_USAGE_STAGING;
        staging_desc.BindFlags = 0;
        staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        staging_desc.MiscFlags = 0;

        let mut staging = None::<ID3D11Texture2D>;
        unsafe {
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
        }
        .map_err(|e| PipelineError::message(format!("create staging texture failed: {e:?}")))?;
        let staging = staging
            .ok_or_else(|| PipelineError::message("create staging texture returned no texture"))?;

        let source_resource: ID3D11Resource = texture.cast().map_err(|e| {
            PipelineError::message(format!("cast source texture to resource failed: {e:?}"))
        })?;
        let staging_resource: ID3D11Resource = staging.cast().map_err(|e| {
            PipelineError::message(format!("cast staging texture to resource failed: {e:?}"))
        })?;

        unsafe {
            self.context
                .CopyResource(&staging_resource, &source_resource);
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&staging_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .map_err(|e| PipelineError::message(format!("map staging texture failed: {e:?}")))?;

        let copy_result = copy_mapped_bgra_frame(&mapped, width, height);
        unsafe {
            self.context.Unmap(&staging_resource, 0);
        }
        let data = copy_result?;

        Ok(CapturedFrame {
            width,
            height,
            pixel_format: FramePixelFormat::Bgra32,
            timestamp_us: now_us(),
            data,
        })
    }

    /// Check if capture is available
    pub fn probe_available() -> Result<(), PipelineError> {
        // Try to get monitor count to test availability
        let _ = Self::get_monitor_count()
            .map_err(|e| PipelineError::message(format!("WinRT capture not available: {e}")))?;
        Ok(())
    }
}

impl Drop for WinrtCapture {
    fn drop(&mut self) {
        // Ensure capture is stopped
        let _ = self.stop();
    }
}

/// Create a D3D11 device
fn create_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None::<ID3D11Device>;
    let mut context = None::<ID3D11DeviceContext>;

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .or_else(|_| unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    })
    .context("D3D11CreateDevice failed")?;

    Ok((
        device.ok_or_else(|| anyhow!("missing d3d11 device"))?,
        context.ok_or_else(|| anyhow!("missing d3d11 context"))?,
    ))
}

/// Probe WinRT capture availability
pub fn probe_winrt_available() -> Result<(), PipelineError> {
    WinrtCapture::probe_available()
}

/// Get monitor count
pub fn get_monitor_count() -> Result<usize, PipelineError> {
    WinrtCapture::get_monitor_count()
        .map_err(|e| PipelineError::message(format!("get monitor count failed: {e}")))
}

/// Capture information
#[derive(Debug, Clone)]
pub struct WinrtCaptureInfo {
    pub monitor_count: usize,
    pub window_capture_supported: bool,
    pub monitor_capture_supported: bool,
}

/// Get capture information
pub fn get_capture_info() -> Result<WinrtCaptureInfo, PipelineError> {
    let monitor_count = get_monitor_count().unwrap_or(0);

    Ok(WinrtCaptureInfo {
        monitor_count,
        window_capture_supported: true,
        monitor_capture_supported: monitor_count > 0,
    })
}

/// Enumerate visible top-level windows that have a title and non-zero bounds.
pub fn enumerate_window_capture_targets() -> Result<Vec<WindowCaptureTarget>, PipelineError> {
    let mut targets = Vec::<WindowCaptureTarget>::new();
    let targets_ptr = &mut targets as *mut Vec<WindowCaptureTarget>;

    unsafe {
        EnumWindows(
            Some(enum_window_capture_target),
            LPARAM(targets_ptr as isize),
        )
        .map_err(|error| PipelineError::message(format!("EnumWindows failed: {error:?}")))?;
    }

    targets.sort_by(|left, right| left.title.cmp(&right.title));
    Ok(targets)
}

/// Create a WinRT capture item for a window and return its resolved dimensions.
pub fn probe_window_capture_item(
    hwnd_value: isize,
) -> Result<WindowCaptureItemProbe, PipelineError> {
    let hwnd = validate_window_hwnd(hwnd_value)?;
    let title = unsafe { read_window_text(hwnd) };
    let class_name = unsafe { read_class_name(hwnd) };
    let capture = WinrtCapture::from_window(hwnd)?;

    Ok(WindowCaptureItemProbe {
        hwnd: hwnd_value,
        title,
        class_name,
        width: capture.width() as u32,
        height: capture.height() as u32,
    })
}

/// Start a WinRT window capture session and read one BGRA frame into CPU memory.
pub fn probe_window_first_frame(
    hwnd_value: isize,
    timeout: Duration,
) -> Result<WindowCaptureFrameProbe, PipelineError> {
    let hwnd = validate_window_hwnd(hwnd_value)?;
    let title = unsafe { read_window_text(hwnd) };
    let class_name = unsafe { read_class_name(hwnd) };
    let mut capture = WinrtCapture::from_window(hwnd)?;
    capture.start()?;
    let frame = capture.capture_frame_with_timeout(timeout)?;
    let _ = capture.stop();
    let width = frame.width as u32;
    let height = frame.height as u32;
    let byte_len = frame.data.len();
    let pixel_format = frame.pixel_format;

    Ok(WindowCaptureFrameProbe {
        hwnd: hwnd_value,
        title,
        class_name,
        width,
        height,
        byte_len,
        pixel_format,
        frame,
    })
}

fn validate_window_hwnd(hwnd_value: isize) -> Result<HWND, PipelineError> {
    if hwnd_value == 0 {
        return Err(PipelineError::message("window hwnd must not be zero"));
    }

    let hwnd = HWND(hwnd_value as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(PipelineError::message(format!(
            "window hwnd 0x{:X} is not valid",
            hwnd_value as usize
        )));
    }

    Ok(hwnd)
}

fn copy_mapped_bgra_frame(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: usize,
    height: usize,
) -> Result<Vec<u8>, PipelineError> {
    if mapped.pData.is_null() {
        return Err(PipelineError::message("mapped WinRT frame pointer is null"));
    }

    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| PipelineError::message("WinRT frame row size overflowed"))?;
    let total_bytes = row_bytes
        .checked_mul(height)
        .ok_or_else(|| PipelineError::message("WinRT frame buffer size overflowed"))?;
    let row_pitch = mapped.RowPitch as usize;

    if row_pitch < row_bytes {
        return Err(PipelineError::message(format!(
            "mapped WinRT frame row pitch {row_pitch} is smaller than row bytes {row_bytes}"
        )));
    }

    let mut data = vec![0u8; total_bytes];
    for row in 0..height {
        let source = unsafe {
            std::slice::from_raw_parts((mapped.pData as *const u8).add(row * row_pitch), row_bytes)
        };
        let start = row * row_bytes;
        data[start..start + row_bytes].copy_from_slice(source);
    }

    Ok(data)
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

unsafe extern "system" fn enum_window_capture_target(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }

    let title = read_window_text(hwnd);
    if title.trim().is_empty() {
        return true.into();
    }

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return true.into();
    }

    let width = rect.right.saturating_sub(rect.left);
    let height = rect.bottom.saturating_sub(rect.top);
    if width <= 0 || height <= 0 {
        return true.into();
    }

    let mut process_id = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));

    let targets = &mut *(lparam.0 as *mut Vec<WindowCaptureTarget>);
    targets.push(WindowCaptureTarget {
        hwnd: hwnd.0 as isize,
        title,
        class_name: read_class_name(hwnd),
        width: width as u32,
        height: height as u32,
        process_id,
    });

    true.into()
}

unsafe fn read_window_text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd);
    if length <= 0 {
        return String::new();
    }

    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(hwnd, &mut buffer);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

unsafe fn read_class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; 256];
    let copied = GetClassNameW(hwnd, &mut buffer);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_winrt_capture_probe() {
        let result = WinrtCapture::probe_available();
        println!("WinRT capture available: {:?}", result.is_ok());
    }

    #[test]
    #[ignore]
    fn test_get_monitor_count() {
        let count = get_monitor_count();
        println!("Monitor count: {:?}", count);
        if let Ok(count) = count {
            assert!(count > 0);
        }
    }

    #[test]
    #[ignore]
    fn test_create_from_monitor() {
        let result = WinrtCapture::from_monitor_index(0);
        println!("Create from monitor: {:?}", result.is_ok());
        if let Ok(capture) = result {
            println!("Monitor size: {}x{}", capture.width(), capture.height());
        }
    }

    #[test]
    #[ignore]
    fn test_get_capture_info() {
        let info = get_capture_info();
        println!("Capture info: {:?}", info);
    }
}
