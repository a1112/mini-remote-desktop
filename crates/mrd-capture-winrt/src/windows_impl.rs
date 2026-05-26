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
use mrd_pipeline_core::{
    CapturedFrame, FrameCapture, FrameMemoryKind, FramePixelFormat, PipelineError,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::core::*;
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetClientRect, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    GWL_EXSTYLE, GWL_STYLE, WS_CHILD, WS_EX_TOOLWINDOW,
};

// WinRT imports - use Graphics namespace not Win32
use windows::Graphics::Capture::*;
use windows::Graphics::DirectX::Direct3D11::*;
use windows::Graphics::DirectX::*;

/// WinRT screen capture using Windows.Graphics.Capture API
pub struct WinrtCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    direct3d_device: Option<IDirect3DDevice>,
    item: Option<GraphicsCaptureItem>,
    frame_pool: Option<Direct3D11CaptureFramePool>,
    session: Option<GraphicsCaptureSession>,
    frame_event_rx: Option<Receiver<()>>,
    frame_arrived_token: Option<i64>,
    closed_token: Option<i64>,
    closed: Arc<AtomicBool>,
    output_memory_kind: FrameMemoryKind,
    shared_texture: Option<SharedBgraTexture>,
    last_frame: Option<CapturedFrame>,
    source_width: usize,
    source_height: usize,
    width: usize,
    height: usize,
    target_dimensions: WinrtTargetDimensions,
}

unsafe impl Send for WinrtCapture {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WinrtTargetDimensions {
    Native,
    Fixed { width: usize, height: usize },
}

impl WinrtTargetDimensions {
    fn resolve(self, source_width: usize, source_height: usize) -> (usize, usize) {
        match self {
            Self::Native => (
                native_even_target_dimension(source_width),
                native_even_target_dimension(source_height),
            ),
            Self::Fixed { width, height } => (
                clamp_even_target_dimension(width, source_width),
                clamp_even_target_dimension(height, source_height),
            ),
        }
    }
}

struct SharedBgraTexture {
    texture: ID3D11Texture2D,
    shared_handle: isize,
    width: u32,
    height: u32,
}

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

    /// Create a new D3D11 shared-texture capture from a monitor index.
    pub fn from_monitor_index_shared_texture(monitor_index: u32) -> Result<Self, PipelineError> {
        Self::from_monitor_index(monitor_index).map(Self::with_shared_texture_output)
    }

    /// Create a new capture from a DXGI monitor device name, e.g. `\\.\DISPLAY1`.
    pub fn from_monitor_device_name(device_name: &str) -> Result<Self, PipelineError> {
        let item = Self::get_monitor_item_by_device_name(device_name).map_err(|e| {
            PipelineError::message(format!("get monitor item by device name failed: {e}"))
        })?;
        Self::from_item(item)
    }

    /// Create a new D3D11 shared-texture capture from a DXGI monitor device name.
    pub fn from_monitor_device_name_shared_texture(
        device_name: &str,
    ) -> Result<Self, PipelineError> {
        Self::from_monitor_device_name(device_name).map(Self::with_shared_texture_output)
    }

    /// Create a new capture from a window handle (HWND)
    pub fn from_window(hwnd: HWND) -> Result<Self, PipelineError> {
        let item = Self::create_item_for_window(hwnd)
            .map_err(|e| PipelineError::message(format!("create window item failed: {e}")))?;
        Self::from_item(item)
    }

    /// Create a new D3D11 shared-texture capture from a window handle.
    pub fn from_window_shared_texture(hwnd: HWND) -> Result<Self, PipelineError> {
        Self::from_window(hwnd).map(Self::with_shared_texture_output)
    }

    /// Create a new capture from a raw native window handle.
    ///
    /// This keeps callers that use a different `windows` crate version from
    /// exposing incompatible `HWND` types across crate boundaries.
    pub fn from_window_handle(hwnd: isize) -> Result<Self, PipelineError> {
        Self::from_window(HWND(hwnd as *mut core::ffi::c_void))
    }

