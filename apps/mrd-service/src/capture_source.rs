use anyhow::Result;
use mrd_ipc::CaptureSource;
use std::time::{Duration, Instant};

const DEFAULT_CAPTURE_SOURCE_LIMIT: usize = 24;
const MAX_CAPTURE_SOURCE_LIMIT: usize = 48;
#[cfg(any(windows, target_os = "macos"))]
const PREVIEW_MAX_WIDTH: usize = 240;
#[cfg(any(windows, target_os = "macos"))]
const PREVIEW_MAX_HEIGHT: usize = 135;
const PREVIEW_FRAME_TIMEOUT_MS: u64 = 90;
const PREVIEW_TOTAL_BUDGET_MS: u64 = 1_800;

pub fn list_capture_sources(
    include_previews: bool,
    limit: Option<u32>,
) -> Result<Vec<CaptureSource>> {
    let mut sources = list_capture_sources_impl()?;
    let limit = limit
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_CAPTURE_SOURCE_LIMIT)
        .clamp(1, MAX_CAPTURE_SOURCE_LIMIT);
    sources.truncate(limit);

    if include_previews {
        attach_capture_source_previews(&mut sources);
    }

    Ok(sources)
}

pub fn find_capture_source(source_id: &str) -> Result<CaptureSource> {
    let source_id = source_id.trim();
    if source_id.is_empty() {
        anyhow::bail!("capture source id is empty");
    }

    list_capture_sources(false, Some(MAX_CAPTURE_SOURCE_LIMIT as u32))?
        .into_iter()
        .find(|source| {
            source.id.eq_ignore_ascii_case(source_id)
                || source
                    .id
                    .rsplit(':')
                    .next()
                    .is_some_and(|handle| handle.eq_ignore_ascii_case(source_id))
        })
        .ok_or_else(|| anyhow::anyhow!("capture source not found: {source_id}"))
}

pub fn preferred_capture_source(sources: &[CaptureSource]) -> Option<CaptureSource> {
    sources
        .iter()
        .find(|source| source.source_kind == "display_shared")
        .or_else(|| {
            sources
                .iter()
                .find(|source| source.source_kind == "display")
        })
        .or_else(|| sources.iter().find(|source| source.source_kind == "window"))
        .or_else(|| sources.first())
        .cloned()
}

pub fn default_capture_source(include_previews: bool) -> Result<CaptureSource> {
    let sources = list_capture_sources(include_previews, Some(MAX_CAPTURE_SOURCE_LIMIT as u32))?;
    preferred_capture_source(&sources)
        .ok_or_else(|| anyhow::anyhow!("no capture sources are available"))
}

