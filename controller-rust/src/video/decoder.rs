use super::super::webrtc::peer::VideoFrame;
use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum DecodedFrameData {
    CpuNv12(Arc<Vec<u8>>),
    #[cfg(windows)]
    D3d11Nv12 {
        texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        subresource: u32,
    },
    #[cfg(windows)]
    D3d11SharedNv12 {
        shared_handle: isize,
    },
}

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub data: DecodedFrameData,
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub sequence: u64,
    pub capture_start_unix_us: u64,
}

impl DecodedFrame {
    pub fn from_cpu_nv12(
        data: Arc<Vec<u8>>,
        width: u32,
        height: u32,
        timestamp: u64,
        sequence: u64,
        capture_start_unix_us: u64,
    ) -> Self {
        Self {
            data: DecodedFrameData::CpuNv12(data),
            width,
            height,
            timestamp,
            sequence,
            capture_start_unix_us,
        }
    }

    #[cfg(windows)]
    pub fn from_d3d11_nv12(
        texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        subresource: u32,
        width: u32,
        height: u32,
        timestamp: u64,
        sequence: u64,
        capture_start_unix_us: u64,
    ) -> Self {
        Self {
            data: DecodedFrameData::D3d11Nv12 {
                texture,
                subresource,
            },
            width,
            height,
            timestamp,
            sequence,
            capture_start_unix_us,
        }
    }

    pub fn cpu_nv12(&self) -> Option<&[u8]> {
        match &self.data {
            DecodedFrameData::CpuNv12(data) => Some(data.as_slice()),
            #[cfg(windows)]
            DecodedFrameData::D3d11Nv12 { .. } => None,
            #[cfg(windows)]
            DecodedFrameData::D3d11SharedNv12 { .. } => None,
        }
    }

    #[cfg(windows)]
    pub fn d3d11_surface(
        &self,
    ) -> Option<(&windows::Win32::Graphics::Direct3D11::ID3D11Texture2D, u32)> {
        match &self.data {
            DecodedFrameData::D3d11Nv12 {
                texture,
                subresource,
            } => Some((texture, *subresource)),
            _ => None,
        }
    }

    #[cfg(windows)]
    pub fn d3d11_shared_handle(&self) -> Option<isize> {
        match &self.data {
            DecodedFrameData::D3d11SharedNv12 { shared_handle } => Some(*shared_handle),
            _ => None,
        }
    }

    pub fn y_size(&self) -> usize {
        (self.width * self.height) as usize
    }

    pub fn y_plane(&self) -> Option<&[u8]> {
        self.cpu_nv12().map(|data| &data[..self.y_size()])
    }

    pub fn uv_plane(&self) -> Option<&[u8]> {
        self.cpu_nv12().map(|data| &data[self.y_size()..])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderBackend {
    Auto,
    Software,
    D3d11va,
    MfD3d11,
}

#[derive(Debug, Clone)]
pub struct H264DecoderConfig {
    pub num_threads: usize,
    pub enable_hardware: bool,
    pub backend: DecoderBackend,
}

impl Default for H264DecoderConfig {
    fn default() -> Self {
        Self {
            num_threads: 2,
            enable_hardware: true,
            backend: DecoderBackend::Auto,
        }
    }
}

pub trait Decoder {
    fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>>;
    fn flush(&mut self) -> Result<Option<DecodedFrame>>;
    fn output_size(&self) -> Option<(u32, u32)>;
    fn backend_name(&self) -> &'static str;
}

pub enum H264Decoder {
    #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
    MfD3d11(mf_backend::MfH264Decoder),
    #[cfg(feature = "ffmpeg-software")]
    Ffmpeg(ffmpeg_backend::FfmpegH264Decoder),
    Disabled,
}

impl H264Decoder {
    pub fn new(config: H264DecoderConfig) -> Result<Self> {
        #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
        {
            let prefer_mf = matches!(config.backend, DecoderBackend::MfD3d11)
                || std::env::var("MRD_DECODER_TRY_MF")
                    .ok()
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(matches!(config.backend, DecoderBackend::Auto));
            if prefer_mf {
                match mf_backend::MfH264Decoder::new(config.clone()) {
                    Ok(decoder) => return Ok(Self::MfD3d11(decoder)),
                    Err(e) => {
                        if matches!(config.backend, DecoderBackend::MfD3d11) {
                            return Err(e);
                        }
                        tracing::warn!(error = %e, "mf_d3d11 backend unavailable, fallback to ffmpeg backend");
                    }
                }
            }
        }

        #[cfg(feature = "ffmpeg-software")]
        {
            Ok(Self::Ffmpeg(ffmpeg_backend::FfmpegH264Decoder::new(config)?))
        }

        #[cfg(not(feature = "ffmpeg-software"))]
        {
            let _ = config;
            tracing::warn!("decoder feature disabled; build with --features ffmpeg-software");
            Ok(Self::Disabled)
        }
    }
}

impl Decoder for H264Decoder {
    fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
        match self {
            #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
            Self::MfD3d11(decoder) => {
                let mut out = decoder.decode(frame)?;
                if let Some(decoded) = out.as_mut() {
                    decoded.capture_start_unix_us = frame.tx_unix_us;
                }
                Ok(out)
            }
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => {
                let mut out = decoder.decode(frame)?;
                if let Some(decoded) = out.as_mut() {
                    decoded.capture_start_unix_us = frame.tx_unix_us;
                }
                Ok(out)
            }
            Self::Disabled => Ok(None),
        }
    }

    fn flush(&mut self) -> Result<Option<DecodedFrame>> {
        match self {
            #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
            Self::MfD3d11(decoder) => decoder.flush(),
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.flush(),
            Self::Disabled => Ok(None),
        }
    }

    fn output_size(&self) -> Option<(u32, u32)> {
        match self {
            #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
            Self::MfD3d11(decoder) => decoder.output_size(),
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.output_size(),
            Self::Disabled => None,
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
            Self::MfD3d11(decoder) => decoder.backend_name(),
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.backend_name(),
            Self::Disabled => "disabled",
        }
    }
}

#[cfg(all(windows, feature = "mf-decoder", feature = "ffmpeg-software"))]
mod mf_backend {
    use super::*;
    use std::collections::VecDeque;
    use std::mem::ManuallyDrop;
    use windows::core::{Interface, IUnknown};
    use windows::Win32::Foundation::{E_NOTIMPL, HMODULE};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
        D3D11_TEXTURE2D_DESC, ID3D11Device,
        ID3D11DeviceContext, ID3D11Texture2D, D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        D3D11_SDK_VERSION,
    };
    use windows::Win32::Graphics::Dxgi::{IDXGIKeyedMutex, IDXGIResource, DXGI_ERROR_WAIT_TIMEOUT};
    use windows::Win32::Media::MediaFoundation::{
        CLSID_MSH264DecoderMFT, IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaBuffer,
        IMFMediaType, IMFSample, IMFTransform, MFCreateDXGIDeviceManager, MFCreateMediaType,
        MFCreateMemoryBuffer, MFCreateSample, MF_E_NO_MORE_TYPES, MF_E_NOTACCEPTING,
        MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_FRAME_RATE,
        MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO,
        MF_MT_SUBTYPE, MF_SA_D3D11_AWARE, MF_VERSION, MFMediaType_Video, MFSTARTUP_LITE, MFShutdown,
        MFStartup, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
        MFT_INPUT_STATUS_ACCEPT_DATA, MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
        MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
        MFT_OUTPUT_DATA_BUFFER_FORMAT_CHANGE, MFT_OUTPUT_STREAM_INFO, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };

