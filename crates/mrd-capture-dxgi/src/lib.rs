use mrd_pipeline_core::{
    CapturedFrame, FrameCapture, FrameMemoryKind, FramePixelFormat, PipelineError,
};
use scrap::{Capturer, Display};
use std::{
    io::ErrorKind,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use anyhow::{anyhow, Context};
#[cfg(windows)]
use windows::core::Interface;
#[cfg(windows)]
use windows::Win32::Foundation::{HANDLE, HMODULE};
#[cfg(windows)]
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
#[cfg(windows)]
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_RESOURCE_MISC_SHARED, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
};
#[cfg(windows)]
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
#[cfg(windows)]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

#[cfg(windows)]
const DXGI_SHARED_ACQUIRE_TIMEOUT_MS: u32 = 1;

pub struct DxgiDesktopCapture {
    capturer: Capturer,
    width: usize,
    height: usize,
}

impl DxgiDesktopCapture {
    pub fn new_primary() -> Result<Self, PipelineError> {
        let display = Display::primary().map_err(|error| {
            PipelineError::message(format!("open primary display failed: {error}"))
        })?;
        Self::new(display)
    }

    pub fn new(display: Display) -> Result<Self, PipelineError> {
        let width = display.width();
        let height = display.height();
        let capturer = Capturer::new(display).map_err(|error| {
            PipelineError::message(format!("create dxgi capturer failed: {error}"))
        })?;

        Ok(Self {
            capturer,
            width,
            height,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

impl FrameCapture for DxgiDesktopCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        loop {
            match self.capturer.frame() {
                Ok(frame) => {
                    let packed = repack_bgra(frame.as_ref(), self.width, self.height)?;
                    let timestamp_us = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|error| {
                            PipelineError::message(format!("system time failed: {error}"))
                        })?
                        .as_micros() as u64;

                    return Ok(CapturedFrame::from_cpu(
                        self.width,
                        self.height,
                        FramePixelFormat::Bgra32,
                        timestamp_us,
                        packed,
                    ));
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(PipelineError::message(format!(
                        "capture frame failed: {error}"
                    )));
                }
            }
        }
    }
}

#[cfg(windows)]
pub struct DxgiSharedTextureCapture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: IDXGIOutputDuplication,
    shared_texture: Option<SharedBgraTexture>,
    source_width: usize,
    source_height: usize,
    width: usize,
    height: usize,
}

#[cfg(windows)]
unsafe impl Send for DxgiSharedTextureCapture {}

#[cfg(windows)]
struct SharedBgraTexture {
    texture: ID3D11Texture2D,
    shared_handle: isize,
    width: u32,
    height: u32,
}

#[cfg(windows)]
impl DxgiSharedTextureCapture {
    pub fn new_primary() -> Result<Self, PipelineError> {
        Self::new_first_output()
    }

    fn new_first_output() -> Result<Self, PipelineError> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|error| {
            PipelineError::message(format!("CreateDXGIFactory1 failed: {error}"))
        })?;

        for adapter_index in 0..16 {
            let adapter1 = match unsafe { factory.EnumAdapters1(adapter_index) } {
                Ok(adapter) => adapter,
                Err(_) => break,
            };
            let adapter: IDXGIAdapter = adapter1.cast().map_err(|error| {
                PipelineError::message(format!("cast IDXGIAdapter failed: {error}"))
            })?;

            for output_index in 0..16 {
                let output = match unsafe { adapter.EnumOutputs(output_index) } {
                    Ok(output) => output,
                    Err(_) => break,
                };
                let desc = unsafe { output.GetDesc() }.map_err(|error| {
                    PipelineError::message(format!("IDXGIOutput::GetDesc failed: {error}"))
                })?;
                if !desc.AttachedToDesktop.as_bool() {
                    continue;
                }

                let output1: IDXGIOutput1 = output.cast().map_err(|error| {
                    PipelineError::message(format!("cast IDXGIOutput1 failed: {error}"))
                })?;
                let (device, context) =
                    create_d3d11_device_for_adapter(&adapter).map_err(|error| {
                        PipelineError::message(format!("create D3D11 device failed: {error}"))
                    })?;
                let duplication = unsafe { output1.DuplicateOutput(&device) }.map_err(|error| {
                    PipelineError::message(format!("DuplicateOutput failed: {error}"))
                })?;

                let rect = desc.DesktopCoordinates;
                let width = rect.right.saturating_sub(rect.left) as usize;
                let height = rect.bottom.saturating_sub(rect.top) as usize;
                if width == 0 || height == 0 {
                    continue;
                }

                return Ok(Self {
                    device,
                    context,
                    duplication,
                    shared_texture: None,
                    source_width: width,
                    source_height: height,
                    width,
                    height,
                });
            }
        }

        Err(PipelineError::message("no attached DXGI output found"))
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn set_target_dimensions(&mut self, width: usize, height: usize) {
        self.width = width.clamp(2, self.source_width.max(2));
        self.height = height.clamp(2, self.source_height.max(2));
        self.shared_texture = None;
    }

    fn ensure_shared_texture(&mut self) -> Result<&SharedBgraTexture, PipelineError> {
        let width = self.width as u32;
        let height = self.height as u32;
        let needs_new = self
            .shared_texture
            .as_ref()
            .map(|texture| texture.width != width || texture.height != height)
            .unwrap_or(true);

        if needs_new {
            self.shared_texture = Some(
                SharedBgraTexture::new(&self.device, width, height).map_err(|error| {
                    PipelineError::message(format!("create shared BGRA texture failed: {error}"))
                })?,
            );
        }

        Ok(self
            .shared_texture
            .as_ref()
            .expect("shared texture initialized"))
    }
}

