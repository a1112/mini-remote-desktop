use super::super::webrtc::peer::VideoFrame;
use anyhow::{Context, Result};
use std::sync::Arc;

/// 解码后的帧数据
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// NV12 格式数据 (Y 平面 + UV 交错平面)
    pub data: Arc<Vec<u8>>,
    /// 宽度
    pub width: u32,
    /// 高度
    pub height: u32,
    /// 时间戳
    pub timestamp: u64,
    /// 帧序列号
    pub sequence: u64,
}

impl DecodedFrame {
    /// 计算 Y 平面大小
    pub fn y_size(&self) -> usize {
        (self.width * self.height) as usize
    }

    /// 计算 UV 平面大小
    pub fn uv_size(&self) -> usize {
        (self.width * self.height / 2) as usize
    }

    /// 获取总大小
    pub fn total_size(&self) -> usize {
        self.y_size() + self.uv_size()
    }

    /// 获取 Y 平面切片
    pub fn y_plane(&self) -> &[u8] {
        &self.data[..self.y_size()]
    }

    /// 获取 UV 平面切片
    pub fn uv_plane(&self) -> &[u8] {
        &self.data[self.y_size()..]
    }
}

/// H.264 解码器配置
#[derive(Debug, Clone)]
pub struct H264DecoderConfig {
    /// 解码线程数
    pub num_threads: usize,
    /// 是否启用硬件加速
    pub enable_hardware: bool,
}

impl Default for H264DecoderConfig {
    fn default() -> Self {
        Self {
            num_threads: 4,
            enable_hardware: true,
        }
    }
}

/// H.264 解码器特质
pub trait Decoder: Send + Sync {
    /// 解码 H.264 帧
    fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>>;

    /// 刷新解码器
    fn flush(&mut self) -> Result<Option<DecodedFrame>>;

    /// 获取当前输出尺寸
    fn output_size(&self) -> Option<(u32, u32)>;
}

/// H.264 解码器枚举
pub enum H264Decoder {
    #[cfg(feature = "ffmpeg-software")]
    Ffmpeg(ffmpeg_software::FfmpegH264Decoder),
    Disabled(DisabledDecoder),
}

/// 禁用的解码器（用于没有启用解码功能时）
struct DisabledDecoder;

impl Decoder for DisabledDecoder {
    fn decode(&mut self, _frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
        Ok(None)
    }

    fn flush(&mut self) -> Result<Option<DecodedFrame>> {
        Ok(None)
    }

    fn output_size(&self) -> Option<(u32, u32)> {
        None
    }
}

impl H264Decoder {
    /// 创建新的 H.264 解码器
    pub fn new(_config: H264DecoderConfig) -> Result<Self> {
        #[cfg(feature = "ffmpeg-software")]
        {
            Ok(Self::Ffmpeg(ffmpeg_software::FfmpegH264Decoder::new(
                config,
            )?))
        }

        #[cfg(not(feature = "ffmpeg-software"))]
        {
            tracing::warn!("No decoder backend enabled, video decoding will not work");
            Ok(Self::Disabled(DisabledDecoder))
        }
    }
}

impl Decoder for H264Decoder {
    fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
        match self {
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.decode(frame),

            Self::Disabled(decoder) => decoder.decode(frame),
        }
    }

    fn flush(&mut self) -> Result<Option<DecodedFrame>> {
        match self {
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.flush(),

            Self::Disabled(decoder) => decoder.flush(),
        }
    }

    fn output_size(&self) -> Option<(u32, u32)> {
        match self {
            #[cfg(feature = "ffmpeg-software")]
            Self::Ffmpeg(decoder) => decoder.output_size(),

            Self::Disabled(decoder) => decoder.output_size(),
        }
    }
}

/// FFmpeg 软件 H.264 解码器
#[cfg(feature = "ffmpeg-software")]
pub mod ffmpeg_software {
    use super::*;

    use ffmpeg_next::{
        codec, decoder, format, software::scaling, util::frame::Video,
    };

