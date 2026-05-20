use mrd_pipeline_core::{
    CapturedFrame, FrameCapture, FrameMemoryKind, FramePixelFormat, PipelineError,
};
use scrap::{Capturer, Display};
use std::{
    ffi::c_void,
    io::ErrorKind,
    mem, thread,
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
    DXGI_OUTPUT_DESC,
};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};

#[cfg(windows)]
// Keep the media sender paced by the requested profile. When Desktop Duplication
// has no new desktop update, capture_frame reuses the last shared texture.
const DXGI_SHARED_ACQUIRE_TIMEOUT_MS: u32 = 0;
#[cfg(windows)]
const DXGI_SHARED_TEXTURE_RING_SIZE: usize = 3;

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
    shared_textures: Vec<SharedBgraTexture>,
    next_shared_texture_index: usize,
    last_shared_texture_index: Option<usize>,
    source_left: i32,
    source_top: i32,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DxgiOutputTarget {
    pub adapter_index: u32,
    pub output_index: u32,
    pub device_name: String,
    pub left: i32,
    pub top: i32,
    pub width: usize,
    pub height: usize,
}

#[cfg(windows)]
impl DxgiSharedTextureCapture {
    pub fn new_primary() -> Result<Self, PipelineError> {
        Self::new_first_output()
    }

    pub fn new_for_device_name(device_name: &str) -> Result<Self, PipelineError> {
        let requested = device_name.trim().to_string();
        if requested.is_empty() {
            return Err(PipelineError::message("DXGI output device name is empty"));
        }
        Self::new_matching_output(
            |desc| dxgi_device_name_matches(&desc.DeviceName, &requested),
            &format!("no attached DXGI output matched {requested}"),
        )
    }

    fn new_first_output() -> Result<Self, PipelineError> {
        Self::new_matching_output(|_| true, "no attached DXGI output found")
    }

