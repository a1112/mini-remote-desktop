use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FrameMemoryKind, PipelineError, VideoEncoder,
};
#[cfg(feature = "software-vvenc")]
use mrd_pipeline_core::{FramePixelFormat, VideoCodec};

pub struct VvencSoftwareEncoder {
    inner: imp::Inner,
}

impl VvencSoftwareEncoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_with_bitrate(width, height, fps, 12_000_000)
    }

    pub fn new_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            inner: imp::Inner::new(width, height, fps, bitrate)?,
        })
    }
}

impl VideoEncoder for VvencSoftwareEncoder {
    fn input_memory_kind(&self) -> FrameMemoryKind {
        FrameMemoryKind::Cpu
    }

    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        self.inner.encode(frame)
    }
}

pub fn vvenc_software_compiled() -> bool {
    imp::compiled()
}

pub fn probe_vvenc_software_encoder_available() -> Result<(), PipelineError> {
    imp::probe_available()
}

#[cfg(not(feature = "software-vvenc"))]
mod imp {
    use super::*;

    pub struct Inner;

    impl Inner {
        pub fn new(
            _width: usize,
            _height: usize,
            _fps: u32,
            _bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Err(not_compiled_error())
        }

        pub fn encode(
            &mut self,
            _frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            Err(not_compiled_error())
        }
    }

    pub fn compiled() -> bool {
        false
    }

    pub fn probe_available() -> Result<(), PipelineError> {
        Err(not_compiled_error())
    }

    fn not_compiled_error() -> PipelineError {
        PipelineError::message(
            "H.266/VVC software encode requires mrd-encode-vvenc feature software-vvenc and libvvenc >= 1.13.0",
        )
    }
}

#[cfg(feature = "software-vvenc")]
mod imp {
    use super::*;
    use std::{
        ffi::CStr,
        mem,
        os::raw::{c_char, c_int},
        ptr::NonNull,
    };
    use vvenc_sys::*;

