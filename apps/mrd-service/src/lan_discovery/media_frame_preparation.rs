use anyhow::Result;
use mrd_ipc::MediaProfile;
use mrd_pipeline_core::{CapturedFrame, FramePixelFormat};

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

pub(super) fn nv12_to_rgb24(
    data: &[u8],
    pitch: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    if pitch < width {
        anyhow::bail!("NV12 pitch is smaller than frame width");
    }
    let y_bytes = pitch
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("NV12 luma byte size overflow"))?;
    let uv_height = height.div_ceil(2);
    let uv_bytes = pitch
        .checked_mul(uv_height)
        .ok_or_else(|| anyhow::anyhow!("NV12 chroma byte size overflow"))?;
    let expected_len = y_bytes
        .checked_add(uv_bytes)
        .ok_or_else(|| anyhow::anyhow!("NV12 byte size overflow"))?;
    if data.len() < expected_len {
        anyhow::bail!("NV12 frame has invalid byte length");
    }

    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let y_row = y * pitch;
        let uv_row = y_bytes + (y / 2) * pitch;
        for x in 0..width {
            let luma = data[y_row + x] as i32;
            let uv_x = (x / 2) * 2;
            let u = data[uv_row + uv_x] as i32;
            let v = data[uv_row + uv_x + 1] as i32;
            let c = (luma - 16).max(0);
            let d = u - 128;
            let e = v - 128;
            rgb.push(clamp_yuv_to_u8((298 * c + 409 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c - 100 * d - 208 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c + 516 * d + 128) >> 8));
        }
    }
    Ok(rgb)
}

pub(super) fn i420_to_rgb24(
    data: &[u8],
    y_pitch: usize,
    uv_pitch: usize,
    width: usize,
    height: usize,
) -> Result<Vec<u8>> {
    if y_pitch < width {
        anyhow::bail!("I420 Y pitch is smaller than frame width");
    }
    let chroma_width = width.div_ceil(2);
    if uv_pitch < chroma_width {
        anyhow::bail!("I420 UV pitch is smaller than chroma width");
    }
    let chroma_height = height.div_ceil(2);
    let y_bytes = y_pitch
        .checked_mul(height)
        .ok_or_else(|| anyhow::anyhow!("I420 luma byte size overflow"))?;
    let uv_bytes = uv_pitch
        .checked_mul(chroma_height)
        .ok_or_else(|| anyhow::anyhow!("I420 chroma byte size overflow"))?;
    let expected_len = y_bytes
        .checked_add(uv_bytes)
        .and_then(|bytes| bytes.checked_add(uv_bytes))
        .ok_or_else(|| anyhow::anyhow!("I420 byte size overflow"))?;
    if data.len() < expected_len {
        anyhow::bail!("I420 frame has invalid byte length");
    }

    let u_base = y_bytes;
    let v_base = y_bytes + uv_bytes;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let y_row = y * y_pitch;
        let uv_row = (y / 2) * uv_pitch;
        for x in 0..width {
            let luma = data[y_row + x] as i32;
            let u = data[u_base + uv_row + x / 2] as i32;
            let v = data[v_base + uv_row + x / 2] as i32;
            let c = (luma - 16).max(0);
            let d = u - 128;
            let e = v - 128;
            rgb.push(clamp_yuv_to_u8((298 * c + 409 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c - 100 * d - 208 * e + 128) >> 8));
            rgb.push(clamp_yuv_to_u8((298 * c + 516 * d + 128) >> 8));
        }
    }
    Ok(rgb)
}

fn clamp_yuv_to_u8(value: i32) -> u8 {
    value.clamp(0, 255) as u8
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i420_to_rgb24_converts_decoder_planes_to_rgb_pixels() {
        let width = 2;
        let height = 2;
        let y_pitch = 2;
        let uv_pitch = 1;
        let data = vec![
            16, 235, 81, 145, // Y plane
            90,  // U plane
            240, // V plane
        ];

        let rgb = i420_to_rgb24(&data, y_pitch, uv_pitch, width, height).unwrap();

        assert_eq!(rgb.len(), width * height * 3);
        assert_eq!(&rgb[0..3], &[179, 0, 0]);
    }
}
