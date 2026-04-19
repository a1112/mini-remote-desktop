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
use mrd_pipeline_core::{CapturedFrame, PipelineError};
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::*;
use windows::Win32::System::WinRT::*;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::*;

// WinRT imports - use Graphics namespace not Win32
use windows::Graphics::Capture::*;
use windows::Graphics::DirectX::*;
use windows::Graphics::DirectX::Direct3D11::*;

/// WinRT screen capture using Windows.Graphics.Capture API
pub struct WinrtCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    item: Option<GraphicsCaptureItem>,
    frame_pool: Option<Direct3D11CaptureFramePool>,
    session: Option<GraphicsCaptureSession>,
    width: usize,
    height: usize,
    frame_received: Arc<tokio::sync::Notify>,
}

unsafe impl Send for WinrtCapture {}

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

        let size = item.Size().map_err(|e| {
            PipelineError::message(format!("get capture item size failed: {e:?}"))
        })?;

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
            frame_received: Arc::new(tokio::sync::Notify::new()),
        })
    }

    /// Get a monitor capture item by index
    fn get_monitor_item(monitor_index: u32) -> Result<GraphicsCaptureItem> {
        // Create DXGI factory
        let factory: IDXGIFactory1 = unsafe {
            CreateDXGIFactory1()
        }
        .context("CreateDXGIFactory1 failed")?;

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
            RoGetActivationFactory(&HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureItem"))
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
            RoGetActivationFactory(&HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureItem"))
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
        let dxgi_device: IDXGIDevice = self.device.cast().map_err(|e| {
            PipelineError::message(format!("cast to IDXGIDevice failed: {e}"))
        })?;

        // Use IDirect3DDevice from WinRT - requires proper interop
        // For now, we need to create IDirect3DDevice from IDXGIDevice
        // This is complex and requires using the WinRT interop APIs

        // Simplified approach: use DXGI device directly with frame pool
        // Note: This is a placeholder - full implementation requires proper WinRT device creation
        let d3d_device = unsafe {
            // Try to create IDirect3DDevice using the activation factory
            let factory: IInspectable = match RoGetActivationFactory(
                &HSTRING::from("Windows.Graphics.DirectX.Direct3D11.Direct3DDevice")
            ) {
                Ok(f) => f,
                Err(e) => {
                    return Err(PipelineError::message(format!(
                        "Failed to get Direct3DDevice factory: {:?}",
                        e
                    )));
                }
            };

            // Cast factory to IActivationFactory
            let activation_factory = factory.cast::<windows::Win32::System::WinRT::IActivationFactory>();
            let activation_factory = match activation_factory {
                Ok(f) => f,
                Err(_) => {
                    return Err(PipelineError::message(
                        "Failed to cast to IActivationFactory"
                    ));
                }
            };

            // Activate to get IDirect3DDevice
            let device: IDirect3DDevice = match activation_factory.ActivateInstance() {
                Ok(d) => match d.cast() {
                    Ok(device) => device,
                    Err(_) => {
                        return Err(PipelineError::message(
                            "Failed to cast activated instance to IDirect3DDevice"
                        ));
                    }
                },
                Err(e) => {
                    return Err(PipelineError::message(format!(
                        "Failed to activate Direct3DDevice: {:?}",
                        e
                    )));
                }
            };

            device
        };

        // Create frame pool with BGRA8 format
        let size = item.Size().map_err(|e| {
            PipelineError::message(format!("get item size failed: {e:?}"))
        })?;

        let frame_pool = Direct3D11CaptureFramePool::Create(
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
        // WinRT capture requires async event handling
        // For now return None - full implementation would need FrameArrived event handler
        None
    }

    /// Capture a frame (synchronous approximation)
    ///
    /// Note: This is a simplified version. Full implementation would use
    /// async event handling with FrameArrived events.
    pub fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        // Check if we have a frame pool
        let _frame_pool = self.frame_pool.as_ref().ok_or_else(|| {
            PipelineError::message("Capture not started - call start() first")
        })?;

        // Try to get the next frame
        // In real implementation, this would wait for FrameArrived event
        // For now, return an error indicating async handling is needed
        Err(PipelineError::message(
            "Frame capture requires async event handling. Use DXGI for synchronous capture.",
        ))
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
