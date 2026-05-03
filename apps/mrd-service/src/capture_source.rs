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

#[cfg(windows)]
fn list_capture_sources_impl() -> Result<Vec<CaptureSource>> {
    let targets = mrd_capture_winrt::enumerate_window_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(targets
        .into_iter()
        .map(|target| CaptureSource {
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
        })
        .collect())
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
    let hwnd = parse_windows_capture_source_hwnd(source_id)?;
    let mut capture = mrd_capture_winrt::WinrtCapture::from_window_handle(hwnd)
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
fn parse_windows_capture_source_hwnd(source_id: &str) -> Result<isize> {
    let value = source_id
        .trim()
        .rsplit(':')
        .next()
        .unwrap_or(source_id)
        .trim();
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
    #[test]
    fn source_lookup_rejects_empty_id() {
        let error = super::find_capture_source(" ").expect_err("empty id should fail");
        assert!(error.to_string().contains("empty"));
    }
}