    pub struct FfmpegH264Decoder {
        decoder: decoder::Video,
        video_frame: Video,
        scaler: Option<scaling::Context>,
        output_width: u32,
        output_height: u32,
        num_threads: usize,
    }

    impl FfmpegH264Decoder {
        pub fn new(config: H264DecoderConfig) -> Result<Self> {
            // 初始化 FFmpeg
            ffmpeg_next::init()?;

            // 查找 H.264 解码器
            let decoder_id = codec::find(codec::Id::H264)
                .context("H.264 decoder not found")?;

            // 创建解码器
            let mut decoder = decoder::Video::new(
                decoder_id,
                config.num_threads as i32,
            );

            // 设置解码器选项
            decoder.set_threading(config.num_threads as i32);
            decoder.set_thread_type(ffmpeg_next::threading::Type::Frame);

            Ok(Self {
                decoder,
                video_frame: Video::empty(),
                scaler: None,
                output_width: 0,
                output_height: 0,
                num_threads: config.num_threads,
            })
        }

        /// 发送数据到解码器
        fn send_packet(&mut self, data: &[u8]) -> Result<()> {
            let packet = {
                let mut pkt = ffmpeg_next::packet::Packet::copy(data);
                pkt.set_stream(0);
                pkt
            };

            self.decoder.send_packet(&packet).context(
                "failed to send packet to decoder"
            )?;

            Ok(())
        }

        /// 从解码器接收解码后的帧
        fn receive_frame(&mut self) -> Result<Option<DecodedFrame>> {
            match self.decoder.receive_frame(&mut self.video_frame) {
                Ok(_) => {
                    let width = self.video_frame.width();
                    let height = self.video_frame.height();

                    // 如果输出尺寸改变，重新创建 scaler
                    if width != self.output_width || height != self.output_height {
                        self.output_width = width;
                        self.output_height = height;
                        self.scaler = None;
                    }

                    // 如果帧不是 NV12 格式，需要转换
                    let format = self.video_frame.format();

                    // 获取 YUV 数据
                    let decoded_data = if format == format::Pixel::NV12 {
                        // 已经是 NV12 格式，直接使用
                        self.extract_nv12_data()?
                    } else {
                        // 需要转换为 NV12
                        self.convert_to_nv12()?
                    };

                    Ok(Some(DecodedFrame {
                        data: Arc::new(decoded_data),
                        width,
                        height,
                        timestamp: self.video_frame.ts() as u64,
                        sequence: self.video_frame.pts().map_or(0, |p| p as u64),
                    }))
                }
                Err(ffmpeg_next::Error::Other { errno: 35 }) => {
                    // EAGAIN: 需要更多数据
                    Ok(None)
                }
                Err(e) => {
                    Err(anyhow::anyhow!("decoder error: {}", e))
                }
            }
        }

        /// 提取 NV12 数据
        fn extract_nv12_data(&mut self) -> Result<Vec<u8>> {
            let width = self.output_width as usize;
            let height = self.output_height as usize;

            let y_size = width * height;
            let uv_size = width * height / 2;
            let total_size = y_size + uv_size;

            let mut data = vec![0u8; total_size];

            // 复制 Y 平面
            let y_plane = self.video_frame.data(0);
            let y_stride = self.video_frame.stride(0);
            for y in 0..height {
                let src_offset = y * y_stride;
                let dst_offset = y * width;
                data[dst_offset..dst_offset + width]
                    .copy_from_slice(&y_plane[src_offset..src_offset + width]);
            }

            // 复制 UV 平面
            let uv_plane = self.video_frame.data(1);
            let uv_stride = self.video_frame.stride(1);
            for y in 0..height / 2 {
                let src_offset = y * uv_stride;
                let dst_offset = y_size + y * width;
                data[dst_offset..dst_offset + width]
                    .copy_from_slice(&uv_plane[src_offset..src_offset + width]);
            }

            Ok(data)
        }