    pub struct Inner {
        encoder: NonNull<vvencEncoder>,
        width: usize,
        height: usize,
        frame_index: u64,
        input: OwnedYuvBuffer,
        output: OwnedAccessUnit,
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe {
                vvenc_encoder_close(self.encoder.as_ptr());
            }
        }
    }

    impl Inner {
        pub fn new(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            validate_even_dimensions(width, height)?;

            let fps = fps.max(1);
            let width_c = usize_to_c_int("width", width)?;
            let height_c = usize_to_c_int("height", height)?;
            let fps_c = u32_to_c_int("fps", fps)?;
            let bitrate_bps = u32_to_c_int("bitrate bps", bitrate)?;

            let mut config = unsafe { mem::zeroed::<vvenc_config>() };
            vvenc_result(
                "VVenC default config",
                unsafe {
                    vvenc_init_default(
                        &mut config,
                        width_c,
                        height_c,
                        fps_c,
                        bitrate_bps,
                        VVENC_AUTO_QP,
                        vvencPresetMode_VVENC_FASTER,
                    )
                },
                None,
            )?;

            config.m_numThreads = vvenc_thread_count();
            config.m_RCNumPasses = 1;
            config.m_verbosity = vvencMsgLevel_VVENC_ERROR;

            let Some(encoder) = NonNull::new(unsafe { vvenc_encoder_create() }) else {
                return Err(PipelineError::message("VVenC encoder allocation failed"));
            };
            if let Err(error) = vvenc_result(
                "VVenC init",
                unsafe { vvenc_encoder_open(encoder.as_ptr(), &mut config) },
                Some(encoder),
            ) {
                unsafe {
                    vvenc_encoder_close(encoder.as_ptr());
                }
                return Err(error);
            }

            Ok(Self {
                encoder,
                width,
                height,
                frame_index: 0,
                input: OwnedYuvBuffer::new(width, height)?,
                output: OwnedAccessUnit::new(vvenc_output_len(width, height)?)?,
            })
        }

        pub fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "VVenC frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }
            validate_even_dimensions(frame.width, frame.height)?;

            write_yuv420_i16(frame, &mut self.input)?;
            self.input.raw.sequenceNumber = self.frame_index;
            self.input.raw.cts = timestamp_to_i64(frame.timestamp_us);
            self.input.raw.ctsValid = true;
            self.output.reset_for_encode();

            let mut encode_done = false;
            vvenc_result(
                "VVenC encode",
                unsafe {
                    vvenc_encode(
                        self.encoder.as_ptr(),
                        &mut self.input.raw,
                        &mut self.output.raw,
                        &mut encode_done,
                    )
                },
                Some(self.encoder),
            )?;
            self.frame_index += 1;

            let bytes = self.output.payload()?.to_vec();
            if bytes.is_empty() {
                return Ok(Vec::new());
            }

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Vvc,
                timestamp_us: self.output.timestamp_us(frame.timestamp_us),
                is_keyframe: self.output.raw.rap || annex_b_contains_vvc_keyframe(&bytes),
                bytes,
            }])
        }
    }

    pub fn compiled() -> bool {
        true
    }

    pub fn probe_available() -> Result<(), PipelineError> {
        Inner::new(176, 144, 30, 1_000_000).map(|_| ())
    }

    struct OwnedYuvBuffer {
        raw: vvencYUVBuffer,
    }

    impl OwnedYuvBuffer {
        fn new(width: usize, height: usize) -> Result<Self, PipelineError> {
            let mut raw = unsafe { mem::zeroed::<vvencYUVBuffer>() };
            unsafe {
                vvenc_YUVBuffer_alloc_buffer(
                    &mut raw,
                    vvencChromaFormat_VVENC_CHROMA_420,
                    usize_to_c_int("width", width)?,
                    usize_to_c_int("height", height)?,
                );
            }
            if raw.planes.iter().any(|plane| plane.ptr.is_null()) {
                unsafe {
                    vvenc_YUVBuffer_free_buffer(&mut raw);
                }
                return Err(PipelineError::message("VVenC YUV buffer allocation failed"));
            }
            Ok(Self { raw })
        }
    }

    impl Drop for OwnedYuvBuffer {
        fn drop(&mut self) {
            unsafe {
                vvenc_YUVBuffer_free_buffer(&mut self.raw);
            }
        }
    }

    struct OwnedAccessUnit {
        raw: vvencAccessUnit,
    }

    impl OwnedAccessUnit {
        fn new(payload_len: usize) -> Result<Self, PipelineError> {
            let payload_size = usize_to_c_int("VVenC output buffer", payload_len)?;
            let mut raw = unsafe { mem::zeroed::<vvencAccessUnit>() };
            unsafe {
                vvenc_accessUnit_default(&mut raw);
                vvenc_accessUnit_alloc_payload(&mut raw, payload_size);
            }
            if raw.payload.is_null() || raw.payloadSize < payload_size {
                unsafe {
                    vvenc_accessUnit_free_payload(&mut raw);
                }
                return Err(PipelineError::message(
                    "VVenC access unit payload allocation failed",
                ));
            }
            Ok(Self { raw })
        }

        fn reset_for_encode(&mut self) {
            self.raw.payloadUsedSize = 0;
            self.raw.rap = false;
        }

        fn payload(&self) -> Result<&[u8], PipelineError> {
            if self.raw.payloadUsedSize < 0 {
                return Err(PipelineError::message(format!(
                    "VVenC returned negative payload size {}",
                    self.raw.payloadUsedSize
                )));
            }
            let used = self.raw.payloadUsedSize as usize;
            let capacity = usize::try_from(self.raw.payloadSize).map_err(|_| {
                PipelineError::message(format!(
                    "VVenC returned invalid payload capacity {}",
                    self.raw.payloadSize
                ))
            })?;
            if used > capacity {
                return Err(PipelineError::message(format!(
                    "VVenC payload overflow: used {used}, capacity {capacity}"
                )));
            }
            if self.raw.payload.is_null() {
                return Err(PipelineError::message("VVenC payload pointer is null"));
            }
            Ok(unsafe { std::slice::from_raw_parts(self.raw.payload, used) })
        }

        fn timestamp_us(&self, fallback: u64) -> u64 {
            if self.raw.ctsValid && self.raw.cts >= 0 {
                self.raw.cts as u64
            } else {
                fallback
            }
        }
    }

    impl Drop for OwnedAccessUnit {
        fn drop(&mut self) {
            unsafe {
                vvenc_accessUnit_free_payload(&mut self.raw);
            }
        }
    }

    struct PlaneMut<'a> {
        data: &'a mut [i16],
        stride: usize,
    }

    fn validate_even_dimensions(width: usize, height: usize) -> Result<(), PipelineError> {
        if width == 0 || height == 0 {
            return Err(PipelineError::message(format!(
                "VVenC frame dimensions must be non-zero, got {width}x{height}"
            )));
        }
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(PipelineError::message(format!(
                "VVenC requires even frame dimensions, got {width}x{height}"
            )));
        }
        Ok(())
    }

    fn vvenc_thread_count() -> i32 {
        std::thread::available_parallelism()
            .map(|count| count.get().clamp(1, 8) as i32)
            .unwrap_or(1)
    }

    fn usize_to_c_int(label: &str, value: usize) -> Result<c_int, PipelineError> {
        c_int::try_from(value).map_err(|_| {
            PipelineError::message(format!("{label} does not fit in VVenC c_int: {value}"))
        })
    }

    fn u32_to_c_int(label: &str, value: u32) -> Result<c_int, PipelineError> {
        c_int::try_from(value).map_err(|_| {
            PipelineError::message(format!("{label} does not fit in VVenC c_int: {value}"))
        })
    }

    fn timestamp_to_i64(timestamp_us: u64) -> i64 {
        timestamp_us.min(i64::MAX as u64) as i64
    }

    fn vvenc_result(
        context: &str,
        code: c_int,
        encoder: Option<NonNull<vvencEncoder>>,
    ) -> Result<(), PipelineError> {
        if code == ErrorCodes_VVENC_OK {
            return Ok(());
        }

        let last_error =
            encoder.and_then(|encoder| unsafe { c_string(vvenc_get_last_error(encoder.as_ptr())) });
        let error_name = unsafe { c_string(vvenc_get_error_msg(code)) };
        let mut message = format!(
            "{context} failed: {} ({code})",
            error_name.unwrap_or_else(|| "unknown VVenC error".to_string())
        );
        if let Some(last_error) = last_error.filter(|error| !error.is_empty()) {
            message.push_str(": ");
            message.push_str(&last_error);
        }
        Err(PipelineError::message(message))
    }

    unsafe fn c_string(value: *const c_char) -> Option<String> {
        if value.is_null() {
            return None;
        }
        Some(CStr::from_ptr(value).to_string_lossy().into_owned())
    }

    fn vvenc_output_len(width: usize, height: usize) -> Result<usize, PipelineError> {
        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(6))
            .map(|bytes| bytes.max(1_048_576))
            .ok_or_else(|| PipelineError::message("VVenC output buffer size overflow"))
    }

    fn frame_input_len(frame: &CapturedFrame) -> Result<usize, PipelineError> {
        let pixels = frame
            .width
            .checked_mul(frame.height)
            .ok_or_else(|| PipelineError::message("frame pixel count overflow"))?;
        match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels
                .checked_mul(4)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
            FramePixelFormat::Rgb24 => pixels
                .checked_mul(3)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
            FramePixelFormat::Nv12 => pixels
                .checked_mul(3)
                .map(|bytes| bytes / 2)
                .ok_or_else(|| PipelineError::message("frame buffer size overflow")),
        }
    }

    fn write_yuv420_i16(
        frame: &CapturedFrame,
        yuv: &mut OwnedYuvBuffer,
    ) -> Result<(), PipelineError> {
        let expected_len = frame_input_len(frame)?;
        if frame.data.len() != expected_len {
            return Err(PipelineError::message(format!(
                "VVenC frame bytes mismatch: expected {expected_len}, got {}",
                frame.data.len()
            )));
        }

        let (y_plane, u_plane, v_plane) = planes_mut(yuv)?;
        let y_stride = y_plane.stride;
        let u_stride = u_plane.stride;
        let v_stride = v_plane.stride;
        let y_data = y_plane.data;
        let u_data = u_plane.data;
        let v_data = v_plane.data;

        if frame.pixel_format == FramePixelFormat::Nv12 {
            let y_size = frame.width * frame.height;
            for row in 0..frame.height {
                let src_row = row * frame.width;
                let dst_row = row * y_stride;
                for col in 0..frame.width {
                    y_data[dst_row + col] = i16::from(frame.data[src_row + col]);
                }
            }

            let uv_plane = &frame.data[y_size..expected_len];
            for row in 0..frame.height / 2 {
                for col in 0..frame.width / 2 {
                    let nv12_index = row * frame.width + col * 2;
                    u_data[row * u_stride + col] = i16::from(uv_plane[nv12_index]);
                    v_data[row * v_stride + col] = i16::from(uv_plane[nv12_index + 1]);
                }
            }
            return Ok(());
        }

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

                y_data[block_y * y_stride + block_x] = i16::from(rgb_to_y(p00));
                y_data[block_y * y_stride + block_x + 1] = i16::from(rgb_to_y(p10));
                y_data[(block_y + 1) * y_stride + block_x] = i16::from(rgb_to_y(p01));
                y_data[(block_y + 1) * y_stride + block_x + 1] = i16::from(rgb_to_y(p11));

                let avg = average_rgb([p00, p10, p01, p11]);
                let uv_x = block_x / 2;
                let uv_y = block_y / 2;
                u_data[uv_y * u_stride + uv_x] = i16::from(rgb_to_u(avg));
                v_data[uv_y * v_stride + uv_x] = i16::from(rgb_to_v(avg));
            }
        }

        Ok(())
    }

    fn planes_mut(
        yuv: &mut OwnedYuvBuffer,
    ) -> Result<(PlaneMut<'_>, PlaneMut<'_>, PlaneMut<'_>), PipelineError> {
        let y_plane = yuv.raw.planes[0];
        let u_plane = yuv.raw.planes[1];
        let v_plane = yuv.raw.planes[2];
        unsafe {
            Ok((
                plane_mut("Y", y_plane)?,
                plane_mut("U", u_plane)?,
                plane_mut("V", v_plane)?,
            ))
        }
    }

    unsafe fn plane_mut<'a>(
        label: &str,
        plane: vvencYUVPlane,
    ) -> Result<PlaneMut<'a>, PipelineError> {
        if plane.ptr.is_null() {
            return Err(PipelineError::message(format!(
                "VVenC {label} plane pointer is null"
            )));
        }
        if plane.height < 0 || plane.stride < 0 {
            return Err(PipelineError::message(format!(
                "VVenC {label} plane has invalid shape {}x{} stride {}",
                plane.width, plane.height, plane.stride
            )));
        }
        let height = plane.height as usize;
        let stride = plane.stride as usize;
        let len = height
            .checked_mul(stride)
            .ok_or_else(|| PipelineError::message("VVenC plane size overflow"))?;
        Ok(PlaneMut {
            data: std::slice::from_raw_parts_mut(plane.ptr, len),
            stride,
        })
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

    fn annex_b_contains_vvc_keyframe(access_unit: &[u8]) -> bool {
        let mut offset = 0usize;
        while offset + 5 < access_unit.len() {
            if access_unit[offset..].starts_with(&[0, 0, 0, 1]) {
                if vvc_nal_is_keyframe(&access_unit[offset + 4..]) {
                    return true;
                }
                offset += 4;
            } else if access_unit[offset..].starts_with(&[0, 0, 1]) {
                if vvc_nal_is_keyframe(&access_unit[offset + 3..]) {
                    return true;
                }
                offset += 3;
            } else {
                offset += 1;
            }
        }
        false
    }

    fn vvc_nal_is_keyframe(nal: &[u8]) -> bool {
        if nal.len() < 2 {
            return false;
        }
        matches!((nal[1] >> 3) & 0x1f, 7 | 8 | 9 | 10 | 14 | 15 | 16)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "software-vvenc")]
    use mrd_pipeline_core::VideoDecoder;

    #[test]
    #[cfg(not(feature = "software-vvenc"))]
    fn default_build_reports_vvenc_not_compiled() {
        let error = match VvencSoftwareEncoder::new(16, 16, 30) {
            Ok(_) => panic!("VVenC should be gated"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("software-vvenc"));
        assert!(!vvenc_software_compiled());
    }

    #[test]
    #[cfg(feature = "software-vvenc")]
    fn feature_build_initializes_and_encodes_with_vvenc() {
        assert!(vvenc_software_compiled());
        probe_vvenc_software_encoder_available().unwrap();

        let width = 176;
        let height = 144;
        let mut encoder = VvencSoftwareEncoder::new(width, height, 30).unwrap();
        let frame = CapturedFrame::from_cpu(
            width,
            height,
            FramePixelFormat::Nv12,
            12_345,
            vec![128; width * height * 3 / 2],
        );

        let access_units = encoder.encode(&frame).unwrap();
        assert!(access_units
            .iter()
            .all(|access_unit| access_unit.codec == VideoCodec::Vvc));
    }

    #[test]
    #[cfg(feature = "software-vvenc")]
    fn feature_build_encodes_720p_bgra_without_crashing() {
        let width = 1280;
        let height = 720;
        let mut encoder = VvencSoftwareEncoder::new_with_bitrate(width, height, 30, 8_000_000)
            .expect("create VVenC encoder");
        let mut data = vec![0_u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                data[idx] = x as u8;
                data[idx + 1] = y as u8;
                data[idx + 2] = 192;
                data[idx + 3] = 255;
            }
        }
        let frame = CapturedFrame::from_cpu(width, height, FramePixelFormat::Bgra32, 16_667, data);

        let access_units = encoder.encode(&frame).expect("encode 720p BGRA");
        assert!(access_units
            .iter()
            .all(|access_unit| access_unit.codec == VideoCodec::Vvc));
    }

    #[test]
    #[cfg(feature = "software-vvenc")]
    fn feature_build_encodes_repeated_720p_bgra_without_crashing() {
        let width = 1280;
        let height = 720;
        let mut encoder = VvencSoftwareEncoder::new_with_bitrate(width, height, 30, 8_000_000)
            .expect("create VVenC encoder");
        let mut total_access_units = 0;

        for frame_index in 0..90_u64 {
            let phase = (frame_index & 0xff) as u8;
            let mut data = vec![0_u8; width * height * 4];
            for y in 0..height {
                for x in 0..width {
                    let idx = (y * width + x) * 4;
                    data[idx] = (x as u8).wrapping_add(phase);
                    data[idx + 1] = (y as u8).wrapping_add(phase / 2);
                    data[idx + 2] = 192_u8.wrapping_sub(phase / 3);
                    data[idx + 3] = 255;
                }
            }
            let frame = CapturedFrame::from_cpu(
                width,
                height,
                FramePixelFormat::Bgra32,
                frame_index * 33_333,
                data,
            );

            let access_units = encoder.encode(&frame).expect("encode repeated 720p BGRA");
            total_access_units += access_units.len();
        }

        assert!(
            total_access_units > 0,
            "VVenC should emit at least one access unit after repeated frames"
        );
    }

    #[test]
    #[cfg(feature = "software-vvenc")]
    fn feature_build_decodes_repeated_vvenc_access_units_with_ffmpeg_vvc() {
        use mrd_decode::{FfmpegCliDecoder, FfmpegDecodeCodec};

        let width = 176;
        let height = 144;
        let mut encoder = VvencSoftwareEncoder::new_with_bitrate(width, height, 144, 2_000_000)
            .expect("create VVenC encoder");
        let Ok(mut decoder) = FfmpegCliDecoder::new(FfmpegDecodeCodec::Vvc) else {
            return;
        };
        let mut decoded_frames = 0;

        for frame_index in 0..180_u64 {
            let phase = (frame_index & 0xff) as u8;
            let frame = CapturedFrame::from_cpu(
                width,
                height,
                FramePixelFormat::Nv12,
                frame_index * 6_944,
                vec![phase; width * height * 3 / 2],
            );

            for access_unit in encoder.encode(&frame).expect("encode VVC frame") {
                decoder
                    .push_access_unit(&access_unit.bytes)
                    .expect("FFmpeg native VVC should accept VVenC access unit stream");
                decoded_frames += decoder.drain_decoded_frames().len();
            }
        }

        assert!(
            decoded_frames > 0,
            "VVdeC should decode at least one VVenC frame"
        );
    }
}
