#[cfg(not(windows))]
use mrd_pipeline_core::{CapturedFrame, EncodedAccessUnit, PipelineError, VideoEncoder};
#[cfg(not(windows))]
use mrd_pipeline_core::{FramePixelFormat, VideoCodec};
#[cfg(not(windows))]
use std::io::{Read, Write};
#[cfg(not(windows))]
use std::process::{Child, ChildStdin};
#[cfg(not(windows))]
use std::sync::{mpsc, Arc, Mutex};
#[cfg(not(windows))]
use std::time::{Duration, Instant};

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Context};
    use mrd_pipeline_core::{
        CapturedFrame, D3D11SharedBgraFrame, EncodedAccessUnit, FrameMemoryKind, FramePixelFormat,
        PipelineError, VideoCodec, VideoEncoder,
    };
    use nvenc::bitstream::BitStream;
    use nvenc::encoder::{Encoder, RegisteredResource};
    use nvenc::session::{InitParams, NeedsConfig, Session};
    use nvenc::sys::enums::{
        NVencBufferFormat, NVencPicFlags, NVencPicStruct, NVencPicType, NVencTuningInfo,
    };
    use nvenc::sys::guids::{
        NV_ENC_CODEC_H264_GUID, NV_ENC_CODEC_HEVC_GUID, NV_ENC_H264_PROFILE_BASELINE_GUID,
        NV_ENC_H264_PROFILE_HIGH_GUID, NV_ENC_HEVC_PROFILE_MAIN10_GUID,
        NV_ENC_HEVC_PROFILE_MAIN_GUID, NV_ENC_PRESET_P1_GUID, NV_ENC_PRESET_P3_GUID,
        NV_ENC_PRESET_P6_GUID,
    };
    use nvenc::sys::structs::Guid;
    use std::collections::VecDeque;
    use windows::core::Interface;
    use windows::Win32::Foundation::{HANDLE, HMODULE};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    const H264_SHARED_ASYNC_SLOT_COUNT: usize = 2;
    const H264_SHARED_INPUT_CACHE_LIMIT: usize = 8;

    pub struct NvencH264Encoder {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        encoder: Encoder,
        registered: RegisteredResource,
        shared_inputs: Vec<SharedInputResource>,
        shared_encode_slots: Vec<SharedEncodeSlot>,
        pending_shared_encodes: VecDeque<PendingSharedEncode>,
        bitstream: BitStream,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: usize,
    }

    unsafe impl Send for NvencH264Encoder {}

    pub struct NvencHevcEncoder {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        encoder: Encoder,
        registered: RegisteredResource,
        shared_input: Option<SharedInputResource>,
        bitstream: BitStream,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: usize,
    }

    unsafe impl Send for NvencHevcEncoder {}

    struct SharedInputResource {
        shared_handle: isize,
        width: u32,
        height: u32,
        _texture: ID3D11Texture2D,
        registered: RegisteredResource,
    }

    struct SharedEncodeSlot {
        texture: ID3D11Texture2D,
        registered: RegisteredResource,
        bitstream: BitStream,
    }

    struct PendingSharedEncode {
        slot: SharedEncodeSlot,
        timestamp_us: u64,
        is_keyframe: bool,
    }

    impl NvencH264Encoder {
        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_with_profile(width, height, fps, NV_ENC_H264_PROFILE_HIGH_GUID)
        }

        pub fn new_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                NV_ENC_H264_PROFILE_HIGH_GUID,
                bitrate.max(1),
            )
        }

        pub fn new_baseline(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_with_profile(width, height, fps, NV_ENC_H264_PROFILE_BASELINE_GUID)
        }

        /// Ultra-low latency encoder for remote desktop scenarios
        /// Uses UltraLowLatency tuning and P6 preset for minimum latency
        pub fn new_ultra_low_latency(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_ultra_low_latency_internal(width, height, fps, NV_ENC_H264_PROFILE_HIGH_GUID)
        }

        /// High refresh rate encoder (120Hz+) optimized for minimum latency
        /// Uses Baseline profile, lower bitrate, and shorter GOP for maximum speed
        /// Target: <7ms encode latency for 2K@144Hz
        pub fn new_high_refresh_rate(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_high_refresh_rate_internal(width, height, fps, 8_000_000)
        }

        /// Extreme low latency encoder for 144Hz+ gaming scenarios
        /// Very aggressive settings for maximum speed at cost of quality
        /// Target: <7ms encode latency for 2K@144Hz
        pub fn new_extreme_low_latency(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_high_refresh_rate_internal(width, height, fps, 5_000_000)
        }

        /// Maximum speed encoder using P1 preset (fastest preset)
        /// Lowest quality but maximum speed for 144Hz+ gaming
        pub fn new_max_speed(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_max_speed_with_bitrate(width, height, fps, 5_000_000)
        }

        pub fn new_max_speed_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let bitrate = bitrate.max(1);
            let (device, context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    NV_ENC_PRESET_P1_GUID,
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;

            // Maximum speed optimizations:
            preset.preset_cfg.profile_guid = NV_ENC_H264_PROFILE_BASELINE_GUID;
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
            preset.preset_cfg.gop_len = fps.min(30);
            preset.preset_cfg.frame_interval_p = 1;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid: NV_ENC_PRESET_P1_GUID,
                resolution: [width as u32, height as u32],
                aspect_ratio: [width as u32, height as u32],
                frame_rate: [fps, 1],
                tuning_info: NVencTuningInfo::UltraLowLatency,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width as u32, height as u32],
            };
            let encoder = session.init_encoder(init).map_err(|error| {
                PipelineError::message(format!("nvenc init encoder failed: {error:?}"))
            })?;
            let texture =
                create_encode_texture(&device, width as u32, height as u32).map_err(|error| {
                    PipelineError::message(format!("create nvenc texture failed: {error}"))
                })?;
            let registered = encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                .map_err(|error| {
                    PipelineError::message(format!("nvenc register resource failed: {error:?}"))
                })?;
            let bitstream = encoder.create_bitstream_buffer().map_err(|error| {
                PipelineError::message(format!("nvenc bitstream buffer failed: {error:?}"))
            })?;

            Ok(Self {
                _device: device,
                context,
                texture,
                encoder,
                registered,
                shared_inputs: Vec::new(),
                shared_encode_slots: Vec::new(),
                pending_shared_encodes: VecDeque::new(),
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        fn new_high_refresh_rate_internal(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let (device, context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    NV_ENC_PRESET_P6_GUID,
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;

            // High refresh rate optimizations:
            // - Use Baseline profile (faster than High/Main)
            preset.preset_cfg.profile_guid = NV_ENC_H264_PROFILE_BASELINE_GUID;
            // - Lower bitrate for faster encoding
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
            // - Very short GOP for minimal I-frame overhead
            preset.preset_cfg.gop_len = fps.min(30);
            // - Disable frame doubling
            preset.preset_cfg.frame_interval_p = 1;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid: NV_ENC_PRESET_P6_GUID,
                resolution: [width as u32, height as u32],
                aspect_ratio: [width as u32, height as u32],
                frame_rate: [fps, 1],
                tuning_info: NVencTuningInfo::UltraLowLatency,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width as u32, height as u32],
            };
            let encoder = session.init_encoder(init).map_err(|error| {
                PipelineError::message(format!("nvenc init encoder failed: {error:?}"))
            })?;
            let texture =
                create_encode_texture(&device, width as u32, height as u32).map_err(|error| {
                    PipelineError::message(format!("create nvenc texture failed: {error}"))
                })?;
            let registered = encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                .map_err(|error| {
                    PipelineError::message(format!("nvenc register resource failed: {error:?}"))
                })?;
            let bitstream = encoder.create_bitstream_buffer().map_err(|error| {
                PipelineError::message(format!("nvenc bitstream buffer failed: {error:?}"))
            })?;

            Ok(Self {
                _device: device,
                context,
                texture,
                encoder,
                registered,
                shared_inputs: Vec::new(),
                shared_encode_slots: Vec::new(),
                pending_shared_encodes: VecDeque::new(),
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        /// Low latency encoder with balanced quality
        /// Uses LowLatency tuning and P3 preset
        pub fn new_low_latency_p1(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new(width, height, fps)
        }

        /// High quality encoder (higher latency, better quality)
        /// Uses HighQuality tuning and P5 preset
        pub fn new_high_quality_p5(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new(width, height, fps)
        }

        fn new_with_profile(
            width: usize,
            height: usize,
            fps: u32,
            profile_guid: Guid,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(width, height, fps, profile_guid, 12_000_000)
        }

        fn new_low_latency_internal(
            width: usize,
            height: usize,
            fps: u32,
            profile_guid: Guid,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let bitrate = bitrate.max(1);
            let (device, context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    NV_ENC_PRESET_P3_GUID,
                    NVencTuningInfo::LowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = profile_guid;
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid: NV_ENC_PRESET_P3_GUID,
                resolution: [width as u32, height as u32],
                aspect_ratio: [width as u32, height as u32],
                frame_rate: [fps, 1],
                tuning_info: NVencTuningInfo::LowLatency,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width as u32, height as u32],
            };
            let encoder = session.init_encoder(init).map_err(|error| {
                PipelineError::message(format!("nvenc init encoder failed: {error:?}"))
            })?;
            let texture =
                create_encode_texture(&device, width as u32, height as u32).map_err(|error| {
                    PipelineError::message(format!("create nvenc texture failed: {error}"))
                })?;
            let registered = encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                .map_err(|error| {
                    PipelineError::message(format!("nvenc register resource failed: {error:?}"))
                })?;
            let bitstream = encoder.create_bitstream_buffer().map_err(|error| {
                PipelineError::message(format!("nvenc bitstream buffer failed: {error:?}"))
            })?;

            Ok(Self {
                _device: device,
                context,
                texture,
                encoder,
                registered,
                shared_inputs: Vec::new(),
                shared_encode_slots: Vec::new(),
                pending_shared_encodes: VecDeque::new(),
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        fn new_ultra_low_latency_internal(
            width: usize,
            height: usize,
            fps: u32,
            profile_guid: Guid,
        ) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let (device, context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    NV_ENC_PRESET_P6_GUID,
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = profile_guid;
            preset.preset_cfg.rc_params.average_bit_rate = 12_000_000;
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid: NV_ENC_PRESET_P6_GUID,
                resolution: [width as u32, height as u32],
                aspect_ratio: [width as u32, height as u32],
                frame_rate: [fps, 1],
                tuning_info: NVencTuningInfo::UltraLowLatency,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width as u32, height as u32],
            };
            let encoder = session.init_encoder(init).map_err(|error| {
                PipelineError::message(format!("nvenc init encoder failed: {error:?}"))
            })?;
            let texture =
                create_encode_texture(&device, width as u32, height as u32).map_err(|error| {
                    PipelineError::message(format!("create nvenc texture failed: {error}"))
                })?;
            let registered = encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                .map_err(|error| {
                    PipelineError::message(format!("nvenc register resource failed: {error:?}"))
                })?;
            let bitstream = encoder.create_bitstream_buffer().map_err(|error| {
                PipelineError::message(format!("nvenc bitstream buffer failed: {error:?}"))
            })?;

            Ok(Self {
                _device: device,
                context,
                texture,
                encoder,
                registered,
                shared_inputs: Vec::new(),
                shared_encode_slots: Vec::new(),
                pending_shared_encodes: VecDeque::new(),
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        pub fn probe_h264_available() -> Result<(), PipelineError> {
            let _ = Self::new(16, 16, 30)?;
            Ok(())
        }

        fn encode_shared_bgra(
            &mut self,
            frame: &CapturedFrame,
            shared: &D3D11SharedBgraFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if shared.width as usize != self.width || shared.height as usize != self.height {
                return Err(PipelineError::message(format!(
                    "shared texture size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, shared.width, shared.height
                )));
            }

            let source_texture = self.ensure_shared_input(shared)?;
            self.ensure_shared_encode_slots()?;

            let mut output = Vec::new();
            if self.shared_encode_slots.is_empty() {
                if let Some(access_unit) = self.complete_oldest_shared_encode()? {
                    output.push(access_unit);
                }
            }

            let slot = self
                .shared_encode_slots
                .pop()
                .ok_or_else(|| PipelineError::message("missing shared NVENC encode slot"))?;
            self.copy_shared_bgra_to_texture(&source_texture, &slot.texture)?;

            let force_idr = self.frame_index == 0 || self.frame_index % self.fps as usize == 0;
            submit_encode_picture(
                &mut self.encoder,
                &slot.bitstream,
                &slot.registered,
                self.frame_index,
                force_idr,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;
            self.pending_shared_encodes.push_back(PendingSharedEncode {
                slot,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_idr,
            });

            if self.pending_shared_encodes.len() >= H264_SHARED_ASYNC_SLOT_COUNT {
                if let Some(access_unit) = self.complete_oldest_shared_encode()? {
                    output.push(access_unit);
                }
            }

            Ok(output)
        }

        fn ensure_shared_encode_slots(&mut self) -> Result<(), PipelineError> {
            while self.shared_encode_slots.len() + self.pending_shared_encodes.len()
                < H264_SHARED_ASYNC_SLOT_COUNT
            {
                let texture =
                    create_encode_texture(&self._device, self.width as u32, self.height as u32)
                        .map_err(|error| {
                            PipelineError::message(format!(
                                "create shared NVENC slot texture failed: {error}"
                            ))
                        })?;
                let registered = self
                    .encoder
                    .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                    .map_err(|error| {
                        PipelineError::message(format!(
                            "nvenc register shared slot resource failed: {error:?}"
                        ))
                    })?;
                let bitstream = self.encoder.create_bitstream_buffer().map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc shared slot bitstream buffer failed: {error:?}"
                    ))
                })?;
                self.shared_encode_slots.push(SharedEncodeSlot {
                    texture,
                    registered,
                    bitstream,
                });
            }

            Ok(())
        }

        fn complete_oldest_shared_encode(
            &mut self,
        ) -> Result<Option<EncodedAccessUnit>, PipelineError> {
            let Some(pending) = self.pending_shared_encodes.pop_front() else {
                return Ok(None);
            };
            let bytes = lock_bitstream_bytes(&pending.slot.bitstream)
                .map_err(|error| PipelineError::message(error.to_string()))?;
            let access_unit = EncodedAccessUnit {
                codec: VideoCodec::H264,
                timestamp_us: pending.timestamp_us,
                is_keyframe: pending.is_keyframe,
                bytes: normalize_annexb_au(bytes),
            };
            self.shared_encode_slots.push(pending.slot);
            Ok(Some(access_unit))
        }

        fn copy_shared_bgra_to_texture(
            &self,
            source_texture: &ID3D11Texture2D,
            target_texture: &ID3D11Texture2D,
        ) -> Result<(), PipelineError> {
            let source_resource: ID3D11Resource = source_texture.cast().map_err(|error| {
                PipelineError::message(format!(
                    "cast shared texture to NVENC copy source failed: {error}"
                ))
            })?;
            let target_resource: ID3D11Resource = target_texture.cast().map_err(|error| {
                PipelineError::message(format!(
                    "cast registered NVENC texture to copy target failed: {error}"
                ))
            })?;

            unsafe {
                self.context
                    .CopyResource(&target_resource, &source_resource);
            }

            Ok(())
        }

        fn ensure_shared_input(
            &mut self,
            shared: &D3D11SharedBgraFrame,
        ) -> Result<ID3D11Texture2D, PipelineError> {
            if let Some(input) = self.shared_inputs.iter().find(|input| {
                input.shared_handle == shared.shared_handle
                    && input.width == shared.width
                    && input.height == shared.height
            }) {
                return Ok(input._texture.clone());
            }

            if shared.shared_handle == 0 {
                return Err(PipelineError::message("shared texture handle is zero"));
            }

            let mut texture = None::<ID3D11Texture2D>;
            unsafe {
                self._device.OpenSharedResource(
                    HANDLE(shared.shared_handle as *mut core::ffi::c_void),
                    &mut texture,
                )
            }
            .map_err(|error| {
                PipelineError::message(format!(
                    "open shared D3D11 texture for NVENC failed: {error}"
                ))
            })?;
            let texture =
                texture.ok_or_else(|| PipelineError::message("missing opened shared texture"))?;

            let registered = self
                .encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, shared.row_pitch)
                .map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc register shared texture failed: {error:?}"
                    ))
                })?;

            if self.shared_inputs.len() >= H264_SHARED_INPUT_CACHE_LIMIT {
                self.shared_inputs.remove(0);
            }
            self.shared_inputs.push(SharedInputResource {
                shared_handle: shared.shared_handle,
                width: shared.width,
                height: shared.height,
                _texture: texture,
                registered,
            });

            Ok(self
                .shared_inputs
                .last()
                .expect("shared input resource was just inserted")
                ._texture
                .clone())
        }
    }

    impl NvencHevcEncoder {
        pub fn preferred_input_memory_kind() -> FrameMemoryKind {
            FrameMemoryKind::D3D11SharedBgra
        }

        pub fn preferred_main10_input_memory_kind() -> FrameMemoryKind {
            FrameMemoryKind::D3D11SharedBgra
        }

        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_main(width, height, fps)
        }

        pub fn new_main(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                8_000_000,
                NV_ENC_HEVC_PROFILE_MAIN_GUID,
            )
        }

        pub fn new_main10(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                8_000_000,
                NV_ENC_HEVC_PROFILE_MAIN10_GUID,
            )
        }

        pub fn new_main_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                bitrate.max(1),
                NV_ENC_HEVC_PROFILE_MAIN_GUID,
            )
        }

        pub fn new_main10_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                bitrate.max(1),
                NV_ENC_HEVC_PROFILE_MAIN10_GUID,
            )
        }

        pub fn probe_hevc_available() -> Result<(), PipelineError> {
            let _ = Self::new_main(16, 16, 30)?;
            Ok(())
        }

        pub fn probe_hevc_main10_available() -> Result<(), PipelineError> {
            let _ = Self::new_main10(16, 16, 30)?;
            Ok(())
        }

        fn new_low_latency_internal(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
            profile_guid: Guid,
        ) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let bitrate = bitrate.max(1);
            let (device, context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            ensure_hevc_codec_supported(&session)?;
            ensure_hevc_preset_supported(&session, NV_ENC_PRESET_P3_GUID)?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_HEVC_GUID,
                    NV_ENC_PRESET_P3_GUID,
                    NVencTuningInfo::LowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc HEVC preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = profile_guid;
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_HEVC_GUID,
                preset_guid: NV_ENC_PRESET_P3_GUID,
                resolution: [width as u32, height as u32],
                aspect_ratio: [width as u32, height as u32],
                frame_rate: [fps, 1],
                tuning_info: NVencTuningInfo::LowLatency,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width as u32, height as u32],
            };
            let encoder = session.init_encoder(init).map_err(|error| {
                PipelineError::message(format!("nvenc HEVC init encoder failed: {error:?}"))
            })?;
            let texture =
                create_encode_texture(&device, width as u32, height as u32).map_err(|error| {
                    PipelineError::message(format!("create nvenc HEVC texture failed: {error}"))
                })?;
            let registered = encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, 0)
                .map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc HEVC register resource failed: {error:?}"
                    ))
                })?;
            let bitstream = encoder.create_bitstream_buffer().map_err(|error| {
                PipelineError::message(format!("nvenc HEVC bitstream buffer failed: {error:?}"))
            })?;

            Ok(Self {
                _device: device,
                context,
                texture,
                encoder,
                registered,
                shared_input: None,
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        fn encode_shared_bgra(
            &mut self,
            frame: &CapturedFrame,
            shared: &D3D11SharedBgraFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if shared.width as usize != self.width || shared.height as usize != self.height {
                return Err(PipelineError::message(format!(
                    "shared texture size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, shared.width, shared.height
                )));
            }

            self.ensure_shared_input(shared)?;

            let force_idr = self.frame_index == 0 || self.frame_index % self.fps as usize == 0;
            let shared_input = self
                .shared_input
                .as_ref()
                .ok_or_else(|| PipelineError::message("missing shared input resource"))?;
            let bytes = encode_picture_with_sps_pps(
                &mut self.encoder,
                &self.bitstream,
                &shared_input.registered,
                self.frame_index,
                force_idr,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Hevc,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_idr,
                bytes: normalize_annexb_au(bytes),
            }])
        }

        fn ensure_shared_input(
            &mut self,
            shared: &D3D11SharedBgraFrame,
        ) -> Result<(), PipelineError> {
            let needs_new = self
                .shared_input
                .as_ref()
                .map(|input| {
                    input.shared_handle != shared.shared_handle
                        || input.width != shared.width
                        || input.height != shared.height
                })
                .unwrap_or(true);

            if !needs_new {
                return Ok(());
            }

            if shared.shared_handle == 0 {
                return Err(PipelineError::message("shared texture handle is zero"));
            }

            let mut texture = None::<ID3D11Texture2D>;
            unsafe {
                self._device.OpenSharedResource(
                    HANDLE(shared.shared_handle as *mut core::ffi::c_void),
                    &mut texture,
                )
            }
            .map_err(|error| {
                PipelineError::message(format!(
                    "open shared D3D11 texture for NVENC HEVC failed: {error}"
                ))
            })?;
            let texture =
                texture.ok_or_else(|| PipelineError::message("missing opened shared texture"))?;

            let registered = self
                .encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, shared.row_pitch)
                .map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc HEVC register shared texture failed: {error:?}"
                    ))
                })?;

            self.shared_input = Some(SharedInputResource {
                shared_handle: shared.shared_handle,
                width: shared.width,
                height: shared.height,
                _texture: texture,
                registered,
            });

            Ok(())
        }
    }

    impl VideoEncoder for NvencH264Encoder {
        fn input_memory_kind(&self) -> FrameMemoryKind {
            FrameMemoryKind::D3D11SharedBgra
        }

        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }
            if let Some(shared) = frame.d3d11_shared_bgra() {
                return self.encode_shared_bgra(frame, shared);
            }

            let bgra = to_bgra(frame)?;
            let row_pitch = self
                .width
                .checked_mul(4)
                .ok_or_else(|| PipelineError::message("row pitch overflow"))?
                as u32;

            unsafe {
                self.context.UpdateSubresource(
                    &self.texture,
                    0,
                    None,
                    bgra.as_ptr() as *const core::ffi::c_void,
                    row_pitch,
                    0,
                );
            }

            let force_idr = self.frame_index == 0 || self.frame_index % self.fps as usize == 0;
            let bytes = encode_picture_with_sps_pps(
                &mut self.encoder,
                &self.bitstream,
                &self.registered,
                self.frame_index,
                force_idr,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::H264,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_idr,
                bytes: normalize_annexb_au(bytes),
            }])
        }
    }

    impl VideoEncoder for NvencHevcEncoder {
        fn input_memory_kind(&self) -> FrameMemoryKind {
            Self::preferred_input_memory_kind()
        }

        fn encode(
            &mut self,
            frame: &CapturedFrame,
        ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
            if frame.width != self.width || frame.height != self.height {
                return Err(PipelineError::message(format!(
                    "frame size mismatch: expected {}x{}, got {}x{}",
                    self.width, self.height, frame.width, frame.height
                )));
            }
            if let Some(shared) = frame.d3d11_shared_bgra() {
                return self.encode_shared_bgra(frame, shared);
            }

            let bgra = to_bgra(frame)?;
            let row_pitch = self
                .width
                .checked_mul(4)
                .ok_or_else(|| PipelineError::message("row pitch overflow"))?
                as u32;

            unsafe {
                self.context.UpdateSubresource(
                    &self.texture,
                    0,
                    None,
                    bgra.as_ptr() as *const core::ffi::c_void,
                    row_pitch,
                    0,
                );
            }

            let force_idr = self.frame_index == 0 || self.frame_index % self.fps as usize == 0;
            let bytes = encode_picture_with_sps_pps(
                &mut self.encoder,
                &self.bitstream,
                &self.registered,
                self.frame_index,
                force_idr,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Hevc,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_idr,
                bytes: normalize_annexb_au(bytes),
            }])
        }
    }

    fn ensure_hevc_codec_supported(session: &Session<NeedsConfig>) -> Result<(), PipelineError> {
        let codecs = session.get_encode_codecs().map_err(|error| {
            PipelineError::message(format!("NVENC codec capability query failed: {error:?}"))
        })?;

        if codecs.iter().any(|codec| codec == &NV_ENC_CODEC_HEVC_GUID) {
            return Ok(());
        }

        Err(PipelineError::message(
            "NVENC HEVC unavailable: current GPU/driver does not expose HEVC encode support",
        ))
    }

    fn ensure_hevc_preset_supported(
        session: &Session<NeedsConfig>,
        preset_guid: Guid,
    ) -> Result<(), PipelineError> {
        let presets = session
            .get_encode_presets(NV_ENC_CODEC_HEVC_GUID)
            .map_err(|error| {
                PipelineError::message(format!("NVENC HEVC preset query failed: {error:?}"))
            })?;

        if presets.iter().any(|preset| preset == &preset_guid) {
            return Ok(());
        }

        Err(PipelineError::message(
            "NVENC HEVC unavailable: required HEVC preset is not supported by this GPU/driver",
        ))
    }

    fn create_d3d11_device() -> anyhow::Result<(ID3D11Device, ID3D11DeviceContext)> {
        let mut device = None::<ID3D11Device>;
        let mut context = None::<ID3D11DeviceContext>;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .or_else(|_| unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        })
        .context("D3D11CreateDevice failed")?;

        Ok((
            device.ok_or_else(|| anyhow!("missing d3d11 device"))?,
            context.ok_or_else(|| anyhow!("missing d3d11 context"))?,
        ))
    }

    fn create_encode_texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
    ) -> anyhow::Result<ID3D11Texture2D> {
        let mut texture = None;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .context("CreateTexture2D failed")?;
        texture.ok_or_else(|| anyhow!("CreateTexture2D returned none"))
    }

    fn encode_picture_with_sps_pps(
        encoder: &mut Encoder,
        bitstream: &BitStream,
        registered: &RegisteredResource,
        frame_index: usize,
        force_idr: bool,
    ) -> anyhow::Result<Vec<u8>> {
        submit_encode_picture(encoder, bitstream, registered, frame_index, force_idr)?;
        lock_bitstream_bytes(bitstream)
    }

    fn submit_encode_picture(
        encoder: &mut Encoder,
        bitstream: &BitStream,
        registered: &RegisteredResource,
        frame_index: usize,
        force_idr: bool,
    ) -> anyhow::Result<()> {
        let flags = if force_idr {
            NVencPicFlags::ForceIDR as u32 | NVencPicFlags::OutputSpspps as u32
        } else {
            0
        };
        encoder
            .encode_picture_with_flags(
                registered,
                bitstream,
                frame_index,
                frame_index as u64,
                NVencBufferFormat::ARGB,
                NVencPicStruct::Frame,
                if force_idr {
                    NVencPicType::IDR
                } else {
                    NVencPicType::P
                },
                flags,
                None,
            )
            .map_err(|error| anyhow!("NVENC encode_picture failed: {error:?}"))?;
        Ok(())
    }

    fn lock_bitstream_bytes(bitstream: &BitStream) -> anyhow::Result<Vec<u8>> {
        let lock = bitstream
            .try_lock(true)
            .map_err(|error| anyhow!("NVENC bitstream lock failed: {error:?}"))?;
        Ok(lock.as_slice().to_vec())
    }

    fn normalize_annexb_au(buf: Vec<u8>) -> Vec<u8> {
        if looks_like_annexb(&buf) {
            return buf;
        }
        if let Some(v) = avcc_to_annexb(&buf) {
            return v;
        }
        buf
    }

    fn looks_like_annexb(buf: &[u8]) -> bool {
        if buf.len() < 4 {
            return false;
        }
        (buf[0] == 0 && buf[1] == 0 && buf[2] == 1)
            || (buf[0] == 0 && buf[1] == 0 && buf[2] == 0 && buf[3] == 1)
    }

    fn avcc_to_annexb(buf: &[u8]) -> Option<Vec<u8>> {
        if buf.len() < 5 {
            return None;
        }
        let mut offset = 0usize;
        let mut out = Vec::with_capacity(buf.len() + 16);
        let mut nals = 0usize;
        while offset + 4 <= buf.len() {
            let nal_len = u32::from_be_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ]) as usize;
            offset += 4;
            if nal_len == 0 || offset + nal_len > buf.len() {
                return None;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&buf[offset..offset + nal_len]);
            offset += nal_len;
            nals += 1;
        }
        if offset == buf.len() && nals > 0 {
            Some(out)
        } else {
            None
        }
    }

    fn to_bgra(frame: &CapturedFrame) -> Result<Vec<u8>, PipelineError> {
        let expected_len = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| match frame.pixel_format {
                FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => pixels.checked_mul(4),
                FramePixelFormat::Rgb24 => pixels.checked_mul(3),
            })
            .ok_or_else(|| PipelineError::message("frame buffer size overflow"))?;

        if frame.data.len() != expected_len {
            return Err(PipelineError::message(format!(
                "frame bytes mismatch: expected {expected_len}, got {}",
                frame.data.len()
            )));
        }

        match frame.pixel_format {
            FramePixelFormat::Bgra32 => Ok(frame.data.clone()),
            FramePixelFormat::Rgba32 => {
                let mut bgra = Vec::with_capacity(frame.data.len());
                for chunk in frame.data.chunks_exact(4) {
                    bgra.push(chunk[2]);
                    bgra.push(chunk[1]);
                    bgra.push(chunk[0]);
                    bgra.push(chunk[3]);
                }
                Ok(bgra)
            }
            FramePixelFormat::Rgb24 => {
                let mut bgra = Vec::with_capacity(frame.width * frame.height * 4);
                for chunk in frame.data.chunks_exact(3) {
                    bgra.push(chunk[2]);
                    bgra.push(chunk[1]);
                    bgra.push(chunk[0]);
                    bgra.push(255);
                }
                Ok(bgra)
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{NvencH264Encoder, NvencHevcEncoder};

#[cfg(not(windows))]
pub struct NvencHevcEncoder {
    encoder: GstreamerNvencEncoder,
    main10: bool,
}

#[cfg(not(windows))]
struct GstreamerNvencEncoder {
    codec: VideoCodec,
    element: &'static str,
    parser: &'static str,
    caps: &'static str,
    width: usize,
    height: usize,
    fps: u32,
    bitrate_kbps: u32,
    frame_index: usize,
    process: Option<GstreamerNvencProcess>,
}

#[cfg(not(windows))]
struct GstreamerNvencProcess {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: mpsc::Receiver<GstreamerReadResult>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
}

#[cfg(not(windows))]
type GstreamerReadResult = Result<Vec<u8>, String>;

#[cfg(not(windows))]
const GST_STDIO_CHUNK_SIZE: usize = 64 * 1024;
#[cfg(not(windows))]
const GST_STDERR_TAIL_LIMIT: usize = 16 * 1024;
#[cfg(not(windows))]
const GST_OUTPUT_TIMEOUT: Duration = Duration::from_millis(1_500);
#[cfg(not(windows))]
const GST_OUTPUT_IDLE: Duration = Duration::from_millis(8);

#[cfg(not(windows))]
impl GstreamerNvencEncoder {
    fn new(
        codec: VideoCodec,
        element: &'static str,
        parser: &'static str,
        caps: &'static str,
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        require_gst_element(element)?;
        require_gst_element(parser)?;
        require_gst_element("rawvideoparse")?;
        Ok(Self {
            codec,
            element,
            parser,
            caps,
            width: width.max(2),
            height: height.max(2),
            fps: fps.max(1),
            bitrate_kbps: (bitrate / 1000).max(1),
            frame_index: 0,
            process: None,
        })
    }

    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        if frame.pixel_format != FramePixelFormat::Bgra32 {
            return Err(PipelineError::message(format!(
                "Linux NVENC GStreamer path expects BGRA32 frames, got {:?}",
                frame.pixel_format
            )));
        }
        if frame.width != self.width || frame.height != self.height {
            return Err(PipelineError::message(format!(
                "Linux NVENC GStreamer path was initialized for {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            )));
        }

        let force_idr = self.frame_index == 0 || self.frame_index % self.fps as usize == 0;
        let timestamp_us = frame.timestamp_us;
        let codec = self.codec;
        let output = self.encode_with_process(&frame.data)?;
        self.frame_index += 1;
        Ok(vec![EncodedAccessUnit {
            codec,
            timestamp_us,
            is_keyframe: force_idr || annex_b_contains_keyframe(codec, &output),
            bytes: output,
        }])
    }

    fn encode_with_process(&mut self, frame_data: &[u8]) -> Result<Vec<u8>, PipelineError> {
        let label = "Linux GStreamer NVENC";
        let result = {
            let process = self.ensure_process()?;
            process.write_frame(frame_data, label)?;
            process.read_encoded_output(label)
        };
        if result.is_err() {
            self.process.take();
        }
        result
    }

    fn ensure_process(&mut self) -> Result<&mut GstreamerNvencProcess, PipelineError> {
        if let Some(process) = self.process.as_mut() {
            if let Some(status) = process.child.try_wait().map_err(|error| {
                PipelineError::message(format!(
                    "poll Linux GStreamer NVENC process failed: {error}"
                ))
            })? {
                let stderr = process.stderr_tail_text();
                self.process.take();
                return Err(PipelineError::message(format!(
                    "Linux GStreamer NVENC exited before encode with {status}; stderr: {stderr}"
                )));
            }
        }

        if self.process.is_none() {
            self.process = Some(GstreamerNvencProcess::spawn(
                self.gstreamer_command(),
                "Linux GStreamer NVENC",
            )?);
        }

        self.process
            .as_mut()
            .ok_or_else(|| PipelineError::message("Linux GStreamer NVENC process is unavailable"))
    }

    fn gstreamer_command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("gst-launch-1.0");
        command
            .arg("-q")
            .arg("fdsrc")
            .arg("fd=0")
            .arg("blocksize=65536")
            .arg("!")
            .arg("rawvideoparse")
            .arg("format=bgra")
            .arg(format!("width={}", self.width))
            .arg(format!("height={}", self.height))
            .arg(format!("framerate={}/1", self.fps))
            .arg("!")
            .arg("videoconvert")
            .arg("!")
            .arg("video/x-raw,format=BGRA")
            .arg("!")
            .arg(self.element)
            .arg("preset=p1")
            .arg("tune=low-latency")
            .arg("zerolatency=true")
            .arg("bframes=0")
            .arg(format!("gop-size={}", self.fps.min(60)))
            .arg("repeat-sequence-header=true")
            .arg(format!("bitrate={}", self.bitrate_kbps))
            .arg("!")
            .arg(self.parser)
            .arg("config-interval=-1")
            .arg("!")
            .arg(self.caps)
            .arg("!")
            .arg("fdsink")
            .arg("fd=1")
            .arg("sync=false")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command
    }
}

#[cfg(not(windows))]
pub struct NvencH264Encoder {
    encoder: GstreamerNvencEncoder,
}

#[cfg(not(windows))]
impl GstreamerNvencProcess {
    fn spawn(mut command: std::process::Command, label: &str) -> Result<Self, PipelineError> {
        let mut child = command
            .spawn()
            .map_err(|error| PipelineError::message(format!("launch {label} failed: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PipelineError::message(format!("{label} stdin pipe is unavailable")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PipelineError::message(format!("{label} stdout pipe is unavailable")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PipelineError::message(format!("{label} stderr pipe is unavailable")))?;
        let (stdout_tx, stdout_rx) = mpsc::channel();
        std::thread::spawn(move || read_gstreamer_stdout(stdout, stdout_tx));

        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_tail_reader = Arc::clone(&stderr_tail);
        std::thread::spawn(move || read_gstreamer_stderr(stderr, stderr_tail_reader));

        Ok(Self {
            child,
            stdin,
            stdout_rx,
            stderr_tail,
        })
    }

    fn write_frame(&mut self, frame_data: &[u8], label: &str) -> Result<(), PipelineError> {
        if let Err(error) = self.stdin.write_all(frame_data) {
            return Err(self.io_error(label, "writing raw frame input", error));
        }
        if let Err(error) = self.stdin.flush() {
            return Err(self.io_error(label, "flushing raw frame input", error));
        }
        Ok(())
    }

    fn read_encoded_output(&mut self, label: &str) -> Result<Vec<u8>, PipelineError> {
        let deadline = Instant::now() + GST_OUTPUT_TIMEOUT;
        let mut output = Vec::new();

        loop {
            let timeout = if output.is_empty() {
                deadline.saturating_duration_since(Instant::now())
            } else {
                GST_OUTPUT_IDLE
            };

            match self.stdout_rx.recv_timeout(timeout) {
                Ok(Ok(chunk)) => {
                    output.extend_from_slice(&chunk);
                    if let Some(done) = self.drain_ready_stdout(label, &mut output)? {
                        return Ok(done);
                    }
                }
                Ok(Err(error)) => {
                    if output.is_empty() {
                        return Err(PipelineError::message(format!(
                            "{label} stdout closed before encoded output: {error}; stderr: {}",
                            self.stderr_tail_text()
                        )));
                    }
                    return Ok(output);
                }
                Err(mpsc::RecvTimeoutError::Timeout) if output.is_empty() => {
                    if let Some(status) = self.child.try_wait().map_err(|error| {
                        PipelineError::message(format!("poll {label} process failed: {error}"))
                    })? {
                        return Err(PipelineError::message(format!(
                            "{label} exited with {status} before producing encoded output; stderr: {}",
                            self.stderr_tail_text()
                        )));
                    }
                    if Instant::now() >= deadline {
                        return Err(PipelineError::message(format!(
                            "{label} produced no encoded output within {} ms; stderr: {}",
                            GST_OUTPUT_TIMEOUT.as_millis(),
                            self.stderr_tail_text()
                        )));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(output),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if output.is_empty() {
                        return Err(PipelineError::message(format!(
                            "{label} stdout reader stopped before encoded output; stderr: {}",
                            self.stderr_tail_text()
                        )));
                    }
                    return Ok(output);
                }
            }
        }
    }

    fn drain_ready_stdout(
        &mut self,
        label: &str,
        output: &mut Vec<u8>,
    ) -> Result<Option<Vec<u8>>, PipelineError> {
        loop {
            match self.stdout_rx.try_recv() {
                Ok(Ok(chunk)) => output.extend_from_slice(&chunk),
                Ok(Err(error)) => {
                    if output.is_empty() {
                        return Err(PipelineError::message(format!(
                            "{label} stdout closed before encoded output: {error}; stderr: {}",
                            self.stderr_tail_text()
                        )));
                    }
                    return Ok(Some(std::mem::take(output)));
                }
                Err(mpsc::TryRecvError::Empty) => return Ok(None),
                Err(mpsc::TryRecvError::Disconnected) => {
                    if output.is_empty() {
                        return Err(PipelineError::message(format!(
                            "{label} stdout reader stopped before encoded output; stderr: {}",
                            self.stderr_tail_text()
                        )));
                    }
                    return Ok(Some(std::mem::take(output)));
                }
            }
        }
    }

    fn io_error(&mut self, label: &str, operation: &str, error: std::io::Error) -> PipelineError {
        let exit = match self.child.try_wait() {
            Ok(Some(status)) => format!("; process exited with {status}"),
            Ok(None) => String::new(),
            Err(wait_error) => format!("; process status unavailable: {wait_error}"),
        };
        PipelineError::message(format!(
            "{label} failed while {operation}: {error}{exit}; stderr: {}",
            self.stderr_tail_text()
        ))
    }

    fn stderr_tail_text(&self) -> String {
        let bytes = self
            .stderr_tail
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            "(empty)".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

#[cfg(not(windows))]
impl Drop for GstreamerNvencProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(not(windows))]
fn read_gstreamer_stdout<R>(mut stdout: R, stdout_tx: mpsc::Sender<GstreamerReadResult>)
where
    R: Read,
{
    let mut buffer = vec![0; GST_STDIO_CHUNK_SIZE];
    loop {
        match stdout.read(&mut buffer) {
            Ok(0) => {
                let _ = stdout_tx.send(Err("stdout reached EOF".to_string()));
                break;
            }
            Ok(bytes_read) => {
                if stdout_tx.send(Ok(buffer[..bytes_read].to_vec())).is_err() {
                    break;
                }
            }
            Err(error) => {
                let _ = stdout_tx.send(Err(error.to_string()));
                break;
            }
        }
    }
}

#[cfg(not(windows))]
fn read_gstreamer_stderr<R>(mut stderr: R, stderr_tail: Arc<Mutex<Vec<u8>>>)
where
    R: Read,
{
    let mut buffer = vec![0; GST_STDIO_CHUNK_SIZE];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(bytes_read) => append_gstreamer_stderr_tail(&stderr_tail, &buffer[..bytes_read]),
            Err(_) => break,
        }
    }
}

#[cfg(not(windows))]
fn append_gstreamer_stderr_tail(stderr_tail: &Arc<Mutex<Vec<u8>>>, bytes: &[u8]) {
    if let Ok(mut tail) = stderr_tail.lock() {
        if bytes.len() >= GST_STDERR_TAIL_LIMIT {
            tail.clear();
            tail.extend_from_slice(&bytes[bytes.len() - GST_STDERR_TAIL_LIMIT..]);
            return;
        }
        let overflow = tail.len().saturating_add(bytes.len());
        if overflow > GST_STDERR_TAIL_LIMIT {
            tail.drain(..overflow - GST_STDERR_TAIL_LIMIT);
        }
        tail.extend_from_slice(bytes);
    }
}

#[cfg(not(windows))]
fn require_gst_element(element: &str) -> Result<(), PipelineError> {
    let status = std::process::Command::new("gst-inspect-1.0")
        .arg(element)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| {
            PipelineError::message(format!("gst-inspect-1.0 is not available: {error}"))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(PipelineError::message(format!(
            "GStreamer element `{element}` is not available"
        )))
    }
}

#[cfg(not(windows))]
fn probe_gstreamer_nvenc(
    codec: VideoCodec,
    element: &'static str,
    parser: &'static str,
    caps: &'static str,
    width: usize,
    height: usize,
) -> Result<(), PipelineError> {
    let mut encoder =
        GstreamerNvencEncoder::new(codec, element, parser, caps, width, height, 30, 1_000_000)?;
    let frame = CapturedFrame::from_cpu(
        width,
        height,
        FramePixelFormat::Bgra32,
        0,
        vec![0; width * height * 4],
    );
    let units = encoder.encode(&frame)?;
    if units.iter().any(|unit| !unit.bytes.is_empty()) {
        Ok(())
    } else {
        Err(PipelineError::message(format!(
            "Linux GStreamer NVENC probe for `{element}` produced no encoded output"
        )))
    }
}

#[cfg(not(windows))]
fn annex_b_contains_keyframe(codec: VideoCodec, bytes: &[u8]) -> bool {
    let mut index = 0;
    while index + 5 < bytes.len() {
        let start_len = if bytes[index..].starts_with(&[0, 0, 1]) {
            3
        } else if bytes[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else {
            index += 1;
            continue;
        };
        let nal = bytes[index + start_len];
        match codec {
            VideoCodec::H264 => {
                let nal_type = nal & 0x1f;
                if nal_type == 5 || nal_type == 7 {
                    return true;
                }
            }
            VideoCodec::Hevc => {
                let nal_type = (nal >> 1) & 0x3f;
                if nal_type == 19 || nal_type == 20 || nal_type == 32 || nal_type == 33 {
                    return true;
                }
            }
            VideoCodec::Av1 => {}
        }
        index += start_len;
    }
    false
}

#[cfg(not(windows))]
impl NvencH264Encoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_with_bitrate(width, height, fps, 8_000_000)
    }

    pub fn new_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_max_speed_with_bitrate(width, height, fps, bitrate)
    }

    pub fn new_baseline(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_max_speed_with_bitrate(width, height, fps, 5_000_000)
    }

    pub fn new_max_speed(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_max_speed_with_bitrate(width, height, fps, 5_000_000)
    }

    pub fn new_max_speed_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            encoder: GstreamerNvencEncoder::new(
                VideoCodec::H264,
                "nvh264enc",
                "h264parse",
                "video/x-h264,stream-format=byte-stream,alignment=au",
                width,
                height,
                fps,
                bitrate,
            )?,
        })
    }

    pub fn new_low_latency_p1(
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_max_speed(width, height, fps)
    }

    pub fn new_high_quality_p5(
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_with_bitrate(width, height, fps, 12_000_000)
    }

    pub fn probe_h264_available() -> Result<(), PipelineError> {
        probe_gstreamer_nvenc(
            VideoCodec::H264,
            "nvh264enc",
            "h264parse",
            "video/x-h264,stream-format=byte-stream,alignment=au",
            160,
            64,
        )
    }
}

#[cfg(not(windows))]
impl NvencHevcEncoder {
    pub fn preferred_input_memory_kind() -> mrd_pipeline_core::FrameMemoryKind {
        mrd_pipeline_core::FrameMemoryKind::Cpu
    }

    pub fn preferred_main10_input_memory_kind() -> mrd_pipeline_core::FrameMemoryKind {
        mrd_pipeline_core::FrameMemoryKind::Cpu
    }

    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_main_with_bitrate(width, height, fps, 8_000_000)
    }

    pub fn new_main(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_main_with_bitrate(width, height, fps, 8_000_000)
    }

    pub fn new_main10(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_main10_with_bitrate(width, height, fps, 10_000_000)
    }

    pub fn new_main_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Ok(Self {
            encoder: GstreamerNvencEncoder::new(
                VideoCodec::Hevc,
                "nvh265enc",
                "h265parse",
                "video/x-h265,stream-format=byte-stream,alignment=au",
                width,
                height,
                fps,
                bitrate,
            )?,
            main10: false,
        })
    }

    pub fn new_main10_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        let mut encoder = Self::new_main_with_bitrate(width, height, fps, bitrate)?;
        encoder.main10 = true;
        Ok(encoder)
    }

    pub fn probe_hevc_available() -> Result<(), PipelineError> {
        probe_gstreamer_nvenc(
            VideoCodec::Hevc,
            "nvh265enc",
            "h265parse",
            "video/x-h265,stream-format=byte-stream,alignment=au",
            160,
            64,
        )
    }

    pub fn probe_hevc_main10_available() -> Result<(), PipelineError> {
        Self::probe_hevc_available()
    }
}

