use anyhow::Result;
use mrd_ipc::CaptureSource;

const DEFAULT_CAPTURE_SOURCE_LIMIT: usize = 24;
const MAX_CAPTURE_SOURCE_LIMIT: usize = 48;
const PREVIEW_MAX_WIDTH: usize = 240;
const PREVIEW_MAX_HEIGHT: usize = 135;

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

#[cfg(not(windows))]
pub fn create_frame_capture(_source_id: &str) -> Result<()> {
    anyhow::bail!("remote desktop capture is currently only available on Windows")
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsCaptureSourceRef {
    Window(isize),
    Display { index: u32 },
    DisplayShared { index: u32 },
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

#[cfg(not(windows))]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    anyhow::bail!("remote window capture source enumeration is currently only available on Windows")
}

fn attach_capture_source_previews(sources: &mut [CaptureSource]) {
    for source in sources.iter_mut() {
        let Ok((preview_data_url, preview_width, preview_height)) =
            capture_source_preview_data_url(&source.id)
        else {
            continue;
        };
        source.preview_data_url = Some(preview_data_url);
        source.preview_width = Some(preview_width);
        source.preview_height = Some(preview_height);
    }
}

#[cfg(windows)]
fn capture_source_preview_data_url(source_id: &str) -> Result<(String, u32, u32)> {
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
        .capture_frame_with_timeout(std::time::Duration::from_millis(250))
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let _ = capture.stop();
    window_preview_data_url(&frame, PREVIEW_MAX_WIDTH, PREVIEW_MAX_HEIGHT)
}

#[cfg(not(windows))]
fn capture_source_preview_data_url(_source_id: &str) -> Result<(String, u32, u32)> {
    anyhow::bail!("remote window capture previews are currently only available on Windows")
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

#[cfg(windows)]
fn window_preview_data_url(
    frame: &mrd_pipeline_core::CapturedFrame,
    max_width: usize,
    max_height: usize,
) -> Result<(String, u32, u32)> {
    use base64::Engine;
    use image::{codecs::png::PngEncoder, ColorType, ImageEncoder};
    use mrd_pipeline_core::FramePixelFormat;

    if frame.width == 0 || frame.height == 0 || max_width == 0 || max_height == 0 {
        anyhow::bail!("window preview frame has invalid dimensions");
    }

    let bytes_per_pixel = match frame.pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
    };
    let source_stride = frame
        .width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| anyhow::anyhow!("window preview stride overflow"))?;
    let required_len = source_stride
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow::anyhow!("window preview byte size overflow"))?;
    if frame.data.len() < required_len {
        anyhow::bail!(
            "window preview frame is truncated: {} < {}",
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
}