    /// Create a new D3D11 shared-texture capture from a raw native window handle.
    pub fn from_window_handle_shared_texture(hwnd: isize) -> Result<Self, PipelineError> {
        Self::from_window_handle(hwnd).map(Self::with_shared_texture_output)
    }

    /// Switch the capture output to a D3D11 shared BGRA texture.
    pub fn with_shared_texture_output(mut self) -> Self {
        self.output_memory_kind = FrameMemoryKind::D3D11SharedBgra;
        self.refresh_output_dimensions_for_source();
        self.shared_texture = None;
        self
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
            direct3d_device: None,
            item: Some(item),
            frame_pool: None,
            session: None,
            frame_event_rx: None,
            frame_arrived_token: None,
            closed_token: None,
            closed: Arc::new(AtomicBool::new(false)),
            output_memory_kind: FrameMemoryKind::Cpu,
            shared_texture: None,
            last_frame: None,
            source_width: width,
            source_height: height,
            width,
            height,
            target_dimensions: WinrtTargetDimensions::Native,
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

    /// Get a monitor capture item by DXGI device name.
    fn get_monitor_item_by_device_name(device_name: &str) -> Result<GraphicsCaptureItem> {
        let requested = device_name.trim();
        if requested.is_empty() {
            return Err(anyhow!("Monitor device name is empty"));
        }

        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.context("CreateDXGIFactory1 failed")?;

        for adapter_i in 0..10 {
            let adapter = unsafe { factory.EnumAdapters1(adapter_i) };
            let Ok(adapter) = adapter else { continue };

            for output_i in 0..10 {
                let output = unsafe { adapter.EnumOutputs(output_i) };
                let Ok(output) = output else { continue };

                let desc = unsafe {
                    match output.GetDesc() {
                        Ok(d) => d,
                        Err(_) => continue,
                    }
                };

                if dxgi_device_name_matches(&desc.DeviceName, requested) {
                    return Self::create_item_for_monitor(desc.Monitor);
                }
            }
        }

        Err(anyhow!("Monitor device name '{requested}' not found"))
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

    /// Set target output dimensions for D3D11 shared-texture output.
    pub fn set_target_dimensions(&mut self, width: usize, height: usize) {
        self.target_dimensions = WinrtTargetDimensions::Fixed { width, height };
        self.refresh_output_dimensions_for_source();
        self.shared_texture = None;
    }

    /// Start the capture session
    pub fn start(&mut self) -> Result<(), PipelineError> {
        if self.session.is_some() {
            return Err(PipelineError::message("WinRT capture is already started"));
        }

        let item = self.item.as_ref().cloned().ok_or_else(|| {
            PipelineError::message("Cannot start capture: no capture item available")
        })?;
        self.closed.store(false, Ordering::Relaxed);

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
        self.direct3d_device = Some(d3d_device.clone());

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

        configure_capture_session(&session)?;

        let (frame_event_tx, frame_event_rx) = sync_channel::<()>(1);
        let frame_arrived_token = frame_pool
            .FrameArrived(
                &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                    let _ = frame_event_tx.try_send(());
                    Ok(())
                }),
            )
            .map_err(|e| PipelineError::message(format!("register FrameArrived failed: {e:?}")))?;

        let closed = self.closed.clone();
        let closed_token = item
            .Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    closed.store(true, Ordering::Relaxed);
                    Ok(())
                }),
            )
            .map_err(|e| PipelineError::message(format!("register Closed failed: {e:?}")))?;

        self.frame_pool = Some(frame_pool);
        self.session = Some(session);
        self.frame_event_rx = Some(frame_event_rx);
        self.frame_arrived_token = Some(frame_arrived_token);
        self.closed_token = Some(closed_token);

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
        self.closed.store(true, Ordering::Relaxed);

        if let Some(frame_pool) = self.frame_pool.as_ref() {
            if let Some(token) = self.frame_arrived_token.take() {
                let _ = frame_pool.RemoveFrameArrived(token);
            }
            let _ = frame_pool.Close();
        }

        if let Some(session) = self.session.take() {
            let _ = session.Close();
        }

        if let Some(item) = self.item.as_ref() {
            if let Some(token) = self.closed_token.take() {
                let _ = item.RemoveClosed(token);
            }
        }

        self.frame_pool = None;
        self.frame_event_rx = None;
        self.direct3d_device = None;
        self.shared_texture = None;
        self.last_frame = None;
        Ok(())
    }

    /// Try to get the latest captured frame
    pub fn try_get_frame(&mut self) -> Option<CapturedFrame> {
        let frame_pool = self.frame_pool.clone()?;
        let frame = frame_pool.TryGetNextFrame().ok()?;
        let frame = self.frame_to_captured_frame(&frame).ok()?;
        self.last_frame = Some(frame.clone());
        Some(frame)
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
            .clone()
            .ok_or_else(|| PipelineError::message("Capture not started - call start() first"))?;

        let deadline = Instant::now() + timeout;
        let mut last_error: Option<String>;

        loop {
            if self.closed.load(Ordering::Relaxed) {
                return Err(PipelineError::message("WinRT capture item was closed"));
            }

            match frame_pool.TryGetNextFrame() {
                Ok(frame) => {
                    let frame = self.frame_to_captured_frame(&frame)?;
                    self.last_frame = Some(frame.clone());
                    return Ok(frame);
                }
                Err(error) => last_error = Some(format!("{error:?}")),
            }

            if let Some(frame) = self.last_frame.as_ref() {
                return Ok(reused_last_frame_with_timestamp(frame, now_us()));
            }

            let now = Instant::now();
            if now >= deadline {
                let suffix = last_error
                    .map(|error| format!("; last TryGetNextFrame error: {error}"))
                    .unwrap_or_default();
                return Err(PipelineError::message(format!(
                    "WinRT capture produced no frame within {} ms{}",
                    timeout.as_millis(),
                    suffix
                )));
            }

            let remaining = deadline.saturating_duration_since(now);
            let wait_for = remaining.min(Duration::from_millis(250));
            let recv_result = self
                .frame_event_rx
                .as_ref()
                .ok_or_else(|| PipelineError::message("Capture event receiver is not initialized"))?
                .recv_timeout(wait_for);

            match recv_result {
                Ok(()) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(PipelineError::message(
                        "WinRT capture event receiver disconnected",
                    ));
                }
            }
        }
    }

    fn frame_to_captured_frame(
        &mut self,
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

        self.recreate_frame_pool_if_needed(content_size, width, height)?;

        if desc.SampleDesc.Count > 1 {
            return Err(PipelineError::message(format!(
                "unsupported multisampled WinRT frame texture: {} samples",
                desc.SampleDesc.Count
            )));
        }

        if self.output_memory_kind == FrameMemoryKind::D3D11SharedBgra {
            return self.frame_to_shared_texture_frame(&texture, width, height);
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

        Ok(CapturedFrame::from_cpu(
            width,
            height,
            FramePixelFormat::Bgra32,
            now_us(),
            data,
        ))
    }

    fn frame_to_shared_texture_frame(
        &mut self,
        texture: &ID3D11Texture2D,
        source_width: usize,
        source_height: usize,
    ) -> Result<CapturedFrame, PipelineError> {
        if !source_supports_shared_even_target(source_width)
            || !source_supports_shared_even_target(source_height)
        {
            return Err(PipelineError::message(format!(
                "WinRT shared texture source is too small for even target: {source_width}x{source_height}"
            )));
        }

        let source_resource: ID3D11Resource = texture.cast().map_err(|e| {
            PipelineError::message(format!("cast source texture to resource failed: {e:?}"))
        })?;

        let (width, height) = shared_target_dimensions_for_source(
            self.target_dimensions,
            source_width,
            source_height,
        );
        self.ensure_shared_texture(width, height)?;
        let shared = self
            .shared_texture
            .as_ref()
            .ok_or_else(|| PipelineError::message("shared texture not initialized"))?;
        let shared_handle = shared.shared_handle;
        let shared_texture = shared.texture.clone();
        let target_resource: ID3D11Resource = shared_texture.cast().map_err(|e| {
            PipelineError::message(format!("cast shared texture to resource failed: {e:?}"))
        })?;

        unsafe {
            if width == source_width && height == source_height {
                self.context
                    .CopyResource(&target_resource, &source_resource);
            } else {
                let left = source_width.saturating_sub(width) as u32 / 2;
                let top = source_height.saturating_sub(height) as u32 / 2;
                let source_box = D3D11_BOX {
                    left,
                    top,
                    front: 0,
                    right: left + width as u32,
                    bottom: top + height as u32,
                    back: 1,
                };
                self.context.CopySubresourceRegion(
                    &target_resource,
                    0,
                    0,
                    0,
                    0,
                    &source_resource,
                    0,
                    Some(&source_box),
                );
            }
            self.context.Flush();
        }

        Ok(CapturedFrame::from_d3d11_shared_bgra(
            width,
            height,
            now_us(),
            shared_handle,
            width.saturating_mul(4) as u32,
        ))
    }

    fn ensure_shared_texture(&mut self, width: usize, height: usize) -> Result<(), PipelineError> {
        let width = width as u32;
        let height = height as u32;
        let needs_new = self
            .shared_texture
            .as_ref()
            .map(|texture| texture.width != width || texture.height != height)
            .unwrap_or(true);

        if needs_new {
            self.shared_texture = Some(
                SharedBgraTexture::new(&self.device, width, height).map_err(|error| {
                    PipelineError::message(format!("create WinRT shared texture failed: {error}"))
                })?,
            );
        }

        Ok(())
    }

    fn recreate_frame_pool_if_needed(
        &mut self,
        content_size: SizeInt32,
        width: usize,
        height: usize,
    ) -> Result<(), PipelineError> {
        if self.source_width == width && self.source_height == height {
            return Ok(());
        }

        let frame_pool = self
            .frame_pool
            .as_ref()
            .ok_or_else(|| PipelineError::message("Capture frame pool is not initialized"))?;
        let d3d_device = self
            .direct3d_device
            .as_ref()
            .ok_or_else(|| PipelineError::message("Capture D3D device is not initialized"))?;

        frame_pool
            .Recreate(
                d3d_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                content_size,
            )
            .map_err(|e| PipelineError::message(format!("recreate frame pool failed: {e:?}")))?;

        self.source_width = width;
        self.source_height = height;
        self.refresh_output_dimensions_for_source();
        self.shared_texture = None;
        Ok(())
    }

    fn refresh_output_dimensions_for_source(&mut self) {
        (self.width, self.height) = output_dimensions_for_source(
            self.output_memory_kind,
            self.target_dimensions,
            self.source_width,
            self.source_height,
        );
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

impl FrameCapture for WinrtCapture {
    fn output_memory_kind(&self) -> FrameMemoryKind {
        self.output_memory_kind
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        WinrtCapture::capture_frame(self)
    }
}

impl SharedBgraTexture {
    fn new(device: &ID3D11Device, width: u32, height: u32) -> anyhow::Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
        };

        let mut texture = None::<ID3D11Texture2D>;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .context("CreateTexture2D failed")?;
        let texture = texture.ok_or_else(|| anyhow!("CreateTexture2D returned none"))?;
        let dxgi_resource: IDXGIResource =
            texture.cast().context("cast to IDXGIResource failed")?;
        let shared_handle =
            unsafe { dxgi_resource.GetSharedHandle() }.context("GetSharedHandle failed")?;

        if shared_handle == HANDLE::default() {
            return Err(anyhow!("GetSharedHandle returned null handle"));
        }

        Ok(Self {
            texture,
            shared_handle: shared_handle.0 as isize,
            width,
            height,
        })
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

fn configure_capture_session(session: &GraphicsCaptureSession) -> Result<(), PipelineError> {
    if graphics_capture_session_property_supported("IsCursorCaptureEnabled") {
        session.SetIsCursorCaptureEnabled(true).map_err(|e| {
            PipelineError::message(format!("set WinRT cursor capture failed: {e:?}"))
        })?;
    }

    if graphics_capture_session_property_supported("IsBorderRequired") {
        session.SetIsBorderRequired(false).map_err(|e| {
            PipelineError::message(format!("disable WinRT capture border failed: {e:?}"))
        })?;
    }

    if graphics_capture_session_property_supported("IncludeSecondaryWindows") {
        session.SetIncludeSecondaryWindows(true).map_err(|e| {
            PipelineError::message(format!("set WinRT secondary window capture failed: {e:?}"))
        })?;
    }

    Ok(())
}

fn graphics_capture_session_property_supported(property_name: &str) -> bool {
    ApiInformation::IsPropertyPresent(
        &HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureSession"),
        &HSTRING::from(property_name),
    )
    .unwrap_or(false)
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

fn reused_last_frame_with_timestamp(frame: &CapturedFrame, timestamp_us: u64) -> CapturedFrame {
    let mut frame = frame.clone();
    frame.timestamp_us = timestamp_us;
    frame
}

unsafe extern "system" fn enum_window_capture_target(hwnd: HWND, lparam: LPARAM) -> BOOL {
    if !is_window_capture_candidate(hwnd) {
        return true.into();
    }

    let title = read_window_text(hwnd);
    let title = title.trim().to_string();
    if title.is_empty() {
        return true.into();
    }

    let Some(rect) = read_window_capture_bounds(hwnd) else {
        return true.into();
    };

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

unsafe fn is_window_capture_candidate(hwnd: HWND) -> bool {
    if !IsWindowVisible(hwnd).as_bool() {
        return false;
    }

    let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    if style & WS_CHILD.0 != 0 {
        return false;
    }

    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return false;
    }

    let mut client_rect = RECT::default();
    if GetClientRect(hwnd, &mut client_rect).is_err() {
        return false;
    }
    if client_rect.right.saturating_sub(client_rect.left) <= 0
        || client_rect.bottom.saturating_sub(client_rect.top) <= 0
    {
        return false;
    }

    let mut process_id = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut process_id));
    process_id != 0 && process_id != GetCurrentProcessId()
}

unsafe fn read_window_capture_bounds(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    if DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut rect as *mut RECT as *mut std::ffi::c_void,
        std::mem::size_of::<RECT>() as u32,
    )
    .is_ok()
        && rect.right > rect.left
        && rect.bottom > rect.top
    {
        return Some(rect);
    }

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_ok() && rect.right > rect.left && rect.bottom > rect.top {
        Some(rect)
    } else {
        None
    }
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

fn clamp_even_target_dimension(requested: usize, source: usize) -> usize {
    (requested.clamp(2, source.max(2)) & !1).max(2)
}

fn native_even_target_dimension(source: usize) -> usize {
    clamp_even_target_dimension(source, source)
}

fn shared_target_dimensions_for_source(
    target_dimensions: WinrtTargetDimensions,
    source_width: usize,
    source_height: usize,
) -> (usize, usize) {
    target_dimensions.resolve(source_width, source_height)
}

fn output_dimensions_for_source(
    output_memory_kind: FrameMemoryKind,
    target_dimensions: WinrtTargetDimensions,
    source_width: usize,
    source_height: usize,
) -> (usize, usize) {
    if output_memory_kind == FrameMemoryKind::D3D11SharedBgra {
        shared_target_dimensions_for_source(target_dimensions, source_width, source_height)
    } else {
        (source_width, source_height)
    }
}

fn source_supports_shared_even_target(source: usize) -> bool {
    source >= 2
}

fn dxgi_device_name_from_raw(raw: &[u16]) -> Option<String> {
    let end = raw.iter().position(|unit| *unit == 0).unwrap_or(raw.len());
    if end == 0 {
        return None;
    }
    let value = String::from_utf16_lossy(&raw[..end]);
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn dxgi_device_name_matches(raw: &[u16], requested: &str) -> bool {
    let requested = requested.trim();
    !requested.is_empty()
        && dxgi_device_name_from_raw(raw)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(requested))
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
    fn dxgi_device_name_matches_trimmed_case_insensitive_names() {
        let mut raw = [0_u16; 32];
        for (index, unit) in "\\\\.\\DISPLAY2".encode_utf16().enumerate() {
            raw[index] = unit;
        }

        assert_eq!(
            dxgi_device_name_from_raw(&raw).as_deref(),
            Some("\\\\.\\DISPLAY2")
        );
        assert!(dxgi_device_name_matches(&raw, " \\\\.\\display2 "));
        assert!(!dxgi_device_name_matches(&raw, "\\\\.\\DISPLAY1"));
    }

    #[test]
    fn clamp_even_target_dimension_keeps_shared_targets_even_after_source_clamp() {
        assert_eq!(clamp_even_target_dimension(1920, 1001), 1000);
        assert_eq!(clamp_even_target_dimension(1080, 777), 776);
        assert_eq!(clamp_even_target_dimension(1, 1), 2);
        assert_eq!(clamp_even_target_dimension(800, 1000), 800);
    }

    #[test]
    fn native_even_target_dimension_reconciles_resized_sources() {
        assert_eq!(native_even_target_dimension(1001), 1000);
        assert_eq!(native_even_target_dimension(777), 776);
        assert_eq!(native_even_target_dimension(1), 2);
    }

    #[test]
    fn fixed_target_dimension_preserves_requested_profile_on_resize() {
        let target_dimensions = WinrtTargetDimensions::Fixed {
            width: 1000,
            height: 776,
        };

        assert_eq!(target_dimensions.resolve(1001, 777), (1000, 776));
        assert_eq!(target_dimensions.resolve(1200, 900), (1000, 776));
        assert_eq!(target_dimensions.resolve(640, 480), (640, 480));
    }

    #[test]
    fn native_target_dimension_follows_source_resize() {
        let target_dimensions = WinrtTargetDimensions::Native;

        assert_eq!(target_dimensions.resolve(1001, 777), (1000, 776));
        assert_eq!(target_dimensions.resolve(1200, 900), (1200, 900));
    }

    #[test]
    fn native_shared_target_dimension_uses_even_size_for_default_first_frame() {
        assert_eq!(
            output_dimensions_for_source(
                FrameMemoryKind::D3D11SharedBgra,
                WinrtTargetDimensions::Native,
                1001,
                777,
            ),
            (1000, 776)
        );
    }

    #[test]
    fn cpu_native_target_dimension_keeps_raw_source_size() {
        assert_eq!(
            output_dimensions_for_source(
                FrameMemoryKind::Cpu,
                WinrtTargetDimensions::Native,
                1001,
                777,
            ),
            (1001, 777)
        );
    }

    #[test]
    fn fixed_shared_target_dimension_survives_resize_and_clamps_when_source_shrinks() {
        let target_dimensions = WinrtTargetDimensions::Fixed {
            width: 1000,
            height: 776,
        };

        assert_eq!(
            output_dimensions_for_source(
                FrameMemoryKind::D3D11SharedBgra,
                target_dimensions,
                1200,
                900,
            ),
            (1000, 776)
        );
        assert_eq!(
            output_dimensions_for_source(
                FrameMemoryKind::D3D11SharedBgra,
                target_dimensions,
                640,
                480,
            ),
            (640, 480)
        );
    }

    #[test]
    fn shared_target_dimension_rejects_sources_smaller_than_two_pixels() {
        assert!(!source_supports_shared_even_target(0));
        assert!(!source_supports_shared_even_target(1));
        assert!(source_supports_shared_even_target(2));
        assert!(source_supports_shared_even_target(3));
    }

    #[test]
    fn reused_last_frame_refreshes_timestamp() {
        let previous = CapturedFrame::from_cpu(2, 2, FramePixelFormat::Bgra32, 10, vec![0; 16]);

        let reused = reused_last_frame_with_timestamp(&previous, 20);

        assert_eq!(reused.width, 2);
        assert_eq!(reused.height, 2);
        assert_eq!(reused.pixel_format, FramePixelFormat::Bgra32);
        assert_eq!(reused.data, previous.data);
        assert_eq!(reused.timestamp_us, 20);
    }

    #[test]
    #[ignore]
    fn test_get_capture_info() {
        let info = get_capture_info();
        println!("Capture info: {:?}", info);
    }
}
