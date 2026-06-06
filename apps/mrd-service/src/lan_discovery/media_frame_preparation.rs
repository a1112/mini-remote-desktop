use anyhow::Result;
use mrd_ipc::MediaProfile;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};

use super::nv12_to_rgb24;

pub(super) fn captured_frame_memory_path(frame: &CapturedFrame) -> &'static str {
    #[cfg(target_os = "macos")]
    {
        if frame.macos_cv_pixel_buffer().is_some() {
            return "macos_cv_pixel_buffer";
        }
    }

    #[cfg(windows)]
    {
        if frame.d3d11_shared_bgra().is_some() {
            return "d3d11_shared_bgra";
        }
    }

    "cpu"
}

pub(super) fn prepare_frame_for_h264(
    frame: CapturedFrame,
    profile: &MediaProfile,
) -> Result<CapturedFrame> {
    if frame.width < 2 || frame.height < 2 {
        anyhow::bail!(
            "captured frame is too small: {}x{}",
            frame.width,
            frame.height
        );
    }

    let (target_width, target_height) = h264_target_dimensions(frame.width, frame.height, profile);

    #[cfg(target_os = "macos")]
    if frame.macos_cv_pixel_buffer().is_some() {
        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }
        anyhow::bail!(
            "macOS CVPixelBuffer capture requires exact selected profile dimensions: source {}x{}, selected {}x{}",
            frame.width,
            frame.height,
            target_width,
            target_height
        );
    }

    #[cfg(windows)]
    if frame.d3d11_shared_bgra().is_some() {
        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }
        anyhow::bail!(
            "D3D11 shared capture requires exact selected profile dimensions: source {}x{}, selected {}x{}",
            frame.width,
            frame.height,
            target_width,
            target_height
        );
    }

    if frame.pixel_format == FramePixelFormat::Nv12 {
        let required_len = nv12_cpu_frame_len(frame.width, frame.height)
            .ok_or_else(|| anyhow::anyhow!("captured NV12 byte size overflow"))?;
        if frame.data.len() < required_len {
            anyhow::bail!(
                "captured NV12 frame is truncated: {} < {}",
                frame.data.len(),
                required_len
            );
        }

        if target_width == frame.width && target_height == frame.height {
            return Ok(frame);
        }

        let source_rgb = nv12_to_rgb24(&frame.data, frame.width, frame.width, frame.height)?;
        let mut rgb = Vec::with_capacity(target_width * target_height * 3);
        for y in 0..target_height {
            let source_y = y * frame.height / target_height;
            for x in 0..target_width {
                let source_x = x * frame.width / target_width;
                let offset = (source_y * frame.width + source_x) * 3;
                rgb.extend_from_slice(&source_rgb[offset..offset + 3]);
            }
        }

        return Ok(CapturedFrame::from_cpu(
            target_width,
            target_height,
            FramePixelFormat::Rgb24,
            frame.timestamp_us,
            rgb,
        ));
    }

    let bytes_per_pixel = frame_bytes_per_pixel(frame.pixel_format);
    let source_stride = frame
        .width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| anyhow::anyhow!("captured frame stride overflow"))?;
    let required_len = source_stride
        .checked_mul(frame.height)
        .ok_or_else(|| anyhow::anyhow!("captured frame byte size overflow"))?;
    if frame.data.len() < required_len {
        anyhow::bail!(
            "captured frame is truncated: {} < {}",
            frame.data.len(),
            required_len
        );
    }

    if target_width == frame.width && target_height == frame.height {
        return Ok(frame);
    }

    let mut rgb = Vec::with_capacity(target_width * target_height * 3);
    for y in 0..target_height {
        let source_y = y * frame.height / target_height;
        for x in 0..target_width {
            let source_x = x * frame.width / target_width;
            let (r, g, b) = read_captured_rgb(&frame, source_x, source_y, source_stride);
            rgb.extend_from_slice(&[r, g, b]);
        }
    }

    Ok(CapturedFrame::from_cpu(
        target_width,
        target_height,
        FramePixelFormat::Rgb24,
        frame.timestamp_us,
        rgb,
    ))
}

pub(super) fn h264_target_dimensions(
    width: usize,
    height: usize,
    profile: &MediaProfile,
) -> (usize, usize) {
    let max_width = profile.width.max(2) as f64;
    let max_height = profile.height.max(2) as f64;
    let scale = (max_width / width as f64)
        .min(max_height / height as f64)
        .min(1.0);
    let target_width = even_dimension(((width as f64 * scale).round() as usize).max(2));
    let target_height = even_dimension(((height as f64 * scale).round() as usize).max(2));
    (target_width.max(2), target_height.max(2))
}

#[cfg(any(windows, test))]
pub(super) fn window_h264_capture_dimensions(width: usize, height: usize) -> (usize, usize) {
    (even_dimension(width).max(2), even_dimension(height).max(2))
}

pub(super) fn even_dimension(value: usize) -> usize {
    value & !1
}

fn frame_bytes_per_pixel(pixel_format: FramePixelFormat) -> usize {
    match pixel_format {
        FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
        FramePixelFormat::Rgb24 => 3,
        FramePixelFormat::Nv12 => 1,
    }
}

fn nv12_cpu_frame_len(width: usize, height: usize) -> Option<usize> {
    width.checked_mul(height).and_then(|y_size| {
        width
            .checked_mul(height.div_ceil(2))
            .and_then(|uv_size| y_size.checked_add(uv_size))
    })
}

fn read_captured_rgb(
    frame: &CapturedFrame,
    x: usize,
    y: usize,
    source_stride: usize,
) -> (u8, u8, u8) {
    let bytes_per_pixel = frame_bytes_per_pixel(frame.pixel_format);
    let index = y * source_stride + x * bytes_per_pixel;
    match frame.pixel_format {
        FramePixelFormat::Bgra32 => (
            frame.data[index + 2],
            frame.data[index + 1],
            frame.data[index],
        ),
        FramePixelFormat::Rgba32 => (
            frame.data[index],
            frame.data[index + 1],
            frame.data[index + 2],
        ),
        FramePixelFormat::Rgb24 => (
            frame.data[index],
            frame.data[index + 1],
            frame.data[index + 2],
        ),
        FramePixelFormat::Nv12 => unreachable!("NV12 is handled before packed RGB scaling"),
    }
}
