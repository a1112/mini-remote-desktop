#[cfg(not(windows))]
use mrd_pipeline_core::{CapturedFrame, EncodedAccessUnit, PipelineError, VideoEncoder};

#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Context};
    use mrd_pipeline_core::{
        CapturedFrame, EncodedAccessUnit, FramePixelFormat, PipelineError, VideoCodec, VideoEncoder,
    };
    use nvenc::bitstream::BitStream;
    use nvenc::encoder::{Encoder, RegisteredResource};
    use nvenc::session::{InitParams, NeedsConfig, Session};
    use nvenc::sys::enums::{NVencBufferFormat, NVencPicStruct, NVencPicType, NVencTuningInfo};
    use nvenc::sys::guids::{NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P3_GUID};
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice,
        ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

    pub struct NvencH264Encoder {
        _device: ID3D11Device,
        context: ID3D11DeviceContext,
        texture: ID3D11Texture2D,
        encoder: Encoder,
        registered: RegisteredResource,
        bitstream: BitStream,
        width: usize,
        height: usize,
        fps: u32,
        frame_index: usize,
    }

    impl NvencH264Encoder {
        pub fn new(width: usize, height: usize, fps: u32) -> Result<Self, PipelineError> {
            let width = width.max(2);
            let height = height.max(2);
            let fps = fps.max(1);
            let (device, context) = create_d3d11_device()
                .map_err(|error| PipelineError::message(format!("create d3d11 device failed: {error}")))?;

            let session: Session<NeedsConfig> =
                Session::open_dx(&device).map_err(|error| {
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
            preset.preset_cfg.rc_params.average_bit_rate = 12_000_000;
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
            let texture = create_encode_texture(&device, width as u32, height as u32).map_err(
                |error| PipelineError::message(format!("create nvenc texture failed: {error}")),
            )?;
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
    }

    impl VideoEncoder for NvencH264Encoder {
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
            let bytes = encode_picture(
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
        force_idr: bool,
    ) -> anyhow::Result<Vec<u8>> {
        encoder
            .encode_picture(
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
                None,
            )
            .map_err(|error| anyhow!("NVENC encode_picture failed: {error:?}"))?;
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
pub use imp::NvencH264Encoder;

#[cfg(not(windows))]
pub struct NvencH264Encoder;

#[cfg(not(windows))]
impl NvencH264Encoder {
    pub fn new(_width: usize, _height: usize, _fps: u32) -> Result<Self, PipelineError> {
        Err(PipelineError::message("nvenc encoder only supports Windows"))
    }

    pub fn probe_h264_available() -> Result<(), PipelineError> {
        Err(PipelineError::message("nvenc encoder only supports Windows"))
    }
}

#[cfg(not(windows))]
impl VideoEncoder for NvencH264Encoder {
    fn encode(
        &mut self,
        _frame: &CapturedFrame,
    ) -> Result<Vec<EncodedAccessUnit>, PipelineError> {
        Err(PipelineError::message("nvenc encoder only supports Windows"))
    }
}