    fn new_matching_output(
        mut accepts_output: impl FnMut(&DXGI_OUTPUT_DESC) -> bool,
        missing_message: &str,
    ) -> Result<Self, PipelineError> {
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(|error| {
            PipelineError::message(format!("CreateDXGIFactory1 failed: {error}"))
        })?;
        let mut duplicate_errors = Vec::new();

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
                if !accepts_output(&desc) {
                    continue;
                }

                let output1: IDXGIOutput1 = output.cast().map_err(|error| {
                    PipelineError::message(format!("cast IDXGIOutput1 failed: {error}"))
                })?;
                let (device, context) =
                    create_d3d11_device_for_adapter(&adapter).map_err(|error| {
                        PipelineError::message(format!("create D3D11 device failed: {error}"))
                    })?;
                let duplication = match unsafe { output1.DuplicateOutput(&device) } {
                    Ok(duplication) => duplication,
                    Err(error) => {
                        duplicate_errors.push(format!(
                            "{} adapter={} output={}: {error}",
                            dxgi_device_name_from_raw(&desc.DeviceName)
                                .unwrap_or_else(|| "<unknown>".to_string()),
                            adapter_index,
                            output_index
                        ));
                        continue;
                    }
                };

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
                    shared_textures: Vec::new(),
                    next_shared_texture_index: 0,
                    last_shared_texture_index: None,
                    source_left: rect.left,
                    source_top: rect.top,
                    source_width: width,
                    source_height: height,
                    width,
                    height,
                });
            }
        }

        if duplicate_errors.is_empty() {
            Err(PipelineError::message(missing_message.to_string()))
        } else {
            Err(PipelineError::message(format!(
                "{missing_message}; DuplicateOutput failures: {}",
                duplicate_errors.join("; ")
            )))
        }
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
        self.shared_textures.clear();
        self.next_shared_texture_index = 0;
        self.last_shared_texture_index = None;
    }

    fn ensure_shared_textures(&mut self) -> Result<(), PipelineError> {
        let width = self.width as u32;
        let height = self.height as u32;
        let needs_new = self
            .shared_textures
            .iter()
            .any(|texture| texture.width != width || texture.height != height)
            || self.shared_textures.len() != DXGI_SHARED_TEXTURE_RING_SIZE;

        if needs_new {
            self.shared_textures.clear();
            for _ in 0..DXGI_SHARED_TEXTURE_RING_SIZE {
                self.shared_textures.push(
                    SharedBgraTexture::new(&self.device, width, height).map_err(|error| {
                        PipelineError::message(format!(
                            "create shared BGRA texture failed: {error}"
                        ))
                    })?,
                );
            }
            self.next_shared_texture_index = 0;
            self.last_shared_texture_index = None;
        }

        Ok(())
    }

    fn next_shared_texture(&mut self) -> Result<(isize, ID3D11Texture2D), PipelineError> {
        self.ensure_shared_textures()?;
        let len = self.shared_textures.len();
        if len == 0 {
            return Err(PipelineError::message("shared texture ring is empty"));
        }
        let index = self.next_shared_texture_index % len;
        self.next_shared_texture_index = (index + 1) % len;
        self.last_shared_texture_index = Some(index);
        let shared = &self.shared_textures[index];
        Ok((shared.shared_handle, shared.texture.clone()))
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
                    return self.seed_shared_texture_from_gdi();
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
    fn seed_shared_texture_from_gdi(&mut self) -> Result<CapturedFrame, PipelineError> {
        let width = self.width;
        let height = self.height;
        let bgra = capture_gdi_bgra_region(
            self.source_left,
            self.source_top,
            self.source_width,
            self.source_height,
            width,
            height,
        )?;
        let (shared_handle, shared_texture) = self.next_shared_texture()?;
        let target_resource: ID3D11Resource = shared_texture.cast().map_err(|error| {
            PipelineError::message(format!(
                "cast seeded shared texture to resource failed: {error}"
            ))
        })?;
        let row_pitch = width
            .checked_mul(4)
            .ok_or_else(|| PipelineError::message("seeded frame row pitch overflow"))?
            as u32;
        unsafe {
            self.context.UpdateSubresource(
                &target_resource,
                0,
                None,
                bgra.as_ptr() as *const c_void,
                row_pitch,
                0,
            );
            self.context.Flush();
        }

        Ok(CapturedFrame::from_d3d11_shared_bgra(
            width,
            height,
            now_us()?,
            shared_handle,
            row_pitch,
        ))
    }

    fn last_shared_frame(&self) -> Result<Option<CapturedFrame>, PipelineError> {
        let Some(index) = self.last_shared_texture_index else {
            return Ok(None);
        };
        let Some(shared) = self.shared_textures.get(index) else {
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
        let (shared_handle, shared_texture) = self.next_shared_texture()?;
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
fn capture_gdi_bgra_region(
    source_left: i32,
    source_top: i32,
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
) -> Result<Vec<u8>, PipelineError> {
    let target_width_i32 = i32::try_from(target_width)
        .map_err(|_| PipelineError::message("GDI target width exceeds i32"))?;
    let target_height_i32 = i32::try_from(target_height)
        .map_err(|_| PipelineError::message("GDI target height exceeds i32"))?;
    let (source_x, source_y) =
        centered_crop_origin(source_width, source_height, target_width, target_height);
    let source_x = source_left.saturating_add(source_x);
    let source_y = source_top.saturating_add(source_y);

    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(PipelineError::message("GetDC returned invalid screen DC"));
        }

        let memory_dc = CreateCompatibleDC(Some(screen_dc));
        if memory_dc.is_invalid() {
            let _ = ReleaseDC(None, screen_dc);
            return Err(PipelineError::message(
                "CreateCompatibleDC returned invalid memory DC",
            ));
        }

        let bitmap = CreateCompatibleBitmap(screen_dc, target_width_i32, target_height_i32);
        if bitmap.is_invalid() {
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err(PipelineError::message(
                "CreateCompatibleBitmap returned invalid bitmap",
            ));
        }

        let previous_object = SelectObject(memory_dc, bitmap.into());
        if previous_object.is_invalid() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory_dc);
            let _ = ReleaseDC(None, screen_dc);
            return Err(PipelineError::message("SelectObject failed for GDI bitmap"));
        }

        let blit_result = BitBlt(
            memory_dc,
            0,
            0,
            target_width_i32,
            target_height_i32,
            Some(screen_dc),
            source_x,
            source_y,
            SRCCOPY,
        );

        let mut pixels = vec![0_u8; target_width * target_height * 4];
        let mut bitmap_info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: target_width_i32,
                biHeight: -target_height_i32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: pixels.len() as u32,
                ..Default::default()
            },
            ..Default::default()
        };

        let read_lines = if blit_result.is_ok() {
            GetDIBits(
                memory_dc,
                bitmap,
                0,
                target_height as u32,
                Some(pixels.as_mut_ptr() as *mut c_void),
                &mut bitmap_info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        let _ = SelectObject(memory_dc, previous_object);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory_dc);
        let _ = ReleaseDC(None, screen_dc);

        blit_result.map_err(|error| PipelineError::message(format!("BitBlt failed: {error}")))?;
        if read_lines == 0 {
            return Err(PipelineError::message("GetDIBits returned no scanlines"));
        }

        Ok(pixels)
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

fn centered_crop_origin(
    source_width: usize,
    source_height: usize,
    target_width: usize,
    target_height: usize,
) -> (i32, i32) {
    let x = source_width.saturating_sub(target_width) / 2;
    let y = source_height.saturating_sub(target_height) / 2;
    (
        i32::try_from(x).unwrap_or(i32::MAX),
        i32::try_from(y).unwrap_or(i32::MAX),
    )
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
    let Some(actual) = dxgi_device_name_from_raw(raw) else {
        return false;
    };
    actual.eq_ignore_ascii_case(requested.trim())
}

fn now_us() -> Result<u64, PipelineError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| PipelineError::message(format!("system time failed: {error}")))?
        .as_micros() as u64)
}

