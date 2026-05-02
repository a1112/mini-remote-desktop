//! Hardware codec detection and factory
//!
//! Provides automatic hardware codec detection and fallback mechanisms
//! similar to RustDesk's implementation.
//!
//! # Example
//!
//! ```no_run
//! use mrd_hardware::{choose_encoder_backend, EncoderSelectorConfig, VideoEncoderBackend};
//!
//! // Detect and select encoder with automatic fallback
//! let config = EncoderSelectorConfig::new(1920, 1080, 30);
//! let selection = choose_encoder_backend(config);
//! println!("Selected encoder: {:?}", selection.backend);
//! ```

use std::fmt;

pub mod decoder;
pub mod encoder;
pub mod gpu;

pub use decoder::{choose_decoder_backend, create_decoder, DecoderDescriptor};
pub use encoder::{
    choose_encoder_backend, create_encoder, EncoderDescriptor, EncoderSelectorConfig,
};
pub use gpu::{detect_gpu_caps, GpuCaps};

/// Video encoder backend types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoEncoderBackend {
    /// NVIDIA NVENC hardware encoder
    Nvenc,
    /// OpenH264 software encoder
    OpenH264,
}

impl VideoEncoderBackend {
    /// Get the backend name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::OpenH264 => "openh264",
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "nvenc" | "nvidia" => Some(Self::Nvenc),
            "openh264" | "software" | "sw" | "software_h264" | "h264_software"
            | "software-h264" | "h264-software" | "sw_h264" => Some(Self::OpenH264),
            _ => None,
        }
    }

    /// Check if this is a hardware backend
    pub fn is_hardware(self) -> bool {
        matches!(self, Self::Nvenc)
    }

    /// Check if this is a software backend
    pub fn is_software(self) -> bool {
        matches!(self, Self::OpenH264)
    }
}

impl fmt::Display for VideoEncoderBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Video decoder backend types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoDecoderBackend {
    /// NVIDIA NVDEC hardware decoder
    Nvdec,
    /// Software H.264 decoder
    Software,
}

impl VideoDecoderBackend {
    /// Get the backend name as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nvdec => "nvdec",
            Self::Software => "software",
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "nvdec" | "nvidia" => Some(Self::Nvdec),
            "software" | "sw" | "software_h264" | "h264_software" | "software-h264"
            | "h264-software" | "openh264" => Some(Self::Software),
            _ => None,
        }
    }

    /// Check if this is a hardware backend
    pub fn is_hardware(self) -> bool {
        matches!(self, Self::Nvdec)
    }

    /// Check if this is a software backend
    pub fn is_software(self) -> bool {
        matches!(self, Self::Software)
    }
}

impl fmt::Display for VideoDecoderBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result of encoder backend selection
#[derive(Debug, Clone)]
pub struct EncoderSelection {
    /// Selected backend
    pub backend: VideoEncoderBackend,
    /// Selection log messages
    pub logs: Vec<String>,
    /// Whether hardware was selected
    pub using_hardware: bool,
}

/// Result of decoder backend selection
#[derive(Debug, Clone)]
pub struct DecoderSelection {
    /// Selected backend
    pub backend: VideoDecoderBackend,
    /// Selection log messages
    pub logs: Vec<String>,
    /// Whether hardware was selected
    pub using_hardware: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_backend_from_str_works() {
        assert_eq!(
            VideoEncoderBackend::from_str("nvenc"),
            Some(VideoEncoderBackend::Nvenc)
        );
        assert_eq!(
            VideoEncoderBackend::from_str("NVENC"),
            Some(VideoEncoderBackend::Nvenc)
        );
        assert_eq!(
            VideoEncoderBackend::from_str("openh264"),
            Some(VideoEncoderBackend::OpenH264)
        );
        assert_eq!(
            VideoEncoderBackend::from_str("software_h264"),
            Some(VideoEncoderBackend::OpenH264)
        );
        assert_eq!(
            VideoEncoderBackend::from_str("h264-software"),
            Some(VideoEncoderBackend::OpenH264)
        );
        assert_eq!(VideoEncoderBackend::from_str("unknown"), None);
    }

    #[test]
    fn encoder_backend_is_hardware_detection() {
        assert!(VideoEncoderBackend::Nvenc.is_hardware());
        assert!(!VideoEncoderBackend::OpenH264.is_hardware());
    }

    #[test]
    fn decoder_backend_from_str_works() {
        assert_eq!(
            VideoDecoderBackend::from_str("nvdec"),
            Some(VideoDecoderBackend::Nvdec)
        );
        assert_eq!(
            VideoDecoderBackend::from_str("software"),
            Some(VideoDecoderBackend::Software)
        );
        assert_eq!(
            VideoDecoderBackend::from_str("h264_software"),
            Some(VideoDecoderBackend::Software)
        );
        assert_eq!(
            VideoDecoderBackend::from_str("software-h264"),
            Some(VideoDecoderBackend::Software)
        );
        assert_eq!(VideoDecoderBackend::from_str("unknown"), None);
    }

    #[test]
    fn gpu_caps_default_has_sensible_fields() {
        let caps = GpuCaps::default();
        // Default should assume no hardware
        assert!(!caps.has_nvidia);
    }
}
