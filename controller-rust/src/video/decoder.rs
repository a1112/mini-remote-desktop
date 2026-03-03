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
    #[cfg(feature = "ffmpeg-software")]
    Ffmpeg(ffmpeg_backend::FfmpegH264Decoder),
    Disabled,
}

impl H264Decoder {
    pub fn new(config: H264DecoderConfig) -> Result<Self> {
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
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.flush(),
            Self::Disabled => Ok(None),
        }
    }

    fn output_size(&self) -> Option<(u32, u32)> {
        match self {
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.output_size(),
            Self::Disabled => None,
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.backend_name(),
            Self::Disabled => "disabled",
        }
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