#[cfg(windows)]
impl FrameCapture for DxgiSharedTextureCapture {
    fn output_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::D3D11SharedBgra
    }

    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError> {
        loop {
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_resource = None::<IDXGIResource>;
            let acquire = unsafe {
                self.duplication.AcquireNextFrame(
                    DXGI_SHARED_ACQUIRE_TIMEOUT_MS,
                    &mut frame_info,
                    &mut desktop_resource,
                )
            };

            match acquire {
                Ok(()) => {
                    let result = self.copy_acquired_frame_to_shared(desktop_resource);
                    let _ = unsafe { self.duplication.ReleaseFrame() };
                    return result;
                }
                Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    if let Some(frame) = self.last_shared_frame()? {
                        return Ok(frame);
                    }
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                    return Err(PipelineError::message("DXGI duplication access lost"));
                }
                Err(error) => {
                    return Err(PipelineError::message(format!(
                        "AcquireNextFrame failed: {error}"
                    )));
                }
            }
        }
    }
}

#[cfg(windows)]
impl DxgiSharedTextureCapture {
    fn last_shared_frame(&self) -> Result<Option<CapturedFrame>, PipelineError> {
        let Some(shared) = self.shared_texture.as_ref() else {
            return Ok(None);
        };

        Ok(Some(CapturedFrame::from_d3d11_shared_bgra(
            self.width,
            self.height,
            now_us()?,
            shared.shared_handle,
            self.width.saturating_mul(4) as u32,
        )))
    }

    fn copy_acquired_frame_to_shared(
        &mut self,
        desktop_resource: Option<IDXGIResource>,
    ) -> Result<CapturedFrame, PipelineError> {
        let desktop_resource = desktop_resource
            .ok_or_else(|| PipelineError::message("AcquireNextFrame returned no resource"))?;
        let desktop_texture: ID3D11Texture2D = desktop_resource.cast().map_err(|error| {
            PipelineError::message(format!("cast desktop frame to texture failed: {error}"))
        })?;
        let source_resource: ID3D11Resource = desktop_texture.cast().map_err(|error| {
            PipelineError::message(format!("cast desktop texture to resource failed: {error}"))
        })?;

        let width = self.width;
        let height = self.height;
        let source_width = self.source_width;
        let source_height = self.source_height;
        self.ensure_shared_texture()?;
        let shared = self
            .shared_texture
            .as_ref()
            .ok_or_else(|| PipelineError::message("shared texture not initialized"))?;
        let shared_handle = shared.shared_handle;
        let shared_texture = shared.texture.clone();
        let target_resource: ID3D11Resource = shared_texture.cast().map_err(|error| {
            PipelineError::message(format!("cast shared texture to resource failed: {error}"))
        })?;

        let copy_full = width == source_width && height == source_height;
        unsafe {
            if copy_full {
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
            now_us()?,
            shared_handle,
            width.saturating_mul(4) as u32,
        ))
    }
}

#[cfg(windows)]
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

#[cfg(windows)]
fn create_d3d11_device_for_adapter(
    adapter: &IDXGIAdapter,
) -> anyhow::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None::<ID3D11Device>;
    let mut context = None::<ID3D11DeviceContext>;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .context("D3D11CreateDevice failed")?;

    Ok((
        device.ok_or_else(|| anyhow!("missing D3D11 device"))?,
        context.ok_or_else(|| anyhow!("missing D3D11 context"))?,
    ))
}

fn repack_bgra(frame: &[u8], width: usize, height: usize) -> Result<Vec<u8>, PipelineError> {
    let stride = frame
        .len()
        .checked_div(height.max(1))
        .ok_or_else(|| PipelineError::message("invalid captured frame height"))?;
    let row_bytes = width
        .checked_mul(4)
        .ok_or_else(|| PipelineError::message("captured frame width overflow"))?;

    if stride < row_bytes || frame.len() < stride * height {
        return Err(PipelineError::message("invalid captured frame stride"));
    }

    let mut packed = Vec::with_capacity(row_bytes * height);
    for row in 0..height {
        let start = row * stride;
        packed.extend_from_slice(&frame[start..start + row_bytes]);
    }
    Ok(packed)
}

fn now_us() -> Result<u64, PipelineError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PipelineError::message(format!("system time failed: {error}")))?
        .as_micros() as u64)
}

#[cfg(test)]
mod tests {
    use super::repack_bgra;

    #[test]
    fn repack_bgra_strips_padding_stride() {
        let frame = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0, 9, 10, 11, 12, 13, 14, 15, 16, 0, 0, 0, 0,
        ];

        let packed = repack_bgra(&frame, 2, 2).expect("packed frame");

        assert_eq!(
            packed,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }
}
