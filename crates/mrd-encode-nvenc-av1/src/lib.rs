//! NVENC AV1 encoder implementation
//!
//! This encoder requires a GPU and driver that expose NVENC AV1 encoding support.
//! AV1 provides better compression efficiency than H.264 at the same quality.

#[cfg(not(windows))]
use mrd_pipeline_core::{
    CapturedFrame, EncodedAccessUnit, FramePixelFormat, PipelineError, VideoCodec, VideoEncoder,
};

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
        NV_ENC_AV1_PROFILE_MAIN_GUID, NV_ENC_CODEC_AV1_GUID, NV_ENC_PRESET_P1_GUID,
        NV_ENC_PRESET_P3_GUID, NV_ENC_PRESET_P6_GUID,
    };
    use nvenc::sys::structs::Guid;
    use std::borrow::Cow;
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
        native: RetainedOnFailure<NativeResources>,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: usize,
    }

    struct NativeResources {
        // Drop NVENC buffers/registrations before their D3D11 backing
        // resources. Rust drops struct fields in declaration order.
        bitstream: BitStream,
        shared_input: Option<SharedInputResource>,
        registered: RegisteredResource,
        encoder: Encoder,
        texture: ID3D11Texture2D,
        context: ID3D11DeviceContext,
        _device: ID3D11Device,
    }

    unsafe impl Send for NvencAv1Encoder {}

    struct SharedInputResource {
        shared_handle: isize,
        width: u32,
        height: u32,
        registered: RegisteredResource,
        _texture: ID3D11Texture2D,
    }

    struct RetainedOnFailure<T> {
        resource: Option<T>,
        pending: bool,
    }

    impl<T> RetainedOnFailure<T> {
        fn new(resource: T) -> Self {
            Self {
                resource: Some(resource),
                pending: false,
            }
        }

        fn ensure_idle(&self) -> Result<(), PipelineError> {
            if self.pending {
                return Err(PipelineError::message(
                    "previous NVENC AV1 input has not completed",
                ));
            }
            Ok(())
        }

        fn idle_mut(&mut self) -> Result<&mut T, PipelineError> {
            self.ensure_idle()?;
            Ok(self.resource.as_mut().expect("active AV1 resources"))
        }

        fn run<O>(
            &mut self,
            action: impl FnOnce(&mut T) -> anyhow::Result<O>,
        ) -> anyhow::Result<O> {
            self.ensure_idle()?;
            self.pending = true;
            let output = action(self.resource.as_mut().expect("active AV1 resources"))?;
            self.pending = false;
            Ok(output)
        }
    }

    impl<T> Drop for RetainedOnFailure<T> {
        fn drop(&mut self) {
            if self.pending {
                // No successful completion is known. Preserve the entire native
                // owner, including its encoder Arc and both input textures.
                if let Some(unfinished) = self.resource.take() {
                    std::mem::forget(unfinished);
                }
                use std::io::Write;
                let _ = writeln!(std::io::stderr(), "NVENC AV1 failed before completion; native resources retained until process exit");
            }
        }
    }

    fn complete_av1_output<O, E>(
        lock_copy_unlock: impl FnOnce() -> Result<O, E>,
        unmap: impl FnOnce() -> Result<(), E>,
    ) -> Result<O, E> {
        let output = lock_copy_unlock()?;
        unmap()?;
        Ok(output)
    }

    fn prepare_cpu_input<E>(
        unmap: impl FnOnce() -> Result<(), E>,
        upload: impl FnOnce(),
    ) -> Result<(), E> {
        unmap()?;
        upload();
        Ok(())
    }

    #[cfg(test)]
    mod lifecycle_tests {
        use super::*;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct NativeOwner(Arc<AtomicUsize>);
        impl Drop for NativeOwner {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        #[test]
        fn initial_mapping_is_released_before_cpu_upload() {
            let calls = std::cell::RefCell::new(Vec::new());
            prepare_cpu_input(
                || {
                    calls.borrow_mut().push("unmap");
                    Ok::<_, &str>(())
                },
                || calls.borrow_mut().push("upload"),
            )
            .unwrap();
            assert_eq!(*calls.borrow(), ["unmap", "upload"]);
        }

        #[test]
        fn initial_unmap_failure_prevents_upload_and_retains_native_owner() {
            let drops = Arc::new(AtomicUsize::new(0));
            let mut owner = RetainedOnFailure::new(NativeOwner(drops.clone()));
            let uploaded = std::cell::Cell::new(false);
            assert!(owner
                .run(|_| prepare_cpu_input(
                    || Err(anyhow!("initial unmap failed")),
                    || uploaded.set(true)
                ))
                .is_err());
            assert!(!uploaded.get());
            assert!(owner.ensure_idle().is_err());
            drop(owner);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
        }

        #[test]
        fn submission_or_completion_failure_poison_and_retain_native_owner() {
            for phase in ["submit", "lock", "unlock", "unmap"] {
                let drops = Arc::new(AtomicUsize::new(0));
                let mut state = RetainedOnFailure::new(NativeOwner(drops.clone()));
                assert!(state.run::<()>(|_| Err(anyhow!("{phase} failed"))).is_err());
                assert!(state.ensure_idle().is_err());
                assert!(state.idle_mut().is_err());
                assert!(state.run(|_| Ok(())).is_err());
                drop(state);
                assert_eq!(drops.load(Ordering::SeqCst), 0, "{phase}");
            }
        }

        #[test]
        fn successful_completion_reuses_and_releases_native_owner() {
            let drops = Arc::new(AtomicUsize::new(0));
            let mut state = RetainedOnFailure::new(NativeOwner(drops.clone()));
            state.run(|_| Ok(())).unwrap();
            state.run(|_| Ok(())).unwrap();
            assert!(state.idle_mut().is_ok());
            drop(state);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn output_unlock_precedes_input_unmap() {
            let calls = std::cell::RefCell::new(Vec::new());
            let bytes = complete_av1_output(
                || {
                    calls.borrow_mut().extend(["lock", "copy", "unlock"]);
                    Ok::<_, &str>(vec![1])
                },
                || {
                    calls.borrow_mut().push("unmap");
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(bytes, [1]);
            assert_eq!(*calls.borrow(), ["lock", "copy", "unlock", "unmap"]);
        }

        #[test]
        fn failed_lock_or_unlock_does_not_unmap_input() {
            let unmapped = std::cell::Cell::new(false);
            assert_eq!(
                complete_av1_output(
                    || Err::<(), _>("unlock failed"),
                    || {
                        unmapped.set(true);
                        Ok(())
                    }
                ),
                Err("unlock failed")
            );
            assert!(!unmapped.get());
        }

        #[test]
        fn failed_unmap_is_reported_to_resource_owner() {
            assert_eq!(
                complete_av1_output(|| Ok(()), || Err("unmap failed")),
                Err("unmap failed")
            );
        }

        #[test]
        fn cpu_bgra_borrows_frame_storage() {
            let frame = CapturedFrame::from_cpu(2, 2, FramePixelFormat::Bgra32, 0, vec![7; 16]);
            let bgra = to_bgra(&frame).unwrap();
            assert_eq!(bgra.as_ptr(), frame.data.as_ptr());
        }

        #[test]
        fn cpu_bgra_rejects_truncated_input() {
            let frame = CapturedFrame::from_cpu(2, 2, FramePixelFormat::Bgra32, 0, vec![7; 15]);
            assert!(to_bgra(&frame).is_err());
        }

        #[test]
        #[ignore = "requires a working NVIDIA AV1 encoder and runtime"]
        fn strict_av1_cpu_bgra_encodes_repeated_frames() {
            let mut encoder =
                NvencAv1Encoder::new(1280, 720, 60).expect("AV1 encoder must initialize");
            for index in 0..3 {
                let frame = CapturedFrame::from_cpu(
                    1280,
                    720,
                    FramePixelFormat::Bgra32,
                    index,
                    vec![(index * 24) as u8; 1280 * 720 * 4],
                );
                let output = encoder.encode(&frame).expect("CPU AV1 frame encode");
                assert_eq!(output.len(), 1);
                assert!(!output[0].bytes.is_empty());
                assert_eq!(output[0].timestamp_us, index);
                assert_eq!(output[0].codec, VideoCodec::Av1);
            }
        }

        #[test]
        #[ignore = "requires a working NVIDIA AV1 encoder and shared D3D11 runtime"]
        fn strict_av1_shared_bgra_encodes_repeated_frames() {
            use windows::core::Interface;
            use windows::Win32::Graphics::Direct3D11::D3D11_RESOURCE_MISC_SHARED;
            use windows::Win32::Graphics::Dxgi::IDXGIResource;
            let (device, context) = create_d3d11_device().expect("capture D3D11 device");
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 1280,
                Height: 720,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
                ..Default::default()
            };
            let mut texture = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.unwrap();
            let texture = texture.unwrap();
            let dxgi: IDXGIResource = texture.cast().unwrap();
            let handle = unsafe { dxgi.GetSharedHandle() }.unwrap();
            let mut encoder =
                NvencAv1Encoder::new(1280, 720, 60).expect("AV1 encoder must initialize");
            for index in 0..4 {
                let pixels = vec![(index * 24) as u8; 1280 * 720 * 4];
                unsafe {
                    context.UpdateSubresource(
                        &texture,
                        0,
                        None,
                        pixels.as_ptr().cast(),
                        1280 * 4,
                        0,
                    );
                    context.Flush();
                }
                let frame = CapturedFrame::from_d3d11_shared_bgra(
                    1280,
                    720,
                    index,
                    handle.0 as isize,
                    1280 * 4,
                );
                let output = encoder.encode(&frame).expect("shared AV1 frame encode");
                assert_eq!(output.len(), 1);
                assert!(!output[0].bytes.is_empty());
                assert_eq!(output[0].timestamp_us, index);
                assert_eq!(output[0].codec, VideoCodec::Av1);
            }
        }
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
            Self::new_low_latency_with_bitrate(width, height, fps, 8_000_000)
        }

        pub fn new_low_latency_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_low_latency_internal(
                width,
                height,
                fps,
                NV_ENC_AV1_PROFILE_MAIN_GUID,
                bitrate.max(1),
            )
        }

        /// Ultra-low latency AV1 encoder for remote desktop scenarios
        /// Uses UltraLowLatency tuning and P6 preset for minimum latency
        pub fn new_ultra_low_latency(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_ultra_low_latency_with_bitrate(width, height, fps, 6_000_000)
        }

        pub fn new_ultra_low_latency_with_bitrate(
            width: usize,
            height: usize,
            fps: u32,
            bitrate: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_ultra_low_latency_internal(
                width,
                height,
                fps,
                NV_ENC_AV1_PROFILE_MAIN_GUID,
                bitrate.max(1),
            )
        }

        /// High refresh rate AV1 encoder (120Hz+) optimized for minimum latency
        /// Lower bitrate and shorter GOP for maximum speed
        pub fn new_high_refresh_rate(
            width: usize,
            height: usize,
            fps: u32,
        ) -> Result<Self, PipelineError> {
            Self::new_high_refresh_rate_with_bitrate(width, height, fps, 6_000_000)
        }

        pub fn new_high_refresh_rate_with_bitrate(
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
            ensure_av1_codec_supported(&session)?;
            let preset_guid = select_av1_high_refresh_preset(&session)?;
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_AV1_GUID,
                    preset_guid.clone(),
                    NVencTuningInfo::UltraLowLatency,
                )
                .map_err(|error| {
                    PipelineError::message(format!("nvenc preset config failed: {error:?}"))
                })?;
            preset.preset_cfg.profile_guid = NV_ENC_AV1_PROFILE_MAIN_GUID;
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
            preset.preset_cfg.frame_interval_p = 1;
            preset.preset_cfg.gop_len = fps * 2;

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_AV1_GUID,
                preset_guid,
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
                native: RetainedOnFailure::new(NativeResources {
                    _device: device,
                    context,
                    texture,
                    encoder,
                    registered,
                    shared_input: None,
                    bitstream,
                }),
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
            Self::new_low_latency_internal(width, height, fps, profile_guid, 8_000_000)
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
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
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
                native: RetainedOnFailure::new(NativeResources {
                    _device: device,
                    context,
                    texture,
                    encoder,
                    registered,
                    shared_input: None,
                    bitstream,
                }),
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
            preset.preset_cfg.rc_params.average_bit_rate = bitrate;
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
                native: RetainedOnFailure::new(NativeResources {
                    _device: device,
                    context,
                    texture,
                    encoder,
                    registered,
                    shared_input: None,
                    bitstream,
                }),
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
                self.frame_index == 0 || self.frame_index.is_multiple_of(self.fps as usize * 2);
            let bytes = self
                .native
                .run(|native| {
                    let shared_input = native
                        .shared_input
                        .as_mut()
                        .ok_or_else(|| anyhow!("missing shared input resource"))?;
                    encode_picture(
                        &mut native.encoder,
                        &native.bitstream,
                        &mut shared_input.registered,
                        self.frame_index,
                        force_key,
                    )
                })
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
            let native = self.native.idle_mut()?;
            let needs_new = native
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
                native._device.OpenSharedResource(
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

            let registered = native
                .encoder
                .register_resource_dx11(&texture, NVencBufferFormat::ARGB, shared.row_pitch)
                .map_err(|error| {
                    PipelineError::message(format!(
                        "nvenc AV1 register shared texture failed: {error:?}"
                    ))
                })?;

            native.shared_input = Some(SharedInputResource {
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
            self.native.ensure_idle()?;
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

            // AV1 uses key frames instead of IDR frames
            let force_key =
                self.frame_index == 0 || self.frame_index.is_multiple_of(self.fps as usize * 2);
            let bytes = self
                .native
                .run(|native| {
                    // Native registration starts mapped. Unmap before the first
                    // upload, and map again only after all graphics writes.
                    prepare_cpu_input(
                        || {
                            native.registered.unmap().map_err(|error| {
                                anyhow!("NVENC AV1 unmap before upload failed: {error:?}")
                            })
                        },
                        || unsafe {
                            native.context.UpdateSubresource(
                                &native.texture,
                                0,
                                None,
                                bgra.as_ptr() as *const core::ffi::c_void,
                                row_pitch,
                                0,
                            );
                        },
                    )?;
                    encode_picture(
                        &mut native.encoder,
                        &native.bitstream,
                        &mut native.registered,
                        self.frame_index,
                        force_key,
                    )
                })
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

    impl Drop for NativeResources {
        fn drop(&mut self) {
            // Make the D3D11 submission queue and the encoder session quiescent
            // before registered resources are dropped in field order.
            unsafe { self.context.Flush() };
            let _ = self.encoder.end_encode();
            let _ = self.registered.unmap();
            if let Some(shared_input) = self.shared_input.as_mut() {
                let _ = shared_input.registered.unmap();
            }
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

    fn select_av1_high_refresh_preset(
        session: &Session<NeedsConfig>,
    ) -> Result<Guid, PipelineError> {
        let presets = session
            .get_encode_presets(NV_ENC_CODEC_AV1_GUID)
            .map_err(|error| {
                PipelineError::message(format!("NVENC AV1 preset query failed: {error:?}"))
            })?;

        if presets
            .iter()
            .any(|preset| preset == &NV_ENC_PRESET_P1_GUID)
        {
            return Ok(NV_ENC_PRESET_P1_GUID);
        }
        if presets
            .iter()
            .any(|preset| preset == &NV_ENC_PRESET_P6_GUID)
        {
            return Ok(NV_ENC_PRESET_P6_GUID);
        }

        Err(PipelineError::message(
            "NVENC AV1 unavailable: required high-refresh AV1 preset is not supported by this GPU/driver",
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
        registered: &mut RegisteredResource,
        frame_index: usize,
        force_key: bool,
    ) -> anyhow::Result<Vec<u8>> {
        registered
            .map()
            .map_err(|error| anyhow!("NVENC AV1 map input failed: {error:?}"))?;
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
        complete_av1_output(
            || {
                let lock = bitstream
                    .try_lock(true)
                    .map_err(|error| anyhow!("NVENC bitstream lock failed: {error:?}"))?;
                let bytes = lock.as_slice().to_vec();
                lock.unlock()
                    .map_err(|error| anyhow!("NVENC bitstream unlock failed: {error:?}"))?;
                Ok(bytes)
            },
            || {
                registered
                    .unmap()
                    .map_err(|error| anyhow!("NVENC AV1 unmap input failed: {error:?}"))
            },
        )
    }

    fn to_bgra(frame: &CapturedFrame) -> Result<Cow<'_, [u8]>, PipelineError> {
        let pixels = frame
            .width
            .checked_mul(frame.height)
            .ok_or_else(|| PipelineError::message("frame size overflow"))?;
        let expected_output_len = pixels
            .checked_mul(4)
            .ok_or_else(|| PipelineError::message("BGRA output size overflow"))?;
        let expected_input_len = match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => Some(expected_output_len),
            FramePixelFormat::Rgb24 => pixels.checked_mul(3),
            FramePixelFormat::Nv12 => nv12_len(frame.width, frame.height),
        }
        .ok_or_else(|| {
            PipelineError::message("frame buffer size overflow or odd NV12 dimensions")
        })?;
        if frame.data.len() != expected_input_len {
            return Err(PipelineError::message(format!(
                "frame bytes mismatch: expected {expected_input_len}, got {}",
                frame.data.len()
            )));
        }
        match frame.pixel_format {
            FramePixelFormat::Bgra32 => Ok(Cow::Borrowed(&frame.data)),
            FramePixelFormat::Rgba32 => {
                let mut bgra = Vec::with_capacity(expected_output_len);
                for pixel in frame.data.chunks_exact(4) {
                    bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
                Ok(Cow::Owned(bgra))
            }
            FramePixelFormat::Rgb24 => {
                let mut bgra = Vec::with_capacity(expected_output_len);
                for pixel in frame.data.chunks_exact(3) {
                    bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
                }
                Ok(Cow::Owned(bgra))
            }
            FramePixelFormat::Nv12 => nv12_to_bgra(frame, expected_output_len).map(Cow::Owned),
        }
    }

    fn nv12_len(width: usize, height: usize) -> Option<usize> {
        if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return None;
        }
        let y_size = width.checked_mul(height)?;
        y_size.checked_add(y_size / 2)
    }

    fn nv12_to_bgra(
        frame: &CapturedFrame,
        expected_output_len: usize,
    ) -> Result<Vec<u8>, PipelineError> {
        let expected_input_len = nv12_len(frame.width, frame.height)
            .ok_or_else(|| PipelineError::message("NV12 frame size overflow or odd dimensions"))?;
        if frame.data.len() != expected_input_len {
            return Err(PipelineError::message(format!(
                "NV12 frame bytes mismatch: expected {expected_input_len}, got {}",
                frame.data.len()
            )));
        }

        let y_size = frame
            .width
            .checked_mul(frame.height)
            .ok_or_else(|| PipelineError::message("NV12 luma byte size overflow"))?;
        let mut bgra = Vec::with_capacity(expected_output_len);
        for y in 0..frame.height {
            let y_row = y * frame.width;
            let uv_row = y_size + (y / 2) * frame.width;
            for x in 0..frame.width {
                let luma = frame.data[y_row + x] as i32;
                let uv_x = (x / 2) * 2;
                let u = frame.data[uv_row + uv_x] as i32;
                let v = frame.data[uv_row + uv_x + 1] as i32;
                let c = (luma - 16).max(0);
                let d = u - 128;
                let e = v - 128;
                let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
                let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
                let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;
                bgra.extend_from_slice(&[b, g, r, 255]);
            }
        }
        Ok(bgra)
    }
}

#[cfg(windows)]
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
pub struct NvencAv1Encoder {
    width: usize,
    height: usize,
    fps: u32,
    frame_index: usize,
}

#[cfg(not(windows))]
impl NvencAv1Encoder {
    pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_low_latency(width, height, fps)
    }

    pub fn new_low_latency(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
        Self::new_low_latency_with_bitrate(width, height, fps, 8_000_000)
    }

    pub fn new_low_latency_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        _bitrate: u32,
    ) -> Result<Self, PipelineError> {
        require_gst_element("nvav1enc")?;
        require_gst_element("av1parse")?;
        Ok(Self {
            width: width.max(2),
            height: height.max(2),
            fps: fps.max(1),
            frame_index: 0,
        })
    }

    pub fn new_ultra_low_latency(
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_ultra_low_latency_with_bitrate(width, height, fps, 6_000_000)
    }

    pub fn new_ultra_low_latency_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_low_latency_with_bitrate(width, height, fps, bitrate)
    }

    pub fn new_high_refresh_rate(
        width: usize,
        height: usize,
        fps: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_high_refresh_rate_with_bitrate(width, height, fps, 6_000_000)
    }

    pub fn new_high_refresh_rate_with_bitrate(
        width: usize,
        height: usize,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self, PipelineError> {
        Self::new_low_latency_with_bitrate(width, height, fps, bitrate)
    }

    pub fn preferred_input_memory_kind() -> mrd_pipeline_core::FrameMemoryKind {
        mrd_pipeline_core::FrameMemoryKind::Cpu
    }

    pub fn probe_av1_available() -> Result<(), PipelineError> {
        require_gst_element("nvav1enc")
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
impl VideoEncoder for NvencAv1Encoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        if frame.pixel_format != FramePixelFormat::Bgra32 {
            return Err(PipelineError::message(format!(
                "Linux NVENC AV1 GStreamer path expects BGRA32 frames, got {:?}",
                frame.pixel_format
            )));
        }
        if frame.width != self.width || frame.height != self.height {
            return Err(PipelineError::message(format!(
                "Linux NVENC AV1 GStreamer path was initialized for {}x{}, got {}x{}",
                self.width, self.height, frame.width, frame.height
            )));
        }

        let output = std::process::Command::new("gst-launch-1.0")
            .arg("-q")
            .arg("fdsrc")
            .arg("fd=0")
            .arg(format!("blocksize={}", frame.data.len()))
            .arg("num-buffers=1")
            .arg("!")
            .arg(format!(
                "video/x-raw,format=BGRA,width={},height={},framerate={}/1",
                self.width, self.height, self.fps
            ))
            .arg("!")
            .arg("videoconvert")
            .arg("!")
            .arg("video/x-raw,format=BGRA")
            .arg("!")
            .arg("nvav1enc")
            .arg("preset=p1")
            .arg("tune=ultra-low-latency")
            .arg("zerolatency=true")
            .arg("bframes=0")
            .arg(format!("gop-size={}", self.fps.min(60)))
            .arg("!")
            .arg("av1parse")
            .arg("!")
            .arg("fdsink")
            .arg("fd=1")
            .arg("sync=false")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    use std::io::Write;
                    stdin.write_all(&frame.data)?;
                }
                child.wait_with_output()
            })
            .map_err(|error| {
                PipelineError::message(format!("launch Linux GStreamer NVENC AV1 failed: {error}"))
            })?;

        if !output.status.success() {
            return Err(PipelineError::message(format!(
                "Linux GStreamer NVENC AV1 failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        if output.stdout.is_empty() {
            return Ok(vec![]);
        }

        let force_idr = self.frame_index == 0 || self.frame_index.is_multiple_of(self.fps as usize);
        self.frame_index += 1;
        Ok(vec![EncodedAccessUnit {
            codec: VideoCodec::Av1,
            timestamp_us: frame.timestamp_us,
            is_keyframe: force_idr,
            bytes: output.stdout,
        }])
    }
}
