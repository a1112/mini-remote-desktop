#[cfg(target_os = "macos")]
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame as CoreDecodedFrame, EncodedAccessUnit, FrameMemoryKind,
    FramePixelFormat, PipelineError, VideoCodec, VideoDecoder, VideoEncoder,
};
#[cfg(not(target_os = "macos"))]
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame as CoreDecodedFrame, EncodedAccessUnit, PipelineError,
    VideoDecoder, VideoEncoder,
};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use shiguredo_video_toolbox as vt;
    use std::{collections::VecDeque, ffi::c_void, num::NonZeroU32, ptr, slice, sync::Mutex};

    const CV_SUCCESS: i32 = 0;
    const CV_PIXEL_FORMAT_NV12_VIDEO_RANGE: u32 = u32::from_be_bytes(*b"420v");
    const CF_NUMBER_SINT32_TYPE: i32 = 3;
    const CM_TIME_FLAGS_VALID: u32 = 1;
    const CM_VIDEO_CODEC_TYPE_AV1: u32 = u32::from_be_bytes(*b"av01");
    const DEFAULT_H264_STARTUP_KEYFRAME_BURST_MS: u64 = 2_000;
    const DEFAULT_H264_STARTUP_KEYFRAME_INTERVAL_MS: u64 = 50;
    const DEFAULT_HEVC_STARTUP_KEYFRAME_BURST_MS: u64 = 2_000;
    const DEFAULT_HEVC_HIGH_THROUGHPUT_STARTUP_KEYFRAME_BURST_MS: u64 = 0;
    const DEFAULT_HEVC_STARTUP_KEYFRAME_INTERVAL_MS: u64 = 50;
    const H264_STARTUP_KEYFRAME_BURST_MS_ENV: &str =
        "MRD_VIDEOTOOLBOX_H264_STARTUP_KEYFRAME_BURST_MS";
    const H264_STARTUP_KEYFRAME_INTERVAL_MS_ENV: &str =
        "MRD_VIDEOTOOLBOX_H264_STARTUP_KEYFRAME_INTERVAL_MS";
    const HEVC_STARTUP_KEYFRAME_BURST_MS_ENV: &str =
        "MRD_VIDEOTOOLBOX_HEVC_STARTUP_KEYFRAME_BURST_MS";
    const HEVC_STARTUP_KEYFRAME_INTERVAL_MS_ENV: &str =
        "MRD_VIDEOTOOLBOX_HEVC_STARTUP_KEYFRAME_INTERVAL_MS";
    const H264_REUSABLE_PIXEL_BUFFER_POOL_CAPACITY_ENV: &str =
        "MRD_VIDEOTOOLBOX_H264_PIXEL_BUFFER_POOL_CAPACITY";
    const DEFAULT_H264_REUSABLE_PIXEL_BUFFER_POOL_CAPACITY: usize = 0;
    const HEVC_RAW_DECODE_ASYNC_ENV: &str = "MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_ASYNC";
    const HEVC_RAW_DECODE_MAX_PENDING_INPUTS_ENV: &str =
        "MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_MAX_PENDING_INPUTS";
    const DEFAULT_HEVC_RAW_DECODE_ASYNC: bool = false;
    const DEFAULT_HEVC_RAW_DECODE_MAX_PENDING_INPUTS: usize = 512;

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
        fn CVPixelBufferGetWidth(pixel_buffer: *mut c_void) -> usize;
        fn CVPixelBufferGetHeight(pixel_buffer: *mut c_void) -> usize;
        fn CVPixelBufferGetPixelFormatType(pixel_buffer: *mut c_void) -> u32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFAllocatorNull: *const c_void;
        static kCFBooleanTrue: *const c_void;
        static kCFBooleanFalse: *const c_void;
        fn CFRetain(cf: *const c_void) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        fn CFNumberCreate(
            allocator: *const c_void,
            the_type: i32,
            value_ptr: *const c_void,
        ) -> *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *mut *const c_void,
            values: *mut *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
    }

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static kCVPixelBufferPixelFormatTypeKey: *const c_void;
    }

    #[repr(C, packed(4))]
    #[derive(Debug, Copy, Clone)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    type VTDecompressionOutputCallback = Option<
        unsafe extern "C" fn(
            decompression_output_ref_con: *mut c_void,
            source_frame_ref_con: *mut c_void,
            status: i32,
            info_flags: u32,
            image_buffer: *mut c_void,
            presentation_time_stamp: CMTime,
            presentation_duration: CMTime,
        ),
    >;

    type VTCompressionOutputCallback = Option<
        unsafe extern "C" fn(
            output_callback_ref_con: *mut c_void,
            source_frame_ref_con: *mut c_void,
            status: i32,
            info_flags: u32,
            sample_buffer: *mut c_void,
        ),
    >;

    #[repr(C, packed(4))]
    #[derive(Debug, Copy, Clone)]
    struct VTDecompressionOutputCallbackRecord {
        decompression_output_callback: VTDecompressionOutputCallback,
        decompression_output_ref_con: *mut c_void,
    }

    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMBlockBufferCreateWithMemoryBlock(
            structure_allocator: *const c_void,
            memory_block: *mut c_void,
            block_length: usize,
            block_allocator: *const c_void,
            custom_block_source: *const c_void,
            offset_to_data: usize,
            data_length: usize,
            flags: u32,
            block_buffer_out: *mut *mut c_void,
        ) -> i32;
        fn CMSampleBufferCreateReady(
            allocator: *const c_void,
            data_buffer: *mut c_void,
            format_description: *mut c_void,
            num_samples: isize,
            num_sample_timing_entries: isize,
            sample_timing_array: *const c_void,
            num_sample_size_entries: isize,
            sample_size_array: *const usize,
            sample_buffer_out: *mut *mut c_void,
        ) -> i32;
        fn CMSampleBufferGetDataBuffer(sample_buffer: *mut c_void) -> *mut c_void;
        fn CMBlockBufferGetDataLength(the_buffer: *mut c_void) -> usize;
        fn CMBlockBufferCopyDataBytes(
            the_buffer: *mut c_void,
            offset_to_data: usize,
            data_length: usize,
            destination: *mut c_void,
        ) -> i32;
        fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: *const c_void,
            parameter_set_count: usize,
            parameter_set_pointers: *const *const u8,
            parameter_set_sizes: *const usize,
            nal_unit_header_length: i32,
            format_description_out: *mut *mut c_void,
        ) -> i32;
        fn CMVideoFormatDescriptionCreateFromHEVCParameterSets(
            allocator: *const c_void,
            parameter_set_count: usize,
            parameter_set_pointers: *const *const u8,
            parameter_set_sizes: *const usize,
            nal_unit_header_length: i32,
            extensions: *const c_void,
            format_description_out: *mut *mut c_void,
        ) -> i32;
    }

    #[link(name = "VideoToolbox", kind = "framework")]
    unsafe extern "C" {
        static kVTCompressionPropertyKey_AverageBitRate: *const c_void;
        static kVTCompressionPropertyKey_ExpectedFrameRate: *const c_void;
        static kVTCompressionPropertyKey_RealTime: *const c_void;
        static kVTCompressionPropertyKey_AllowFrameReordering: *const c_void;
        static kVTCompressionPropertyKey_AllowTemporalCompression: *const c_void;
        static kVTCompressionPropertyKey_MaxKeyFrameInterval: *const c_void;
        static kVTCompressionPropertyKey_MaxFrameDelayCount: *const c_void;
        static kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality: *const c_void;
        static kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: *const c_void;
        static kVTEncodeFrameOptionKey_ForceKeyFrame: *const c_void;
        fn VTCompressionSessionCreate(
            allocator: *const c_void,
            width: i32,
            height: i32,
            codec_type: u32,
            encoder_specification: *const c_void,
            image_buffer_attributes: *const c_void,
            compressed_data_allocator: *const c_void,
            output_callback: VTCompressionOutputCallback,
            output_callback_ref_con: *mut c_void,
            compression_session_out: *mut *mut c_void,
        ) -> i32;
        fn VTCompressionSessionInvalidate(session: *mut c_void);
        fn VTCompressionSessionEncodeFrame(
            session: *mut c_void,
            image_buffer: *mut c_void,
            presentation_time_stamp: CMTime,
            duration: CMTime,
            frame_properties: *const c_void,
            source_frame_ref_con: *mut c_void,
            info_flags_out: *mut u32,
        ) -> i32;
        fn VTCompressionSessionCompleteFrames(
            session: *mut c_void,
            complete_until_presentation_time_stamp: CMTime,
        ) -> i32;
        fn VTSessionSetProperty(
            session: *mut c_void,
            property_key: *const c_void,
            property_value: *const c_void,
        ) -> i32;
        fn VTDecompressionSessionCreate(
            allocator: *const c_void,
            video_format_description: *mut c_void,
            video_decoder_specification: *const c_void,
            destination_image_buffer_attributes: *const c_void,
            output_callback: *const VTDecompressionOutputCallbackRecord,
            decompression_session_out: *mut *mut c_void,
        ) -> i32;
        fn VTDecompressionSessionInvalidate(session: *mut c_void);
        fn VTDecompressionSessionDecodeFrame(
            session: *mut c_void,
            sample_buffer: *mut c_void,
            decode_flags: u32,
            source_frame_ref_con: *mut c_void,
            info_flags_out: *mut u32,
        ) -> i32;
        fn VTDecompressionSessionWaitForAsynchronousFrames(session: *mut c_void) -> i32;
    }

    pub struct VideoToolboxH264Encoder {
        encoder: vt::Encoder,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: u64,
        force_next_keyframe: bool,
        startup_keyframe_burst_frames: u64,
        startup_keyframe_interval_frames: u64,
        nv12: Vec<u8>,
        timestamps: VecDeque<u64>,
        reusable_pixel_buffer_capacity: usize,
        reusable_pixel_buffers: Vec<VideoToolboxReusablePixelBuffer>,
        pending_pixel_buffers: VecDeque<VideoToolboxReusablePixelBuffer>,
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
                force_next_keyframe: false,
                startup_keyframe_burst_frames: h264_startup_keyframe_burst_frames(fps),
                startup_keyframe_interval_frames: h264_startup_keyframe_interval_frames(fps),
                nv12: vec![0; nv12_len(width, height)?],
                timestamps: VecDeque::new(),
                reusable_pixel_buffer_capacity: h264_reusable_pixel_buffer_pool_capacity(),
                reusable_pixel_buffers: Vec::new(),
                pending_pixel_buffers: VecDeque::new(),
            })
        }

        fn take_reusable_pixel_buffer(
            &mut self,
        ) -> Result<VideoToolboxReusablePixelBuffer, PipelineError> {
            self.reusable_pixel_buffers
                .pop()
                .map(Ok)
                .unwrap_or_else(|| {
                    VideoToolboxReusablePixelBuffer::new_nv12(self.width, self.height)
                })
        }

        fn recycle_pixel_buffer(&mut self, pixel_buffer: VideoToolboxReusablePixelBuffer) {
            if self.reusable_pixel_buffer_capacity == 0 {
                return;
            }
            if self.reusable_pixel_buffers.len() < self.reusable_pixel_buffer_capacity {
                self.reusable_pixel_buffers.push(pixel_buffer);
            }
        }

        fn drain_encoded(&mut self) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            let mut units = Vec::new();
            while let Some(frame) = self.encoder.next_frame().map_err(|error| {
                PipelineError::message(format!("VideoToolbox encoder drain failed: {error}"))
            })? {
                let timestamp_us = self.timestamps.pop_front().unwrap_or_default();
                if let Some(pixel_buffer) = self.pending_pixel_buffers.pop_front() {
                    self.recycle_pixel_buffer(pixel_buffer);
                }
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
        fn input_memory_kind(&self) -> FrameMemoryKind {
            FrameMemoryKind::MacosCvPixelBuffer
        }

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

            let options = vt::EncodeOptions {
                force_key_frame: h264_should_force_keyframe(
                    self.frame_index,
                    self.fps,
                    self.force_next_keyframe,
                    self.startup_keyframe_burst_frames,
                    self.startup_keyframe_interval_frames,
                ),
            };

            #[cfg(target_os = "macos")]
            if let Some(pixel_buffer) = frame.macos_cv_pixel_buffer() {
                if pixel_buffer.pixel_format != FramePixelFormat::Nv12 {
                    return Err(PipelineError::message(format!(
                        "VideoToolbox direct CVPixelBuffer encode requires NV12 input, got {:?}",
                        pixel_buffer.pixel_format
                    )));
                }
                unsafe {
                    self.encoder
                        .encode_pixel_buffer(pixel_buffer.as_ptr(), &options)
                        .map_err(|error| {
                            PipelineError::message(format!(
                                "VideoToolbox direct CVPixelBuffer encode failed: {error}"
                            ))
                        })?;
                }
                self.timestamps.push_back(frame.timestamp_us);
                self.force_next_keyframe = false;
                self.frame_index = self.frame_index.wrapping_add(1);
                return self.drain_encoded();
            }

            let y_size = self.width * self.height;
            let expected_nv12 = nv12_len(self.width, self.height)?;
            let pixel_buffer = self.take_reusable_pixel_buffer()?;
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

            match self.encoder.encode_nv12_planes_from_pixel_buffer(
                pixel_buffer.as_ptr(),
                self.width,
                self.height,
                y_plane,
                uv_plane,
                &options,
            ) {
                Ok(()) => {
                    if self.reusable_pixel_buffer_capacity > 0 {
                        self.pending_pixel_buffers.push_back(pixel_buffer);
                    }
                }
                Err(error) => {
                    self.recycle_pixel_buffer(pixel_buffer);
                    return Err(error);
                }
            }
            self.timestamps.push_back(frame.timestamp_us);
            self.force_next_keyframe = false;
            self.frame_index = self.frame_index.wrapping_add(1);
            self.drain_encoded()
        }

        fn request_keyframe(&mut self) {
            self.force_next_keyframe = true;
        }
    }

    pub struct VideoToolboxHevcEncoder {
        encoder: vt::Encoder,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: u64,
        force_next_keyframe: bool,
        startup_keyframe_burst_frames: u64,
        startup_keyframe_interval_frames: u64,
        nv12: Vec<u8>,
        timestamps: VecDeque<u64>,
        reusable_pixel_buffer_capacity: usize,
        reusable_pixel_buffers: Vec<VideoToolboxReusablePixelBuffer>,
        pending_pixel_buffers: VecDeque<VideoToolboxReusablePixelBuffer>,
    }

    impl VideoToolboxHevcEncoder {
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
                codec: vt::CodecConfig::Hevc(vt::HevcEncoderConfig {
                    profile: vt::HevcProfile::Main,
                    allow_open_gop: false,
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
                PipelineError::message(format!("create VideoToolbox HEVC encoder failed: {error}"))
            })?;

            Ok(Self {
                encoder,
                width,
                height,
                fps,
                frame_index: 0,
                force_next_keyframe: false,
                startup_keyframe_burst_frames: hevc_startup_keyframe_burst_frames(
                    width, height, fps,
                ),
                startup_keyframe_interval_frames: hevc_startup_keyframe_interval_frames(fps),
                nv12: vec![0; nv12_len(width, height)?],
                timestamps: VecDeque::new(),
                reusable_pixel_buffer_capacity: h264_reusable_pixel_buffer_pool_capacity(),
                reusable_pixel_buffers: Vec::new(),
                pending_pixel_buffers: VecDeque::new(),
            })
        }

        fn take_reusable_pixel_buffer(
            &mut self,
        ) -> Result<VideoToolboxReusablePixelBuffer, PipelineError> {
            self.reusable_pixel_buffers
                .pop()
                .map(Ok)
                .unwrap_or_else(|| {
                    VideoToolboxReusablePixelBuffer::new_nv12(self.width, self.height)
                })
        }

        fn recycle_pixel_buffer(&mut self, pixel_buffer: VideoToolboxReusablePixelBuffer) {
            if self.reusable_pixel_buffer_capacity == 0 {
                return;
            }
            if self.reusable_pixel_buffers.len() < self.reusable_pixel_buffer_capacity {
                self.reusable_pixel_buffers.push(pixel_buffer);
            }
        }

        fn drain_encoded(&mut self) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            let mut units = Vec::new();
            while let Some(frame) = self.encoder.next_frame().map_err(|error| {
                PipelineError::message(format!("VideoToolbox HEVC encoder drain failed: {error}"))
            })? {
                let timestamp_us = self.timestamps.pop_front().unwrap_or_default();
                if let Some(pixel_buffer) = self.pending_pixel_buffers.pop_front() {
                    self.recycle_pixel_buffer(pixel_buffer);
                }
                let bytes = encoded_hevc_to_annex_b(&frame)?;
                units.push(EncodedAccessUnit {
                    codec: VideoCodec::Hevc,
                    timestamp_us,
                    is_keyframe: frame.keyframe,
                    bytes,
                });
            }
            Ok(units)
        }
    }

    impl VideoEncoder for VideoToolboxHevcEncoder {
        fn input_memory_kind(&self) -> FrameMemoryKind {
            FrameMemoryKind::MacosCvPixelBuffer
        }

        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            validate_even_dimensions(frame.width, frame.height, "videotoolbox")?;
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "VideoToolbox HEVC frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }

            let options = vt::EncodeOptions {
                force_key_frame: h264_should_force_keyframe(
                    self.frame_index,
                    self.fps,
                    self.force_next_keyframe,
                    self.startup_keyframe_burst_frames,
                    self.startup_keyframe_interval_frames,
                ),
            };

            if let Some(pixel_buffer) = frame.macos_cv_pixel_buffer() {
                if pixel_buffer.pixel_format != FramePixelFormat::Nv12 {
                    return Err(PipelineError::message(format!(
                        "VideoToolbox direct CVPixelBuffer HEVC encode requires NV12 input, got {:?}",
                        pixel_buffer.pixel_format
                    )));
                }
                unsafe {
                    self.encoder
                        .encode_pixel_buffer(pixel_buffer.as_ptr(), &options)
                        .map_err(|error| {
                            PipelineError::message(format!(
                                "VideoToolbox direct CVPixelBuffer HEVC encode failed: {error}"
                            ))
                        })?;
                }
                self.timestamps.push_back(frame.timestamp_us);
                self.force_next_keyframe = false;
                self.frame_index = self.frame_index.wrapping_add(1);
                return self.drain_encoded();
            }

            let y_size = self.width * self.height;
            let expected_nv12 = nv12_len(self.width, self.height)?;
            let pixel_buffer = self.take_reusable_pixel_buffer()?;
            let (y_plane, uv_plane) = match frame.pixel_format {
                FramePixelFormat::Nv12 => {
                    if frame.data.len() != expected_nv12 {
                        return Err(PipelineError::message(format!(
                            "VideoToolbox HEVC NV12 frame bytes mismatch: expected {expected_nv12}, got {}",
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

            match self.encoder.encode_nv12_planes_from_pixel_buffer(
                pixel_buffer.as_ptr(),
                self.width,
                self.height,
                y_plane,
                uv_plane,
                &options,
            ) {
                Ok(()) => {
                    if self.reusable_pixel_buffer_capacity > 0 {
                        self.pending_pixel_buffers.push_back(pixel_buffer);
                    }
                }
                Err(error) => {
                    self.recycle_pixel_buffer(pixel_buffer);
                    return Err(error);
                }
            }
            self.timestamps.push_back(frame.timestamp_us);
            self.force_next_keyframe = false;
            self.frame_index = self.frame_index.wrapping_add(1);
            self.drain_encoded()
        }

        fn request_keyframe(&mut self) {
            self.force_next_keyframe = true;
        }
    }

    pub struct VideoToolboxAv1Encoder {
        session: *mut c_void,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: u64,
        force_next_keyframe: bool,
        nv12: Vec<u8>,
        output_queue: Box<Mutex<VecDeque<VideoToolboxAv1EncodedFrame>>>,
    }

    unsafe impl Send for VideoToolboxAv1Encoder {}

    struct VideoToolboxAv1EncodedFrame {
        timestamp_us: u64,
        is_keyframe: bool,
        bytes: Vec<u8>,
    }

    struct VideoToolboxAv1FrameRef {
        timestamp_us: u64,
        is_keyframe: bool,
        pixel_buffer: *mut c_void,
    }

    impl Drop for VideoToolboxAv1FrameRef {
        fn drop(&mut self) {
            if !self.pixel_buffer.is_null() {
                unsafe {
                    CFRelease(self.pixel_buffer.cast_const());
                }
                self.pixel_buffer = ptr::null_mut();
            }
        }
    }

    impl VideoToolboxAv1Encoder {
        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_with_bitrate(width, height, fps, 8_000_000)
        }

        pub fn new_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            validate_even_dimensions(width, height, "videotoolbox AV1")?;
            let fps = fps.max(1);
            let width_i32 = i32::try_from(width).map_err(|_| {
                PipelineError::message(format!("VideoToolbox AV1 width too large: {width}"))
            })?;
            let height_i32 = i32::try_from(height).map_err(|_| {
                PipelineError::message(format!("VideoToolbox AV1 height too large: {height}"))
            })?;
            let output_queue = Box::new(Mutex::new(VecDeque::new()));
            let mut session = ptr::null_mut();
            let encoder_specification = cf_dictionary(&[(
                unsafe { kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder },
                unsafe { kCFBooleanTrue },
            )])?;
            let status = unsafe {
                VTCompressionSessionCreate(
                    ptr::null(),
                    width_i32,
                    height_i32,
                    CM_VIDEO_CODEC_TYPE_AV1,
                    encoder_specification.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    Some(av1_compression_output_callback),
                    (&*output_queue as *const Mutex<VecDeque<VideoToolboxAv1EncodedFrame>>)
                        .cast_mut()
                        .cast(),
                    &mut session,
                )
            };
            check_os_status(status, "VTCompressionSessionCreate(AV1)")?;
            if session.is_null() {
                return Err(PipelineError::message(
                    "VTCompressionSessionCreate(AV1) returned null",
                ));
            }

            let configure_result = configure_av1_compression_session(session, fps, bitrate);
            if let Err(error) = configure_result {
                unsafe {
                    VTCompressionSessionInvalidate(session);
                    CFRelease(session.cast_const());
                }
                return Err(error);
            }

            Ok(Self {
                session,
                width,
                height,
                fps,
                frame_index: 0,
                force_next_keyframe: false,
                nv12: vec![0; nv12_len(width, height)?],
                output_queue,
            })
        }

        fn drain_encoded(&mut self) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            let mut queue = self.output_queue.lock().map_err(|_| {
                PipelineError::message("VideoToolbox AV1 output queue lock poisoned")
            })?;
            let mut units = Vec::with_capacity(queue.len());
            while let Some(frame) = queue.pop_front() {
                units.push(EncodedAccessUnit {
                    codec: VideoCodec::Av1,
                    timestamp_us: frame.timestamp_us,
                    is_keyframe: frame.is_keyframe,
                    bytes: frame.bytes,
                });
            }
            Ok(units)
        }

        fn encode_pixel_buffer(
            &mut self,
            pixel_buffer: *mut c_void,
            timestamp_us: u64,
            force_keyframe: bool,
            retain_pixel_buffer: bool,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if retain_pixel_buffer {
                unsafe {
                    CFRetain(pixel_buffer.cast_const());
                }
            }
            let frame_ref = Box::new(VideoToolboxAv1FrameRef {
                timestamp_us,
                is_keyframe: force_keyframe,
                pixel_buffer,
            });
            let frame_ref_ptr = Box::into_raw(frame_ref).cast::<c_void>();
            let frame_properties = if force_keyframe {
                Some(cf_dictionary(&[(
                    unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame },
                    unsafe { kCFBooleanTrue },
                )])?)
            } else {
                None
            };
            let frame_properties_ptr = frame_properties
                .as_ref()
                .map(|properties| properties.as_ptr().cast_const())
                .unwrap_or_else(ptr::null);
            let status = unsafe {
                VTCompressionSessionEncodeFrame(
                    self.session,
                    pixel_buffer,
                    cm_time(self.frame_index as i64, self.fps),
                    invalid_cm_time(),
                    frame_properties_ptr,
                    frame_ref_ptr,
                    ptr::null_mut(),
                )
            };
            if status != CV_SUCCESS {
                unsafe {
                    drop(Box::from_raw(
                        frame_ref_ptr.cast::<VideoToolboxAv1FrameRef>(),
                    ));
                }
                return Err(PipelineError::message(format!(
                    "VTCompressionSessionEncodeFrame(AV1) failed: status={status}"
                )));
            }
            self.force_next_keyframe = false;
            self.frame_index = self.frame_index.wrapping_add(1);
            self.drain_encoded()
        }
    }

    impl Drop for VideoToolboxAv1Encoder {
        fn drop(&mut self) {
            if self.session.is_null() {
                return;
            }
            unsafe {
                let _ = VTCompressionSessionCompleteFrames(self.session, invalid_cm_time());
                VTCompressionSessionInvalidate(self.session);
                CFRelease(self.session.cast_const());
            }
            self.session = ptr::null_mut();
        }
    }

    impl VideoEncoder for VideoToolboxAv1Encoder {
        fn input_memory_kind(&self) -> FrameMemoryKind {
            FrameMemoryKind::MacosCvPixelBuffer
        }

        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            validate_even_dimensions(frame.width, frame.height, "videotoolbox AV1")?;
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "VideoToolbox AV1 frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }

            let force_keyframe = self.force_next_keyframe
                || self.frame_index == 0
                || self
                    .frame_index
                    .is_multiple_of(u64::from(self.fps.max(1)).saturating_mul(2));

            if let Some(pixel_buffer) = frame.macos_cv_pixel_buffer() {
                if pixel_buffer.pixel_format != FramePixelFormat::Nv12 {
                    return Err(PipelineError::message(format!(
                        "VideoToolbox direct CVPixelBuffer AV1 encode requires NV12 input, got {:?}",
                        pixel_buffer.pixel_format
                    )));
                }
                if unsafe { CVPixelBufferGetPixelFormatType(pixel_buffer.as_ptr()) }
                    != CV_PIXEL_FORMAT_NV12_VIDEO_RANGE
                {
                    return Err(PipelineError::message(
                        "VideoToolbox direct CVPixelBuffer AV1 encode requires 420v NV12 input",
                    ));
                }
                return self.encode_pixel_buffer(
                    pixel_buffer.as_ptr(),
                    frame.timestamp_us,
                    force_keyframe,
                    true,
                );
            }

            let y_size = self.width * self.height;
            let expected_nv12 = nv12_len(self.width, self.height)?;
            let pixel_buffer = VideoToolboxReusablePixelBuffer::new_nv12(self.width, self.height)?;
            let (y_plane, uv_plane) = match frame.pixel_format {
                FramePixelFormat::Nv12 => {
                    if frame.data.len() != expected_nv12 {
                        return Err(PipelineError::message(format!(
                            "VideoToolbox AV1 NV12 frame bytes mismatch: expected {expected_nv12}, got {}",
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
            copy_nv12_planes_into_pixel_buffer(
                pixel_buffer.as_ptr(),
                self.width,
                self.height,
                y_plane,
                uv_plane,
                "VideoToolbox AV1",
            )?;
            let pixel_buffer_ptr = pixel_buffer.as_ptr();
            std::mem::forget(pixel_buffer);
            self.encode_pixel_buffer(pixel_buffer_ptr, frame.timestamp_us, force_keyframe, false)
        }

        fn request_keyframe(&mut self) {
            self.force_next_keyframe = true;
        }
    }

    fn h264_should_force_keyframe(
        frame_index: u64,
        fps: u32,
        force_next: bool,
        startup_keyframe_burst_frames: u64,
        startup_keyframe_interval_frames: u64,
    ) -> bool {
        should_force_keyframe(
            frame_index,
            force_next,
            u64::from(fps.max(1)),
            startup_keyframe_burst_frames,
            startup_keyframe_interval_frames,
        )
    }

    fn should_force_keyframe(
        frame_index: u64,
        force_next: bool,
        periodic_keyframe_interval_frames: u64,
        startup_keyframe_burst_frames: u64,
        startup_keyframe_interval_frames: u64,
    ) -> bool {
        if force_next {
            return true;
        }

        if frame_index == 0 {
            return true;
        }

        if startup_keyframe_burst_frames > 0
            && startup_keyframe_interval_frames > 0
            && frame_index < startup_keyframe_burst_frames
            && frame_index.is_multiple_of(startup_keyframe_interval_frames)
        {
            return true;
        }

        periodic_keyframe_interval_frames > 0
            && frame_index.is_multiple_of(periodic_keyframe_interval_frames)
    }

    fn h264_startup_keyframe_burst_frames(fps: u32) -> u64 {
        h264_frames_for_millis(
            fps,
            parse_env_u64(
                H264_STARTUP_KEYFRAME_BURST_MS_ENV,
                DEFAULT_H264_STARTUP_KEYFRAME_BURST_MS,
            ),
        )
    }

    fn h264_startup_keyframe_interval_frames(fps: u32) -> u64 {
        h264_frames_for_millis(
            fps,
            parse_env_u64(
                H264_STARTUP_KEYFRAME_INTERVAL_MS_ENV,
                DEFAULT_H264_STARTUP_KEYFRAME_INTERVAL_MS,
            ),
        )
    }

    fn hevc_startup_keyframe_burst_frames(width: usize, height: usize, fps: u32) -> u64 {
        let default = if hevc_high_throughput_profile(width, height, fps) {
            DEFAULT_HEVC_HIGH_THROUGHPUT_STARTUP_KEYFRAME_BURST_MS
        } else {
            DEFAULT_HEVC_STARTUP_KEYFRAME_BURST_MS
        };
        h264_frames_for_millis(
            fps,
            parse_env_u64(HEVC_STARTUP_KEYFRAME_BURST_MS_ENV, default),
        )
    }

    fn hevc_startup_keyframe_interval_frames(fps: u32) -> u64 {
        h264_frames_for_millis(
            fps,
            parse_env_u64(
                HEVC_STARTUP_KEYFRAME_INTERVAL_MS_ENV,
                DEFAULT_HEVC_STARTUP_KEYFRAME_INTERVAL_MS,
            ),
        )
    }

    fn h264_frames_for_millis(fps: u32, millis: u64) -> u64 {
        if fps == 0 || millis == 0 {
            return 0;
        }
        u64::from(fps)
            .saturating_mul(millis)
            .checked_div(1_000)
            .unwrap_or(0)
            .max(1)
    }

    fn hevc_high_throughput_profile(width: usize, height: usize, fps: u32) -> bool {
        let pixels = width.saturating_mul(height);
        fps >= 120 && pixels >= 2_560 * 1_440
    }

    fn parse_env_u64(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(default)
    }

    fn h264_reusable_pixel_buffer_pool_capacity() -> usize {
        parse_env_usize(
            H264_REUSABLE_PIXEL_BUFFER_POOL_CAPACITY_ENV,
            DEFAULT_H264_REUSABLE_PIXEL_BUFFER_POOL_CAPACITY,
        )
        .min(64)
    }

    fn parse_env_usize(name: &str, default: usize) -> usize {
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default)
    }

    fn parse_env_bool(name: &str, default: bool) -> bool {
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| {
                !matches!(
                    value.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(default)
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

    pub struct VideoToolboxH264PixelBufferDecoder {
        session: Option<VideoToolboxRawSession>,
        sps: Vec<u8>,
        pps: Vec<u8>,
        decoded_frames: Vec<VideoToolboxPixelBufferFrame>,
    }

    unsafe impl Send for VideoToolboxH264PixelBufferDecoder {}

    impl VideoToolboxH264PixelBufferDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Ok(Self {
                session: None,
                sps: Vec::new(),
                pps: Vec::new(),
                decoded_frames: Vec::new(),
            })
        }

        pub fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
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

            self.ensure_session(format_changed)?;
            if avcc_payload.is_empty() {
                return Ok(());
            }

            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            for pixel_buffer in session.decode(&avcc_payload)? {
                let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) };
                let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) };
                self.decoded_frames
                    .push(VideoToolboxPixelBufferFrame::from_retained(
                        pixel_buffer,
                        width,
                        height,
                    )?);
            }
            Ok(())
        }

        pub fn drain_decoded_frames(&mut self) -> Vec<VideoToolboxPixelBufferFrame> {
            std::mem::take(&mut self.decoded_frames)
        }

        fn ensure_session(&mut self, format_changed: bool) -> Result<(), PipelineError> {
            if self.sps.is_empty() || self.pps.is_empty() {
                return Ok(());
            }
            if self.session.is_some() && !format_changed {
                return Ok(());
            }
            self.session = Some(VideoToolboxRawSession::new(&self.sps, &self.pps)?);
            Ok(())
        }
    }

    pub struct VideoToolboxHevcPixelBufferDecoder {
        session: Option<VideoToolboxRawSession>,
        vps: Vec<u8>,
        sps: Vec<u8>,
        pps: Vec<u8>,
        decoded_frames: Vec<VideoToolboxPixelBufferFrame>,
    }

    unsafe impl Send for VideoToolboxHevcPixelBufferDecoder {}

    impl VideoToolboxHevcPixelBufferDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Ok(Self {
                session: None,
                vps: Vec::new(),
                sps: Vec::new(),
                pps: Vec::new(),
                decoded_frames: Vec::new(),
            })
        }

        pub fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
            let nals = access_unit_nals(access_unit)?;
            let mut hvcc_payload = Vec::with_capacity(access_unit.len());
            let mut format_changed = false;

            for nal in nals {
                if nal.len() < 2 {
                    continue;
                }
                match hevc_nal_type(nal) {
                    32 => {
                        if self.vps != nal {
                            self.vps.clear();
                            self.vps.extend_from_slice(nal);
                            format_changed = true;
                        }
                    }
                    33 => {
                        if self.sps != nal {
                            self.sps.clear();
                            self.sps.extend_from_slice(nal);
                            format_changed = true;
                        }
                    }
                    34 => {
                        if self.pps != nal {
                            self.pps.clear();
                            self.pps.extend_from_slice(nal);
                            format_changed = true;
                        }
                    }
                    35 => {}
                    _ => append_length_prefixed_nal(&mut hvcc_payload, nal)?,
                }
            }

            self.ensure_session(format_changed)?;
            if hvcc_payload.is_empty() {
                return Ok(());
            }

            let Some(session) = self.session.as_mut() else {
                return Ok(());
            };
            for pixel_buffer in session.decode(&hvcc_payload)? {
                let width = unsafe { CVPixelBufferGetWidth(pixel_buffer) };
                let height = unsafe { CVPixelBufferGetHeight(pixel_buffer) };
                self.decoded_frames
                    .push(VideoToolboxPixelBufferFrame::from_retained(
                        pixel_buffer,
                        width,
                        height,
                    )?);
            }
            Ok(())
        }

        pub fn drain_decoded_frames(&mut self) -> Vec<VideoToolboxPixelBufferFrame> {
            std::mem::take(&mut self.decoded_frames)
        }

        fn ensure_session(&mut self, format_changed: bool) -> Result<(), PipelineError> {
            if self.vps.is_empty() || self.sps.is_empty() || self.pps.is_empty() {
                return Ok(());
            }
            if self.session.is_some() && !format_changed {
                return Ok(());
            }
            self.session = Some(VideoToolboxRawSession::new_hevc(
                &self.vps, &self.sps, &self.pps,
            )?);
            Ok(())
        }
    }

    pub struct VideoToolboxHevcDecoder {
        decoder: VideoToolboxHevcPixelBufferDecoder,
        decoded_frames: Vec<CoreDecodedFrame>,
    }

    impl VideoToolboxHevcDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Ok(Self {
                decoder: VideoToolboxHevcPixelBufferDecoder::new()?,
                decoded_frames: Vec::new(),
            })
        }
    }

    impl VideoDecoder for VideoToolboxHevcDecoder {
        fn push_access_unit(&mut self, access_unit: &[u8]) -> Result<(), PipelineError> {
            self.decoder.push_access_unit(access_unit)?;
            let frames = self.decoder.drain_decoded_frames();
            self.decoded_frames.reserve(frames.len());
            for frame in frames {
                self.decoded_frames.push(vt_pixel_buffer_to_core(&frame)?);
            }
            Ok(())
        }

        fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
            std::mem::take(&mut self.decoded_frames)
        }
    }

    pub struct VideoToolboxPixelBufferFrame {
        pixel_buffer: *mut c_void,
        width: usize,
        height: usize,
    }

    unsafe impl Send for VideoToolboxPixelBufferFrame {}

    impl VideoToolboxPixelBufferFrame {
        fn from_retained(
            pixel_buffer: *mut c_void,
            width: usize,
            height: usize,
        ) -> Result<Self, PipelineError> {
            if pixel_buffer.is_null() {
                return Err(PipelineError::message(
                    "VideoToolbox returned a null CVPixelBuffer",
                ));
            }
            if width == 0 || height == 0 {
                unsafe {
                    CFRelease(pixel_buffer.cast_const());
                }
                return Err(PipelineError::message(format!(
                    "VideoToolbox returned an invalid CVPixelBuffer size: {width}x{height}"
                )));
            }
            Ok(Self {
                pixel_buffer,
                width,
                height,
            })
        }

        pub fn pixel_buffer_ptr(&self) -> *mut c_void {
            self.pixel_buffer
        }

        pub fn width(&self) -> usize {
            self.width
        }

        pub fn height(&self) -> usize {
            self.height
        }
    }

    impl Drop for VideoToolboxPixelBufferFrame {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.pixel_buffer.cast_const());
            }
        }
    }

    struct VideoToolboxRawSession {
        description: *mut c_void,
        session: *mut c_void,
        output_queue: Box<VideoToolboxRawDecodeOutputQueue>,
        wait_policy: VideoToolboxRawDecodeWaitPolicy,
    }

    impl VideoToolboxRawSession {
        fn new(sps: &[u8], pps: &[u8]) -> Result<Self, PipelineError> {
            let description = create_h264_format_description(sps, pps)?;
            let output_queue = Box::<VideoToolboxRawDecodeOutputQueue>::default();
            match create_nv12_decompression_session(description, output_queue.as_ref()) {
                Ok(session) => Ok(Self {
                    description,
                    session,
                    output_queue,
                    wait_policy: VideoToolboxRawDecodeWaitPolicy::WaitPerFrame,
                }),
                Err(error) => {
                    unsafe {
                        CFRelease(description.cast_const());
                    }
                    Err(error)
                }
            }
        }

        fn new_hevc(vps: &[u8], sps: &[u8], pps: &[u8]) -> Result<Self, PipelineError> {
            let description = create_hevc_format_description(vps, sps, pps)?;
            let output_queue = Box::<VideoToolboxRawDecodeOutputQueue>::default();
            match create_nv12_decompression_session(description, output_queue.as_ref()) {
                Ok(session) => Ok(Self {
                    description,
                    session,
                    output_queue,
                    wait_policy: hevc_raw_decode_wait_policy(),
                }),
                Err(error) => {
                    unsafe {
                        CFRelease(description.cast_const());
                    }
                    Err(error)
                }
            }
        }

        fn decode(&mut self, avcc_payload: &[u8]) -> Result<Vec<*mut c_void>, PipelineError> {
            let mut owned = Vec::new();
            let source_frame_ref_con;
            let memory_block;
            match self.wait_policy {
                VideoToolboxRawDecodeWaitPolicy::WaitPerFrame => {
                    owned.extend_from_slice(avcc_payload);
                    source_frame_ref_con = ptr::null_mut();
                    memory_block = owned.as_mut_ptr().cast::<c_void>();
                }
                VideoToolboxRawDecodeWaitPolicy::Async { .. } => {
                    let mut input = Box::new(VideoToolboxRawDecodeInput {
                        bytes: avcc_payload.to_vec(),
                    });
                    memory_block = input.bytes.as_mut_ptr().cast::<c_void>();
                    source_frame_ref_con = self.output_queue.track_input(input);
                }
            }

            let mut block_buffer = ptr::null_mut();
            let status = unsafe {
                CMBlockBufferCreateWithMemoryBlock(
                    ptr::null(),
                    memory_block,
                    avcc_payload.len(),
                    kCFAllocatorNull,
                    ptr::null(),
                    0,
                    avcc_payload.len(),
                    0,
                    &mut block_buffer,
                )
            };
            if let Err(error) = check_os_status(status, "CMBlockBufferCreateWithMemoryBlock") {
                self.output_queue
                    .release_input_if_tracked(source_frame_ref_con);
                return Err(error);
            }
            let block_buffer = match CoreFoundationOwned::new(block_buffer, "CMBlockBuffer") {
                Ok(block_buffer) => block_buffer,
                Err(error) => {
                    self.output_queue
                        .release_input_if_tracked(source_frame_ref_con);
                    return Err(error);
                }
            };

            let mut sample_buffer = ptr::null_mut();
            let status = unsafe {
                CMSampleBufferCreateReady(
                    ptr::null(),
                    block_buffer.as_ptr(),
                    self.description,
                    1,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    &mut sample_buffer,
                )
            };
            if let Err(error) = check_os_status(status, "CMSampleBufferCreateReady") {
                self.output_queue
                    .release_input_if_tracked(source_frame_ref_con);
                return Err(error);
            }
            let sample_buffer = match CoreFoundationOwned::new(sample_buffer, "CMSampleBuffer") {
                Ok(sample_buffer) => sample_buffer,
                Err(error) => {
                    self.output_queue
                        .release_input_if_tracked(source_frame_ref_con);
                    return Err(error);
                }
            };

            let mut info_flags = 0_u32;
            let status = unsafe {
                VTDecompressionSessionDecodeFrame(
                    self.session,
                    sample_buffer.as_ptr(),
                    0,
                    source_frame_ref_con,
                    &mut info_flags,
                )
            };
            if let Err(error) = check_os_status(status, "VTDecompressionSessionDecodeFrame") {
                self.output_queue
                    .release_input_if_tracked(source_frame_ref_con);
                return Err(error);
            }
            match self.wait_policy {
                VideoToolboxRawDecodeWaitPolicy::WaitPerFrame => {
                    self.wait_for_asynchronous_frames()?;
                }
                VideoToolboxRawDecodeWaitPolicy::Async { max_pending_inputs } => {
                    if self.output_queue.pending_input_count() >= max_pending_inputs {
                        self.wait_for_asynchronous_frames()?;
                    }
                }
            }
            Ok(self.output_queue.drain())
        }

        fn wait_for_asynchronous_frames(&mut self) -> Result<(), PipelineError> {
            let status = unsafe { VTDecompressionSessionWaitForAsynchronousFrames(self.session) };
            check_os_status(status, "VTDecompressionSessionWaitForAsynchronousFrames")
        }
    }

    impl Drop for VideoToolboxRawSession {
        fn drop(&mut self) {
            if !self.session.is_null() {
                unsafe {
                    let _ = VTDecompressionSessionWaitForAsynchronousFrames(self.session);
                    VTDecompressionSessionInvalidate(self.session);
                    CFRelease(self.session.cast_const());
                }
                self.session = ptr::null_mut();
            }
            if !self.description.is_null() {
                unsafe {
                    CFRelease(self.description.cast_const());
                }
                self.description = ptr::null_mut();
            }
        }
    }

    #[derive(Copy, Clone)]
    enum VideoToolboxRawDecodeWaitPolicy {
        WaitPerFrame,
        Async { max_pending_inputs: usize },
    }

    fn hevc_raw_decode_wait_policy() -> VideoToolboxRawDecodeWaitPolicy {
        if !parse_env_bool(HEVC_RAW_DECODE_ASYNC_ENV, DEFAULT_HEVC_RAW_DECODE_ASYNC) {
            return VideoToolboxRawDecodeWaitPolicy::WaitPerFrame;
        }
        VideoToolboxRawDecodeWaitPolicy::Async {
            max_pending_inputs: parse_env_usize(
                HEVC_RAW_DECODE_MAX_PENDING_INPUTS_ENV,
                DEFAULT_HEVC_RAW_DECODE_MAX_PENDING_INPUTS,
            )
            .clamp(1, 4096),
        }
    }

    struct VideoToolboxRawDecodeInput {
        bytes: Vec<u8>,
    }

    #[derive(Default)]
    struct VideoToolboxRawDecodeOutputQueue {
        frames: Mutex<Vec<usize>>,
        pending_inputs: Mutex<Vec<usize>>,
    }

    impl VideoToolboxRawDecodeOutputQueue {
        fn track_input(&self, input: Box<VideoToolboxRawDecodeInput>) -> *mut c_void {
            let ptr = Box::into_raw(input) as usize;
            let mut pending = self
                .pending_inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pending.push(ptr);
            ptr as *mut c_void
        }

        fn pending_input_count(&self) -> usize {
            self.pending_inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len()
        }

        fn release_input_if_tracked(&self, input: *mut c_void) {
            if input.is_null() {
                return;
            }
            let ptr = input as usize;
            let mut pending = self
                .pending_inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(index) = pending.iter().position(|candidate| *candidate == ptr) else {
                return;
            };
            pending.swap_remove(index);
            drop(pending);
            unsafe {
                drop(Box::from_raw(input.cast::<VideoToolboxRawDecodeInput>()));
            }
        }

        unsafe fn push_retained(&self, image_buffer: *mut c_void) {
            let retained = unsafe { CFRetain(image_buffer.cast_const()).cast_mut() } as usize;
            match self.frames.lock() {
                Ok(mut frames) => frames.push(retained),
                Err(_) => unsafe {
                    CFRelease((retained as *mut c_void).cast_const());
                },
            }
        }

        fn drain(&self) -> Vec<*mut c_void> {
            let Ok(mut frames) = self.frames.lock() else {
                return Vec::new();
            };
            frames.drain(..).map(|frame| frame as *mut c_void).collect()
        }
    }

    impl Drop for VideoToolboxRawDecodeOutputQueue {
        fn drop(&mut self) {
            let mut frames = self
                .frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for frame in frames.drain(..) {
                unsafe {
                    CFRelease((frame as *mut c_void).cast_const());
                }
            }
            let mut pending_inputs = self
                .pending_inputs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for input in pending_inputs.drain(..) {
                unsafe {
                    drop(Box::from_raw(
                        (input as *mut c_void).cast::<VideoToolboxRawDecodeInput>(),
                    ));
                }
            }
        }
    }

    unsafe extern "C" fn raw_decode_output_callback(
        decompression_output_ref_con: *mut c_void,
        _source_frame_ref_con: *mut c_void,
        status: i32,
        _info_flags: u32,
        image_buffer: *mut c_void,
        _presentation_time_stamp: CMTime,
        _presentation_duration: CMTime,
    ) {
        if status != CV_SUCCESS || image_buffer.is_null() || decompression_output_ref_con.is_null()
        {
            if !decompression_output_ref_con.is_null() {
                let output_queue =
                    decompression_output_ref_con.cast::<VideoToolboxRawDecodeOutputQueue>();
                unsafe {
                    (*output_queue).release_input_if_tracked(_source_frame_ref_con);
                }
            }
            return;
        }
        let output_queue = decompression_output_ref_con.cast::<VideoToolboxRawDecodeOutputQueue>();
        unsafe {
            (*output_queue).release_input_if_tracked(_source_frame_ref_con);
            (*output_queue).push_retained(image_buffer);
        }
    }

    struct CoreFoundationOwned {
        ptr: *mut c_void,
    }

    impl CoreFoundationOwned {
        fn new(ptr: *mut c_void, label: &str) -> Result<Self, PipelineError> {
            if ptr.is_null() {
                return Err(PipelineError::message(format!("{label} pointer is null")));
            }
            Ok(Self { ptr })
        }

        fn as_ptr(&self) -> *mut c_void {
            self.ptr
        }
    }

    impl Drop for CoreFoundationOwned {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe {
                    CFRelease(self.ptr.cast_const());
                }
                self.ptr = ptr::null_mut();
            }
        }
    }

    fn create_h264_format_description(
        sps: &[u8],
        pps: &[u8],
    ) -> Result<*mut c_void, PipelineError> {
        let mut description = ptr::null_mut();
        let parameter_set_pointers = [sps.as_ptr(), pps.as_ptr()];
        let parameter_set_sizes = [sps.len(), pps.len()];
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                ptr::null(),
                2,
                parameter_set_pointers.as_ptr(),
                parameter_set_sizes.as_ptr(),
                4,
                &mut description,
            )
        };
        check_os_status(
            status,
            "CMVideoFormatDescriptionCreateFromH264ParameterSets",
        )?;
        if description.is_null() {
            return Err(PipelineError::message(
                "CMVideoFormatDescriptionCreateFromH264ParameterSets returned null",
            ));
        }
        Ok(description)
    }

    fn create_hevc_format_description(
        vps: &[u8],
        sps: &[u8],
        pps: &[u8],
    ) -> Result<*mut c_void, PipelineError> {
        let mut description = ptr::null_mut();
        let parameter_set_pointers = [vps.as_ptr(), sps.as_ptr(), pps.as_ptr()];
        let parameter_set_sizes = [vps.len(), sps.len(), pps.len()];
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromHEVCParameterSets(
                ptr::null(),
                3,
                parameter_set_pointers.as_ptr(),
                parameter_set_sizes.as_ptr(),
                4,
                ptr::null(),
                &mut description,
            )
        };
        check_os_status(
            status,
            "CMVideoFormatDescriptionCreateFromHEVCParameterSets",
        )?;
        if description.is_null() {
            return Err(PipelineError::message(
                "CMVideoFormatDescriptionCreateFromHEVCParameterSets returned null",
            ));
        }
        Ok(description)
    }

    fn create_nv12_decompression_session(
        description: *mut c_void,
        output_queue: &VideoToolboxRawDecodeOutputQueue,
    ) -> Result<*mut c_void, PipelineError> {
        let pixel_format = cf_number_i32(CV_PIXEL_FORMAT_NV12_VIDEO_RANGE as i32)?;
        let destination_attributes = cf_dictionary(&[(
            unsafe { kCVPixelBufferPixelFormatTypeKey },
            pixel_format.as_ptr(),
        )])?;
        let callback = VTDecompressionOutputCallbackRecord {
            decompression_output_callback: Some(raw_decode_output_callback),
            decompression_output_ref_con: (output_queue as *const VideoToolboxRawDecodeOutputQueue)
                .cast_mut()
                .cast(),
        };
        let mut session = ptr::null_mut();
        let status = unsafe {
            VTDecompressionSessionCreate(
                ptr::null(),
                description,
                ptr::null(),
                destination_attributes.as_ptr(),
                &callback,
                &mut session,
            )
        };
        check_os_status(status, "VTDecompressionSessionCreate")?;
        if session.is_null() {
            return Err(PipelineError::message(
                "VTDecompressionSessionCreate returned null",
            ));
        }
        Ok(session)
    }

    fn cf_number_i32(value: i32) -> Result<CoreFoundationOwned, PipelineError> {
        let ptr = unsafe {
            CFNumberCreate(
                ptr::null(),
                CF_NUMBER_SINT32_TYPE,
                (&value as *const i32).cast(),
            )
        };
        CoreFoundationOwned::new(ptr.cast_mut(), "CFNumber")
    }

    fn cf_dictionary(
        kvs: &[(*const c_void, *const c_void)],
    ) -> Result<CoreFoundationOwned, PipelineError> {
        let mut keys: Vec<*const c_void> = kvs.iter().map(|(key, _)| *key).collect();
        let mut values: Vec<*const c_void> = kvs.iter().map(|(_, value)| *value).collect();
        let ptr = unsafe {
            CFDictionaryCreate(
                ptr::null(),
                keys.as_mut_ptr(),
                values.as_mut_ptr(),
                kvs.len() as isize,
                ptr::null(),
                ptr::null(),
            )
        };
        CoreFoundationOwned::new(ptr.cast_mut(), "CFDictionary")
    }

    fn check_os_status(status: i32, label: &str) -> Result<(), PipelineError> {
        if status == CV_SUCCESS {
            Ok(())
        } else {
            let status_name = os_status_name(status)
                .map(|name| format!(" ({name})"))
                .unwrap_or_default();
            Err(PipelineError::message(format!(
                "{label} failed: status={status}{status_name}"
            )))
        }
    }

    fn os_status_name(status: i32) -> Option<&'static str> {
        match status {
            -12900 => Some("kVTPropertyNotSupportedErr"),
            -12901 => Some("kVTPropertyReadOnlyErr"),
            -12902 => Some("kVTParameterErr"),
            -12903 => Some("kVTInvalidSessionErr"),
            -12904 => Some("kVTAllocationFailedErr"),
            -12905 => Some("kVTPixelTransferNotSupportedErr"),
            -12906 => Some("kVTCouldNotFindVideoDecoderErr"),
            -12907 => Some("kVTCouldNotCreateInstanceErr"),
            -12908 => Some("kVTCouldNotFindVideoEncoderErr"),
            -12909 => Some("kVTVideoDecoderBadDataErr"),
            -12910 => Some("kVTVideoDecoderUnsupportedDataFormatErr"),
            -12911 => Some("kVTVideoDecoderMalfunctionErr"),
            -12912 => Some("kVTVideoEncoderMalfunctionErr"),
            -12913 => Some("kVTVideoDecoderNotAvailableNowErr"),
            -12914 => Some("kVTPixelRotationNotSupportedErr"),
            -12915 => Some("kVTVideoEncoderNotAvailableNowErr"),
            -12916 => Some("kVTFormatDescriptionChangeNotSupportedErr"),
            -12917 => Some("kVTInsufficientSourceColorDataErr"),
            -12918 => Some("kVTCouldNotCreateColorCorrectionDataErr"),
            -12919 => Some("kVTColorSyncTransformConvertFailedErr"),
            -12210 => Some("kVTVideoDecoderAuthorizationErr"),
            -12211 => Some("kVTVideoEncoderAuthorizationErr"),
            -12212 => Some("kVTColorCorrectionPixelTransferFailedErr"),
            -12213 => Some("kVTMultiPassStorageIdentifierMismatchErr"),
            -12214 => Some("kVTMultiPassStorageInvalidErr"),
            -12215 => Some("kVTFrameSiloInvalidTimeStampErr"),
            -12216 => Some("kVTFrameSiloInvalidTimeRangeErr"),
            -12217 => Some("kVTCouldNotFindTemporalFilterErr"),
            -12218 => Some("kVTPixelTransferNotPermittedErr"),
            -12219 => Some("kVTColorCorrectionImageRotationFailedErr"),
            -17690 => Some("kVTVideoDecoderRemovedErr"),
            -17691 => Some("kVTSessionMalfunctionErr"),
            -17692 => Some("kVTVideoDecoderNeedsRosettaErr"),
            -17693 => Some("kVTVideoEncoderNeedsRosettaErr"),
            -17694 => Some("kVTVideoDecoderReferenceMissingErr"),
            -17695 => Some("kVTVideoDecoderCallbackMessagingErr"),
            -17696 => Some("kVTVideoDecoderUnknownErr"),
            -17697 => Some("kVTExtensionDisabledErr"),
            -17698 => Some("kVTVideoEncoderMVHEVCVideoLayerIDsMismatchErr"),
            -17699 => Some("kVTCouldNotOutputTaggedBufferGroupErr"),
            -19510 => Some("kVTCouldNotFindExtensionErr"),
            -19511 => Some("kVTExtensionConflictErr"),
            -19512 => Some("kVTVideoEncoderAutoWhiteBalanceNotLockedErr"),
            _ => None,
        }
    }

    fn cm_time(value: i64, fps: u32) -> CMTime {
        CMTime {
            value,
            timescale: i32::try_from(fps.max(1)).unwrap_or(i32::MAX),
            flags: CM_TIME_FLAGS_VALID,
            epoch: 0,
        }
    }

    fn invalid_cm_time() -> CMTime {
        CMTime {
            value: 0,
            timescale: 0,
            flags: 0,
            epoch: 0,
        }
    }

    fn configure_av1_compression_session(
        session: *mut c_void,
        fps: u32,
        bitrate: u32,
    ) -> Result<(), PipelineError> {
        let fps_i32 = i32::try_from(fps.max(1)).map_err(|_| {
            PipelineError::message(format!("VideoToolbox AV1 fps too large: {fps}"))
        })?;
        let bitrate_i32 = i32::try_from(bitrate.max(1)).map_err(|_| {
            PipelineError::message(format!("VideoToolbox AV1 bitrate too large: {bitrate}"))
        })?;
        set_vt_property_bool(session, unsafe { kVTCompressionPropertyKey_RealTime }, true)?;
        set_vt_property_bool(
            session,
            unsafe { kVTCompressionPropertyKey_AllowFrameReordering },
            false,
        )?;
        set_vt_property_bool(
            session,
            unsafe { kVTCompressionPropertyKey_AllowTemporalCompression },
            false,
        )?;
        let _ = set_vt_property_bool(
            session,
            unsafe { kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality },
            true,
        );
        set_vt_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_ExpectedFrameRate },
            fps_i32,
        )?;
        set_vt_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_AverageBitRate },
            bitrate_i32,
        )?;
        set_vt_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_MaxKeyFrameInterval },
            fps_i32.saturating_mul(2).max(1),
        )?;
        set_vt_property_i32(
            session,
            unsafe { kVTCompressionPropertyKey_MaxFrameDelayCount },
            1,
        )?;
        Ok(())
    }

    fn set_vt_property_bool(
        session: *mut c_void,
        key: *const c_void,
        value: bool,
    ) -> Result<(), PipelineError> {
        let value = if value {
            unsafe { kCFBooleanTrue }
        } else {
            unsafe { kCFBooleanFalse }
        };
        let status = unsafe { VTSessionSetProperty(session, key, value) };
        check_os_status(status, "VTSessionSetProperty(bool)")
    }

    fn set_vt_property_i32(
        session: *mut c_void,
        key: *const c_void,
        value: i32,
    ) -> Result<(), PipelineError> {
        let cf_value = cf_number_i32(value)?;
        let status = unsafe { VTSessionSetProperty(session, key, cf_value.as_ptr().cast_const()) };
        check_os_status(status, "VTSessionSetProperty(i32)")
    }

    unsafe extern "C" fn av1_compression_output_callback(
        output_callback_ref_con: *mut c_void,
        source_frame_ref_con: *mut c_void,
        status: i32,
        _info_flags: u32,
        sample_buffer: *mut c_void,
    ) {
        let frame_ref = if source_frame_ref_con.is_null() {
            None
        } else {
            Some(unsafe { Box::from_raw(source_frame_ref_con.cast::<VideoToolboxAv1FrameRef>()) })
        };
        if status != CV_SUCCESS
            || output_callback_ref_con.is_null()
            || sample_buffer.is_null()
            || frame_ref.is_none()
        {
            return;
        }
        let frame_ref = frame_ref.expect("checked frame ref");
        let data_buffer = unsafe { CMSampleBufferGetDataBuffer(sample_buffer) };
        if data_buffer.is_null() {
            return;
        }
        let block_len = unsafe { CMBlockBufferGetDataLength(data_buffer) };
        if block_len == 0 {
            return;
        }
        let mut bytes = vec![0u8; block_len];
        let copy_status = unsafe {
            CMBlockBufferCopyDataBytes(data_buffer, 0, block_len, bytes.as_mut_ptr().cast())
        };
        if copy_status != CV_SUCCESS {
            return;
        }
        let output_queue =
            output_callback_ref_con.cast::<Mutex<VecDeque<VideoToolboxAv1EncodedFrame>>>();
        if let Ok(mut queue) = unsafe { &*output_queue }.lock() {
            queue.push_back(VideoToolboxAv1EncodedFrame {
                timestamp_us: frame_ref.timestamp_us,
                is_keyframe: frame_ref.is_keyframe,
                bytes,
            });
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

    fn encoded_hevc_to_annex_b(frame: &vt::EncodedFrame) -> Result<Vec<u8>, PipelineError> {
        let mut out = Vec::with_capacity(
            frame.data.len()
                + frame.vps_list.iter().map(|v| v.len() + 4).sum::<usize>()
                + frame.sps_list.iter().map(|s| s.len() + 4).sum::<usize>()
                + frame.pps_list.iter().map(|p| p.len() + 4).sum::<usize>(),
        );
        for vps in &frame.vps_list {
            append_annex_b_nal(&mut out, vps);
        }
        for sps in &frame.sps_list {
            append_annex_b_nal(&mut out, sps);
        }
        for pps in &frame.pps_list {
            append_annex_b_nal(&mut out, pps);
        }
        avcc_payload_to_annex_b(&frame.data, &mut out)?;
        Ok(out)
    }

    struct VideoToolboxReusablePixelBuffer {
        ptr: *mut c_void,
    }

    unsafe impl Send for VideoToolboxReusablePixelBuffer {}

    impl VideoToolboxReusablePixelBuffer {
        fn new_nv12(width: usize, height: usize) -> Result<Self, PipelineError> {
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
            Ok(Self { ptr: pixel_buffer })
        }

        fn as_ptr(&self) -> *mut c_void {
            self.ptr
        }
    }

    impl Drop for VideoToolboxReusablePixelBuffer {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                unsafe {
                    CFRelease(self.ptr.cast_const());
                }
                self.ptr = ptr::null_mut();
            }
        }
    }

    trait VideoToolboxNv12EncodeExt {
        fn encode_nv12_planes_from_pixel_buffer(
            &mut self,
            pixel_buffer: *mut c_void,
            width: usize,
            height: usize,
            y_plane: &[u8],
            uv_plane: &[u8],
            options: &vt::EncodeOptions,
        ) -> Result<(), PipelineError>;
    }

    impl VideoToolboxNv12EncodeExt for vt::Encoder {
        fn encode_nv12_planes_from_pixel_buffer(
            &mut self,
            pixel_buffer: *mut c_void,
            width: usize,
            height: usize,
            y_plane: &[u8],
            uv_plane: &[u8],
            options: &vt::EncodeOptions,
        ) -> Result<(), PipelineError> {
            validate_nv12_planes(width, height, y_plane, uv_plane)?;
            if pixel_buffer.is_null() {
                return Err(PipelineError::message(
                    "CVPixelBufferCreate(NV12) returned null",
                ));
            }

            copy_and_encode_nv12_pixel_buffer(
                self,
                pixel_buffer,
                width,
                height,
                y_plane,
                uv_plane,
                options,
            )
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

    fn copy_nv12_planes_into_pixel_buffer(
        pixel_buffer: *mut c_void,
        width: usize,
        height: usize,
        y_plane: &[u8],
        uv_plane: &[u8],
        label: &str,
    ) -> Result<(), PipelineError> {
        validate_nv12_planes(width, height, y_plane, uv_plane)?;
        if pixel_buffer.is_null() {
            return Err(PipelineError::message(format!(
                "{label} CVPixelBufferCreate(NV12) returned null"
            )));
        }

        let status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 0) };
        if status != CV_SUCCESS {
            return Err(PipelineError::message(format!(
                "{label} CVPixelBufferLockBaseAddress(NV12) failed: status={status}"
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
                "{label} CVPixelBufferUnlockBaseAddress(NV12) failed: status={unlock_status}"
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
            vt::DecodedFrame::Nv12(frame) => vt_nv12_planes_to_core(
                frame.width(),
                frame.height(),
                0,
                frame.y_plane(),
                frame.y_stride(),
                frame.uv_plane(),
                frame.uv_stride(),
            ),
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

    fn vt_pixel_buffer_to_core(
        frame: &VideoToolboxPixelBufferFrame,
    ) -> Result<CoreDecodedFrame, PipelineError> {
        let pixel_buffer = frame.pixel_buffer_ptr();
        let status = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, 0) };
        if status != CV_SUCCESS {
            return Err(PipelineError::message(format!(
                "CVPixelBufferLockBaseAddress(decode) failed: status={status}"
            )));
        }

        let copy_result = unsafe {
            let y_base = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 0);
            let uv_base = CVPixelBufferGetBaseAddressOfPlane(pixel_buffer, 1);
            if y_base.is_null() || uv_base.is_null() {
                Err(PipelineError::message(
                    "VideoToolbox decoded NV12 plane base address is null",
                ))
            } else {
                let y_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 0);
                let uv_stride = CVPixelBufferGetBytesPerRowOfPlane(pixel_buffer, 1);
                let y_height = CVPixelBufferGetHeightOfPlane(pixel_buffer, 0);
                let uv_height = CVPixelBufferGetHeightOfPlane(pixel_buffer, 1);
                let y_len = y_stride.checked_mul(y_height).ok_or_else(|| {
                    PipelineError::message("VideoToolbox decoded NV12 luma size overflow")
                })?;
                let uv_len = uv_stride.checked_mul(uv_height).ok_or_else(|| {
                    PipelineError::message("VideoToolbox decoded NV12 chroma size overflow")
                })?;
                let y_plane = slice::from_raw_parts(y_base.cast::<u8>(), y_len);
                let uv_plane = slice::from_raw_parts(uv_base.cast::<u8>(), uv_len);
                vt_nv12_planes_to_core(
                    frame.width(),
                    frame.height(),
                    0,
                    y_plane,
                    y_stride,
                    uv_plane,
                    uv_stride,
                )
            }
        };
        let unlock_status = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, 0) };
        if unlock_status != CV_SUCCESS {
            return Err(PipelineError::message(format!(
                "CVPixelBufferUnlockBaseAddress(decode) failed: status={unlock_status}"
            )));
        }
        copy_result
    }

    fn vt_nv12_planes_to_core(
        width: usize,
        height: usize,
        timestamp_us: u64,
        y_plane: &[u8],
        y_stride: usize,
        uv_plane: &[u8],
        uv_stride: usize,
    ) -> Result<CoreDecodedFrame, PipelineError> {
        let uv_height = height.div_ceil(2);
        if y_stride < width || uv_stride < width {
            return Err(PipelineError::message(
                "VideoToolbox returned an invalid NV12 stride",
            ));
        }
        let y_bytes = y_stride
            .checked_mul(height)
            .ok_or_else(|| PipelineError::message("VideoToolbox NV12 luma size overflow"))?;
        let uv_bytes = uv_stride
            .checked_mul(uv_height)
            .ok_or_else(|| PipelineError::message("VideoToolbox NV12 chroma size overflow"))?;
        if y_plane.len() < y_bytes || uv_plane.len() < uv_bytes {
            return Err(PipelineError::message(
                "VideoToolbox returned an invalid NV12 plane",
            ));
        }

        if y_stride == uv_stride {
            let mut data = Vec::with_capacity(y_bytes + uv_bytes);
            data.extend_from_slice(&y_plane[..y_bytes]);
            data.extend_from_slice(&uv_plane[..uv_bytes]);
            return Ok(CoreDecodedFrame::from_cpu_nv12(
                width,
                height,
                timestamp_us,
                y_stride,
                data,
            ));
        }

        let pitch = width;
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
            width,
            height,
            timestamp_us,
            pitch,
            data,
        ))
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
        append_length_prefixed_nal(out, nal)
    }

    fn append_length_prefixed_nal(out: &mut Vec<u8>, nal: &[u8]) -> Result<(), PipelineError> {
        let len = u32::try_from(nal.len()).map_err(|_| {
            PipelineError::message(format!("NAL unit too large: {} bytes", nal.len()))
        })?;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
        Ok(())
    }

    fn hevc_nal_type(nal: &[u8]) -> u8 {
        (nal[0] >> 1) & 0x3f
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
        fn os_status_name_labels_missing_video_encoder() {
            assert_eq!(
                os_status_name(-12908),
                Some("kVTCouldNotFindVideoEncoderErr")
            );
            assert_eq!(os_status_name(0), None);
        }

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
        fn vt_nv12_planes_preserve_decoder_stride_when_planes_match() {
            let frame = vt_nv12_planes_to_core(
                4,
                2,
                123,
                &[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 88, 88],
                6,
                &[9, 10, 11, 12, 77, 77],
                6,
            )
            .expect("copy nv12 planes");

            assert_eq!(frame.timestamp_us, 123);
            match frame.data {
                mrd_pipeline_core::DecodedFrameData::CpuNv12 { data, pitch } => {
                    assert_eq!(pitch, 6);
                    assert_eq!(
                        data,
                        vec![1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 88, 88, 9, 10, 11, 12, 77, 77]
                    );
                }
                other => panic!("expected CPU NV12 frame, got {other:?}"),
            }
        }

        #[test]
        fn vt_nv12_planes_compact_when_plane_strides_differ() {
            let frame = vt_nv12_planes_to_core(
                4,
                2,
                0,
                &[1, 2, 3, 4, 99, 99, 5, 6, 7, 8, 88, 88],
                6,
                &[9, 10, 11, 12],
                4,
            )
            .expect("copy nv12 planes");

            match frame.data {
                mrd_pipeline_core::DecodedFrameData::CpuNv12 { data, pitch } => {
                    assert_eq!(pitch, 4);
                    assert_eq!(data, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
                }
                other => panic!("expected CPU NV12 frame, got {other:?}"),
            }
        }

        #[test]
        fn videotoolbox_h264_startup_keyframe_burst_covers_high_fps_attach_delay() {
            let fps = 144;
            let startup_burst_frames = h264_frames_for_millis(fps, 2_000);
            let startup_interval_frames = h264_frames_for_millis(fps, 50);

            assert_eq!(startup_burst_frames, 288);
            assert_eq!(startup_interval_frames, 7);
            assert!(h264_should_force_keyframe(
                0,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
            assert!(!h264_should_force_keyframe(
                6,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
            assert!(h264_should_force_keyframe(
                7,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
            assert!(h264_should_force_keyframe(
                280,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
            assert!(h264_should_force_keyframe(
                288,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
            assert!(!h264_should_force_keyframe(
                302,
                fps,
                false,
                startup_burst_frames,
                startup_interval_frames
            ));
        }

        #[test]
        fn videotoolbox_h264_startup_keyframe_burst_can_be_disabled() {
            let fps = 144;

            assert!(h264_should_force_keyframe(0, fps, false, 0, 0));
            assert!(!h264_should_force_keyframe(14, fps, false, 0, 0));
            assert!(h264_should_force_keyframe(14, fps, true, 0, 0));
            assert!(h264_should_force_keyframe(144, fps, false, 0, 0));
        }

        #[test]
        fn videotoolbox_hevc_startup_keyframe_burst_defaults_cover_receiver_warmup() {
            let fps = 144;
            let width = 1_920;
            let height = 1_080;

            assert_eq!(hevc_startup_keyframe_burst_frames(width, height, fps), 288);
            assert_eq!(hevc_startup_keyframe_interval_frames(fps), 7);
            assert!(h264_should_force_keyframe(
                0,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
            assert!(h264_should_force_keyframe(
                7,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
            assert!(!h264_should_force_keyframe(
                8,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
            assert!(h264_should_force_keyframe(
                144,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
        }

        #[test]
        fn videotoolbox_hevc_2k144_defaults_disable_startup_keyframe_burst() {
            let fps = 144;
            let width = 2_560;
            let height = 1_440;

            assert_eq!(hevc_startup_keyframe_burst_frames(width, height, fps), 0);
            assert_eq!(hevc_startup_keyframe_interval_frames(fps), 7);
            assert!(h264_should_force_keyframe(
                0,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
            assert!(!h264_should_force_keyframe(
                7,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
            assert!(h264_should_force_keyframe(
                144,
                fps,
                false,
                hevc_startup_keyframe_burst_frames(width, height, fps),
                hevc_startup_keyframe_interval_frames(fps)
            ));
        }

        #[test]
        fn raw_decode_output_queue_releases_tracked_input_once() {
            let queue = VideoToolboxRawDecodeOutputQueue::default();
            let input = Box::new(VideoToolboxRawDecodeInput {
                bytes: vec![1, 2, 3, 4],
            });
            let ptr = queue.track_input(input);

            assert_eq!(queue.pending_input_count(), 1);
            queue.release_input_if_tracked(ptr);
            assert_eq!(queue.pending_input_count(), 0);
            queue.release_input_if_tracked(ptr);
            assert_eq!(queue.pending_input_count(), 0);
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

        #[test]
        fn videotoolbox_h264_pixel_buffer_roundtrip() {
            let mut encoder = VideoToolboxH264Encoder::new_with_bitrate(64, 64, 30, 1_000_000)
                .expect("create videotoolbox encoder");
            let mut decoder = VideoToolboxH264PixelBufferDecoder::new()
                .expect("create videotoolbox pixel buffer decoder");

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
                    decoder
                        .push_access_unit(&unit.bytes)
                        .expect("decode synthetic access unit to CVPixelBuffer");
                    let frames = decoder.drain_decoded_frames();
                    if let Some(frame) = frames.first() {
                        assert!(!frame.pixel_buffer_ptr().is_null());
                        assert_eq!(frame.width(), 64);
                        assert_eq!(frame.height(), 64);
                        return;
                    }
                }
            }

            panic!("VideoToolbox pixel buffer roundtrip did not produce a decoded frame");
        }

        #[test]
        fn videotoolbox_hevc_synthetic_roundtrip() {
            let mut encoder = VideoToolboxHevcEncoder::new_with_bitrate(64, 64, 30, 1_000_000)
                .expect("create videotoolbox HEVC encoder");
            let mut decoder =
                VideoToolboxHevcDecoder::new().expect("create videotoolbox HEVC decoder");

            for index in 0..8_u64 {
                let frame = CapturedFrame::from_cpu(
                    64,
                    64,
                    FramePixelFormat::Bgra32,
                    index * 33_333,
                    synthetic_bgra(64, 64, index as u8),
                );
                let units = encoder.encode(&frame).expect("encode synthetic HEVC frame");
                for unit in units {
                    assert_eq!(unit.codec, VideoCodec::Hevc);
                    decoder
                        .push_access_unit(&unit.bytes)
                        .expect("decode synthetic HEVC access unit");
                    let frames = decoder.drain_decoded_frames();
                    if let Some(frame) = frames.first() {
                        assert_eq!(frame.width, 64);
                        assert_eq!(frame.height, 64);
                        match frame.data {
                            mrd_pipeline_core::DecodedFrameData::CpuNv12 { .. } => return,
                            ref other => panic!("expected CPU NV12 HEVC frame, got {other:?}"),
                        }
                    }
                }
            }

            panic!("VideoToolbox HEVC roundtrip did not produce a decoded frame");
        }

        #[test]
        fn videotoolbox_hevc_pixel_buffer_roundtrip_outputs_most_frames() {
            let mut encoder = VideoToolboxHevcEncoder::new_with_bitrate(128, 128, 144, 4_000_000)
                .expect("create videotoolbox HEVC encoder");
            let mut decoder = VideoToolboxHevcPixelBufferDecoder::new()
                .expect("create videotoolbox HEVC pixel buffer decoder");
            let mut encoded_units = 0_usize;
            let mut decoded_frames = 0_usize;

            for index in 0..48_u64 {
                let frame = CapturedFrame::from_cpu(
                    128,
                    128,
                    FramePixelFormat::Bgra32,
                    index * 6_944,
                    synthetic_bgra(128, 128, index as u8),
                );
                let units = encoder.encode(&frame).expect("encode synthetic HEVC frame");
                for unit in units {
                    assert_eq!(unit.codec, VideoCodec::Hevc);
                    encoded_units = encoded_units.saturating_add(1);
                    decoder
                        .push_access_unit(&unit.bytes)
                        .expect("decode synthetic HEVC access unit to CVPixelBuffer");
                    let frames = decoder.drain_decoded_frames();
                    for frame in frames {
                        assert!(!frame.pixel_buffer_ptr().is_null());
                        assert_eq!(frame.width(), 128);
                        assert_eq!(frame.height(), 128);
                        decoded_frames = decoded_frames.saturating_add(1);
                    }
                }
            }

            assert!(
                encoded_units >= 24,
                "encoded too few HEVC units: {encoded_units}"
            );
            assert!(
                decoded_frames * 4 >= encoded_units * 3,
                "decoded {decoded_frames} CVPixelBuffers from {encoded_units} HEVC access units"
            );
        }

        #[test]
        #[ignore = "2K HEVC diagnostic is too heavy for regular test runs"]
        fn videotoolbox_hevc_2k144_pixel_buffer_output_ratio_diagnostic() {
            let mut encoder =
                VideoToolboxHevcEncoder::new_with_bitrate(2560, 1440, 144, 40_000_000)
                    .expect("create 2K VideoToolbox HEVC encoder");
            let mut decoder = VideoToolboxHevcPixelBufferDecoder::new()
                .expect("create 2K VideoToolbox HEVC pixel buffer decoder");
            let mut encoded_units = 0_usize;
            let mut decoded_frames = 0_usize;

            for index in 0..60_u64 {
                let frame = CapturedFrame::from_cpu(
                    2560,
                    1440,
                    FramePixelFormat::Bgra32,
                    index * 6_944,
                    synthetic_bgra(2560, 1440, index as u8),
                );
                let units = encoder.encode(&frame).expect("encode 2K HEVC frame");
                for unit in units {
                    encoded_units = encoded_units.saturating_add(1);
                    decoder
                        .push_access_unit(&unit.bytes)
                        .expect("decode 2K HEVC access unit to CVPixelBuffer");
                    decoded_frames =
                        decoded_frames.saturating_add(decoder.drain_decoded_frames().len());
                }
            }

            eprintln!(
                "2K HEVC diagnostic encoded_units={encoded_units} decoded_frames={decoded_frames}"
            );
            assert!(
                encoded_units >= 30,
                "encoded too few HEVC units: {encoded_units}"
            );
            assert!(
                decoded_frames * 4 >= encoded_units * 3,
                "decoded {decoded_frames} CVPixelBuffers from {encoded_units} HEVC access units"
            );
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

    pub struct VideoToolboxHevcEncoder;

    impl VideoToolboxHevcEncoder {
        pub fn new(_width: usize, _height: usize, _fps: u32) -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox HEVC encoder is only available on macOS",
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

    impl VideoEncoder for VideoToolboxHevcEncoder {
        fn encode(
            &mut self,
            _frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox HEVC encoder is only available on macOS",
            ))
        }
    }

    pub struct VideoToolboxAv1Encoder;

    impl VideoToolboxAv1Encoder {
        pub fn new(_width: usize, _height: usize, _fps: u32) -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox AV1 encoder is only available on macOS",
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

    impl VideoEncoder for VideoToolboxAv1Encoder {
        fn encode(
            &mut self,
            _frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox AV1 encoder is only available on macOS",
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

    pub struct VideoToolboxH264PixelBufferDecoder;

    impl VideoToolboxH264PixelBufferDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox CVPixelBuffer decoder is only available on macOS",
            ))
        }
    }

    pub struct VideoToolboxHevcPixelBufferDecoder;

    impl VideoToolboxHevcPixelBufferDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox HEVC CVPixelBuffer decoder is only available on macOS",
            ))
        }
    }

    pub struct VideoToolboxHevcDecoder;

    impl VideoToolboxHevcDecoder {
        pub fn new() -> Result<Self, PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox HEVC decoder is only available on macOS",
            ))
        }
    }

    impl VideoDecoder for VideoToolboxHevcDecoder {
        fn push_access_unit(&mut self, _access_unit: &[u8]) -> Result<(), PipelineError> {
            Err(PipelineError::message(
                "VideoToolbox HEVC decoder is only available on macOS",
            ))
        }

        fn drain_decoded_frames(&mut self) -> Vec<CoreDecodedFrame> {
            Vec::new()
        }
    }

    pub struct VideoToolboxPixelBufferFrame;
}

pub use imp::{
    VideoToolboxAv1Encoder, VideoToolboxH264Decoder, VideoToolboxH264Encoder,
    VideoToolboxH264PixelBufferDecoder, VideoToolboxHevcDecoder, VideoToolboxHevcEncoder,
    VideoToolboxHevcPixelBufferDecoder, VideoToolboxPixelBufferFrame,
};