    struct SharedNv12Slot {
        texture: ID3D11Texture2D,
        keyed_mutex: IDXGIKeyedMutex,
        shared_handle: isize,
        primed: bool,
    }

    pub struct MfH264Decoder {
        transform: IMFTransform,
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        _manager: IMFDXGIDeviceManager,
        input_stream_id: u32,
        output_stream_id: u32,
        output_info: MFT_OUTPUT_STREAM_INFO,
        queued: VecDeque<DecodedFrame>,
        render_surfaces: Vec<ID3D11Texture2D>,
        render_surface_cursor: usize,
        render_surface_w: u32,
        render_surface_h: u32,
        shared_slots: Vec<SharedNv12Slot>,
        shared_slot_cursor: usize,
        shared_slot_w: u32,
        shared_slot_h: u32,
        output_width: u32,
        output_height: u32,
        frame_index: u64,
        com_inited: bool,
        mf_started: bool,
    }

    // Accessed behind a mutex in decode task; single-thread use.
    unsafe impl Send for MfH264Decoder {}

    impl Drop for MfH264Decoder {
        fn drop(&mut self) {
            if self.mf_started {
                unsafe { let _ = MFShutdown(); }
            }
            if self.com_inited {
                unsafe { CoUninitialize(); }
            }
        }
    }

    impl MfH264Decoder {
        pub fn new(config: H264DecoderConfig) -> Result<Self> {
            let _ = config;
            let mut com_inited = false;
            unsafe {
                if CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() {
                    com_inited = true;
                }
                MFStartup(MF_VERSION, MFSTARTUP_LITE).context("MFStartup failed")?;
            }

            let transform: IMFTransform = unsafe {
                CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)
                    .context("CoCreateInstance(MSH264DecoderMFT) failed")?
            };

            let (device, context) = create_d3d11_device()?;
            let manager = create_dxgi_device_manager(&device)?;
            unsafe {
                let attrs: IMFAttributes = transform.GetAttributes().context("MFT GetAttributes failed")?;
                let _ = attrs.SetUINT32(&MF_SA_D3D11_AWARE, 1);
                transform
                    .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
                    .context("MFT_MESSAGE_SET_D3D_MANAGER failed")?;
            }

            let (input_stream_id, output_stream_id) = query_stream_ids(&transform)?;
            let mut me = Self {
                transform,
                device,
                context,
                _manager: manager,
                input_stream_id,
                output_stream_id,
                output_info: MFT_OUTPUT_STREAM_INFO::default(),
                queued: VecDeque::new(),
                render_surfaces: Vec::new(),
                render_surface_cursor: 0,
                render_surface_w: 0,
                render_surface_h: 0,
                shared_slots: Vec::new(),
                shared_slot_cursor: 0,
                shared_slot_w: 0,
                shared_slot_h: 0,
                output_width: 0,
                output_height: 0,
                frame_index: 0,
                com_inited,
                mf_started: true,
            };
            me.configure_input_type()?;
            me.configure_output_type_nv12()?;
            me.output_info = unsafe {
                me.transform
                    .GetOutputStreamInfo(me.output_stream_id)
                    .context("GetOutputStreamInfo failed")?
            };
            unsafe {
                me.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                    .context("MFT begin streaming failed")?;
                me.transform
                    .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                    .context("MFT start stream failed")?;
            }
            Ok(me)
        }

