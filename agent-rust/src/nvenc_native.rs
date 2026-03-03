use anyhow::{Context, Result, anyhow};

#[cfg(windows)]
mod imp {
    use super::*;
    use nvenc::bitstream::BitStream;
    use nvenc::encoder::{Encoder, RegisteredResource};
    use nvenc::session::{InitParams, NeedsConfig, Session};
    use nvenc::sys::enums::{NVencBufferFormat, NVencPicStruct, NVencPicType, NVencTuningInfo};
    use nvenc::sys::guids::{
        NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P1_GUID, NV_ENC_PRESET_P2_GUID,
        NV_ENC_PRESET_P3_GUID, NV_ENC_PRESET_P4_GUID, NV_ENC_PRESET_P5_GUID, NV_ENC_PRESET_P6_GUID,
        NV_ENC_PRESET_P7_GUID,
    };
    use std::collections::VecDeque;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG,
        D3D11_SDK_VERSION, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
        D3D11_USAGE_DEFAULT,
        D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
        D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
        D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
        D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11CreateDevice,
        ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D, ID3D11VideoContext,
        ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
        ID3D11VideoProcessorOutputView,
    };
    use windows::Win32::Graphics::Dxgi::{
        Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM,
            DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
        },
        CreateDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG3_REMOTE,
        DXGI_ADAPTER_FLAG3_SOFTWARE, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIAdapter1,
        IDXGIFactory1, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource,
        DXGI_ERROR_WAIT_TIMEOUT,
    };
    use windows::core::Interface;

    #[derive(Debug, Clone, Copy)]
    pub enum NativeEncodePath {
        DirectTexture,
        CopyResource,
        ScaleBlt,
    }

    pub struct NativeEncodeResult {
        pub bytes: Vec<u8>,
        pub path: NativeEncodePath,
        pub capture_start_us: u64,
    }

    #[derive(Debug, Clone, Copy, Default)]
    pub struct NativePathStats {
        pub direct_frames: u64,
        pub copy_frames: u64,
        pub scale_frames: u64,
        pub direct_register_failures: u64,
        pub acquire_ok: u64,
        pub acquire_timeout: u64,
        pub acquire_errors: u64,
    }

    pub struct NativeNvencPipeline {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        video_device: ID3D11VideoDevice,
        video_context: ID3D11VideoContext,
        duplication: IDXGIOutputDuplication,
        encode_resource: ID3D11Resource,
        target_width: u32,
        target_height: u32,
        scaler: Option<VideoScaler>,
        encoder: Encoder,
        registered: RegisteredResource,
        bitstream: BitStream,
        frame_idx: usize,
        strict_gpu_direct: bool,
        adapter_summary: String,
        direct_resources: VecDeque<(usize, RegisteredResource)>,
        direct_resource_capacity: usize,
        stats: NativePathStats,
    }

    pub struct NativeNvencTexturePipeline {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        video_device: ID3D11VideoDevice,
        video_context: ID3D11VideoContext,
        encode_resource: ID3D11Resource,
        target_width: u32,
        target_height: u32,
        scaler: Option<VideoScaler>,
        encoder: Encoder,
        registered: RegisteredResource,
        bitstream: BitStream,
        frame_idx: usize,
        strict_gpu_direct: bool,
        direct_resources: VecDeque<(usize, RegisteredResource)>,
        direct_resource_capacity: usize,
        stats: NativePathStats,
    }

    unsafe impl Send for NativeNvencPipeline {}
    unsafe impl Send for NativeNvencTexturePipeline {}

    struct DupFrame {
        duplication: IDXGIOutputDuplication,
        texture: ID3D11Texture2D,
    }

    struct VideoScaler {
        src_width: u32,
        src_height: u32,
        processor: ID3D11VideoProcessor,
        output_view: ID3D11VideoProcessorOutputView,
        source_rect: RECT,
        target_rect: RECT,
        _enumerator: ID3D11VideoProcessorEnumerator,
    }

    impl Drop for DupFrame {
        fn drop(&mut self) {
            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }
        }
    }

    impl NativeNvencPipeline {
        pub fn new(width: u32, height: u32, cfg: &agent_rust::CaptureConfig) -> Result<Self> {
            let fps = cfg.fps.max(1);
            let factory: IDXGIFactory1 =
                unsafe { CreateDXGIFactory1() }.context("create DXGI factory failed")?;
            let mut device = None::<ID3D11Device>;
            let mut context = None::<ID3D11DeviceContext>;
            let mut duplication = None::<IDXGIOutputDuplication>;
            let mut adapter_desc = None::<DXGI_ADAPTER_DESC1>;
            let mut output_index = 0_u32;
            let mut duplicate_api = "none";
            let mut duplicate_errors: Vec<String> = Vec::new();

            let mut adapters: Vec<(IDXGIAdapter1, DXGI_ADAPTER_DESC1, u8)> = Vec::new();
            for ai in 0..16_u32 {
                let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(ai) } {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let desc = unsafe { adapter.GetDesc1() }.context("GetDesc1 failed")?;
                let score = adapter_priority_score(&desc);
                adapters.push((adapter, desc, score));
            }
            adapters.sort_by_key(|(_, _, score)| *score);

            for (adapter, desc, _score) in adapters {
                let adapter0: IDXGIAdapter = adapter.cast().context("cast IDXGIAdapter failed")?;
                let (dev, ctx) = match create_d3d11_device(&adapter0) {
                    Ok(v) => v,
                    Err(e) => {
                        duplicate_errors.push(format!(
                            "adapter='{}' create_device failed: {e}",
                            utf16z_to_string(&desc.Description)
                        ));
                        continue;
                    }
                };
                let mut outputs: Vec<(IDXGIOutput1, u32, bool, String)> = Vec::new();
                for oi in 0..16_u32 {
                    let output = match unsafe { adapter.EnumOutputs(oi) } {
                        Ok(v) => v,
                        Err(_) => break,
                    };
                    let out_desc = unsafe { output.GetDesc() }.ok();
                    let attached = out_desc
                        .as_ref()
                        .map(|d| d.AttachedToDesktop.as_bool())
                        .unwrap_or(false);
                    let out_name = out_desc
                        .as_ref()
                        .map(|d| utf16z_to_string(&d.DeviceName))
                        .unwrap_or_else(|| "unknown".to_string());
                    let output1: IDXGIOutput1 = match output.cast() {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    outputs.push((output1, oi, attached, out_name));
                }
                outputs.sort_by_key(|(_, _, attached, _)| if *attached { 0_u8 } else { 1_u8 });
                for (output1, oi, _attached, out_name) in outputs {
                    match try_duplicate_output(&output1, &dev) {
                        Ok((dup, api)) => {
                            device = Some(dev);
                            context = Some(ctx);
                            duplication = Some(dup);
                            adapter_desc = Some(desc);
                            output_index = oi;
                            duplicate_api = api;
                            break;
                        }
                        Err(e) => {
                            duplicate_errors.push(format!(
                                "adapter='{}' output_index={} output='{}' duplicate failed: {}",
                                utf16z_to_string(&desc.Description),
                                oi,
                                out_name,
                                e
                            ));
                        }
                    }
                }
                if duplication.is_some() {
                    break;
                }
            }

            let duplication = duplication.ok_or_else(|| {
                let tail = duplicate_errors
                    .iter()
                    .rev()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join(" | ");
                anyhow!(
                    "desktop duplication unavailable; recent_attempts=[{}]",
                    tail
                )
            })?;

            if device.is_none() || context.is_none() || adapter_desc.is_none() {
                return Err(anyhow!(
                    "desktop duplication selected but device/context metadata missing"
                ));
            }
            let device = if let Some(v) = device {
                v
            } else {
                return Err(anyhow!("D3D11 device missing"));
            };
            let context = if let Some(v) = context {
                v
            } else {
                return Err(anyhow!("D3D11 context missing"));
            };
            let adapter_desc = if let Some(v) = adapter_desc {
                v
            } else {
                return Err(anyhow!("adapter description missing"));
            };
            let video_device: ID3D11VideoDevice =
                device.cast().context("cast ID3D11VideoDevice failed")?;
            let video_context: ID3D11VideoContext =
                context.cast().context("cast ID3D11VideoContext failed")?;

            let session: Session<NeedsConfig> =
                Session::open_dx(&device).map_err(|e| anyhow!("NVENC open_dx failed: {e:?}"))?;
            let preset_guid = preset_guid(&cfg.encoder_preset);
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    preset_guid.clone(),
                    tuning_from_cfg(cfg),
                )
                .map_err(|e| anyhow!("NVENC get preset config failed: {e:?}"))?;

            let avg_br = cfg.bitrate_kbps.max(100).saturating_mul(1000);
            preset.preset_cfg.rc_params.average_bit_rate = avg_br;
            preset.preset_cfg.frame_interval_p = cfg.bframes.saturating_add(1).max(1) as i32;
            preset.preset_cfg.gop_len = cfg.gop.max(1);

            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid,
                resolution: [width.max(2), height.max(2)],
                aspect_ratio: [width.max(2), height.max(2)],
                frame_rate: [fps.max(1), 1],
                tuning_info: tuning_from_cfg(cfg),
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [width.max(2), height.max(2)],
            };
            let encoder = session
                .init_encoder(init)
                .map_err(|e| anyhow!("NVENC init_encoder failed: {e:?}"))?;
            let encode_texture = create_encode_texture(&device, width.max(2), height.max(2))
                .context("create encode texture failed")?;
            let encode_resource: ID3D11Resource = encode_texture
                .cast()
                .context("cast encode resource failed")?;
            let registered = encoder
                .register_resource_dx11(&encode_texture, NVencBufferFormat::ARGB, 0)
                .map_err(|e| anyhow!("NVENC register encode texture failed: {e:?}"))?;
            let bitstream = encoder
                .create_bitstream_buffer()
                .map_err(|e| anyhow!("NVENC create bitstream buffer failed: {e:?}"))?;
            let adapter_summary = format!(
                "adapter='{}' luid={:08x}:{:08x} output_index={} duplicate_api={}",
                utf16z_to_string(&adapter_desc.Description),
                adapter_desc.AdapterLuid.HighPart as u32,
                adapter_desc.AdapterLuid.LowPart,
                output_index,
                duplicate_api,
            );

            Ok(Self {
                _device: device,
                context,
                video_device,
                video_context,
                duplication,
                encode_resource,
                target_width: width.max(2),
                target_height: height.max(2),
                scaler: None,
                encoder,
                registered,
                bitstream,
                frame_idx: 0,
                strict_gpu_direct: cfg.strict_gpu_direct,
                adapter_summary,
                direct_resources: VecDeque::new(),
                direct_resource_capacity: direct_resource_cache_capacity(),
                stats: NativePathStats::default(),
            })
        }

        pub fn adapter_summary(&self) -> &str {
            &self.adapter_summary
        }

        pub fn path_stats(&self) -> NativePathStats {
            self.stats
        }

        pub fn encode_next(&mut self, force_idr: bool) -> Result<Option<NativeEncodeResult>> {
            let frame_idx = self.frame_idx;
            let capture_start_us = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|v| v.as_micros().min(u64::MAX as u128) as u64)
                .unwrap_or(0);
            let Some(frame) = self.acquire_frame()? else {
                return Ok(None);
            };
            let mut src_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                frame.texture.GetDesc(&mut src_desc);
            }
            let needs_scale =
                src_desc.Width != self.target_width || src_desc.Height != self.target_height;
            let path = if needs_scale {
                self.try_gpu_scale_copy(&frame.texture)
                    .context("strict scale path failed")?;
                self.stats.scale_frames = self.stats.scale_frames.saturating_add(1);
                NativeEncodePath::ScaleBlt
            } else {
                let src_key = frame.texture.as_raw() as usize;
                let mut direct_idx = self
                    .direct_resources
                    .iter()
                    .position(|(k, _)| *k == src_key);
                if direct_idx.is_none() {
                    match self.encoder.register_resource_dx11(
                        &frame.texture,
                        NVencBufferFormat::ARGB,
                        0,
                    ) {
                        Ok(resource) => {
                            self.direct_resources.push_back((src_key, resource));
                            while self.direct_resources.len() > self.direct_resource_capacity {
                                let _ = self.direct_resources.pop_front();
                            }
                            direct_idx = self
                                .direct_resources
                                .iter()
                                .position(|(k, _)| *k == src_key);
                        }
                        Err(e) => {
                            self.stats.direct_register_failures =
                                self.stats.direct_register_failures.saturating_add(1);
                            if self.strict_gpu_direct {
                                return Err(anyhow!(
                                    "strict_gpu_direct direct register failed: {e:?}"
                                ));
                            }
                        }
                    }
                }
                if let Some(idx) = direct_idx {
                    let registered = &self.direct_resources[idx].1;
                    let bytes = encode_picture(
                        &mut self.encoder,
                        &self.bitstream,
                        registered,
                        frame_idx,
                        force_idr,
                    )?;
                    drop(frame);
                    self.frame_idx = frame_idx.saturating_add(1);
                    self.stats.direct_frames = self.stats.direct_frames.saturating_add(1);
                    return Ok(Some(NativeEncodeResult {
                        bytes: normalize_h264_au(bytes),
                        path: NativeEncodePath::DirectTexture,
                        capture_start_us,
                    }));
                }

                let src: ID3D11Resource =
                    frame.texture.cast().context("cast frame resource failed")?;
                unsafe {
                    self.context.CopyResource(&self.encode_resource, &src);
                }
                self.stats.copy_frames = self.stats.copy_frames.saturating_add(1);
                NativeEncodePath::CopyResource
            };

            let bytes = encode_picture(
                &mut self.encoder,
                &self.bitstream,
                &self.registered,
                frame_idx,
                force_idr,
            )?;
            drop(frame);
            self.frame_idx = frame_idx.saturating_add(1);
            Ok(Some(NativeEncodeResult {
                bytes: normalize_h264_au(bytes),
                path,
                capture_start_us,
            }))
        }

        fn try_gpu_scale_copy(&mut self, src_texture: &ID3D11Texture2D) -> Result<()> {
            let mut src_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                src_texture.GetDesc(&mut src_desc);
            }
            let src_w = src_desc.Width.max(2);
            let src_h = src_desc.Height.max(2);
            if src_w == self.target_width && src_h == self.target_height {
                return Err(anyhow!("no scaling needed"));
            }

            if self
                .scaler
                .as_ref()
                .map(|s| s.src_width != src_w || s.src_height != src_h)
                .unwrap_or(true)
            {
                self.scaler = Some(self.build_scaler(src_w, src_h)?);
            }
            let scaler = self
                .scaler
                .as_ref()
                .ok_or_else(|| anyhow!("scaler not initialized"))?;

            let src_resource: ID3D11Resource =
                src_texture.cast().context("cast source resource failed")?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut input_view = None;
            unsafe {
                self.video_device
                    .CreateVideoProcessorInputView(
                        &src_resource,
                        &scaler._enumerator,
                        &input_desc,
                        Some(&mut input_view),
                    )
                    .context("CreateVideoProcessorInputView failed")?;
            }
            let input_view =
                input_view.ok_or_else(|| anyhow!("CreateVideoProcessorInputView returned none"))?;

            unsafe {
                self.video_context.VideoProcessorSetStreamFrameFormat(
                    &scaler.processor,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                self.video_context.VideoProcessorSetStreamSourceRect(
                    &scaler.processor,
                    0,
                    true,
                    Some(&scaler.source_rect),
                );
                self.video_context.VideoProcessorSetStreamDestRect(
                    &scaler.processor,
                    0,
                    true,
                    Some(&scaler.target_rect),
                );
                self.video_context.VideoProcessorSetOutputTargetRect(
                    &scaler.processor,
                    true,
                    Some(&scaler.target_rect),
                );
            }

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                OutputIndex: 0,
                InputFrameOrField: 0,
                PastFrames: 0,
                FutureFrames: 0,
                ppPastSurfaces: std::ptr::null_mut(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(input_view)),
                ppFutureSurfaces: std::ptr::null_mut(),
                ppPastSurfacesRight: std::ptr::null_mut(),
                pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
                ppFutureSurfacesRight: std::ptr::null_mut(),
            };
            unsafe {
                self.video_context
                    .VideoProcessorBlt(&scaler.processor, &scaler.output_view, 0, &[stream])
                    .context("VideoProcessorBlt failed")?;
            }
            Ok(())
        }

        fn build_scaler(&self, src_width: u32, src_height: u32) -> Result<VideoScaler> {
            let content_desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                InputWidth: src_width,
                InputHeight: src_height,
                OutputFrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                OutputWidth: self.target_width,
                OutputHeight: self.target_height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };

            let enumerator = unsafe {
                self.video_device
                    .CreateVideoProcessorEnumerator(&content_desc)
                    .context("CreateVideoProcessorEnumerator failed")?
            };
            let processor = unsafe {
                self.video_device
                    .CreateVideoProcessor(&enumerator, 0)
                    .context("CreateVideoProcessor failed")?
            };

            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view = None;
            unsafe {
                self.video_device
                    .CreateVideoProcessorOutputView(
                        &self.encode_resource,
                        &enumerator,
                        &output_desc,
                        Some(&mut output_view),
                    )
                    .context("CreateVideoProcessorOutputView failed")?;
            }
            let output_view = output_view
                .ok_or_else(|| anyhow!("CreateVideoProcessorOutputView returned none"))?;

            Ok(VideoScaler {
                src_width,
                src_height,
                processor,
                output_view,
                source_rect: RECT {
                    left: 0,
                    top: 0,
                    right: src_width as i32,
                    bottom: src_height as i32,
                },
                target_rect: RECT {
                    left: 0,
                    top: 0,
                    right: self.target_width as i32,
                    bottom: self.target_height as i32,
                },
                _enumerator: enumerator,
            })
        }

        fn acquire_frame(&mut self) -> Result<Option<DupFrame>> {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match unsafe {
                self.duplication
                    .AcquireNextFrame(16, &mut info, &mut resource)
            } {
                Ok(()) => {
                    self.stats.acquire_ok = self.stats.acquire_ok.saturating_add(1);
                }
                Err(e) => {
                    if e.code() == DXGI_ERROR_WAIT_TIMEOUT {
                        self.stats.acquire_timeout = self.stats.acquire_timeout.saturating_add(1);
                        return Ok(None);
                    }
                    self.stats.acquire_errors = self.stats.acquire_errors.saturating_add(1);
                    return Err(anyhow!("AcquireNextFrame failed: {e}"));
                }
            }

            let resource = resource.ok_or_else(|| anyhow!("AcquireNextFrame resource is none"))?;
            let texture: ID3D11Texture2D =
                resource.cast().context("cast ID3D11Texture2D failed")?;
            Ok(Some(DupFrame {
                duplication: self.duplication.clone(),
                texture,
            }))
        }
    }

    impl NativeNvencTexturePipeline {
        pub fn new(
            device: ID3D11Device,
            context: ID3D11DeviceContext,
            width: u32,
            height: u32,
            cfg: &agent_rust::CaptureConfig,
        ) -> Result<Self> {
            let fps = cfg.fps.max(1);
            let video_device: ID3D11VideoDevice =
                device.cast().context("cast ID3D11VideoDevice failed")?;
            let video_context: ID3D11VideoContext =
                context.cast().context("cast ID3D11VideoContext failed")?;

            let session: Session<NeedsConfig> =
                Session::open_dx(&device).map_err(|e| anyhow!("NVENC open_dx failed: {e:?}"))?;
            let preset_guid = preset_guid(&cfg.encoder_preset);
            let (session, mut preset) = session
                .get_encode_preset_config_ex(
                    NV_ENC_CODEC_H264_GUID,
                    preset_guid.clone(),
                    tuning_from_cfg(cfg),
                )
                .map_err(|e| anyhow!("NVENC get preset config failed: {e:?}"))?;

            let avg_br = cfg.bitrate_kbps.max(100).saturating_mul(1000);
            preset.preset_cfg.rc_params.average_bit_rate = avg_br;
            preset.preset_cfg.frame_interval_p = cfg.bframes.saturating_add(1).max(1) as i32;
            preset.preset_cfg.gop_len = cfg.gop.max(1);

            let target_width = width.max(2);
            let target_height = height.max(2);
            let init = InitParams {
                encode_guid: NV_ENC_CODEC_H264_GUID,
                preset_guid,
                resolution: [target_width, target_height],
                aspect_ratio: [target_width, target_height],
                frame_rate: [fps.max(1), 1],
                tuning_info: tuning_from_cfg(cfg),
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset.preset_cfg,
                enable_ptd: true,
                max_encoder_resolution: [target_width, target_height],
            };
            let encoder = session
                .init_encoder(init)
                .map_err(|e| anyhow!("NVENC init_encoder failed: {e:?}"))?;
            let encode_texture = create_encode_texture(&device, target_width, target_height)
                .context("create encode texture failed")?;
            let encode_resource: ID3D11Resource = encode_texture
                .cast()
                .context("cast encode resource failed")?;
            let registered = encoder
                .register_resource_dx11(&encode_texture, NVencBufferFormat::ARGB, 0)
                .map_err(|e| anyhow!("NVENC register encode texture failed: {e:?}"))?;
            let bitstream = encoder
                .create_bitstream_buffer()
                .map_err(|e| anyhow!("NVENC create bitstream buffer failed: {e:?}"))?;

            Ok(Self {
                _device: device,
                context,
                video_device,
                video_context,
                encode_resource,
                target_width,
                target_height,
                scaler: None,
                encoder,
                registered,
                bitstream,
                frame_idx: 0,
                strict_gpu_direct: cfg.strict_gpu_direct,
                direct_resources: VecDeque::new(),
                direct_resource_capacity: direct_resource_cache_capacity(),
                stats: NativePathStats::default(),
            })
        }

        pub fn path_stats(&self) -> NativePathStats {
            self.stats
        }

        pub fn encode_texture(
            &mut self,
            src_texture: &ID3D11Texture2D,
            force_idr: bool,
        ) -> Result<Option<NativeEncodeResult>> {
            let frame_idx = self.frame_idx;
            let mut src_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                src_texture.GetDesc(&mut src_desc);
            }
            if src_desc.Width == 0 || src_desc.Height == 0 {
                return Ok(None);
            }
            let needs_scale =
                src_desc.Width != self.target_width || src_desc.Height != self.target_height;
            let path = if needs_scale {
                self.try_gpu_scale_copy(src_texture)
                    .context("strict scale path failed")?;
                self.stats.scale_frames = self.stats.scale_frames.saturating_add(1);
                NativeEncodePath::ScaleBlt
            } else {
                let src_key = src_texture.as_raw() as usize;
                let mut direct_idx = self
                    .direct_resources
                    .iter()
                    .position(|(k, _)| *k == src_key);
                if direct_idx.is_none() {
                    match self
                        .encoder
                        .register_resource_dx11(src_texture, NVencBufferFormat::ARGB, 0)
                    {
                        Ok(resource) => {
                            self.direct_resources.push_back((src_key, resource));
                            while self.direct_resources.len() > self.direct_resource_capacity {
                                let _ = self.direct_resources.pop_front();
                            }
                            direct_idx = self
                                .direct_resources
                                .iter()
                                .position(|(k, _)| *k == src_key);
                        }
                        Err(e) => {
                            self.stats.direct_register_failures =
                                self.stats.direct_register_failures.saturating_add(1);
                            if self.strict_gpu_direct {
                                return Err(anyhow!(
                                    "strict_gpu_direct direct register failed: {e:?}"
                                ));
                            }
                        }
                    }
                }
                if let Some(idx) = direct_idx {
                    let registered = &self.direct_resources[idx].1;
                    let bytes = encode_picture(
                        &mut self.encoder,
                        &self.bitstream,
                        registered,
                        frame_idx,
                        force_idr,
                    )?;
                    self.frame_idx = frame_idx.saturating_add(1);
                    self.stats.direct_frames = self.stats.direct_frames.saturating_add(1);
                    return Ok(Some(NativeEncodeResult {
                        bytes: normalize_h264_au(bytes),
                        path: NativeEncodePath::DirectTexture,
                        capture_start_us: 0,
                    }));
                }

                let src: ID3D11Resource =
                    src_texture.cast().context("cast frame resource failed")?;
                unsafe {
                    self.context.CopyResource(&self.encode_resource, &src);
                }
                self.stats.copy_frames = self.stats.copy_frames.saturating_add(1);
                NativeEncodePath::CopyResource
            };

            let bytes = encode_picture(
                &mut self.encoder,
                &self.bitstream,
                &self.registered,
                frame_idx,
                force_idr,
            )?;
            self.frame_idx = frame_idx.saturating_add(1);
            Ok(Some(NativeEncodeResult {
                bytes: normalize_h264_au(bytes),
                path,
                capture_start_us: 0,
            }))
        }

        fn try_gpu_scale_copy(&mut self, src_texture: &ID3D11Texture2D) -> Result<()> {
            let mut src_desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                src_texture.GetDesc(&mut src_desc);
            }
            let src_w = src_desc.Width.max(2);
            let src_h = src_desc.Height.max(2);
            if src_w == self.target_width && src_h == self.target_height {
                return Err(anyhow!("no scaling needed"));
            }

            if self
                .scaler
                .as_ref()
                .map(|s| s.src_width != src_w || s.src_height != src_h)
                .unwrap_or(true)
            {
                self.scaler = Some(self.build_scaler(src_w, src_h)?);
            }
            let scaler = self
                .scaler
                .as_ref()
                .ok_or_else(|| anyhow!("scaler not initialized"))?;

            let src_resource: ID3D11Resource =
                src_texture.cast().context("cast source resource failed")?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV {
                        MipSlice: 0,
                        ArraySlice: 0,
                    },
                },
            };
            let mut input_view = None;
            unsafe {
                self.video_device
                    .CreateVideoProcessorInputView(
                        &src_resource,
                        &scaler._enumerator,
                        &input_desc,
                        Some(&mut input_view),
                    )
                    .context("CreateVideoProcessorInputView failed")?;
            }
            let input_view =
                input_view.ok_or_else(|| anyhow!("CreateVideoProcessorInputView returned none"))?;

            unsafe {
                self.video_context.VideoProcessorSetStreamFrameFormat(
                    &scaler.processor,
                    0,
                    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                );
                self.video_context.VideoProcessorSetStreamSourceRect(
                    &scaler.processor,
                    0,
                    true,
                    Some(&scaler.source_rect),
                );
                self.video_context.VideoProcessorSetStreamDestRect(
                    &scaler.processor,
                    0,
                    true,
                    Some(&scaler.target_rect),
                );
                self.video_context.VideoProcessorSetOutputTargetRect(
                    &scaler.processor,
                    true,
                    Some(&scaler.target_rect),
                );
            }

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                OutputIndex: 0,
                InputFrameOrField: 0,
                PastFrames: 0,
                FutureFrames: 0,
                ppPastSurfaces: std::ptr::null_mut(),
                pInputSurface: std::mem::ManuallyDrop::new(Some(input_view)),
                ppFutureSurfaces: std::ptr::null_mut(),
                ppPastSurfacesRight: std::ptr::null_mut(),
                pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
                ppFutureSurfacesRight: std::ptr::null_mut(),
            };
            unsafe {
                self.video_context
                    .VideoProcessorBlt(&scaler.processor, &scaler.output_view, 0, &[stream])
                    .context("VideoProcessorBlt failed")?;
            }
            Ok(())
        }

        fn build_scaler(&self, src_width: u32, src_height: u32) -> Result<VideoScaler> {
            let content_desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                InputWidth: src_width,
                InputHeight: src_height,
                OutputFrameRate: windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL {
                    Numerator: 60,
                    Denominator: 1,
                },
                OutputWidth: self.target_width,
                OutputHeight: self.target_height,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };

            let enumerator = unsafe {
                self.video_device
                    .CreateVideoProcessorEnumerator(&content_desc)
                    .context("CreateVideoProcessorEnumerator failed")?
            };
            let processor = unsafe {
                self.video_device
                    .CreateVideoProcessor(&enumerator, 0)
                    .context("CreateVideoProcessor failed")?
            };

            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut output_view = None;
            unsafe {
                self.video_device
                    .CreateVideoProcessorOutputView(
                        &self.encode_resource,
                        &enumerator,
                        &output_desc,
                        Some(&mut output_view),
                    )
                    .context("CreateVideoProcessorOutputView failed")?;
            }
            let output_view = output_view
                .ok_or_else(|| anyhow!("CreateVideoProcessorOutputView returned none"))?;

            Ok(VideoScaler {
                src_width,
                src_height,
                processor,
                output_view,
                source_rect: RECT {
                    left: 0,
                    top: 0,
                    right: src_width as i32,
                    bottom: src_height as i32,
                },
                target_rect: RECT {
                    left: 0,
                    top: 0,
                    right: self.target_width as i32,
                    bottom: self.target_height as i32,
                },
                _enumerator: enumerator,
            })
        }
    }

    fn tuning_from_cfg(cfg: &agent_rust::CaptureConfig) -> NVencTuningInfo {
        match cfg.encoder_tune.as_str() {
            "ull" => NVencTuningInfo::UltraLowLatency,
            "hq" => NVencTuningInfo::HighQuality,
            _ => NVencTuningInfo::LowLatency,
        }
    }

    fn direct_resource_cache_capacity() -> usize {
        std::env::var("AGENT_NVENC_DIRECT_CACHE")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(32)
            .clamp(8, 256)
    }

    fn encode_picture(
        encoder: &mut Encoder,
        bitstream: &BitStream,
        registered: &RegisteredResource,
        frame_idx: usize,
        force_idr: bool,
    ) -> Result<Vec<u8>> {
        encoder
            .encode_picture(
                registered,
                bitstream,
                frame_idx,
                frame_idx as u64,
                NVencBufferFormat::ARGB,
                NVencPicStruct::Frame,
                if force_idr {
                    NVencPicType::IDR
                } else {
                    NVencPicType::P
                },
                None,
            )
            .map_err(|e| anyhow!("NVENC encode_picture failed: {e:?}"))?;

        let lock = bitstream
            .try_lock(true)
            .map_err(|e| anyhow!("NVENC bitstream lock failed: {e:?}"))?;
        Ok(lock.as_slice().to_vec())
    }

    fn adapter_priority_score(desc: &DXGI_ADAPTER_DESC1) -> u8 {
        let name = utf16z_to_string(&desc.Description).to_ascii_lowercase();
        let flags = desc.Flags;
        let is_software = (flags & (DXGI_ADAPTER_FLAG3_SOFTWARE.0 as u32)) != 0;
        let is_remote = (flags & (DXGI_ADAPTER_FLAG3_REMOTE.0 as u32)) != 0;
        let is_virtual_name = name.contains("virtual")
            || name.contains("idd")
            || name.contains("basic render")
            || name.contains("mirror");
        let is_discrete_vendor = name.contains("nvidia")
            || name.contains("amd")
            || name.contains("radeon")
            || name.contains("intel");
        if is_software || is_remote {
            return 100;
        }
        if is_discrete_vendor && !is_virtual_name {
            return 0;
        }
        if !is_virtual_name {
            return 10;
        }
        50
    }

    fn try_duplicate_output(
        output1: &IDXGIOutput1,
        device: &ID3D11Device,
    ) -> Result<(IDXGIOutputDuplication, &'static str)> {
        if let Ok(output5) = output1.cast::<IDXGIOutput5>() {
            let formats = [
                DXGI_FORMAT_B8G8R8A8_UNORM,
                DXGI_FORMAT_R10G10B10A2_UNORM,
                DXGI_FORMAT_R16G16B16A16_FLOAT,
            ];
            match unsafe { output5.DuplicateOutput1(device, 0, &formats) } {
                Ok(dup) => return Ok((dup, "DuplicateOutput1")),
                Err(e) => match unsafe { output1.DuplicateOutput(device) } {
                    Ok(dup) => return Ok((dup, "DuplicateOutput")),
                    Err(e2) => {
                        return Err(anyhow!(
                            "DuplicateOutput1 failed: {e}; DuplicateOutput failed: {e2}"
                        ));
                    }
                },
            }
        }
        match unsafe { output1.DuplicateOutput(device) } {
            Ok(dup) => Ok((dup, "DuplicateOutput")),
            Err(e) => Err(anyhow!("DuplicateOutput failed: {e}")),
        }
    }

    fn create_d3d11_device(adapter: &IDXGIAdapter) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        fn try_create(
            adapter: Option<&IDXGIAdapter>,
            driver: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
            feature_levels: Option<&[windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL]>,
        ) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
            let mut device = None;
            let mut context = None;
            unsafe {
                D3D11CreateDevice(
                    adapter,
                    driver,
                    HMODULE(std::ptr::null_mut()),
                    D3D11_CREATE_DEVICE_FLAG(0),
                    feature_levels,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            }
            .context("D3D11CreateDevice failed")?;
            let device = device.ok_or_else(|| anyhow!("D3D11 device is none"))?;
            let context = context.ok_or_else(|| anyhow!("D3D11 context is none"))?;
            Ok((device, context))
        }

        try_create(
            Some(adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
        )
        .or_else(|_| try_create(Some(adapter), D3D_DRIVER_TYPE_UNKNOWN, None))
    }

    fn preset_guid(name: &str) -> nvenc::sys::structs::Guid {
        match name {
            "p1" => NV_ENC_PRESET_P1_GUID,
            "p2" => NV_ENC_PRESET_P2_GUID,
            "p3" => NV_ENC_PRESET_P3_GUID,
            "p5" => NV_ENC_PRESET_P5_GUID,
            "p6" => NV_ENC_PRESET_P6_GUID,
            "p7" => NV_ENC_PRESET_P7_GUID,
            _ => NV_ENC_PRESET_P4_GUID,
        }
    }

    fn normalize_h264_au(buf: Vec<u8>) -> Vec<u8> {
        // Native NVENC bitstream format can vary by driver/runtime.
        // WebRTC H264 sender expects AnnexB access units.
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
        // Assume 4-byte AVCC NAL length prefix (common for hardware bitstreams).
        let mut i = 0usize;
        let mut out = Vec::with_capacity(buf.len() + 16);
        let mut nals = 0usize;
        while i + 4 <= buf.len() {
            let n = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
            i += 4;
            if n == 0 || i + n > buf.len() {
                return None;
            }
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&buf[i..i + n]);
            i += n;
            nals += 1;
        }
        if i == buf.len() && nals > 0 {
            Some(out)
        } else {
            None
        }
    }

    fn create_encode_texture(
        device: &ID3D11Device,
        width: u32,
        height: u32,
    ) -> Result<ID3D11Texture2D> {
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

    fn utf16z_to_string(buf: &[u16]) -> String {
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end]).trim().to_string()
    }
}

