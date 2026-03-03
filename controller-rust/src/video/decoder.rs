use super::super::webrtc::peer::VideoFrame;
use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    pub data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub sequence: u64,
    pub capture_start_unix_us: u64,
}

impl DecodedFrame {
    pub fn y_size(&self) -> usize {
        (self.width * self.height) as usize
    }

    pub fn y_plane(&self) -> &[u8] {
        &self.data[..self.y_size()]
    }

    pub fn uv_plane(&self) -> &[u8] {
        &self.data[self.y_size()..]
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
    }

    // Decoder is guarded by a mutex in upper layer; one-thread access.
    unsafe impl Send for FfmpegH264Decoder {}

    impl FfmpegH264Decoder {
        pub fn new(config: H264DecoderConfig) -> Result<Self> {
            ffmpeg_next::init()?;

            let codec = pick_decoder_codec(&config)
                .context("H.264 decoder codec not found")?;
            let backend_name = codec.name().to_string();
            let mut ctx = codec::context::Context::new();
            ctx.set_threading(codec::threading::Config {
                kind: codec::threading::Type::Frame,
                count: config.num_threads,
            });

            let decoder = ctx
                .decoder()
                .open_as(codec)
                .context("open decoder failed")?
                .video()
                .context("video decoder init failed")?;

            Ok(Self {
                decoder,
                video_frame: Video::empty(),
                scaler: None,
                output_width: 0,
                output_height: 0,
                backend_name,
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

                    let nv12 = if self.video_frame.format() == format::Pixel::NV12 {
                        self.extract_nv12(&self.video_frame)?
                    } else {
                        self.convert_to_nv12()?
                    };

                    Ok(Some(DecodedFrame {
                        data: Arc::new(nv12),
                        width,
                        height,
                        timestamp: self.video_frame.pts().unwrap_or_default() as u64,
                        sequence: self.video_frame.pts().unwrap_or_default() as u64,
                        capture_start_unix_us: 0,
                    }))
                }
                Err(Error::Other { errno }) if errno == EAGAIN => Ok(None),
                Err(Error::Eof) => Ok(None),
                Err(e) => Err(anyhow::anyhow!("decoder receive failed: {}", e)),
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

    fn pick_decoder_codec(config: &H264DecoderConfig) -> Option<Codec> {
        if matches!(config.backend, DecoderBackend::D3d11va)
            || (matches!(config.backend, DecoderBackend::Auto) && config.enable_hardware)
        {
            if let Some(c) = decoder::find_by_name("h264_d3d11va") {
                return Some(c);
            }
            tracing::warn!("h264_d3d11va decoder not found, fallback to software h264");
        }
        decoder::find(codec::Id::H264)
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
    fn decoded_frame_layout() {
        let frame = DecodedFrame {
            data: Arc::new(vec![0u8; 1280 * 720 * 3 / 2]),
            width: 1280,
            height: 720,
            timestamp: 0,
            sequence: 0,
            capture_start_unix_us: 0,
        };
        assert_eq!(frame.y_plane().len(), 1280 * 720);
        assert_eq!(frame.uv_plane().len(), 1280 * 720 / 2);
    }
}
