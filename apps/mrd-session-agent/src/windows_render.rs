//! Windows decoder and D3D11 presentation adapter.

use mrd_pipeline_core::{DecodedFrame, DecodedFrameData, PipelineError, VideoDecoder};
use mrd_render::RenderFrame;
use std::fmt;
use thiserror::Error;

/// A decoded frame cannot be represented faithfully by the D3D11 renderer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameConversionError {
    /// The decoder produced a CPU YUV format that requires an explicit color
    /// converter before it can be presented.
    #[error("decoded CPU YUV frame requires conversion before D3D11 presentation")]
    CpuYuvRequiresConversion,
    /// Frame dimensions must be non-zero and even for 4:2:0 sampling.
    #[error("I420 frame dimensions are invalid")]
    InvalidI420Dimensions,
    /// Plane pitches or backing storage cannot describe the declared frame.
    #[error("I420 frame planes are undersized or invalid")]
    InvalidI420Planes,
}

/// Convert limited-range BT.601 I420 into renderer-native BGRA.
pub fn i420_to_bgra(
    width: usize,
    height: usize,
    y_pitch: usize,
    uv_pitch: usize,
    data: &[u8],
) -> Result<Vec<u8>, FrameConversionError> {
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(FrameConversionError::InvalidI420Dimensions);
    }
    let chroma_width = width / 2;
    let chroma_height = height / 2;
    if y_pitch < width || uv_pitch < chroma_width {
        return Err(FrameConversionError::InvalidI420Planes);
    }
    let y_len = y_pitch
        .checked_mul(height)
        .ok_or(FrameConversionError::InvalidI420Planes)?;
    let chroma_len = uv_pitch
        .checked_mul(chroma_height)
        .ok_or(FrameConversionError::InvalidI420Planes)?;
    let u_offset = y_len;
    let v_offset = u_offset
        .checked_add(chroma_len)
        .ok_or(FrameConversionError::InvalidI420Planes)?;
    let required = v_offset
        .checked_add(chroma_len)
        .ok_or(FrameConversionError::InvalidI420Planes)?;
    if data.len() < required {
        return Err(FrameConversionError::InvalidI420Planes);
    }
    let output_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(FrameConversionError::InvalidI420Dimensions)?;
    let mut output = Vec::with_capacity(output_len);
    for row in 0..height {
        for column in 0..width {
            let y = i32::from(data[row * y_pitch + column]);
            let chroma_index = (row / 2) * uv_pitch + column / 2;
            let u = i32::from(data[u_offset + chroma_index]);
            let v = i32::from(data[v_offset + chroma_index]);
            let c = (y - 16).max(0);
            let d = u - 128;
            let e = v - 128;
            let red = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let green = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let blue = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
            output.extend_from_slice(&[blue, green, red, 255]);
        }
    }
    Ok(output)
}

/// Move renderer-native decoded storage into a D3D11 render frame without
/// relabeling or copying its pixels.
pub fn decoded_frame_to_render_frame(
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

/// Selected H.264 decoder and the backend that produced it.
pub struct SelectedDecoder {
    backend: &'static str,
    decoder: Box<dyn VideoDecoder>,
}

impl SelectedDecoder {
    /// Backend identifier selected for diagnostics.
    pub fn backend(&self) -> &'static str {
        self.backend
    }

    /// Consume the selection and return the initialized decoder.
    pub fn into_decoder(self) -> Box<dyn VideoDecoder> {
        self.decoder
    }
}

/// Create the production H.264 decoder, preferring shared-texture NVDEC and
/// falling back to the always-built software decoder.
pub fn create_hybrid_h264_decoder() -> Result<SelectedDecoder, PipelineError> {
    select_h264_decoder(mrd_decode::create_decoder)
}

impl fmt::Debug for SelectedDecoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedDecoder")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

pub(crate) fn select_h264_decoder<F>(mut create: F) -> Result<SelectedDecoder, PipelineError>
where
    F: FnMut(&str) -> Result<Box<dyn VideoDecoder>, PipelineError>,
{
    match create("nvdec_d3d11_shared") {
        Ok(decoder) => Ok(SelectedDecoder {
            backend: "nvdec_d3d11_shared",
            decoder,
        }),
        Err(hardware_error) => create("h264_software")
            .map(|decoder| SelectedDecoder {
                backend: "h264_software",
                decoder,
            })
            .map_err(|software_error| {
                PipelineError::Message(format!(
                    "H.264 decoder initialization failed: hardware={hardware_error}; software={software_error}"
                ))
            }),
    }
}

#[cfg(test)]
mod tests {
    use mrd_pipeline_core::{DecodedFrame, DecodedFrameData, PipelineError, VideoDecoder};
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

    struct EmptyDecoder;

    impl VideoDecoder for EmptyDecoder {
        fn push_access_unit(&mut self, _access_unit: &[u8]) -> Result<(), PipelineError> {
            Ok(())
        }

        fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
            Vec::new()
        }
    }

    #[test]
    fn hybrid_decoder_prefers_shared_nvdec_and_falls_back_to_software() {
        let mut attempts = Vec::new();
        let selected = super::select_h264_decoder(|id| {
            attempts.push(id.to_owned());
            Ok(Box::new(EmptyDecoder) as Box<dyn VideoDecoder>)
        })
        .expect("hardware decoder");
        assert_eq!(attempts, ["nvdec_d3d11_shared"]);
        assert_eq!(selected.backend(), "nvdec_d3d11_shared");

        let mut attempts = Vec::new();
        let selected = super::select_h264_decoder(|id| {
            attempts.push(id.to_owned());
            if id == "nvdec_d3d11_shared" {
                Err(PipelineError::Message("no NVIDIA device".into()))
            } else {
                Ok(Box::new(EmptyDecoder) as Box<dyn VideoDecoder>)
            }
        })
        .expect("software fallback");
        assert_eq!(attempts, ["nvdec_d3d11_shared", "h264_software"]);
        assert_eq!(selected.backend(), "h264_software");
    }

    #[test]
    fn hybrid_decoder_is_unavailable_when_both_backends_fail() {
        let mut attempts = Vec::new();
        let error = super::select_h264_decoder(|id| {
            attempts.push(id.to_owned());
            Err(PipelineError::Message(format!("{id} unavailable")))
        })
        .expect_err("both backends must fail");
        assert_eq!(attempts, ["nvdec_d3d11_shared", "h264_software"]);
        assert!(error.to_string().contains("h264_software unavailable"));
    }

    #[test]
    fn software_i420_conversion_validates_planes_and_outputs_bgra() {
        let black =
            super::i420_to_bgra(2, 2, 2, 1, &[16, 16, 16, 16, 128, 128]).expect("valid I420");
        assert_eq!(black, [0, 0, 0, 255].repeat(4));

        assert!(super::i420_to_bgra(0, 2, 2, 1, &[]).is_err());
        assert!(super::i420_to_bgra(3, 2, 3, 2, &[0; 10]).is_err());
        assert!(super::i420_to_bgra(2, 2, 1, 1, &[0; 6]).is_err());
        assert!(super::i420_to_bgra(2, 2, 2, 1, &[0; 5]).is_err());
    }
}