#[cfg(windows)]
pub use imp::{NativeEncodePath, NativeNvencPipeline, NativeNvencTexturePipeline};

#[cfg(not(windows))]
pub struct NativeNvencPipeline;

#[cfg(not(windows))]
pub struct NativeNvencTexturePipeline;

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy)]
pub enum NativeEncodePath {
    DirectTexture,
    CopyResource,
    ScaleBlt,
}

#[cfg(not(windows))]
pub struct NativeEncodeResult {
    pub bytes: Vec<u8>,
    pub path: NativeEncodePath,
    pub capture_start_us: u64,
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Copy, Default)]
pub struct NativePathStats {
    pub direct_frames: u64,
    pub copy_frames: u64,
    pub scale_frames: u64,
    pub direct_register_failures: u64,
}

#[cfg(not(windows))]
impl NativeNvencPipeline {
    pub fn new(_width: u32, _height: u32, _cfg: &agent_rust::CaptureConfig) -> Result<Self> {
        Err(anyhow!("native NVENC pipeline only supports Windows"))
    }

    pub fn adapter_summary(&self) -> &str {
        "native NVENC pipeline only supports Windows"
    }

    pub fn path_stats(&self) -> NativePathStats {
        NativePathStats::default()
    }

    pub fn encode_next(&mut self, _force_idr: bool) -> Result<Option<NativeEncodeResult>> {
        Err(anyhow!("native NVENC pipeline only supports Windows"))
    }
}

#[cfg(not(windows))]
impl NativeNvencTexturePipeline {
    pub fn new(
        _device: (),
        _context: (),
        _width: u32,
        _height: u32,
        _cfg: &agent_rust::CaptureConfig,
    ) -> Result<Self> {
        Err(anyhow!("native NVENC texture pipeline only supports Windows"))
    }

    pub fn path_stats(&self) -> NativePathStats {
        NativePathStats::default()
    }
}