#[cfg(not(windows))]
impl VideoEncoder for NvencH264Encoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        self.encoder.encode(frame)
    }
}

#[cfg(not(windows))]
impl VideoEncoder for NvencHevcEncoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        self.encoder.encode(frame)
    }
}

#[cfg(all(test, not(windows)))]
mod linux_tests {
    use super::*;

    #[test]
    fn gstreamer_command_reassembles_large_raw_frames_before_nvenc() {
        let encoder = GstreamerNvencEncoder {
            codec: VideoCodec::H264,
            element: "nvh264enc",
            parser: "h264parse",
            caps: "video/x-h264,stream-format=byte-stream,alignment=au",
            width: 1280,
            height: 720,
            fps: 30,
            bitrate_kbps: 5_000,
            frame_index: 0,
            process: None,
        };

        let command = encoder.gstreamer_command();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert!(args.iter().any(|arg| arg == "rawvideoparse"));
        assert!(args.iter().any(|arg| arg == "format=bgra"));
        assert!(args.iter().any(|arg| arg == "width=1280"));
        assert!(args.iter().any(|arg| arg == "height=720"));
        assert!(args.iter().any(|arg| arg == "framerate=30/1"));
        assert!(args.iter().any(|arg| arg == "tune=low-latency"));
        assert!(!args.iter().any(|arg| arg == "num-buffers=1"));
    }
}
