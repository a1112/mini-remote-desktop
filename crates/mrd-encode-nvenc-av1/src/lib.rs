//! NVENC AV1 encoder implementation
//!
//! This encoder uses NVIDIA's AV1 encoding hardware (available on Ampere and newer GPUs).
//! AV1 provides better compression efficiency than H.264 at the same quality.

#[cfg(not(windows))]
compile_error!("mrd-encode-nvenc-av1 is only supported on Windows");

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
        NV_ENC_AV1_PROFILE_MAIN_GUID, NV_ENC_CODEC_AV1_GUID, NV_ENC_PRESET_P3_GUID,
        NV_ENC_PRESET_P6_GUID,
    };
    use nvenc::sys::structs::Guid;
    use windows::Win32::Foundation::{HANDLE, HMODULE};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    pub struct NvencAv1Encoder {
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

    unsafe impl Send for NvencAv1Encoder {}

    struct SharedInputResource {
        shared_handle: isize,
        width: u32,
        height: u32,
        _texture: ID3D11Texture2D,
        registered: RegisteredResource,
    }

    impl NvencAv1Encoder {
        pub fn preferred_input_memory_kind() -> FrameMemoryKind {
            FrameMemoryKind::D3D11SharedBgra
        }

        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            Self::new_with_profile(width, height, fps, NV_ENC_AV1_PROFILE_MAIN_GUID)
        }

        pub fn new_low_latency(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new(width, height, fps)
        }

        /// Ultra-low latency AV1 encoder for remote desktop scenarios
        /// Uses UltraLowLatency tuning and P6 preset for minimum latency
        pub fn new_ultra_low_latency(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_ultra_low_latency_internal(width, height, fps, NV_ENC_AV1_PROFILE_MAIN_GUID)
        }

        /// High refresh rate AV1 encoder (120Hz+) optimized for minimum latency
        /// Lower bitrate and shorter GOP for maximum speed
        pub fn new_high_refresh_rate(
            width: usize,
            height: usize,
            fps: u32,
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
            ensure_av1_codec_supported(&session)?;
            ensure_av1_preset_supported(&session, NV_ENC_PRESET_P6_GUID)?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_AV1_GUID,
                    NV_ENC_PRESET_P6_GUID,
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = NV_ENC_AV1_PROFILE_MAIN_GUID;
            // Lower bitrate for faster encoding
            preset.preset_cfg.rc_params.average_bit_rate = 6_000_000;
            preset.preset_cfg.frame_interval_p = 1;
            // Shorter GOP for high refresh rate
            preset.preset_cfg.gop_len = fps.min(60);

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_AV1_GUID,
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
                shared_input: None,
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        fn new_with_profile(
            width: usize,
            height: usize,
            fps: u32,
            profile_guid: Guid,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(width, height, fps, profile_guid)
        }

        fn new_low_latency_internal(
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
            ensure_av1_codec_supported(&session)?;
            ensure_av1_preset_supported(&session, NV_ENC_PRESET_P3_GUID)?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_AV1_GUID,
                    NV_ENC_PRESET_P3_GUID,
                    NVencTuningInfo::LowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = profile_guid;
            // AV1 specific bitrate settings (may need adjustment based on quality requirements)
            preset.preset_cfg.rc_params.average_bit_rate = 8_000_000; // 8 Mbps default
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps * 2; // AV1 can use longer GOP

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_AV1_GUID,
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
                shared_input: None,
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
            ensure_av1_codec_supported(&session)?;
            ensure_av1_preset_supported(&session, NV_ENC_PRESET_P6_GUID)?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_AV1_GUID,
                    NV_ENC_PRESET_P6_GUID,
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = profile_guid;
            // Ultra-low latency AV1 settings - lower bitrate for faster encoding
            preset.preset_cfg.rc_params.average_bit_rate = 6_000_000;
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps * 2;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_AV1_GUID,
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
                shared_input: None,
                bitstream,
                width,
                height,
                fps,
                frame_index: 0,
            })
        }

        pub fn probe_av1_available() -> Result<(), PipelineError> {
            // Simple probe: try to open NVENC session and check AV1 codec support
            let (device, _context) = create_d3d11_device().map_err(|error| {
                PipelineError::message(format!("create d3d11 device failed: {error}"))
            })?;

            let session: Session<NeedsConfig> = Session::open_dx(&device).map_err(|error| {
                PipelineError::message(format!("nvenc open_dx failed: {error:?}"))
            })?;
            ensure_av1_codec_supported(&session)?;
            ensure_av1_preset_supported(&session, NV_ENC_PRESET_P3_GUID)?;

            // Try to get preset config for AV1 - this will fail if AV1 is not supported
            let _ = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_AV1_GUID,
                    NV_ENC_PRESET_P3_GUID,
                    NVencTuningInfo::LowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("AV1 codec not supported: {error:?}"))
                })?;

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

            self.ensure_shared_input(shared)?;

            let force_key =
                self.frame_index == 0 || self.frame_index % (self.fps as usize * 2) == 0;
            let shared_input = self
                .shared_input
                .as_ref()
                .ok_or_else(|| PipelineError::message("missing shared input resource"))?;
            let bytes = encode_picture(
                &mut self.encoder,
                &self.bitstream,
                &shared_input.registered,
                self.frame_index,
                force_key,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Av1,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_key,
                bytes,
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
                    "open shared D3D11 texture for NVENC AV1 failed: {error}"
                ))
            })?;
            let texture =
                texture.ok_or_else(|| PipelineError::message("missing opened shared texture"))?;

            let registered = self
                .encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, shared.row_pitch)
                .map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc AV1 register shared texture failed: {error:?}"
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

    impl VideoEncoder for NvencAv1Encoder {
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

            // AV1 uses key frames instead of IDR frames
            let force_key =
                self.frame_index == 0 || self.frame_index % (self.fps as usize * 2) == 0;
            let bytes = encode_picture(
                &mut self.encoder,
                &self.bitstream,
                &self.registered,
                self.frame_index,
                force_key,
            )
            .map_err(|error| PipelineError::message(error.to_string()))?;
            self.frame_index += 1;

            Ok(vec![EncodedAccessUnit {
                codec: VideoCodec::Av1,
                timestamp_us: frame.timestamp_us,
                is_keyframe: force_key,
                bytes, // AV1 uses OBUs, no Annex-B conversion needed
            }])
        }
    }

    fn ensure_av1_codec_supported(session: &Session<NeedsConfig>) -> Result<(), PipelineError> {
        let codecs = session.get_encode_codecs().map_err(|error| {
            PipelineError::message(format!("NVENC codec capability query failed: {error:?}"))
        })?;

        if codecs.iter().any(|codec| codec == &NV_ENC_CODEC_AV1_GUID) {
            return Ok(());
        }

        Err(PipelineError::message(
            "NVENC AV1 unavailable: current GPU/driver does not expose AV1 encode support",
        ))
    }

    fn ensure_av1_preset_supported(
        session: &Session<NeedsConfig>,
        preset_guid: Guid,
    ) -> Result<(), PipelineError> {
        let presets = session
            .get_encode_presets(NV_ENC_CODEC_AV1_GUID)
            .map_err(|error| {
                PipelineError::message(format!("NVENC AV1 preset query failed: {error:?}"))
            })?;

        if presets.iter().any(|preset| preset == &preset_guid) {
            return Ok(());
        }

        Err(PipelineError::message(
            "NVENC AV1 unavailable: required AV1 preset is not supported by this GPU/driver",
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

    fn encode_picture(
        encoder: &mut Encoder,
        bitstream: &BitStream,
        registered: &RegisteredResource,
        frame_index: usize,
        force_key: bool,
    ) -> anyhow::Result<Vec<u8>> {
        let encode_pic_flags = if force_key {
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
                if force_key {
                    NVencPicType::IDR // AV1 also uses IDR for key frames in NVENC
                } else {
                    NVencPicType::P
                },
                encode_pic_flags,
                None,
            )
            .map_err(|error| anyhow!("NVENC encode_picture failed: {error:?}"))?;
        let lock = bitstream
            .try_lock(true)
            .map_err(|error| anyhow!("NVENC bitstream lock failed: {error:?}"))?;
        Ok(lock.as_slice().to_vec())
    }

    fn to_bgra(frame: &CapturedFrame) -> Result<Vec<u8>, PipelineError> {
        let expected_len = frame
            .width
            .checked_mul(frame.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| PipelineError::message("frame size overflow"))?;

        let bgra = match frame.pixel_format {
            FramePixelFormat::Bgra32 => frame.data.clone(),
            FramePixelFormat::Rgba32 => {
                let mut bgra = vec![0_u8; expected_len];
                for i in (0..expected_len).step_by(4) {
                    bgra[i] = frame.data[i + 2]; // R -> B
                    bgra[i + 1] = frame.data[i + 1]; // G
                    bgra[i + 2] = frame.data[i]; // B -> R
                    bgra[i + 3] = frame.data[i + 3]; // A
                }
                bgra
            }
            FramePixelFormat::Rgb24 => {
                let mut bgra = vec![0_u8; expected_len];
                for i in 0..frame.data.len() / 3 {
                    let src_offset = i * 3;
                    let dst_offset = i * 4;
                    bgra[dst_offset] = frame.data[src_offset + 2]; // R -> B
                    bgra[dst_offset + 1] = frame.data[src_offset + 1]; // G
                    bgra[dst_offset + 2] = frame.data[src_offset]; // B -> R
                    bgra[dst_offset + 3] = 255; // A
                }
                bgra
            }
        };

        Ok(bgra)
    }
}

pub use imp::NvencAv1Encoder;

#[cfg(all(test, windows))]
mod tests {
    use super::NvencAv1Encoder;
    use mrd_pipeline_core::FrameMemoryKind;

    #[test]
    fn av1_encoder_prefers_d3d11_shared_bgra_input() {
        assert_eq!(
            NvencAv1Encoder::preferred_input_memory_kind(),
            FrameMemoryKind::D3D11SharedBgra
        );
    }
}

#[cfg(not(windows))]
pub struct NvencAv1Encoder;

#[cfg(not(windows))]
impl VideoEncoder for NvencAv1Encoder {
    fn encode(&mut self, _frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        Err(PipelineError::message(
            "NVENC AV1 encoder is only supported on Windows",
        ))
    }
}