#[cfg(windows)]
pub fn create_frame_capture(source_id: &str) -> Result<mrd_capture_winrt::WinrtCapture> {
    let mut capture = match parse_windows_capture_source_ref(source_id)? {
        WindowsCaptureSourceRef::Window(hwnd) => {
            mrd_capture_winrt::WinrtCapture::from_window_handle(hwnd)
        }
        WindowsCaptureSourceRef::Display { index }
        | WindowsCaptureSourceRef::DisplayShared { index } => {
            mrd_capture_winrt::WinrtCapture::from_monitor_index(index)
        }
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    capture
        .start()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(capture)
}

#[cfg(target_os = "macos")]
pub fn create_frame_capture(source_id: &str) -> Result<mrd_capture_macos::MacosScreenCapture> {
    let capture = match parse_macos_capture_source_ref(source_id)? {
        MacosCaptureSourceRef::Display { display_id } => {
            mrd_capture_macos::MacosScreenCapture::new_display_id(display_id)
        }
        MacosCaptureSourceRef::Window { window_id } => {
            mrd_capture_macos::MacosScreenCapture::new_window(window_id)
        }
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(capture)
}

#[cfg(target_os = "linux")]
pub fn create_frame_capture(
    source_id: &str,
) -> Result<mrd_capture_pipewire::PipewireScreenCapture> {
    if tokio::runtime::Handle::try_current().is_ok() {
        anyhow::bail!(
            "Linux capture creation from an async runtime must use create_frame_capture_async"
        )
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| anyhow::anyhow!("create Linux capture runtime failed: {error}"))?;
    runtime.block_on(create_frame_capture_async(source_id))
}

#[cfg(target_os = "linux")]
pub async fn create_frame_capture_async(
    source_id: &str,
) -> Result<mrd_capture_pipewire::PipewireScreenCapture> {
    validate_linux_capture_source_ref(source_id)?;

    let mut capture = mrd_capture_pipewire::PipewireScreenCapture::new()
        .map_err(|error| anyhow::anyhow!("Linux capture init failed: {error}"))?;
    capture
        .start_session()
        .await
        .map_err(|error| anyhow::anyhow!("Linux capture session start failed: {error}"))?;
    Ok(capture)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub fn create_frame_capture(_source_id: &str) -> Result<()> {
    anyhow::bail!("remote desktop capture is currently only available on Windows, macOS, and Linux")
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsCaptureSourceRef {
    Window(isize),
    Display { index: u32 },
    DisplayShared { index: u32 },
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacosCaptureSourceRef {
    Display { display_id: u32 },
    Window { window_id: u32 },
}

#[cfg(windows)]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = list_windows_display_capture_sources();
    let targets = mrd_capture_winrt::enumerate_window_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    sources.extend(targets.into_iter().map(|target| CaptureSource {
        id: format!("windows:window:0x{:X}", target.hwnd as usize),
        platform: "windows".to_string(),
        source_kind: "window".to_string(),
        title: target.title,
        class_name: target.class_name,
        width: target.width,
        height: target.height,
        process_id: target.process_id,
        app_name: None,
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }));

    Ok(sources)
}

#[cfg(windows)]
fn list_windows_display_capture_sources() -> Vec<CaptureSource> {
    let Ok(count) = mrd_capture_winrt::get_monitor_count() else {
        return Vec::new();
    };

    let mut sources = Vec::with_capacity(count.saturating_mul(2));
    for index in 0..count {
        let Ok(capture) = mrd_capture_winrt::WinrtCapture::from_monitor_index(index as u32) else {
            continue;
        };
        let width = capture.width() as u32;
        let height = capture.height() as u32;
        let display_number = index + 1;
        sources.push(CaptureSource {
            id: format!("windows:display-shared:{index}"),
            platform: "windows".to_string(),
            source_kind: "display_shared".to_string(),
            title: format!("Display {display_number} (D3D11 shared copy)"),
            class_name: "WinRTMonitorShared".to_string(),
            width,
            height,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        });
        sources.push(CaptureSource {
            id: format!("windows:display:{index}"),
            platform: "windows".to_string(),
            source_kind: "display".to_string(),
            title: format!("Display {display_number} (full screen copy)"),
            class_name: "WinRTMonitor".to_string(),
            width,
            height,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        });
    }

    sources
}

#[cfg(target_os = "macos")]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = list_macos_display_capture_sources()?;
    match mrd_capture_macos::enumerate_window_capture_targets() {
        Ok(targets) => {
            sources.extend(targets.into_iter().map(|target| CaptureSource {
                id: format!("macos:window:0x{:X}", target.window_id),
                platform: "macos".to_string(),
                source_kind: "window".to_string(),
                title: target.title,
                class_name: "ScreenCaptureKitWindow".to_string(),
                width: target.width,
                height: target.height,
                process_id: target.process_id,
                app_name: non_empty_string(target.app_name),
                bundle_identifier: non_empty_string(target.bundle_identifier),
                preview_data_url: None,
                preview_width: None,
                preview_height: None,
            }));
        }
        Err(error) if sources.is_empty() => {
            return Err(anyhow::anyhow!(error.to_string()));
        }
        Err(_) => {}
    }

    Ok(sources)
}

#[cfg(target_os = "macos")]
fn list_macos_display_capture_sources() -> Result<Vec<CaptureSource>> {
    let targets = mrd_capture_macos::enumerate_display_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(targets
        .into_iter()
        .map(|target| CaptureSource {
            id: format!("macos:display:0x{:X}", target.display_id),
            platform: "macos".to_string(),
            source_kind: "display".to_string(),
            title: target.title,
            class_name: "ScreenCaptureKitDisplay".to_string(),
            width: target.width,
            height: target.height,
            process_id: 0,
            app_name: Some("Display".to_string()),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(target_os = "linux")]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let mut sources = Vec::new();
    let display_targets = mrd_capture_pipewire::PipewireScreenCapture::get_display_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    sources.extend(display_targets.into_iter().map(|target| CaptureSource {
        id: format!("linux:display:{}", target.id),
        platform: "linux".to_string(),
        source_kind: "display".to_string(),
        title: target.name,
        class_name: linux_capture_backend_label().to_string(),
        width: target.width,
        height: target.height,
        process_id: 0,
        app_name: Some("Display".to_string()),
        bundle_identifier: None,
        preview_data_url: None,
        preview_width: None,
        preview_height: None,
    }));

    if let Ok(window_targets) = mrd_capture_pipewire::PipewireScreenCapture::get_window_targets() {
        sources.extend(window_targets.into_iter().map(|target| CaptureSource {
            id: format!("linux:window:{}", target.id),
            platform: "linux".to_string(),
            source_kind: "window".to_string(),
            title: target.title,
            class_name: linux_capture_backend_label().to_string(),
            width: target.width,
            height: target.height,
            process_id: 0,
            app_name: Some(target.app_name),
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }));
    }

    Ok(sources)
}

#[cfg(target_os = "linux")]
fn linux_capture_backend_label() -> &'static str {
    if mrd_capture_pipewire::PipewireScreenCapture::is_wayland_available() {
        "PipeWirePortal"
    } else if mrd_capture_pipewire::PipewireScreenCapture::is_x11_available() {
        "X11ScreenCapture"
    } else {
        "LinuxFallbackCapture"
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    anyhow::bail!("remote window capture source enumeration is currently only available on Windows, macOS, and Linux")
}

fn attach_capture_source_previews(sources: &mut [CaptureSource]) {
    let started_at = Instant::now();
    for source in sources.iter_mut() {
        let Some(frame_timeout) = preview_frame_timeout_for_elapsed(started_at.elapsed()) else {
            break;
        };
        let Ok((preview_data_url, preview_width, preview_height)) =
            capture_source_preview_data_url(&source.id, frame_timeout)
        else {
            continue;
        };
        source.preview_data_url = Some(preview_data_url);
        source.preview_width = Some(preview_width);
        source.preview_height = Some(preview_height);
    }
}

fn preview_frame_timeout_for_elapsed(elapsed: Duration) -> Option<Duration> {
    let budget = Duration::from_millis(PREVIEW_TOTAL_BUDGET_MS);
    let remaining = budget.checked_sub(elapsed)?;
    if remaining.is_zero() {
        return None;
    }
    Some(remaining.min(Duration::from_millis(PREVIEW_FRAME_TIMEOUT_MS)))
}

#[cfg(windows)]
fn capture_source_preview_data_url(
    source_id: &str,
    frame_timeout: Duration,
) -> Result<(String, u32, u32)> {
    let mut capture = match parse_windows_capture_source_ref(source_id)? {
        WindowsCaptureSourceRef::Window(hwnd) => {
            mrd_capture_winrt::WinrtCapture::from_window_handle(hwnd)
        }
        WindowsCaptureSourceRef::Display { index }
        | WindowsCaptureSourceRef::DisplayShared { index } => {
            mrd_capture_winrt::WinrtCapture::from_monitor_index(index)
        }
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    capture
        .start()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let frame = capture
        .capture_frame_with_timeout(frame_timeout)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let _ = capture.stop();
    frame_preview_data_url(&frame, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
}

#[cfg(target_os = "macos")]
fn capture_source_preview_data_url(
    source_id: &str,
    frame_timeout: Duration,
) -> Result<(String, u32, u32)> {
    let mut capture = match parse_macos_capture_source_ref(source_id)? {
        MacosCaptureSourceRef::Display { display_id } => {
            mrd_capture_macos::MacosScreenCapture::new_display_id(display_id)
        }
        MacosCaptureSourceRef::Window { window_id } => {
            mrd_capture_macos::MacosScreenCapture::new_window(window_id)
        }
    }
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let frame = capture
        .capture_frame_with_timeout(frame_timeout)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    frame_preview_data_url(&frame, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
}

#[cfg(target_os = "linux")]
fn capture_source_preview_data_url(
    _source_id: &str,
    _frame_timeout: Duration,
) -> Result<(String, u32, u32)> {
    anyhow::bail!("Linux capture previews require the async capture path and are not wired yet")
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn capture_source_preview_data_url(
    _source_id: &str,
    _frame_timeout: Duration,
) -> Result<(String, u32, u32)> {
    anyhow::bail!(
        "remote window capture previews are currently only available on Windows, macOS, and Linux"
    )
}

#[cfg(windows)]
fn parse_windows_capture_source_ref(source_id: &str) -> Result<WindowsCaptureSourceRef> {
    let trimmed = source_id.trim();
    if let Some(value) = trimmed.strip_prefix("windows:display-shared:") {
        return Ok(WindowsCaptureSourceRef::DisplayShared {
            index: parse_windows_display_index(source_id, value)?,
        });
    }
    if let Some(value) = trimmed.strip_prefix("windows:display:") {
        return Ok(WindowsCaptureSourceRef::Display {
            index: parse_windows_display_index(source_id, value)?,
        });
    }
    if let Some(value) = trimmed.strip_prefix("windows:window:") {
        return Ok(WindowsCaptureSourceRef::Window(parse_windows_hwnd_value(
            source_id, value,
        )?));
    }

    Ok(WindowsCaptureSourceRef::Window(parse_windows_hwnd_value(
        source_id, trimmed,
    )?))
}

#[cfg(target_os = "macos")]
fn parse_macos_capture_source_ref(source_id: &str) -> Result<MacosCaptureSourceRef> {
    let trimmed = source_id.trim();
    if let Some(value) = trimmed.strip_prefix("macos:display:") {
        return Ok(MacosCaptureSourceRef::Display {
            display_id: parse_macos_u32_value(source_id, value)?,
        });
    }
    if let Some(value) = trimmed.strip_prefix("macos:window:") {
        return Ok(MacosCaptureSourceRef::Window {
            window_id: parse_macos_u32_value(source_id, value)?,
        });
    }

    Ok(MacosCaptureSourceRef::Window {
        window_id: parse_macos_u32_value(source_id, trimmed)?,
    })
}

#[cfg(target_os = "macos")]
fn parse_macos_u32_value(source_id: &str, value: &str) -> Result<u32> {
    let value = value.trim();
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse::<u32>()
    };
    parsed
        .map_err(|error| anyhow::anyhow!("invalid macOS capture source id '{source_id}': {error}"))
}

#[cfg(target_os = "linux")]
fn validate_linux_capture_source_ref(source_id: &str) -> Result<()> {
    let trimmed = source_id.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Linux capture source id is empty");
    }

    if let Some(value) = trimmed.strip_prefix("linux:display:") {
        parse_linux_u32_value(source_id, value)?;
        return Ok(());
    }

    if let Some(value) = trimmed.strip_prefix("linux:window:") {
        parse_linux_u32_value(source_id, value)?;
        return Ok(());
    }

    if trimmed == "linux" {
        return Ok(());
    }

    anyhow::bail!("invalid Linux capture source id '{source_id}'")
}

#[cfg(target_os = "linux")]
fn parse_linux_u32_value(source_id: &str, value: &str) -> Result<u32> {
    value
        .trim()
        .parse::<u32>()
        .map_err(|error| anyhow::anyhow!("invalid Linux capture source id '{source_id}': {error}"))
}

#[cfg(windows)]
fn parse_windows_display_index(source_id: &str, value: &str) -> Result<u32> {
    value.trim().parse::<u32>().map_err(|error| {
        anyhow::anyhow!("invalid Windows display source id '{source_id}': {error}")
    })
}

#[cfg(windows)]
fn parse_windows_hwnd_value(source_id: &str, value: &str) -> Result<isize> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    usize::from_str_radix(value, 16)
        .or_else(|_| value.parse::<usize>())
        .map(|value| value as isize)
        .map_err(|error| {
            anyhow::anyhow!("invalid Windows capture source id '{source_id}': {error}")
        })
}

#[cfg(any(windows, target_os = "macos"))]
fn frame_preview_data_url(
    frame: &mrd_pipeline_core::CapturedFrame,
    max_width: usize,
    max_height: usize,
) -> Result<(String, u32, u32)> {
    use base64::Engine;
    use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
    use mrd_pipeline_core::FramePixelFormat;

    if frame.width == 0 || frame.height == 0 || max_width == 0 || max_height == 0 {
        anyhow::bail!("capture preview frame has invalid dimensions");
    }

    let bytes_per_pixel = match frame.pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
    };
    let source_stride = frame
        .width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| anyhow::anyhow!("capture preview stride overflow"))?;
    let required_len = source_stride
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow::anyhow!("capture preview byte size overflow"))?;
    if frame.data.len() < required_len {
        anyhow::bail!(
            "capture preview frame is truncated: {} < {}",
            frame.data.len(),
            required_len
        );
    }

    let scale = (max_width as f64 / frame.width as f64)
        .min(max_height as f64 / frame.height as f64)
        .min(1.0);
    let preview_width = ((frame.width as f64 * scale).round() as usize)
        .max(1)
        .min(max_width);
    let preview_height = ((frame.height as f64 * scale).round() as usize)
        .max(1)
        .min(max_height);

    let mut rgba = Vec::with_capacity(preview_width * preview_height * 4);
    for y in 0..preview_height {
        let source_y = y * frame.height / preview_height;
        for x in 0..preview_width {
            let source_x = x * frame.width / preview_width;
            let source_index = source_y * source_stride + source_x * bytes_per_pixel;
            match frame.pixel_format {
                FramePixelFormat::Bgra32 => {
                    rgba.push(frame.data[source_index + 2]);
                    rgba.push(frame.data[source_index + 1]);
                    rgba.push(frame.data[source_index]);
                    rgba.push(frame.data[source_index + 3]);
                }
                FramePixelFormat::Rgba32 => {
                    rgba.extend_from_slice(&frame.data[source_index..source_index + 4]);
                }
                FramePixelFormat::Rgb24 => {
                    rgba.push(frame.data[source_index]);
                    rgba.push(frame.data[source_index + 1]);
                    rgba.push(frame.data[source_index + 2]);
                    rgba.push(255);
                }
            }
        }
    }

    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        &rgba,
        preview_width as u32,
        preview_height as u32,
        ColorType::Rgba8.into(),
    )?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);

    Ok((
        format!("data:image/png;base64,{encoded}"),
        preview_width as u32,
        preview_height as u32,
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use mrd_ipc::CaptureSource;

    fn source(id: &str, source_kind: &str) -> CaptureSource {
        CaptureSource {
            id: id.to_string(),
            platform: "windows".to_string(),
            source_kind: source_kind.to_string(),
            title: id.to_string(),
            class_name: "Test".to_string(),
            width: 1280,
            height: 720,
            process_id: 0,
            app_name: None,
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    #[test]
    fn source_lookup_rejects_empty_id() {
        let error = super::find_capture_source(" ").expect_err("empty id should fail");
        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn preferred_capture_source_picks_fullscreen_shared_before_window() {
        let sources = vec![
            source("windows:window:0x1234", "window"),
            source("windows:display:0", "display"),
            source("windows:display-shared:0", "display_shared"),
        ];

        let selected = super::preferred_capture_source(&sources).expect("selected source");

        assert_eq!(selected.id, "windows:display-shared:0");
    }

    #[test]
    fn preferred_capture_source_falls_back_to_display_before_window() {
        let sources = vec![
            source("windows:window:0x1234", "window"),
            source("windows:display:0", "display"),
        ];

        let selected = super::preferred_capture_source(&sources).expect("selected source");

        assert_eq!(selected.id, "windows:display:0");
    }

    #[test]
    fn preview_timeout_stays_inside_lan_request_budget() {
        assert_eq!(
            super::preview_frame_timeout_for_elapsed(Duration::from_millis(0)),
            Some(Duration::from_millis(90))
        );
        assert_eq!(
            super::preview_frame_timeout_for_elapsed(Duration::from_millis(1_750)),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            super::preview_frame_timeout_for_elapsed(Duration::from_millis(1_800)),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_capture_source_refs_parse_display_and_window_ids() {
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:display-shared:2").unwrap(),
            super::WindowsCaptureSourceRef::DisplayShared { index: 2 }
        );
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:display:1").unwrap(),
            super::WindowsCaptureSourceRef::Display { index: 1 }
        );
        assert_eq!(
            super::parse_windows_capture_source_ref("windows:window:0x1234").unwrap(),
            super::WindowsCaptureSourceRef::Window(0x1234)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_capture_source_refs_parse_display_and_window_ids() {
        assert_eq!(
            super::parse_macos_capture_source_ref("macos:display:0x1A2B").unwrap(),
            super::MacosCaptureSourceRef::Display { display_id: 0x1A2B }
        );
        assert_eq!(
            super::parse_macos_capture_source_ref("macos:window:0x1234").unwrap(),
            super::MacosCaptureSourceRef::Window { window_id: 0x1234 }
        );
        assert_eq!(
            super::parse_macos_capture_source_ref("5678").unwrap(),
            super::MacosCaptureSourceRef::Window { window_id: 5678 }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "queries live macOS WindowServer state"]
    fn macos_lists_display_capture_sources() {
        let sources = super::list_capture_sources(false, Some(8)).expect("list capture sources");

        assert!(
            sources
                .iter()
                .any(|source| source.platform == "macos" && source.source_kind == "display"),
            "expected at least one macOS display capture source: {sources:?}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_capture_source_refs_parse_display_and_window_ids() {
        super::validate_linux_capture_source_ref("linux").unwrap();
        super::validate_linux_capture_source_ref("linux:display:0").unwrap();
        super::validate_linux_capture_source_ref("linux:window:42").unwrap();

        assert!(super::validate_linux_capture_source_ref("linux:display:not-a-number").is_err());
        assert!(super::validate_linux_capture_source_ref("windows:display:0").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_lists_display_capture_sources_with_fallback() {
        let sources = super::list_capture_sources(false, Some(8)).expect("list capture sources");

        assert!(
            sources
                .iter()
                .any(|source| source.platform == "linux" && source.source_kind == "display"),
            "expected at least one Linux display capture source: {sources:?}"
        );
    }
}