#[cfg(test)]
mod tests {
    use super::{centered_crop_origin, dxgi_device_name_matches, repack_bgra};

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

    #[test]
    fn centered_crop_origin_uses_middle_of_larger_source() {
        assert_eq!(centered_crop_origin(2560, 1600, 1920, 1080), (320, 260));
        assert_eq!(centered_crop_origin(1920, 1080, 1920, 1080), (0, 0));
        assert_eq!(centered_crop_origin(1280, 720, 1920, 1080), (0, 0));
    }

    #[test]
    fn dxgi_device_name_matches_trimmed_case_insensitive_names() {
        let mut raw = [0_u16; 32];
        for (index, unit) in "\\\\.\\DISPLAY2".encode_utf16().enumerate() {
            raw[index] = unit;
        }

        assert!(dxgi_device_name_matches(&raw, "\\\\.\\display2"));
        assert!(!dxgi_device_name_matches(&raw, "\\\\.\\DISPLAY1"));
    }
}

#[cfg(windows)]
pub fn enumerate_dxgi_output_targets() -> Result<Vec<DxgiOutputTarget>, PipelineError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|error| PipelineError::message(format!("CreateDXGIFactory1 failed: {error}")))?;
    let mut targets = Vec::new();

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
            let rect = desc.DesktopCoordinates;
            let width = rect.right.saturating_sub(rect.left) as usize;
            let height = rect.bottom.saturating_sub(rect.top) as usize;
            if width == 0 || height == 0 {
                continue;
            }
            let Some(device_name) = dxgi_device_name_from_raw(&desc.DeviceName) else {
                continue;
            };
            targets.push(DxgiOutputTarget {
                adapter_index,
                output_index,
                device_name,
                left: rect.left,
                top: rect.top,
                width,
                height,
            });
        }
    }

    Ok(targets)
}
