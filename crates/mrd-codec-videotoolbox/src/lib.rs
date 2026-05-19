use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame as CoreDecodedFrame, EncodedAccessUnit, FramePixelFormat,
    PipelineError, VideoCodec, VideoDecoder, VideoEncoder,
};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use shiguredo_video_toolbox as vt;
    use std::{collections::VecDeque, ffi::c_void, num::NonZeroU32, ptr};

    const CV_SUCCESS: i32 = 0;
    const CV_PIXEL_FORMAT_NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVPixelBufferCreate(
            allocator: *const c_void,
            width: usize,
            height: usize,
            pixel_format_type: u32,
            pixel_buffer_attributes: *const c_void,
            pixel_buffer_out: *mut *mut c_void,
        ) -> i32;
        fn CVPixelBufferLockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(pixel_buffer: *mut c_void, lock_flags: u64) -> i32;
        fn CVPixelBufferGetBaseAddressOfPlane(
            pixel_buffer: *mut c_void,
            plane_index: usize,
        ) -> *mut c_void;
        fn CVPixelBufferGetBytesPerRowOfPlane(
            pixel_buffer: *mut c_void,
            plane_index: usize,
        ) -> usize;
        fn CVPixelBufferGetHeightOfPlane(pixel_buffer: *mut c_void, plane_index: usize) -> usize;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub struct VideoToolboxH264Encoder {
        encoder: vt::Encoder,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: u64,
        nv12: Vec<u8>,
        timestamps: VecDeque<u64>,
    }

    impl VideoToolboxH264Encoder {
        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_with_bitrate(width, height, fps, 12_000_000)
        }

        pub fn new_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            validate_even_dimensions(width, height, "videotoolbox")?;
            let fps = fps.max(1);
            let config = vt::EncoderConfig {
                width: width_to_u32(width)?,
                height: height_to_u32(height)?,
                codec: vt::CodecConfig::H264(vt::H264EncoderConfig {
                    profile: vt::H264Profile::Baseline,
                    entropy_mode: vt::H264EntropyMode::Cavlc,
                }),
                pixel_format: vt::PixelFormat::Nv12,
                average_bitrate: Some(u64::from(bitrate.max(1))),
                fps_numerator: fps,
                fps_denominator: 1,
                prioritize_encoding_speed_over_quality: true,
                real_time: true,
                maximize_power_efficiency: false,
                allow_frame_reordering: false,
                allow_temporal_compression: false,
                max_key_frame_interval: NonZeroU32::new(fps),
                max_key_frame_interval_duration: None,
                max_frame_delay_count: NonZeroU32::new(1),
            };
            let encoder = vt::Encoder::new(config).map_err(|error| {
                PipelineError::message(format!("create VideoToolbox H.264 encoder failed: {error}"))
            })?;

            Ok(Self {
                encoder,
                width,
                height,
                fps,
                frame_index: 0,
                nv12: vec![0; nv12_len(width, height)?],
                timestamps: VecDeque::new(),
            })
        }

        fn drain_encoded(&mut self) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            let mut units = Vec::new();
            while let Some(frame) = self.encoder.next_frame().map_err(|error| {
                PipelineError::message(format!("VideoToolbox encoder drain failed: {error}"))
            })? {
                let timestamp_us = self.timestamps.pop_front().unwrap_or_default();
                let bytes = encoded_h264_to_annex_b(&frame)?;
                units.push(EncodedAccessUnit {
                    codec: VideoCodec::H264,
                    timestamp_us,
                    is_keyframe: frame.keyframe,
                    bytes,
                });
            }
            Ok(units)
        }
    }

    impl VideoEncoder for VideoToolboxH264Encoder {
        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            validate_even_dimensions(frame.width, frame.height, "videotoolbox")?;
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "VideoToolbox frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }

            let y_size = self.width * self.height;
            let expected_nv12 = nv12_len(self.width, self.height)?;
            let (y_plane, uv_plane) = match frame.pixel_format {
                FramePixelFormat::Nv12 => {
                    if frame.data.len() != expected_nv12 {
                        return Err(PipelineError::message(format!(
                            "VideoToolbox NV12 frame bytes mismatch: expected {expected_nv12}, got {}",
                            frame.data.len()
                        )));
                    }
                    frame.data.split_at(y_size)
                }
                FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 | FramePixelFormat::Rgb24 => {
                    write_nv12(frame, &mut self.nv12)?;
                    self.nv12.split_at(y_size)
                }
            };
            let options = vt::EncodeOptions {
                force_key_frame: self.frame_index == 0
                    || self.frame_index % u64::from(self.fps) == 0,
            };

            self.encoder.encode_nv12_planes(
                self.width,
                self.height,
                y_plane,
                uv_plane,
                &options,
            )?;
            self.timestamps.push_back(frame.timestamp_us);
            self.frame_index = self.frame_index.wrapping_add(1);
            self.drain_encoded()
        }
    }

    pub struct VideoToolboxH264Decoder {
        decoder: Option<vt::Decoder>,
        sps: Vec<u8>,
        pps: Vec<u8>,
        decoded_frames: Vec<CoreDecodedFrame>,
    }

    impl VideoToolboxH264Decoder {
        pub fn new() -> Result<Self, PipelineError> {
            Ok(Self {
                decoder: None,
                sps: Vec::new(),
                pps: Vec::new(),
                decoded_frames: Vec::new(),
            })
        }

        fn ensure_decoder(&mut self, format_changed: bool) -> Result<(), PipelineError> {
            if self.sps.is_empty() || self.pps.is_empty() {
                return Ok(());
            }

            let sps = self.sps.clone();
            let pps = self.pps.clone();
            match self.decoder.as_mut() {
                Some(decoder) if format_changed => decoder
                    .update_format(vt::DecoderCodec::H264 {
                        sps: &sps,
                        pps: &pps,
                        nalu_len_bytes: 4,
                    })
                    .map_err(|error| {
                        PipelineError::message(format!(
                            "VideoToolbox H.264 decoder format update failed: {error}"
                        ))
                    }),
                Some(_) => Ok(()),
                None => {
                    let decoder = vt::Decoder::new(vt::DecoderConfig {
                        codec: vt::DecoderCodec::H264 {
                            sps: &sps,
                            pps: &pps,
                            nalu_len_bytes: 4,
                        },
                        pixel_format: vt::PixelFormat::Nv12,
                    })
                    .map_err(|error| {
                        PipelineError::message(format!(
                            "create VideoToolbox H.264 decoder failed: {error}"
                        ))
                    })?;
                    self.decoder = Some(decoder);
                    Ok(())
                }
            }
        }
    }

    impl Drop for VideoToolboxH264Decoder {
        fn drop(&mut self) {
            if let Some(decoder) = self.decoder.take() {
                // shiguredo_video_toolbox::Decoder currently invalidates the
                // VTDecompressionSession in Drop. On this macOS runtime that can
                // crash during harness shutdown after screen-capture callbacks
                // have been active. Keep process stability for matrix runs; the
                // session is reclaimed when the process exits.
                std::mem::forget(decoder);
            }
        }
    }

    impl VideoDecoder for VideoToolboxH264Decoder {
        fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
            let nals = access_unit_nals(access_unit)?;
            let mut avcc_payload = Vec::with_capacity(access_unit.len());
            let mut format_changed = false;

            for nal in nals {
                if nal.is_empty() {
                    continue;
                }
                match nal[0] & 0x1f {
                    7 => {
                        if self.sps != nal {
                            self.sps.clear();
                            self.sps.extend_from_slice(nal);
                            format_changed = true;
                        }
                    }
                    8 => {
                        if self.pps != nal {
                            self.pps.clear();
                            self.pps.extend_from_slice(nal);
                            format_changed = true;
                        }
                    }
                    9 => {}
                    _ => append_avcc_nal(&mut avcc_payload, nal)?,
                }
            }

            self.ensure_decoder(format_changed)?;
            if avcc_payload.is_empty() {
                return Ok(());
            }

            let Some(decoder) = self.decoder.as_mut() else {
                return Ok(());
            };
            if let Some(frame) = decoder.decode(&avcc_payload).map_err(|error| {
                PipelineError::message(format!("VideoToolbox H.264 decode failed: {error}"))
            })? {
                self.decoded_frames.push(vt_frame_to_core(frame)?);
            }
            Ok(())
        }

        fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
            std::mem::take(&mut self.decoded_frames)
        }
    }

    fn encoded_h264_to_annex_b(frame: &vt::EncodedFrame) -> Result<Vec<u8>, PipelineError> {
        let mut out = Vec::with_capacity(
            frame.data.len()
                + frame.sps_list.iter().map(|s| s.len() + 4).sum::<usize>()
                + frame.pps_list.iter().map(|p| p.len() + 4).sum::<usize>(),
        );
        for sps in &frame.sps_list {
            append_annex_b_nal(&mut out, sps);
        }
        for pps in &frame.pps_list {
            append_annex_b_nal(&mut out, pps);
        }
        avcc_payload_to_annex_b(&frame.data, &mut out)?;
        Ok(out)
    }

    trait VideoToolboxNv12EncodeExt {
        fn encode_nv12_planes(
            &mut self,
            width: usize,
            height: usize,
            y_plane: &[u8],
            uv_plane: &[u8],
            options: &vt::EncodeOptions,
        ) -> Result<(), PipelineError>;
    }

    impl VideoToolboxNv12EncodeExt for vt::Encoder {
        fn encode_nv12_planes(
            &mut self,
            width: usize,
            height: usize,
            y_plane: &[u8],
            uv_plane: &[u8],
            options: &vt::EncodeOptions,
        ) -> Result<(), PipelineError> {
            validate_nv12_planes(width, height, y_plane, uv_plane)?;
            let mut pixel_buffer = ptr::null_mut();
            let status = unsafe {
                CVPixelBufferCreate(
                    ptr::null(),
                    width,
                    height,
                    CV_PIXEL_FORMAT_NV12_VIDEO_RANGE,
                    ptr::null(),
                    &mut pixel_buffer,
                )
            };
            if status != CV_SUCCESS || pixel_buffer.is_null() {
                return Err(PipelineError::message(format!(
                    "CVPixelBufferCreate(NV12) failed: status={status}"
                )));
            }

            let encode_result = copy_and_encode_nv12_pixel_buffer(
                self,
                pixel_buffer,
                width,
                height,
                y_plane,
                uv_plane,
                options,
            );
            unsafe {
                CFRelease(pixel_buffer.cast_const());
            }
            encode_result
        }
    }

    fn validate_nv12_planes(
        width: usize,
        height: usize,
        y_plane: &[u8],
        uv_plane: &[u8],
    ) -> Result<(), PipelineError> {
        let y_size = width
            .checked_mul(height)
            .ok_or_else(|| PipelineError::message("NV12 luma plane size overflow"))?;
        let uv_size = width
            .checked_mul(height.div_ceil(2))
            .ok_or_else(|| PipelineError::message("NV12 chroma plane size overflow"))?;
        if y_plane.len() < y_size {
            return Err(PipelineError::message(format!(
                "NV12 luma plane too short: {} < {y_size}",
                y_plane.len()
            )));
        }
        if uv_plane.len() < uv_size {
            return Err(PipelineError::message(format!(
                "NV12 chroma plane too short: {} < {uv_size}",
                uv_plane.len()
            )));
        }
        Ok(())
    }

    fn copy_and_encode_nv12_pixel_buffer(
        encoder: &mut vt::Encoder,
        pixel_buffer: *mut c_void,
        width: usize,
        height: usize,
        y_plane: &[u8],
        uv_plane: &[u8],
        options: &vt::EncodeOptions,
    ) -> Result<(), PipelineError> {
        let status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 0) };
        if status != CV_SUCCESS {
            return Err(PipelineError::message(format!(
                "CVPixelBufferLockBaseAddress(NV12) failed: status={status}"
            )));
        }

        let copy_result = unsafe {
            copy_nv12_plane(pixel_buffer, 0, y_plane, width, height)
                .and_then(|_| copy_nv12_plane(pixel_buffer, 1, uv_plane, width, height.div_ceil(2)))
        };
        let unlock_status = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, 0) };
        if let Err(error) = copy_result {
            return Err(error);
        }
        if unlock_status != CV_SUCCESS {
            return Err(PipelineError::message(format!(
                "CVPixelBufferUnlockBaseAddress(NV12) failed: status={unlock_status}"
            )));
        }

        unsafe { encoder.encode_pixel_buffer(pixel_buffer, options) }.map_err(|error| {
            PipelineError::message(format!("VideoToolbox H.264 encode failed: {error}"))
        })
    }

    unsafe fn copy_nv12_plane(
        pixel_buffer: *mut c_void,
        plane_index: usize,
        src: &[u8],
        row_bytes: usize,
        rows: usize,
    ) -> Result<(), PipelineError> {
        let dst = unsafe { CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, plane_index) };
        if dst.is_null() {
            return Err(PipelineError::message(format!(
                "CVPixelBuffer NV12 plane {plane_index} base address is null"
            )));
        }
        let dst_stride = unsafe { CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, plane_index) };
        let dst_height = unsafe { CVPixelBufferGetHeightOfPlane(pixel_buffer, plane_index) };
        if dst_stride < row_bytes {
            return Err(PipelineError::message(format!(
                "CVPixelBuffer NV12 plane {plane_index} stride too small: {dst_stride} < {row_bytes}"
            )));
        }
        if dst_height < rows {
            return Err(PipelineError::message(format!(
                "CVPixelBuffer NV12 plane {plane_index} height too small: {dst_height} < {rows}"
            )));
        }
        let required = row_bytes
            .checked_mul(rows)
            .ok_or_else(|| PipelineError::message("NV12 plane copy size overflow"))?;
        if src.len() < required {
            return Err(PipelineError::message(format!(
                "NV12 plane {plane_index} source too short: {} < {required}",
                src.len()
            )));
        }

        let dst = dst.cast::<u8>();
        if dst_stride == row_bytes {
            unsafe {
                ptr::copy_nonoverlapping(src.as_ptr(), dst, required);
            }
        } else {
            for row in 0..rows {
                let src_offset = row * row_bytes;
                let dst_offset = row * dst_stride;
                unsafe {
                    ptr::copy_nonoverlapping(
                        src.as_ptr().add(src_offset),
                        dst.add(dst_offset),
                        row_bytes,
                    );
                }
            }
        }
        Ok(())
    }

    fn vt_frame_to_core(frame: vt::DecodedFrame<'_>) -> Result<CoreDecodedFrame, PipelineError> {
        match frame {
            vt::DecodedFrame::Nv12(frame) => {
                let width = frame.width();
                let height = frame.height();
                let pitch = width;
                let uv_height = height.div_ceil(2);
                let y_plane = frame.y_plane();
                let uv_plane = frame.uv_plane();
                let y_stride = frame.y_stride();
                let uv_stride = frame.uv_stride();

                if y_plane.len() < y_stride.saturating_mul(height)
                    || uv_plane.len() < uv_stride.saturating_mul(uv_height)
                {
                    return Err(PipelineError::message(
                        "VideoToolbox returned an invalid NV12 plane",
                    ));
                }

                let mut data = vec![0_u8; pitch * height + pitch * uv_height];
                for row in 0..height {
                    let src = row * y_stride;
                    let dst = row * pitch;
                    data[dst..dst + width].copy_from_slice(&y_plane[src..src + width]);
                }
                let uv_base = pitch * height;
                for row in 0..uv_height {
                    let src = row * uv_stride;
                    let dst = uv_base + row * pitch;
                    data[dst..dst + width].copy_from_slice(&uv_plane[src..src + width]);
                }

                Ok(CoreDecodedFrame::from_cpu_nv12(
                    width, height, 0, pitch, data,
                ))
            }
            vt::DecodedFrame::I420(frame) => {
                let rgb = i420_frame_to_rgb24(&frame)?;
                Ok(CoreDecodedFrame::from_cpu_rgb24(
                    frame.width(),
                    frame.height(),
                    0,
                    rgb,
                ))
            }
        }
    }

    fn i420_frame_to_rgb24(frame: &vt::I420Frame<'_>) -> Result<Vec<u8>, PipelineError> {
        let width = frame.width();
        let height = frame.height();
        let y_stride = frame.y_stride();
        let u_stride = frame.u_stride();
        let v_stride = frame.v_stride();
        let y_plane = frame.y_plane();
        let u_plane = frame.u_plane();
        let v_plane = frame.v_plane();
        let uv_height = height.div_ceil(2);

        if y_plane.len() < y_stride.saturating_mul(height)
            || u_plane.len() < u_stride.saturating_mul(uv_height)
            || v_plane.len() < v_stride.saturating_mul(uv_height)
        {
            return Err(PipelineError::message(
                "VideoToolbox returned an invalid I420 plane",
            ));
        }

        let mut rgb = vec![0_u8; width * height * 3];
        let mut out_idx = 0;
        for y in 0..height {
            for x in 0..width {
                let yy = i32::from(y_plane[y * y_stride + x]) - 16;
                let u = i32::from(u_plane[(y / 2) * u_stride + (x / 2)]) - 128;
                let v = i32::from(v_plane[(y / 2) * v_stride + (x / 2)]) - 128;
                rgb[out_idx] = (((298 * yy + 409 * v + 128) >> 8).clamp(0, 255)) as u8;
                rgb[out_idx + 1] =
                    (((298 * yy - 100 * u - 208 * v + 128) >> 8).clamp(0, 255)) as u8;
                rgb[out_idx + 2] = (((298 * yy + 516 * u + 128) >> 8).clamp(0, 255)) as u8;
                out_idx += 3;
            }
        }
        Ok(rgb)
    }

    fn validate_even_dimensions(
        width: usize,
        height: usize,
        backend: &str,
    ) -> Result<(), PipelineError> {
        if width == 0 || height == 0 {
            return Err(PipelineError::message(format!(
                "{backend} frame dimensions must be non-zero, got {width}x{height}"
            )));
        }
        if width % 2 != 0 || height % 2 != 0 {
            return Err(PipelineError::message(format!(
                "{backend} requires even frame dimensions, got {width}x{height}"
            )));
        }
        Ok(())
    }

    fn width_to_u32(width: usize) -> Result<u32, PipelineError> {
        u32::try_from(width)
            .map_err(|_| PipelineError::message(format!("VideoToolbox width too large: {width}")))
    }

    fn height_to_u32(height: usize) -> Result<u32, PipelineError> {
        u32::try_from(height)
            .map_err(|_| PipelineError::message(format!("VideoToolbox height too large: {height}")))
    }

    fn nv12_len(width: usize, height: usize) -> Result<usize, PipelineError> {
        width
            .checked_mul(height)
            .and_then(|y_size| {
                width
                    .checked_mul(height.div_ceil(2))
                    .and_then(|uv_size| y_size.checked_add(uv_size))
            })
            .ok_or_else(|| PipelineError::message("VideoToolbox NV12 buffer size overflow"))
    }

    fn write_nv12(frame: &CapturedFrame, out: &mut [u8]) -> Result<(), PipelineError> {
        let expected_len = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| match frame.pixel_format {
                FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels.checked_mul(4),
                FramePixelFormat::Rgb24 => pixels.checked_mul(3),
                FramePixelFormat::Nv12 => nv12_len(frame.width, frame.height).ok(),
            })
            .ok_or_else(|| PipelineError::message("frame buffer size overflow"))?;

        if frame.data.len() != expected_len {
            return Err(PipelineError::message(format!(
                "frame bytes mismatch: expected {expected_len}, got {}",
                frame.data.len()
            )));
        }

        let expected_nv12 = nv12_len(frame.width, frame.height)?;
        if out.len() != expected_nv12 {
            return Err(PipelineError::message(format!(
                "VideoToolbox NV12 scratch mismatch: expected {expected_nv12}, got {}",
                out.len()
            )));
        }

        if frame.pixel_format == FramePixelFormat::Nv12 {
            out.copy_from_slice(&frame.data);
            return Ok(());
        }

        let y_size = frame.width * frame.height;
        let (y_plane, uv_plane) = out.split_at_mut(y_size);
        let bytes_per_pixel = match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
            FramePixelFormat::Rgb24 => 3,
            FramePixelFormat::Nv12 => unreachable!("NV12 was copied above"),
        };

        for block_y in (0..frame.height).step_by(2) {
            for block_x in (0..frame.width).step_by(2) {
                let p00 = read_rgb(frame, block_x, block_y, bytes_per_pixel);
                let p10 = read_rgb(frame, block_x + 1, block_y, bytes_per_pixel);
                let p01 = read_rgb(frame, block_x, block_y + 1, bytes_per_pixel);
                let p11 = read_rgb(frame, block_x + 1, block_y + 1, bytes_per_pixel);

                y_plane[block_y * frame.width + block_x] = rgb_to_y(p00);
                y_plane[block_y * frame.width + block_x + 1] = rgb_to_y(p10);
                y_plane[(block_y + 1) * frame.width + block_x] = rgb_to_y(p01);
                y_plane[(block_y + 1) * frame.width + block_x + 1] = rgb_to_y(p11);

                let avg = average_rgb([p00, p10, p01, p11]);
                let uv_index = (block_y / 2) * frame.width + block_x;
                uv_plane[uv_index] = rgb_to_u(avg);
                uv_plane[uv_index + 1] = rgb_to_v(avg);
            }
        }

        Ok(())
    }

    fn read_rgb(frame: &CapturedFrame, x: usize, y: usize, bytes_per_pixel: usize) -> (u8, u8, u8) {
        let index = (y * frame.width + x) * bytes_per_pixel;
        match frame.pixel_format {
            FramePixelFormat::Bgra32 => (
                frame.data[index + 2],
                frame.data[index + 1],
                frame.data[index],
            ),
            FramePixelFormat::Rgba32 | FramePixelFormat::Rgb24 => (
                frame.data[index],
                frame.data[index + 1],
                frame.data[index + 2],
            ),
            FramePixelFormat::Nv12 => unreachable!("NV12 is not read as packed RGB"),
        }
    }

    fn average_rgb(pixels: [(u8, u8, u8); 4]) -> (u8, u8, u8) {
        let (mut r, mut g, mut b) = (0_u32, 0_u32, 0_u32);
        for (pr, pg, pb) in pixels {
            r += u32::from(pr);
            g += u32::from(pg);
            b += u32::from(pb);
        }
        ((r / 4) as u8, (g / 4) as u8, (b / 4) as u8)
    }

    fn rgb_to_y((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((66 * i32::from(r) + 129 * i32::from(g) + 25 * i32::from(b) + 128) >> 8) + 16)
    }

    fn rgb_to_u((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((-38 * i32::from(r) - 74 * i32::from(g) + 112 * i32::from(b) + 128) >> 8) + 128)
    }

    fn rgb_to_v((r, g, b): (u8, u8, u8)) -> u8 {
        clamp_u8(((112 * i32::from(r) - 94 * i32::from(g) - 18 * i32::from(b) + 128) >> 8) + 128)
    }

    fn clamp_u8(value: i32) -> u8 {
        value.clamp(0, 255) as u8
    }

    fn append_annex_b_nal(out: &mut Vec<u8>, nal: &[u8]) {
        if nal.is_empty() {
            return;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
    }

    fn append_avcc_nal(out: &mut Vec<u8>, nal: &[u8]) -> Result<(), PipelineError> {
        let len = u32::try_from(nal.len()).map_err(|_| {
            PipelineError::message(format!("H.264 NAL unit too large: {} bytes", nal.len()))
        })?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
        Ok(())
    }

    fn avcc_payload_to_annex_b(bytes: &[u8], out: &mut Vec<u8>) -> Result<(), PipelineError> {
        let mut offset = 0usize;
        while offset + 4 <= bytes.len() {
            let nal_len = u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;
            if nal_len == 0 || offset + nal_len > bytes.len() {
                return Err(PipelineError::message("invalid AVCC H.264 access unit"));
            }
            append_annex_b_nal(out, &bytes[offset..offset + nal_len]);
            offset += nal_len;
        }
        if offset != bytes.len() {
            return Err(PipelineError::message(
                "trailing bytes in AVCC H.264 access unit",
            ));
        }
        Ok(())
    }

    fn access_unit_nals(access_unit: &[u8]) -> Result<Vec<&[u8]>, PipelineError> {
        if looks_like_annex_b(access_unit) {
            Ok(split_annex_b_nals(access_unit))
        } else {
            split_avcc_nals(access_unit)
        }
    }

    fn looks_like_annex_b(bytes: &[u8]) -> bool {
        find_start_code(bytes, 0).is_some()
    }

    fn split_annex_b_nals(bytes: &[u8]) -> Vec<&[u8]> {
        let mut nals = Vec::new();
        let Some((mut code_offset, mut code_len)) = find_start_code(bytes, 0) else {
            return nals;
        };

        loop {
            let nal_start = code_offset + code_len;
            let Some((next_code_offset, next_code_len)) = find_start_code(bytes, nal_start) else {
                if nal_start < bytes.len() {
                    nals.push(trim_trailing_zeroes(&bytes[nal_start..]));
                }
                break;
            };

            if nal_start < next_code_offset {
                nals.push(trim_trailing_zeroes(&bytes[nal_start..next_code_offset]));
            }
            code_offset = next_code_offset;
            code_len = next_code_len;
        }
        nals
    }

    fn split_avcc_nals(bytes: &[u8]) -> Result<Vec<&[u8]>, PipelineError> {
        let mut nals = Vec::new();
        let mut offset = 0usize;
        while offset + 4 <= bytes.len() {
            let nal_len = u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;
            if nal_len == 0 || offset + nal_len > bytes.len() {
                return Err(PipelineError::message("invalid AVCC H.264 access unit"));
            }
            nals.push(&bytes[offset..offset + nal_len]);
            offset += nal_len;
        }
        if offset != bytes.len() {
            return Err(PipelineError::message(
                "trailing bytes in AVCC H.264 access unit",
            ));
        }
        Ok(nals)
    }

    fn find_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
        let mut index = from;
        while index + 3 <= bytes.len() {
            if index + 4 <= bytes.len() && bytes[index..index + 4] == [0, 0, 0, 1] {
                return Some((index, 4));
            }
            if bytes[index..index + 3] == [0, 0, 1] {
                return Some((index, 3));
            }
            index += 1;
        }
        None
    }

    fn trim_trailing_zeroes(bytes: &[u8]) -> &[u8] {
        let mut end = bytes.len();
        while end > 0 && bytes[end - 1] == 0 {
            end -= 1;
        }
        &bytes[..end]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bgra_to_nv12_writes_expected_limited_range_planes() {
            let frame = CapturedFrame::from_cpu(
                2,
                2,
                FramePixelFormat::Bgra32,
                0,
                [0, 0, 255, 255]
                    .into_iter()
                    .cycle()
                    .take(2 * 2 * 4)
                    .collect(),
            );
            let mut nv12 = vec![0; nv12_len(2, 2).expect("nv12 size")];

            write_nv12(&frame, &mut nv12).expect("convert bgra to nv12");

            assert_eq!(&nv12[..4], &[82, 82, 82, 82]);
            assert_eq!(&nv12[4..], &[90, 240]);
        }

        #[test]
        fn annex_b_access_unit_splits_parameter_sets_and_slices() {
            let access_unit = [
                0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 4, 0, 0, 0, 1, 0x65, 5, 6,
            ];

            let nals = access_unit_nals(&access_unit).expect("split annex-b");

            assert_eq!(
                nals,
                vec![&[0x67, 1, 2][..], &[0x68, 3, 4][..], &[0x65, 5, 6][..]]
            );
        }

        #[test]
        fn avcc_payload_converts_to_annex_b() {
            let avcc = [0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x61];
            let mut annex_b = Vec::new();

            avcc_payload_to_annex_b(&avcc, &mut annex_b).expect("convert avcc");

            assert_eq!(annex_b, vec![0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x61]);
        }

        #[test]
        fn videotoolbox_h264_synthetic_roundtrip() {
            let mut encoder = VideoToolboxH264Encoder::new_with_bitrate(64, 64, 30, 1_000_000)
                .expect("create videotoolbox encoder");
            let mut decoder = VideoToolboxH264Decoder::new().expect("create videotoolbox decoder");

            for index in 0..5_u64 {
                let frame = CapturedFrame::from_cpu(
                    64,
                    64,
                    FramePixelFormat::Bgra32,
                    index * 33_333,
                    synthetic_bgra(64, 64, index as u8),
                );
                let units = encoder.encode(&frame).expect("encode synthetic frame");
                for unit in units {
                    assert!(looks_like_annex_b(&unit.bytes));
                    decoder
                        .push_access_unit(&unit.bytes)
                        .expect("decode synthetic access unit");
                    if !decoder.drain_decoded_frames().is_empty() {
                        return;
                    }
                }
            }

            panic!("VideoToolbox roundtrip did not produce a decoded frame");
        }

        fn synthetic_bgra(width: usize, height: usize, tick: u8) -> Vec<u8> {
            let mut data = vec![0_u8; width * height * 4];
            for y in 0..height {
                for x in 0..width {
                    let index = (y * width + x) * 4;
                    data[index] = (x as u8).wrapping_add(tick);
                    data[index + 1] = (y as u8).wrapping_add(tick);
                    data[index + 2] = 192_u8.wrapping_sub(tick);
                    data[index + 3] = 255;
                }
            }
            data
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    pub struct VideoToolboxH264Encoder;

    impl VideoToolboxH264Encoder {
        pub fn new(_width: usize, _height: usize, _fps: u32) -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox H.264 encoder is only available on macOS",
            ))
        }

        pub fn new_with_bitrate(
            _width: usize,
            _height: usize,
            _fps: u32,
            _bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new(_width, _height, _fps)
        }
    }

    impl VideoEncoder for VideoToolboxH264Encoder {
        fn encode(
            &mut self,
            _frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox H.264 encoder is only available on macOS",
            ))
        }
    }

    pub struct VideoToolboxH264Decoder;

    impl VideoToolboxH264Decoder {
        pub fn new() -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox H.264 decoder is only available on macOS",
            ))
        }
    }

    impl VideoDecoder for VideoToolboxH264Decoder {
        fn push_access_unit(&mut self, _access_unit: &[u8]) -> Result<(), PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox H.264 decoder is only available on macOS",
            ))
        }

        fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
            Vec::new()
        }
    }
}

pub use imp::{VideoToolboxH264Decoder, VideoToolboxH264Encoder};