        fn configure_input_type(&self) -> Result<()> {
            let mt = unsafe { MFCreateMediaType().context("MFCreateMediaType(input) failed")? };
            unsafe {
                mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
                mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
                mt.SetUINT64(&MF_MT_FRAME_RATE, pack_u64(240, 1))?;
                mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u64(1, 1))?;
                self.transform
                    .SetInputType(self.input_stream_id, &mt, 0)
                    .context("SetInputType(H264) failed")?;
            }
            Ok(())
        }

        fn configure_output_type_nv12(&mut self) -> Result<()> {
            let mut idx = 0u32;
            loop {
                let mt = unsafe { self.transform.GetOutputAvailableType(self.output_stream_id, idx) };
                let mt = match mt {
                    Ok(v) => v,
                    Err(e) if e.code() == MF_E_NO_MORE_TYPES => break,
                    Err(e) => return Err(anyhow::anyhow!("GetOutputAvailableType failed: {e}")),
                };
                let subtype = unsafe { mt.GetGUID(&MF_MT_SUBTYPE) };
                if let Ok(s) = subtype {
                    if s == MFVideoFormat_NV12 {
                        unsafe {
                            self.transform
                                .SetOutputType(self.output_stream_id, &mt, 0)
                                .context("SetOutputType(NV12) failed")?;
                        }
                        self.update_size_from_media_type(&mt);
                        return Ok(());
                    }
                }
                idx = idx.saturating_add(1);
            }
            anyhow::bail!("No NV12 output type available in MF decoder")
        }

        fn update_size_from_media_type(&mut self, mt: &IMFMediaType) {
            if let Ok(v) = unsafe { mt.GetUINT64(&MF_MT_FRAME_SIZE) } {
                self.output_width = (v >> 32) as u32;
                self.output_height = (v & 0xFFFF_FFFF) as u32;
            }
        }

        fn send_input_sample(&mut self, frame: &VideoFrame) -> Result<()> {
            let input_status = unsafe {
                self.transform
                    .GetInputStatus(self.input_stream_id)
                    .context("GetInputStatus failed")?
            };
            if input_status & (MFT_INPUT_STATUS_ACCEPT_DATA.0 as u32) == 0 {
                return Ok(());
            }

            let sample = unsafe { MFCreateSample().context("MFCreateSample failed")? };
            let buf = unsafe {
                MFCreateMemoryBuffer(frame.data.len() as u32).context("MFCreateMemoryBuffer(input) failed")?
            };
            unsafe {
                let mut p = std::ptr::null_mut::<u8>();
                buf.Lock(&mut p, None, None).context("input buffer lock failed")?;
                std::ptr::copy_nonoverlapping(frame.data.as_ptr(), p, frame.data.len());
                buf.Unlock().ok();
                buf.SetCurrentLength(frame.data.len() as u32)
                    .context("SetCurrentLength failed")?;
                sample.AddBuffer(&buf).context("sample AddBuffer failed")?;
                sample.SetSampleTime((frame.timestamp as i64) * 10_000).ok();
                match self.transform.ProcessInput(self.input_stream_id, &sample, 0) {
                    Ok(()) => {}
                    Err(e) if e.code() == MF_E_NOTACCEPTING => {}
                    Err(e) => return Err(anyhow::anyhow!("ProcessInput failed: {e}")),
                }
            }
            Ok(())
        }

        fn drain_output(&mut self) -> Result<()> {
            loop {
                let provides_samples =
                    (self.output_info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32)) != 0;
                let prealloc = if provides_samples { None } else { Some(self.alloc_output_sample()?) };
                let mut out = MFT_OUTPUT_DATA_BUFFER {
                    dwStreamID: self.output_stream_id,
                    pSample: ManuallyDrop::new(prealloc.clone()),
                    dwStatus: 0,
                    pEvents: ManuallyDrop::new(None),
                };
                let mut status = 0u32;
                let out_slice = std::slice::from_mut(&mut out);
                let r = unsafe { self.transform.ProcessOutput(0, out_slice, &mut status) };
                match r {
                    Ok(()) => {
                        let format_change = (out.dwStatus & (MFT_OUTPUT_DATA_BUFFER_FORMAT_CHANGE.0 as u32)) != 0;
                        let sample = unsafe { ManuallyDrop::take(&mut out.pSample) }
                            .or(prealloc)
                            .context("ProcessOutput returned no sample")?;
                        let _events = unsafe { ManuallyDrop::take(&mut out.pEvents) };
                        if let Some(frame) = self.extract_decoded_frame(&sample)? {
                            self.queued.push_back(frame);
                        }
                        if format_change {
                            self.configure_output_type_nv12()?;
                            self.output_info = unsafe {
                                self.transform
                                    .GetOutputStreamInfo(self.output_stream_id)
                                    .context("GetOutputStreamInfo after change failed")?
                            };
                        }
                    }
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                    Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                        self.configure_output_type_nv12()?;
                        self.output_info = unsafe {
                            self.transform
                                .GetOutputStreamInfo(self.output_stream_id)
                                .context("GetOutputStreamInfo after stream change failed")?
                        };
                    }
                    Err(e) => return Err(anyhow::anyhow!("ProcessOutput failed: {e}")),
                }
            }
            Ok(())
        }

        fn alloc_output_sample(&self) -> Result<IMFSample> {
            let sample = unsafe { MFCreateSample().context("MFCreateSample(output) failed")? };
            let cb = self.output_info.cbSize.max(1_048_576);
            let buf = unsafe { MFCreateMemoryBuffer(cb).context("MFCreateMemoryBuffer(output) failed")? };
            unsafe { sample.AddBuffer(&buf).context("output sample AddBuffer failed")?; }
            Ok(sample)
        }

        fn extract_decoded_frame(&mut self, sample: &IMFSample) -> Result<Option<DecodedFrame>> {
            let pts = unsafe { sample.GetSampleTime().unwrap_or_default().max(0) as u64 };
            let count = unsafe { sample.GetBufferCount().context("GetBufferCount failed")? };
            for i in 0..count {
                let buf = unsafe { sample.GetBufferByIndex(i).context("GetBufferByIndex failed")? };
                if let Some(frame) = self.try_extract_dxgi_frame(&buf, pts)? {
                    return Ok(Some(frame));
                }
            }
            if count > 0 && self.output_width > 0 && self.output_height > 0 {
                let buf = unsafe { sample.GetBufferByIndex(0).context("GetBufferByIndex(0) failed")? };
                let data = extract_media_buffer_bytes(&buf)?;
                self.frame_index = self.frame_index.wrapping_add(1);
                return Ok(Some(DecodedFrame::from_cpu_nv12(
                    Arc::new(data),
                    self.output_width,
                    self.output_height,
                    pts,
                    self.frame_index,
                    0,
                )));
            }
            Ok(None)
        }

        fn try_extract_dxgi_frame(
            &mut self,
            buf: &IMFMediaBuffer,
            pts: u64,
        ) -> Result<Option<DecodedFrame>> {
            let dxgi = match buf.cast::<IMFDXGIBuffer>() {
                Ok(v) => v,
                Err(_) => return Ok(None),
            };
            let mut raw = std::ptr::null_mut::<std::ffi::c_void>();
            unsafe {
                dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)
                    .context("IMFDXGIBuffer::GetResource failed")?;
            }
            if raw.is_null() {
                return Ok(None);
            }
            let unk = unsafe { IUnknown::from_raw(raw as *mut _) };
            let tex: ID3D11Texture2D = unk.cast().context("cast resource to ID3D11Texture2D failed")?;
            let subresource = unsafe { dxgi.GetSubresourceIndex().unwrap_or(0) };
            let mut desc = Default::default();
            unsafe { tex.GetDesc(&mut desc) };
            if desc.Width > 0 && desc.Height > 0 {
                self.output_width = desc.Width;
                self.output_height = desc.Height;
            }
            self.frame_index = self.frame_index.wrapping_add(1);
            let enable_shared_keyed = std::env::var("MRD_ENABLE_SHARED_KEYED")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if enable_shared_keyed {
                match self.copy_to_shared_nv12_slot(&tex, subresource, &desc)? {
                    Some(shared_handle) => {
                        return Ok(Some(DecodedFrame {
                            data: DecodedFrameData::D3d11SharedNv12 { shared_handle },
                            width: self.output_width.max(1),
                            height: self.output_height.max(1),
                            timestamp: pts,
                            sequence: self.frame_index,
                            capture_start_unix_us: 0,
                        }));
                    }
                    None => {
                        tracing::debug!("shared-keyed path unavailable for this frame; fallback to renderable NV12");
                    }
                }
            }

            let render_tex = self.copy_to_renderable_nv12(&tex, subresource, &desc)?;
            Ok(Some(DecodedFrame::from_d3d11_nv12(
                render_tex,
                0,
                self.output_width.max(1),
                self.output_height.max(1),
                pts,
                self.frame_index,
                0,
            )))
        }

        fn copy_to_renderable_nv12(
            &mut self,
            src_tex: &ID3D11Texture2D,
            src_subresource: u32,
            src_desc: &D3D11_TEXTURE2D_DESC,
        ) -> Result<ID3D11Texture2D> {
            if src_desc.Width == 0 || src_desc.Height == 0 {
                anyhow::bail!("invalid MF output texture size {}x{}", src_desc.Width, src_desc.Height);
            }
            if self.render_surfaces.is_empty()
                || self.render_surface_w != src_desc.Width
                || self.render_surface_h != src_desc.Height
            {
                self.render_surfaces.clear();
                self.render_surface_cursor = 0;
                self.render_surface_w = src_desc.Width;
                self.render_surface_h = src_desc.Height;
                for _ in 0..4 {
                    let mut dst_desc = *src_desc;
                    dst_desc.ArraySize = 1;
                    dst_desc.MipLevels = 1;
                    dst_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                    dst_desc.CPUAccessFlags = 0;
                    dst_desc.MiscFlags = 0;
                    let mut tex = None;
                    unsafe {
                        self.device
                            .CreateTexture2D(&dst_desc, None, Some(&mut tex))
                            .context("CreateTexture2D(renderable NV12) failed")?;
                    }
                    self.render_surfaces
                        .push(tex.context("missing renderable NV12 texture")?);
                }
            }

            let idx = self.render_surface_cursor % self.render_surfaces.len();
            self.render_surface_cursor = self.render_surface_cursor.wrapping_add(1);
            let dst = self.render_surfaces[idx].clone();
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: src_desc.Width,
                bottom: src_desc.Height,
                back: 1,
            };
            unsafe {
                self.context.CopySubresourceRegion(
                    &dst,
                    0,
                    0,
                    0,
                    0,
                    src_tex,
                    src_subresource,
                    Some(&region),
                );
            }
            Ok(dst)
        }

        fn copy_to_shared_nv12_slot(
            &mut self,
            src_tex: &ID3D11Texture2D,
            src_subresource: u32,
            src_desc: &D3D11_TEXTURE2D_DESC,
        ) -> Result<Option<isize>> {
            let trace_copy = std::env::var("MRD_SHARED_KEYED_TRACE")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false)
                && self.frame_index < 32;
            if src_desc.Width == 0 || src_desc.Height == 0 {
                anyhow::bail!("invalid MF output texture size {}x{}", src_desc.Width, src_desc.Height);
            }
            let slot_count = std::env::var("MRD_SHARED_KEYED_SLOTS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(8)
                .clamp(4, 16);
            if self.shared_slots.is_empty()
                || self.shared_slot_w != src_desc.Width
                || self.shared_slot_h != src_desc.Height
            {
                self.shared_slots.clear();
                self.shared_slot_cursor = 0;
                self.shared_slot_w = src_desc.Width;
                self.shared_slot_h = src_desc.Height;
                for _ in 0..slot_count {
                    let mut dst_desc = *src_desc;
                    dst_desc.ArraySize = 1;
                    dst_desc.MipLevels = 1;
                    dst_desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0 as u32;
                    dst_desc.CPUAccessFlags = 0;
                    dst_desc.MiscFlags = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32;
                    let mut tex = None;
                    unsafe {
                        self.device
                            .CreateTexture2D(&dst_desc, None, Some(&mut tex))
                            .context("CreateTexture2D(shared keyed NV12) failed")?;
                    }
                    let tex = tex.context("missing shared keyed NV12 texture")?;
                    let keyed_mutex: IDXGIKeyedMutex = tex
                        .cast()
                        .context("cast shared texture to IDXGIKeyedMutex failed")?;
                    let shared_handle = {
                        let dxgi_resource: IDXGIResource = tex
                            .cast()
                            .context("cast shared texture to IDXGIResource failed")?;
                        let h = unsafe { dxgi_resource.GetSharedHandle() }
                            .context("IDXGIResource::GetSharedHandle failed")?;
                        h.0 as isize
                    };
                    self.shared_slots.push(SharedNv12Slot {
                        texture: tex,
                        keyed_mutex,
                        shared_handle,
                        primed: false,
                    });
                }
            }

            let timeout_ms = std::env::var("MRD_D3D11_KEYED_TIMEOUT_MS")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(1)
                .max(1);
            let total = self.shared_slots.len();
            let mut picked: Option<(usize, u64)> = None;
            for offset in 0..total {
                let idx = (self.shared_slot_cursor + offset) % total;
                let slot = &mut self.shared_slots[idx];
                unsafe {
                    if trace_copy {
                        tracing::info!(slot = idx, "shared-keyed decode before AcquireSync(0)");
                    }
                    match slot.keyed_mutex.AcquireSync(0, timeout_ms) {
                        Ok(()) => {
                            picked = Some((idx, 0));
                            break;
                        }
                        Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                            if !slot.primed {
                                continue;
                            }
                            if slot.keyed_mutex.AcquireSync(1, 0).is_ok() {
                                picked = Some((idx, 1));
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "decode keyed mutex AcquireSync failed; resetting shared slot pool");
                            self.shared_slots.clear();
                            self.shared_slot_cursor = 0;
                            return Ok(None);
                        }
                    }
                }
            }
            let Some((idx, acquired_key)) = picked else {
                return Ok(None);
            };
            self.shared_slot_cursor = (idx + 1) % total;
            let slot = &mut self.shared_slots[idx];
            let region = D3D11_BOX {
                left: 0,
                top: 0,
                front: 0,
                right: src_desc.Width,
                bottom: src_desc.Height,
                back: 1,
            };
            unsafe {
                if trace_copy {
                    tracing::info!(slot = idx, "shared-keyed decode before CopySubresourceRegion");
                }
                self.context.CopySubresourceRegion(
                    &slot.texture,
                    0,
                    0,
                    0,
                    0,
                    src_tex,
                    src_subresource,
                    Some(&region),
                );
                if trace_copy {
                    tracing::info!(slot = idx, acquired_key, "shared-keyed decode before ReleaseSync(1)");
                }
                if let Err(e) = slot.keyed_mutex.ReleaseSync(1) {
                    tracing::warn!(
                        error = %e,
                        slot = idx,
                        acquired_key,
                        primed = slot.primed,
                        "decode keyed mutex ReleaseSync failed; resetting shared slot pool"
                    );
                    self.shared_slots.clear();
                    self.shared_slot_cursor = 0;
                    return Ok(None);
                }
                if trace_copy {
                    tracing::info!(slot = idx, "shared-keyed decode released");
                }
            }
            slot.primed = true;
            if trace_copy && acquired_key == 1 {
                tracing::debug!("reclaimed stale shared slot with key=1");
            }
            Ok(Some(slot.shared_handle))
        }
    }

    impl Decoder for MfH264Decoder {
        fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
            self.send_input_sample(frame)?;
            self.drain_output()?;
            Ok(self.queued.pop_front())
        }

        fn flush(&mut self) -> Result<Option<DecodedFrame>> {
            unsafe {
                self.transform
                    .ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)
                    .context("MFT flush failed")?;
            }
            self.queued.clear();
            Ok(None)
        }

        fn output_size(&self) -> Option<(u32, u32)> {
            if self.output_width > 0 && self.output_height > 0 {
                Some((self.output_width, self.output_height))
            } else {
                None
            }
        }

        fn backend_name(&self) -> &'static str {
            "mf_d3d11"
        }
    }

    fn create_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
        unsafe {
            let feature_levels = [D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            let mut chosen_level = D3D_FEATURE_LEVEL_11_0;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut chosen_level),
                Some(&mut context),
            )
            .context("D3D11CreateDevice for MF failed")?;
            Ok((
                device.context("missing D3D11 device for MF")?,
                context.context("missing D3D11 context for MF")?,
            ))
        }
    }

    fn create_dxgi_device_manager(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager> {
        let mut reset_token = 0u32;
        let mut manager: Option<IMFDXGIDeviceManager> = None;
        unsafe {
            MFCreateDXGIDeviceManager(&mut reset_token, &mut manager)
                .context("MFCreateDXGIDeviceManager failed")?;
        }
        let manager = manager.context("MFCreateDXGIDeviceManager returned null")?;
        let dev_unk: IUnknown = device.cast().context("cast D3D11Device->IUnknown failed")?;
        unsafe {
            manager
                .ResetDevice(&dev_unk, reset_token)
                .context("IMFDXGIDeviceManager::ResetDevice failed")?;
        }
        Ok(manager)
    }

    fn query_stream_ids(transform: &IMFTransform) -> Result<(u32, u32)> {
        let mut input_count = 0u32;
        let mut output_count = 0u32;
        unsafe {
            transform
                .GetStreamCount(&mut input_count, &mut output_count)
                .context("GetStreamCount failed")?;
        }
        let mut input_ids = vec![0u32; input_count.max(1) as usize];
        let mut output_ids = vec![0u32; output_count.max(1) as usize];
        let r = unsafe { transform.GetStreamIDs(&mut input_ids, &mut output_ids) };
        match r {
            Ok(()) => Ok((input_ids[0], output_ids[0])),
            Err(e) if e.code() == E_NOTIMPL => Ok((0, 0)),
            Err(e) => Err(anyhow::anyhow!("GetStreamIDs failed: {e}")),
        }
    }

    fn extract_media_buffer_bytes(buf: &IMFMediaBuffer) -> Result<Vec<u8>> {
        let mut ptr = std::ptr::null_mut::<u8>();
        let mut cur = 0u32;
        unsafe {
            buf.Lock(&mut ptr, None, Some(&mut cur))
                .context("IMFMediaBuffer::Lock failed")?;
            let out = std::slice::from_raw_parts(ptr, cur as usize).to_vec();
            buf.Unlock().ok();
            Ok(out)
        }
    }

    fn pack_u64(high: u32, low: u32) -> u64 {
        ((high as u64) << 32) | (low as u64)
    }
}