        /// 转换为 NV12 格式
        fn convert_to_nv12(&mut self) -> Result<Vec<u8>> {
            let width = self.output_width as usize;
            let height = self.output_height as usize;

            // 创建目标帧
            let mut dst_frame = Video::empty();
            dst_frame.set_format(format::Pixel::NV12);
            dst_frame.set_width(self.output_width);
            dst_frame.set_height(self.output_height);

            // 分配缓冲区
            dst_frame.alloc()
                .context("failed to allocate video frame")?;

            // 创建或更新 scaler
            if self.scaler.is_none() {
                self.scaler = Some(scaling::Context::get(
                    scaling::Flags::BILINEAR,
                    self.video_frame.width(),
                    self.video_frame.height(),
                    self.video_frame.format(),
                    self.output_width,
                    self.output_height,
                    format::Pixel::NV12,
                )?);
            }

            // 执行转换
            if let Some(scaler) = &mut self.scaler {
                scaler.run(&self.video_frame, &mut dst_frame)?;
            }

            // 提取 NV12 数据
            let y_size = width * height;
            let uv_size = width * height / 2;
            let total_size = y_size + uv_size;

            let mut data = vec![0u8; total_size];

            // 复制 Y 平面
            let y_plane = dst_frame.data(0);
            let y_stride = dst_frame.stride(0);
            for y in 0..height {
                let src_offset = y * y_stride;
                let dst_offset = y * width;
                data[dst_offset..dst_offset + width]
                    .copy_from_slice(&y_plane[src_offset..src_offset + width]);
            }

            // 复制 UV 平面
            let uv_plane = dst_frame.data(1);
            let uv_stride = dst_frame.stride(1);
            for y in 0..height / 2 {
                let src_offset = y * uv_stride;
                let dst_offset = y_size + y * width;
                data[dst_offset..dst_offset + width]
                    .copy_from_slice(&uv_plane[src_offset..src_offset + width]);
            }

            Ok(data)
        }
    }

    impl Decoder for FfmpegH264Decoder {
        fn decode(&mut self, frame: &VideoFrame) -> Result<Option<DecodedFrame>> {
            // 发送数据到解码器
            self.send_packet(&frame.data)?;

            // 尝试接收解码后的帧
            self.receive_frame()
        }

        fn flush(&mut self) -> Result<Option<DecodedFrame>> {
            // 发送空数据包以刷新解码器
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
    }
}

/// 简单的 NV12 到 RGBA 转换器
pub fn nv12_to_rgba(nv12: &[u8], width: u32, height: u32, rgba: &mut [u8]) {
    let width = width as usize;
    let height = height as usize;

    let y_plane = &nv12[..width * height];
    let uv_plane = &nv12[width * height..];

    for y in 0..height {
        for x in 0..width {
            let y_idx = y * width + x;
            let uv_idx = (y / 2) * width + (x & !1);

            let y_val = y_plane[y_idx] as i32;
            let u_val = uv_plane[uv_idx] as i32 - 128;
            let v_val = uv_plane[uv_idx + 1] as i32 - 128;

            // YUV 到 RGB 转换
            let r = (y_val as f32 + 0.0 * u_val as f32 + 1.402 * v_val as f32) as u8;
            let g = (y_val as f32 - 0.344136 * u_val as f32 - 0.714136 * v_val as f32) as u8;
            let b = (y_val as f32 + 1.772 * u_val as f32 + 0.0 * v_val as f32) as u8;

            let rgba_idx = (y * width + x) * 4;
            rgba[rgba_idx] = r;
            rgba[rgba_idx + 1] = g;
            rgba[rgba_idx + 2] = b;
            rgba[rgba_idx + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoded_frame_size_calculation() {
        let frame = DecodedFrame {
            data: Arc::new(vec![0u8; 1920 * 1080 * 3 / 2]),
            width: 1920,
            height: 1080,
            timestamp: 0,
            sequence: 0,
        };

        assert_eq!(frame.y_size(), 1920 * 1080);
        assert_eq!(frame.uv_size(), 1920 * 1080 / 2);
        assert_eq!(frame.total_size(), 1920 * 1080 * 3 / 2);
    }
}
