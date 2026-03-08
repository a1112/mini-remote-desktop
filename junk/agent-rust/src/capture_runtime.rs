use anyhow::{Context, Result, anyhow};
use image::ImageReader;
use std::io::Cursor;
use std::process::Command;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use tracing::{info, warn};
#[cfg(windows)]
use windows::core::Interface;

#[derive(Clone)]
pub struct RawFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub capture_start_us: u64,
}

pub enum FrameCapturer {
    Dxgi {
        screen: screenshots::Screen,
    },
    #[cfg(windows)]
    WgcWindow {
        capturer: WgcWindowCapturer,
    },
    Powershell,
    Dummy,
}

impl FrameCapturer {
    pub fn capture(&mut self) -> Result<(Vec<u8>, u32, u32)> {
        match self {
            FrameCapturer::Dxgi { screen } => {
                let img = screen.capture().context("dxgi capture failed")?;
                Ok((img.as_raw().to_vec(), img.width(), img.height()))
            }
            #[cfg(windows)]
            FrameCapturer::WgcWindow { capturer } => capturer.capture(),
            FrameCapturer::Powershell => capture_via_powershell(),
            FrameCapturer::Dummy => {
                let w = 640_u32;
                let h = 360_u32;
                let mut rgba = vec![0_u8; (w * h * 4) as usize];
                for px in rgba.chunks_exact_mut(4) {
                    px[0] = 16;
                    px[1] = 16;
                    px[2] = 16;
                    px[3] = 255;
                }
                Ok((rgba, w, h))
            }
        }
    }
}

pub fn build_frame_capturer(
    backend: crate::capture_policy::CaptureBackend,
) -> Result<FrameCapturer> {
    match backend {
        crate::capture_policy::CaptureBackend::Dxgi => {
            let screens = screenshots::Screen::all().context("list screens failed")?;
            let screen = screens
                .first()
                .ok_or_else(|| anyhow!("no screen found"))?
                .clone();
            Ok(FrameCapturer::Dxgi { screen })
        }
        crate::capture_policy::CaptureBackend::Wgc => {
            #[cfg(windows)]
            {
                Ok(FrameCapturer::WgcWindow {
                    capturer: WgcWindowCapturer::new()?,
                })
            }
            #[cfg(not(windows))]
            {
                Err(anyhow!("wgc capture backend only supports windows"))
            }
        }
        crate::capture_policy::CaptureBackend::Powershell => Ok(FrameCapturer::Powershell),
        crate::capture_policy::CaptureBackend::Dummy => Ok(FrameCapturer::Dummy),
    }
}

pub fn detect_input_resolution() -> Result<(u32, u32)> {
    let screens = screenshots::Screen::all().context("list screens failed")?;
    let screen = screens.first().ok_or_else(|| anyhow!("no screen found"))?;
    let img = screen
        .capture()
        .context("capture for resolution detect failed")?;
    Ok((img.width(), img.height()))
}

pub fn resize_rgba_fast(
    rgba: &[u8],
    width: u32,
    height: u32,
    target_width: u32,
    target_height: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let resized = image::imageops::resize(
        &img,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );
    Some((resized.into_raw(), target_width, target_height))
}

pub fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