#[cfg(feature = "ffmpeg-software")]
mod ffmpeg_backend {
    use super::*;
    use ffmpeg_next::{
        codec, decoder, format,
        software::scaling::{context::Context as Scaler, flag::Flags},
        util::error::EAGAIN,
        util::frame::Video,
        Codec, Error,
    };

    pub struct FfmpegH264Decoder {
        decoder: decoder::Video,
        video_frame: Video,
        scaler: Option<Scaler>,
        output_width: u32,
        output_height: u32,
        backend_name: String,
        first_output_logged: bool,
        wants_hw: bool,
        require_hw: bool,
        warned_non_hw_output: bool,
    }

    // Decoder is guarded by a mutex in upper layer; one-thread access.
    unsafe impl Send for FfmpegH264Decoder {}

    impl FfmpegH264Decoder {
        pub fn new(config: H264DecoderConfig) -> Result<Self> {
            ffmpeg_next::init()?;

            let codec = pick_decoder_codec(&config)
                .context("H.264 decoder codec not found")?;
            let backend_name = codec.name().to_string();
            let wants_hw = wants_d3d11va(&config);
            let has_d3d11va_decoder = decoder::find_by_name("h264_d3d11va").is_some();
            let require_hw = std::env::var("MRD_REQUIRE_D3D11VA")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if wants_hw {
                tracing::info!(
                    selected_codec = %backend_name,
                    has_h264_d3d11va_decoder = has_d3d11va_decoder,
                    require_d3d11va = require_hw,
                    "decoder hardware intent"
                );
                if require_hw && !has_d3d11va_decoder {
                    anyhow::bail!("MRD_REQUIRE_D3D11VA=1 but ffmpeg h264_d3d11va decoder is unavailable");
                }
            }

            let opened = if wants_hw && backend_name == "h264_d3d11va" {
                let mut ctx = build_decoder_context(&config);
                match ctx.decoder().open_as(codec) {
                    Ok(opened) => {
                        tracing::info!("opened ffmpeg h264_d3d11va decoder");
                        opened
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "open h264_d3d11va decoder failed, fallback to plain h264 decoder"
                        );
                        let fallback = decoder::find(codec::Id::H264)
                            .context("fallback H.264 decoder codec not found")?;
                        let mut fallback_ctx = build_decoder_context(&config);
                        fallback_ctx
                            .decoder()
                            .open_as(fallback)
                            .context("open fallback h264 decoder failed")?
                    }
                }
            } else if wants_hw {
                let mut opts = ffmpeg_next::Dictionary::new();
                opts.set("hwaccel", "d3d11va");
                opts.set("hwaccel_output_format", "d3d11");
                let mut ctx = build_decoder_context(&config);
                match ctx.decoder().open_as_with(codec, opts) {
                    Ok(opened) => {
                        tracing::info!(
                            "opened ffmpeg h264 decoder with d3d11va options"
                        );
                        opened
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "open_as_with d3d11va failed, fallback to plain h264 decoder"
                        );
                        let mut fallback_ctx = build_decoder_context(&config);
                        fallback_ctx.decoder()
                            .open_as(codec)
                            .context("open fallback h264 decoder failed")?
                    }
                }
            } else {
                let mut ctx = build_decoder_context(&config);
                ctx.decoder()
                    .open_as(codec)
                    .context("open decoder failed")?
            };
            let decoder = opened
                .video()
                .context("video decoder init failed")?;

