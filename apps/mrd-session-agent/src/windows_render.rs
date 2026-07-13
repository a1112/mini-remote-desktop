//! Windows decoder and D3D11 presentation adapter.

use mrd_pipeline_core::{DecodedFrame, DecodedFrameData};
use mrd_render::RenderFrame;
use thiserror::Error;

/// A decoded frame cannot be represented faithfully by the D3D11 renderer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameConversionError {
    /// The decoder produced a CPU YUV format that requires an explicit color
    /// converter before it can be presented.
    #[error("decoded CPU YUV frame requires conversion before D3D11 presentation")]
    CpuYuvRequiresConversion,
}

/// Move renderer-native decoded storage into a D3D11 render frame without
/// relabeling or copying its pixels.
pub(crate) fn decoded_frame_to_render_frame(
    frame: DecodedFrame,
) -> Result<RenderFrame, FrameConversionError> {
    let width = frame.width;
    let height = frame.height;
    match frame.data {
        DecodedFrameData::CpuRgb24(data) => Ok(RenderFrame::from_rgb24(width, height, data)),
        DecodedFrameData::CpuBgra32(data) => Ok(RenderFrame::from_bgra32(width, height, data)),
        DecodedFrameData::D3D11SharedNv12 {
            shared_handle_y,
            shared_handle_uv,
            ..
        } => Ok(RenderFrame::from_d3d11_shared_nv12(
            width,
            height,
            shared_handle_y,
            shared_handle_uv,
        )),
        DecodedFrameData::D3D11SharedP010 {
            shared_handle_y,
            shared_handle_uv,
            ..
        } => Ok(RenderFrame::from_d3d11_shared_p010(
            width,
            height,
            shared_handle_y,
            shared_handle_uv,
        )),
        DecodedFrameData::CpuNv12 { .. }
        | DecodedFrameData::CpuI420 { .. }
        | DecodedFrameData::CpuP010 { .. } => Err(FrameConversionError::CpuYuvRequiresConversion),
    }
}

#[cfg(test)]
mod tests {
    use mrd_pipeline_core::{DecodedFrame, DecodedFrameData};
    use mrd_render::{RenderFrameData, RenderPixelFormat};

    #[test]
    fn decoded_frame_conversion_preserves_renderer_native_formats() {
        let rgb = DecodedFrame::from_cpu_rgb24(2, 1, 7, vec![1, 2, 3, 4, 5, 6]);
        let converted = super::decoded_frame_to_render_frame(rgb).expect("RGB24 conversion");
        assert_eq!(converted.pixel_format, RenderPixelFormat::Rgb24);
        assert_eq!(
            converted.data,
            RenderFrameData::Rgb24(vec![1, 2, 3, 4, 5, 6])
        );

        let bgra = DecodedFrame::from_cpu_bgra32(1, 1, 8, vec![1, 2, 3, 4]);
        let converted = super::decoded_frame_to_render_frame(bgra).expect("BGRA conversion");
        assert_eq!(converted.pixel_format, RenderPixelFormat::Bgra32);
        assert_eq!(converted.data, RenderFrameData::Bgra32(vec![1, 2, 3, 4]));

        let shared = DecodedFrame {
            width: 4,
            height: 2,
            timestamp_us: 9,
            data: DecodedFrameData::D3D11SharedNv12 {
                shared_handle_y: 11,
                shared_handle_uv: 12,
                width: 4,
                height: 2,
            },
        };
        let converted = super::decoded_frame_to_render_frame(shared).expect("shared NV12");
        assert_eq!(converted.pixel_format, RenderPixelFormat::D3D11SharedNv12);
        assert!(matches!(
            converted.data,
            RenderFrameData::D3D11SharedNv12 {
                shared_handle_y: 11,
                shared_handle_uv: 12,
                width: 4,
                height: 2,
            }
        ));

        let shared = DecodedFrame {
            width: 4,
            height: 2,
            timestamp_us: 10,
            data: DecodedFrameData::D3D11SharedP010 {
                shared_handle_y: 21,
                shared_handle_uv: 22,
                width: 4,
                height: 2,
            },
        };
        let converted = super::decoded_frame_to_render_frame(shared).expect("shared P010");
        assert_eq!(converted.pixel_format, RenderPixelFormat::D3D11SharedP010);
    }

    #[test]
    fn decoded_frame_conversion_rejects_unconverted_cpu_yuv() {
        let i420 = DecodedFrame::from_cpu_i420(2, 2, 1, 2, 1, vec![0; 6]);
        assert!(super::decoded_frame_to_render_frame(i420).is_err());

        let nv12 = DecodedFrame::from_cpu_nv12(2, 2, 1, 2, vec![0; 6]);
        assert!(super::decoded_frame_to_render_frame(nv12).is_err());
    }
}
