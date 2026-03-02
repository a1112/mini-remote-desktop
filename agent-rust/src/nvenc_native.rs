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
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
        D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
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
        Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC},
        CreateDXGIFactory1, DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1,
        IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource,
    };
    use windows::core::Interface;

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
    }

    unsafe impl Send for NativeNvencPipeline {}

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
            let adapter: IDXGIAdapter1 =
                unsafe { factory.EnumAdapters1(0) }.context("enum DXGI adapter failed")?;
            let output = unsafe { adapter.EnumOutputs(0) }.context("enum DXGI output failed")?;
            let output1: IDXGIOutput1 = output.cast().context("cast IDXGIOutput1 failed")?;
            let adapter0: IDXGIAdapter = adapter.cast().context("cast IDXGIAdapter failed")?;

            let mut device = None;
            let mut context = None;
            unsafe {
                D3D11CreateDevice(
                    Some(&adapter0),
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE(std::ptr::null_mut()),
                    D3D11_CREATE_DEVICE_FLAG(0),
                    Some(&[D3D_FEATURE_LEVEL_11_0]),
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )
            }
            .context("D3D11CreateDevice failed")?;
            let device = device.ok_or_else(|| anyhow!("D3D11 device is none"))?;
            let context = context.ok_or_else(|| anyhow!("D3D11 context is none"))?;
            let video_device: ID3D11VideoDevice =
                device.cast().context("cast ID3D11VideoDevice failed")?;
            let video_context: ID3D11VideoContext =
                context.cast().context("cast ID3D11VideoContext failed")?;

            let duplication =
                unsafe { output1.DuplicateOutput(&device) }.context("DuplicateOutput failed")?;

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
            })
        }

        pub fn encode_next(&mut self, force_idr: bool) -> Result<Option<Vec<u8>>> {
            let frame_idx = self.frame_idx;
            let Some(frame) = self.acquire_frame()? else {
                return Ok(None);
            };
            let src: ID3D11Resource = frame.texture.cast().context("cast frame resource failed")?;
            if self.try_gpu_scale_copy(&frame.texture).is_ok() {
                // done by video processor
            } else {
                unsafe {
                    self.context.CopyResource(&self.encode_resource, &src);
                }
            }

            let bytes = {
                self.encoder
                    .encode_picture(
                        &self.registered,
                        &self.bitstream,
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

                let lock = self
                    .bitstream
                    .try_lock(true)
                    .map_err(|e| anyhow!("NVENC bitstream lock failed: {e:?}"))?;
                lock.as_slice().to_vec()
            };
            drop(frame);
            self.frame_idx = frame_idx.saturating_add(1);
            Ok(Some(bytes))
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

        fn acquire_frame(&self) -> Result<Option<DupFrame>> {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match unsafe {
                self.duplication
                    .AcquireNextFrame(16, &mut info, &mut resource)
            } {
                Ok(()) => {}
                Err(_) => return Ok(None),
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

    fn tuning_from_cfg(cfg: &agent_rust::CaptureConfig) -> NVencTuningInfo {
        match cfg.encoder_tune.as_str() {
            "ull" => NVencTuningInfo::UltraLowLatency,
            "hq" => NVencTuningInfo::HighQuality,
            _ => NVencTuningInfo::LowLatency,
        }
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
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .context("CreateTexture2D failed")?;
        texture.ok_or_else(|| anyhow!("CreateTexture2D returned none"))
    }
}

#[cfg(windows)]
pub use imp::NativeNvencPipeline;

#[cfg(not(windows))]
pub struct NativeNvencPipeline;

#[cfg(not(windows))]
impl NativeNvencPipeline {
    pub fn new(_width: u32, _height: u32, _cfg: &agent_rust::CaptureConfig) -> Result<Self> {
        Err(anyhow!("native NVENC pipeline only supports Windows"))
    }

    pub fn encode_next(&mut self) -> Result<Option<Vec<u8>>> {
        Err(anyhow!("native NVENC pipeline only supports Windows"))
    }
}