            Ok(Self {
                decoder,
                video_frame: Video::empty(),
                scaler: None,
                output_width: 0,
                output_height: 0,
                backend_name,
                first_output_logged: false,
                wants_hw,
                require_hw,
                warned_non_hw_output: false,
            })
        }

        fn send_packet(&mut self, data: &[u8]) -> Result<()> {
            let mut pkt = ffmpeg_next::Packet::copy(data);
            pkt.set_stream(0);
            match self.decoder.send_packet(&pkt) {
                Ok(()) => Ok(()),
                Err(Error::Other { errno }) if errno == EAGAIN => {
                    // Decoder input queue is full; surface as soft backpressure and
                    // let decode() continue through receive_frame() path.
                    Ok(())
                }
                Err(e) => Err(anyhow::anyhow!("send packet to decoder failed: {}", e)),
            }
        }

        fn receive_frame(&mut self) -> Result<Option<DecodedFrame>> {
            match self.decoder.receive_frame(&mut self.video_frame) {
                Ok(_) => {
                    let width = self.video_frame.width();
                    let height = self.video_frame.height();
                    self.output_width = width;
                    self.output_height = height;
                    let pts = self.video_frame.pts().unwrap_or_default() as u64;
                    if !self.first_output_logged {
                        tracing::info!(
                            decoder = %self.backend_name,
                            output_format = ?self.video_frame.format(),
                            "ffmpeg decoder first output frame format"
                        );
                        self.first_output_logged = true;
                    }

                    #[cfg(windows)]
                    if self.wants_hw
                        && self.video_frame.format() != format::Pixel::D3D11
                        && !self.warned_non_hw_output
                    {
                        self.warned_non_hw_output = true;
                        if self.require_hw {
                            anyhow::bail!(
                                "MRD_REQUIRE_D3D11VA=1 but decoder output is {:?}, not D3D11",
                                self.video_frame.format()
                            );
                        }
                        tracing::warn!(
                            decoder = %self.backend_name,
                            output_format = ?self.video_frame.format(),
                            "hardware decode requested but output is not D3D11; falling back to CPU upload path"
                        );
                    }

                    #[cfg(windows)]
                    if self.video_frame.format() == format::Pixel::D3D11 {
                        if let Some((texture, subresource)) =
                            self.extract_d3d11_surface(&self.video_frame)?
                        {
                            return Ok(Some(DecodedFrame::from_d3d11_nv12(
                                texture,
                                subresource,
                                width,
                                height,
                                pts,
                                pts,
                                0,
                            )));
                        }
                    }

                    let nv12 = if self.video_frame.format() == format::Pixel::NV12 {
                        self.extract_nv12(&self.video_frame)?
                    } else {
                        self.convert_to_nv12()?
                    };

                    Ok(Some(DecodedFrame::from_cpu_nv12(
                        Arc::new(nv12),
                        width,
                        height,
                        pts,
                        pts,
                        0,
                    )))
                }
                Err(Error::Other { errno }) if errno == EAGAIN => Ok(None),
                Err(Error::Eof) => Ok(None),
                Err(e) => Err(anyhow::anyhow!("decoder receive failed: {}", e)),
            }
        }

        #[cfg(windows)]
        fn extract_d3d11_surface(
            &self,
            frame: &Video,
        ) -> Result<Option<(windows::Win32::Graphics::Direct3D11::ID3D11Texture2D, u32)>> {
            use std::ffi::c_void;
            use windows::core::Interface;
            use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

            unsafe {
                let av = frame.as_ptr();
                if av.is_null() || (*av).data[0].is_null() {
                    return Ok(None);
                }

                let raw_ptr = (*av).data[0] as *mut c_void;
                let tex = ID3D11Texture2D::from_raw_borrowed(&raw_ptr)
                    .context("invalid D3D11 texture pointer from AVFrame")?
                    .clone();
                let subresource = (*av).data[1] as usize as u32;
                Ok(Some((tex, subresource)))
            }
        }

        fn extract_nv12(&self, frame: &Video) -> Result<Vec<u8>> {
            let width = frame.width() as usize;
            let height = frame.height() as usize;
            let mut out = vec![0u8; width * height * 3 / 2];

            let y_plane = frame.data(0);
            let y_stride = frame.stride(0);
            for row in 0..height {
                let src = row * y_stride;
                let dst = row * width;
                out[dst..dst + width].copy_from_slice(&y_plane[src..src + width]);
            }

            let uv_plane = frame.data(1);
            let uv_stride = frame.stride(1);
            let y_size = width * height;
            for row in 0..(height / 2) {
                let src = row * uv_stride;
                let dst = y_size + row * width;
                out[dst..dst + width].copy_from_slice(&uv_plane[src..src + width]);
            }
            Ok(out)
        }

        fn convert_to_nv12(&mut self) -> Result<Vec<u8>> {
            let mut dst = Video::empty();
            unsafe {
                dst.alloc(
                    format::Pixel::NV12,
                    self.video_frame.width(),
                    self.video_frame.height(),
                );
            }

            if self.scaler.is_none() {
                self.scaler = Some(Scaler::get(
                    self.video_frame.format(),
                    self.video_frame.width(),
                    self.video_frame.height(),
                    format::Pixel::NV12,
                    self.video_frame.width(),
                    self.video_frame.height(),
                    Flags::BILINEAR,
                )?);
            }
            if let Some(scaler) = &mut self.scaler {
                scaler.run(&self.video_frame, &mut dst)?;
            }
            self.extract_nv12(&dst)
        }
    }

    pub(super) fn preferred_decoder_names(config: &H264DecoderConfig) -> Vec<&'static str> {
        if wants_d3d11va(config) {
            vec!["h264_d3d11va", "h264"]
        } else {
            vec!["h264"]
        }
    }

    fn pick_decoder_codec(config: &H264DecoderConfig) -> Option<Codec> {
        for name in preferred_decoder_names(config) {
            if let Some(c) = decoder::find_by_name(name) {
                return Some(c);
            }
        }
        decoder::find(codec::Id::H264)
    }

    fn build_decoder_context(config: &H264DecoderConfig) -> codec::context::Context {
        let mut ctx = codec::context::Context::new();
        ctx.set_threading(codec::threading::Config {
            kind: codec::threading::Type::Frame,
            count: config.num_threads,
        });
        ctx
    }

    pub(super) fn wants_d3d11va(config: &H264DecoderConfig) -> bool {
        matches!(config.backend, DecoderBackend::D3d11va)
            || (matches!(config.backend, DecoderBackend::Auto) && config.enable_hardware)
    }

    impl Decoder for FfmpegH264Decoder {
        fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
            self.send_packet(&frame.data)?;
            self.receive_frame()
        }

        fn flush(&mut self) -> Result<Option<DecodedFrame>> {
            self.decoder.send_eof()?;
            self.receive_frame()
        }

        fn output_size(&self) -> Option<(u32, u32)> {
            if self.output_width > 0 && self.output_height > 0 {
                Some((self.output_width, self.output_height))
            } else {
                None
            }
        }

        fn backend_name(&self) -> &'static str {
            if self.backend_name == "h264_d3d11va" {
                "h264_d3d11va"
            } else {
                "h264"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_frame_cpu_helpers_expose_planes() {
        let frame = DecodedFrame::from_cpu_nv12(
            Arc::new(vec![0u8; 1280 * 720 * 3 / 2]),
            1280,
            720,
            0,
            0,
            0,
        );
        assert!(frame.cpu_nv12().is_some());
        assert_eq!(frame.y_plane().unwrap().len(), 1280 * 720);
        assert_eq!(frame.uv_plane().unwrap().len(), 1280 * 720 / 2);
    }

    #[test]
    fn decoded_frame_layout() {
        let frame = DecodedFrame::from_cpu_nv12(
            Arc::new(vec![0u8; 1280 * 720 * 3 / 2]),
            1280,
            720,
            0,
            0,
            0,
        );
        assert_eq!(frame.y_plane().unwrap().len(), 1280 * 720);
        assert_eq!(frame.uv_plane().unwrap().len(), 1280 * 720 / 2);
    }

    #[cfg(feature = "ffmpeg-software")]
    #[test]
    fn d3d11va_intent_from_config() {
        let mut c = H264DecoderConfig::default();
        c.backend = DecoderBackend::D3d11va;
        assert!(ffmpeg_backend::wants_d3d11va(&c));

        c.backend = DecoderBackend::Auto;
        c.enable_hardware = true;
        assert!(ffmpeg_backend::wants_d3d11va(&c));

        c.enable_hardware = false;
        assert!(!ffmpeg_backend::wants_d3d11va(&c));
    }

    #[cfg(feature = "ffmpeg-software")]
    #[test]
    fn prefers_d3d11va_decoder_name_when_hw_requested() {
        let mut c = H264DecoderConfig::default();
        c.backend = DecoderBackend::D3d11va;
        let names = ffmpeg_backend::preferred_decoder_names(&c);
        assert_eq!(names.first().copied(), Some("h264_d3d11va"));
    }

    #[cfg(feature = "ffmpeg-software")]
    #[test]
    fn prefers_software_decoder_name_when_hw_disabled() {
        let mut c = H264DecoderConfig::default();
        c.backend = DecoderBackend::Software;
        c.enable_hardware = false;
        let names = ffmpeg_backend::preferred_decoder_names(&c);
        assert_eq!(names, vec!["h264"]);
    }
}