fn unix_time_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn capture_via_powershell() -> Result<(Vec<u8>, u32, u32)> {
    let temp_path = std::env::temp_dir().join("mini-rust-agent-ps-capture.jpg");
    let path = temp_path
        .to_str()
        .ok_or_else(|| anyhow!("temp path invalid"))?
        .replace('\'', "''");

    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
         $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; \
         $g=[System.Drawing.Graphics]::FromImage($bmp); \
         $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); \
         $bmp.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Jpeg); \
         $g.Dispose(); $bmp.Dispose(); \
         Write-Output ($b.Width.ToString() + ',' + $b.Height.ToString());"
    );

    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .context("powershell capture spawn failed")?;
    if !out.status.success() {
        return Err(anyhow!(
            "powershell capture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let size_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = size_str.split(',');
    let width = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse width failed"))?;
    let height = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse height failed"))?;

    let jpg = std::fs::read(&temp_path).context("read captured jpeg failed")?;
    let img = ImageReader::new(Cursor::new(jpg))
        .with_guessed_format()
        .context("guess image format failed")?
        .decode()
        .context("decode jpeg failed")?
        .to_rgba8();

    Ok((img.as_raw().to_vec(), width, height))
}

#[cfg(windows)]
pub(crate) struct WgcWindowCapturer {
    _device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
    frame_pool: windows::Graphics::Capture::Direct3D11CaptureFramePool,
    session: windows::Graphics::Capture::GraphicsCaptureSession,
    staging: Option<windows::Win32::Graphics::Direct3D11::ID3D11Texture2D>,
    staging_width: u32,
    staging_height: u32,
    fixed_hwnd: Option<windows::Win32::Foundation::HWND>,
    active_hwnd: windows::Win32::Foundation::HWND,
    consecutive_timeouts: u32,
    disable_rebind: bool,
    rebind_timeout_threshold: u32,
    rebind_cooldown: Duration,
    last_rebind_at: Instant,
    qpc_freq: i64,
    qpc_base: i64,
    unix_base_us: u64,
}

#[cfg(windows)]
unsafe impl Send for WgcWindowCapturer {}

#[cfg(windows)]
pub(crate) struct WgcGpuFrame {
    pub texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    pub capture_start_us: u64,
    _frame: windows::Graphics::Capture::Direct3D11CaptureFrame,
}

#[cfg(windows)]
impl WgcWindowCapturer {
    pub(crate) fn new() -> Result<Self> {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Graphics::Direct3D::{
            D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
        };
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION,
            D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
        };
        use windows::Win32::Graphics::Dxgi::IDXGIDevice;
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut device = None::<ID3D11Device>;
        let mut context = None::<ID3D11DeviceContext>;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_FLAG(D3D11_CREATE_DEVICE_BGRA_SUPPORT.0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .context("wgc: D3D11CreateDevice failed")?;
        let device = device.ok_or_else(|| anyhow!("wgc: D3D11 device is none"))?;
        let context = context.ok_or_else(|| anyhow!("wgc: D3D11 context is none"))?;
        let _dxgi_device: IDXGIDevice = device.cast().context("wgc: cast IDXGIDevice failed")?;
        let fixed_hwnd = resolve_wgc_window_hwnd();
        let active_hwnd = select_wgc_target_hwnd(fixed_hwnd)?;
        let (frame_pool, session) = create_wgc_capture_session(&device, active_hwnd)?;
        let disable_rebind = env_flag("AGENT_WGC_DISABLE_REBIND", false);
        // Keep legacy behavior by default (rebind immediately on timeout).
        let rebind_timeout_threshold = env_u32("AGENT_WGC_REBIND_TIMEOUTS", 1).max(1);
        let rebind_cooldown =
            Duration::from_millis(env_u64("AGENT_WGC_REBIND_COOLDOWN_MS", 0).clamp(0, 10_000));
        info!(
            disable_rebind,
            rebind_timeout_threshold,
            rebind_cooldown_ms = rebind_cooldown.as_millis() as u64,
            "wgc rebind policy configured"
        );

        Ok(Self {
            _device: device,
            context,
            frame_pool,
            session,
            staging: None,
            staging_width: 0,
            staging_height: 0,
            fixed_hwnd,
            active_hwnd,
            consecutive_timeouts: 0,
            disable_rebind,
            rebind_timeout_threshold,
            rebind_cooldown,
            last_rebind_at: Instant::now(),
            qpc_freq: qpc_frequency(),
            qpc_base: qpc_counter_now(),
            unix_base_us: unix_time_us(),
        })
    }

    fn capture(&mut self) -> Result<(Vec<u8>, u32, u32)> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, ID3D11Resource,
        };
        let frame = self.capture_gpu_frame(Duration::from_millis(120))?;
        let width = frame.width;
        let height = frame.height;
        let src_texture = frame.texture;
        self.ensure_staging(width, height)?;
        let staging = self
            .staging
            .as_ref()
            .ok_or_else(|| anyhow!("wgc: staging texture missing"))?;

        let src_resource: ID3D11Resource = src_texture
            .cast()
            .context("wgc: cast src resource failed")?;
        let dst_resource: ID3D11Resource =
            staging.cast().context("wgc: cast dst resource failed")?;
        unsafe {
            self.context.CopyResource(&dst_resource, &src_resource);
        }

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&dst_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .context("wgc: map staging failed")?;

        let mut rgba = vec![0_u8; (width * height * 4) as usize];
        let row_pitch = mapped.RowPitch as usize;
        let src_ptr = mapped.pData as *const u8;
        for y in 0..height as usize {
            let src_row = unsafe {
                std::slice::from_raw_parts(src_ptr.add(y * row_pitch), (width * 4) as usize)
            };
            let dst_row = &mut rgba[(y * width as usize * 4)..((y + 1) * width as usize * 4)];
            for (src_px, dst_px) in src_row.chunks_exact(4).zip(dst_row.chunks_exact_mut(4)) {
                // WGC frame is BGRA8 -> convert to RGBA for existing pipeline.
                dst_px[0] = src_px[2];
                dst_px[1] = src_px[1];
                dst_px[2] = src_px[0];
                dst_px[3] = src_px[3];
            }
        }
        unsafe {
            self.context.Unmap(&dst_resource, 0);
        }
        Ok((rgba, width, height))
    }

    pub(crate) fn device(&self) -> windows::Win32::Graphics::Direct3D11::ID3D11Device {
        self._device.clone()
    }

    pub(crate) fn context(&self) -> windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext {
        self.context.clone()
    }

    pub(crate) fn capture_gpu_frame(&mut self, timeout: Duration) -> Result<WgcGpuFrame> {
        use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;

        let frame = self.poll_next_frame(timeout)?;
        let size = frame
            .ContentSize()
            .context("wgc: get content size failed")?;
        let width = size.Width.max(1) as u32;
        let height = size.Height.max(1) as u32;
        let surface = frame.Surface().context("wgc: frame surface failed")?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .context("wgc: cast IDirect3DDxgiInterfaceAccess failed")?;
        let texture = unsafe {
            access.GetInterface::<windows::Win32::Graphics::Direct3D11::ID3D11Texture2D>()
        }
        .context("wgc: GetInterface(ID3D11Texture2D) failed")?;
        let capture_start_us = frame
            .SystemRelativeTime()
            .ok()
            .and_then(|v| self.system_relative_to_unix_us(v.Duration))
            .unwrap_or_else(unix_time_us);
        Ok(WgcGpuFrame {
            texture,
            width,
            height,
            capture_start_us,
            _frame: frame,
        })
    }

    fn poll_next_frame(
        &mut self,
        timeout: Duration,
    ) -> Result<windows::Graphics::Capture::Direct3D11CaptureFrame> {
        let start = Instant::now();
        loop {
            if let Ok(frame) = self.frame_pool.TryGetNextFrame() {
                self.consecutive_timeouts = 0;
                return Ok(frame);
            }
            if !is_wgc_window_usable(self.active_hwnd) || start.elapsed() >= timeout {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        self.consecutive_timeouts = self.consecutive_timeouts.saturating_add(1);
        if self.disable_rebind
            || self.consecutive_timeouts < self.rebind_timeout_threshold
            || self.last_rebind_at.elapsed() < self.rebind_cooldown
        {
            return Err(anyhow!(
                "wgc: timed out waiting for next frame (hwnd={:?}, timeouts={})",
                self.active_hwnd.0,
                self.consecutive_timeouts
            ));
        }
        self.rebind_capture_session()?;
        self.last_rebind_at = Instant::now();

        let retry_start = Instant::now();
        loop {
            if let Ok(frame) = self.frame_pool.TryGetNextFrame() {
                self.consecutive_timeouts = 0;
                return Ok(frame);
            }
            if retry_start.elapsed() >= timeout {
                return Err(anyhow!(
                    "wgc: timed out waiting for next frame after rebind (hwnd={:?})",
                    self.active_hwnd.0
                ));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn rebind_capture_session(&mut self) -> Result<()> {
        if let Ok(next_hwnd) = select_wgc_target_hwnd(self.fixed_hwnd)
            && let Ok((frame_pool, session)) = create_wgc_capture_session(&self._device, next_hwnd)
        {
            self.frame_pool = frame_pool;
            self.session = session;
            self.active_hwnd = next_hwnd;
            self.consecutive_timeouts = 0;
            self.qpc_freq = qpc_frequency();
            self.qpc_base = qpc_counter_now();
            self.unix_base_us = unix_time_us();
            info!(hwnd = ?next_hwnd.0, "wgc capture session rebound");
            return Ok(());
        }

        // Keep current capture session alive when no better visible window is available.
        if is_wgc_window_existing(self.active_hwnd) {
            warn!(
                hwnd = ?self.active_hwnd.0,
                "wgc rebind fallback: keep current session"
            );
            return Ok(());
        }

        Err(anyhow!(
            "wgc: no window available for rebind (fixed={:?}, active={:?})",
            self.fixed_hwnd.map(|h| h.0),
            self.active_hwnd.0
        ))
    }

    fn ensure_staging(&mut self, width: u32, height: u32) -> Result<()> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CPU_ACCESS_READ, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };

        if self.staging.is_some() && self.staging_width == width && self.staging_height == height {
            return Ok(());
        }
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
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging = None;
        unsafe {
            self._device
                .CreateTexture2D(&desc, None, Some(&mut staging))
        }
        .context("wgc: create staging texture failed")?;
        self.staging = staging;
        self.staging_width = width;
        self.staging_height = height;
        Ok(())
    }

    fn system_relative_to_unix_us(&self, qpc_ticks: i64) -> Option<u64> {
        if self.qpc_freq <= 0 {
            return None;
        }
        let delta_ticks = qpc_ticks.checked_sub(self.qpc_base)?;
        let delta_us = (delta_ticks as i128)
            .checked_mul(1_000_000)?
            .checked_div(self.qpc_freq as i128)?;
        let ts = (self.unix_base_us as i128).checked_add(delta_us)?;
        if ts < 0 {
            return None;
        }
        Some(ts.min(u64::MAX as i128) as u64)
    }
}

#[cfg(windows)]
fn create_wgc_capture_session(
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    hwnd: windows::Win32::Foundation::HWND,
) -> Result<(
    windows::Graphics::Capture::Direct3D11CaptureFramePool,
    windows::Graphics::Capture::GraphicsCaptureSession,
)> {
    use windows::Foundation::TimeSpan;
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
    use windows::Win32::System::WinRT::RoGetActivationFactory;
    use windows::core::HSTRING;

    let dxgi_device: IDXGIDevice = device.cast().context("wgc: cast IDXGIDevice failed")?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .context("wgc: CreateDirect3D11DeviceFromDXGIDevice failed")?;
    let winrt_device: IDirect3DDevice = inspectable
        .cast()
        .context("wgc: cast IDirect3DDevice failed")?;

    let class = HSTRING::from("Windows.Graphics.Capture.GraphicsCaptureItem");
    let interop: IGraphicsCaptureItemInterop = unsafe { RoGetActivationFactory(&class) }
        .context("wgc: RoGetActivationFactory(IGraphicsCaptureItemInterop) failed")?;
    let item: GraphicsCaptureItem =
        unsafe { interop.CreateForWindow(hwnd) }.context("wgc: CreateForWindow failed")?;
    let size = item.Size().context("wgc: capture item size failed")?;

    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .context("wgc: create frame pool failed")?;
    let session = frame_pool
        .CreateCaptureSession(&item)
        .context("wgc: create capture session failed")?;
    let _ = session.SetIsCursorCaptureEnabled(true);
    let _ = session.SetIsBorderRequired(false);
    // Target 250Hz frame delivery when supported by current Windows build.
    match session.SetMinUpdateInterval(TimeSpan { Duration: 40_000 }) {
        Ok(()) => info!("wgc MinUpdateInterval set to 4ms"),
        Err(e) => warn!(error = %e, "wgc MinUpdateInterval unavailable on this system"),
    }
    session
        .StartCapture()
        .context("wgc: start capture failed")?;
    Ok((frame_pool, session))
}

#[cfg(windows)]
fn qpc_frequency() -> i64 {
    use windows::Win32::System::Performance::QueryPerformanceFrequency;
    let mut freq = 0_i64;
    if unsafe { QueryPerformanceFrequency(&mut freq) }.is_ok() && freq > 0 {
        freq
    } else {
        0
    }
}

#[cfg(windows)]
fn qpc_counter_now() -> i64 {
    use windows::Win32::System::Performance::QueryPerformanceCounter;
    let mut counter = 0_i64;
    let _ = unsafe { QueryPerformanceCounter(&mut counter) };
    counter
}

#[cfg(windows)]
fn select_wgc_target_hwnd(
    fixed_hwnd: Option<windows::Win32::Foundation::HWND>,
) -> Result<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    if let Some(hwnd) = fixed_hwnd
        && is_wgc_window_usable(hwnd)
    {
        return Ok(hwnd);
    }

    let foreground = unsafe { GetForegroundWindow() };
    if is_wgc_window_usable(foreground) {
        return Ok(foreground);
    }

    if let Some(hwnd) = fixed_hwnd
        && is_wgc_window_existing(hwnd)
    {
        return Ok(hwnd);
    }
    if is_wgc_window_existing(foreground) {
        return Ok(foreground);
    }

    Err(anyhow!(
        "wgc: no usable window (fixed={:?}, foreground={:?})",
        fixed_hwnd.map(|h| h.0),
        foreground.0
    ))
}

#[cfg(windows)]
fn is_wgc_window_usable(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow, IsWindowVisible};

    if hwnd.0.is_null() {
        return false;
    }
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }
        rect.right > rect.left && rect.bottom > rect.top
    }
}

#[cfg(windows)]
fn is_wgc_window_existing(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::IsWindow;

    if hwnd.0.is_null() {
        return false;
    }
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

#[cfg(windows)]
fn resolve_wgc_window_hwnd() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::HWND;
    let raw = std::env::var("AGENT_WGC_WINDOW_HWND").ok()?;
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let parsed = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        isize::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<isize>().ok()?
    };
    if parsed == 0 {
        None
    } else {
        Some(HWND(parsed as *mut core::ffi::c_void))
    }
}
