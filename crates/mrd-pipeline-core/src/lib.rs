use mrd_proto::SessionId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RuntimeStatus {
    ProfileOnly,
    RuntimeBacked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeBinding {
    pub session_id: SessionId,
    pub runtime_status: RuntimeStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FramePixelFormat {
    Bgra32,
    Rgba32,
    Rgb24,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapturedFrame {
    pub width: usize,
    pub height: usize,
    pub pixel_format: FramePixelFormat,
    pub timestamp_us: u64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncodedAccessUnit {
    pub codec: VideoCodec,
    pub timestamp_us: u64,
    pub is_keyframe: bool,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("{0}")]
    Message(String),
}

impl PipelineError {
    pub fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

pub trait FrameCapture {
    fn capture_frame(&mut self) -> Result<CapturedFrame, PipelineError>;
}

pub trait VideoEncoder {
    fn encode(&mut self, frame: &CapturedFrame) -> Result<Vec<EncodedAccessUnit>, PipelineError>;
}
