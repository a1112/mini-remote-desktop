//! Video encoder selection and factory
//!
//! Provides automatic hardware encoder detection and fallback.

use crate::{EncoderSelection, VideoEncoderBackend};
use mrd_pipeline_core::{PipelineError, VideoEncoder};
use std::fmt;

/// Encoder selector configuration
#[derive(Debug, Clone)]
pub struct EncoderSelectorConfig {
    /// Requested encoder backend (empty = auto-detect)
    pub requested_backend: Option<VideoEncoderBackend>,
    /// Width of video to encode
    pub width: usize,
    /// Height of video to encode
    pub height: usize,
    /// Target FPS
    pub fps: u32,
    /// Whether to allow fallback to software encoder
    pub allow_fallback: bool,
}

impl EncoderSelectorConfig {
    /// Create a new encoder selector config
    pub fn new(width: usize, height: usize, fps: u32) -> Self {
        Self {
            requested_backend: None,
            width,
            height,
            fps,
            allow_fallback: true,
        }
    }

    /// Set the requested backend
    pub fn with_backend(mut self, backend: VideoEncoderBackend) -> Self {
        self.requested_backend = Some(backend);
        self
    }

    /// Set whether to allow fallback
    pub fn with_fallback(mut self, allow: bool) -> Self {
        self.allow_fallback = allow;
        self
    }

    /// Get a config for low latency encoding
    pub fn low_latency(width: usize, height: usize, fps: u32) -> Self {
        Self::new(width, height, fps)
    }
}

impl Default for EncoderSelectorConfig {
    fn default() -> Self {
        Self {
            requested_backend: None,
            width: 1920,
            height: 1080,
            fps: 30,
            allow_fallback: true,
        }
    }
}

/// Encoder descriptor for capability detection
#[derive(Debug, Clone)]
pub struct EncoderDescriptor {
    /// Backend type
    pub backend: VideoEncoderBackend,
    /// Display name
    pub name: &'static str,
    /// Description
    pub description: &'static str,
}

impl EncoderDescriptor {
    /// Create a new encoder descriptor
    pub const fn new(
        backend: VideoEncoderBackend,
        name: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            backend,
            name,
            description,
        }
    }

    /// Check if this encoder is available
    pub fn is_available(&self) -> bool {
        match self.backend {
            #[cfg(feature = "nvenc")]
            VideoEncoderBackend::Nvenc => {
                mrd_encode_nvenc::NvencH264Encoder::probe_h264_available().is_ok()
            }
            #[cfg(not(feature = "nvenc"))]
            VideoEncoderBackend::Nvenc => false,
            #[cfg(feature = "openh264")]
            VideoEncoderBackend::OpenH264 => true, // Always available
            #[cfg(not(feature = "openh264"))]
            VideoEncoderBackend::OpenH264 => false,
        }
    }
}

impl fmt::Display for EncoderDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.backend)
    }
}

/// Get all available encoder descriptors
pub fn available_encoder_descriptors() -> Vec<EncoderDescriptor> {
    vec![
        EncoderDescriptor::new(
            VideoEncoderBackend::Nvenc,
            "NVENC H.264",
            "NVIDIA hardware encoder",
        ),
        EncoderDescriptor::new(
            VideoEncoderBackend::OpenH264,
            "OpenH264",
            "Software H.264 encoder",
        ),
    ]
}

/// Choose an encoder backend with automatic fallback
///
/// # Arguments
///
/// * `config` - Selector configuration
///
/// # Returns
///
/// Returns an `EncoderSelection` with the chosen backend and logs.
pub fn choose_encoder_backend(config: EncoderSelectorConfig) -> EncoderSelection {
    let mut logs = Vec::new();
    let gpu_caps = crate::gpu::detect_gpu_caps();

    logs.push(format!("GPU capabilities: {}", gpu_caps));

    // Determine the order of backends to try
    let backends_to_try = if let Some(requested) = config.requested_backend {
        logs.push(format!("Requested encoder backend: {}", requested));
        vec![requested]
    } else {
        // Auto-detect: try hardware first, then software
        logs.push("Auto-detecting encoder backend".to_string());
        let mut order = Vec::new();

        if gpu_caps.has_nvidia {
            order.push(VideoEncoderBackend::Nvenc);
        }

        // Always add software as fallback
        order.push(VideoEncoderBackend::OpenH264);

        order
    };

    // Try each backend in order
    for backend in backends_to_try {
        let descriptor = EncoderDescriptor::new(
            backend,
            match backend {
                VideoEncoderBackend::Nvenc => "NVENC H.264",
                VideoEncoderBackend::OpenH264 => "OpenH264",
            },
            "",
        );

        if !descriptor.is_available() {
            logs.push(format!("Encoder '{}' is not available", backend));

            // If this was explicitly requested and not available, check fallback
            if config.requested_backend == Some(backend) {
                if config.allow_fallback {
                    logs.push(format!(
                        "Requested encoder '{}' unavailable, allowing fallback",
                        backend
                    ));
                    continue;
                } else {
                    logs.push(format!(
                        "Requested encoder '{}' unavailable and fallback disabled",
                        backend
                    ));
                    // Return the requested backend even though it's not available
                    return EncoderSelection {
                        backend,
                        logs,
                        using_hardware: backend.is_hardware(),
                    };
                }
            }
            continue;
        }

        // Probe the encoder to ensure it actually works
        let probe_result = match backend {
            #[cfg(feature = "nvenc")]
            VideoEncoderBackend::Nvenc => {
                mrd_encode_nvenc::NvencH264Encoder::probe_h264_available()
            }
            #[cfg(feature = "openh264")]
            VideoEncoderBackend::OpenH264 => Ok(()),
            #[cfg(not(feature = "nvenc"))]
            VideoEncoderBackend::Nvenc => Err(PipelineError::message("nvenc feature not enabled")),
            #[cfg(not(feature = "openh264"))]
            VideoEncoderBackend::OpenH264 => {
                Err(PipelineError::message("openh264 feature not enabled"))
            }
        };

        match probe_result {
            Ok(()) => {
                logs.push(format!("Encoder '{}' selected", backend));
                return EncoderSelection {
                    backend,
                    logs,
                    using_hardware: backend.is_hardware(),
                };
            }
            Err(e) => {
                logs.push(format!("Encoder '{}' probe failed: {}", backend, e));

                // If this was explicitly requested, check fallback
                if config.requested_backend == Some(backend) {
                    if config.allow_fallback {
                        logs.push("Allowing fallback to next backend".to_string());
                        continue;
                    } else {
                        logs.push("Fallback disabled, returning unavailable encoder".to_string());
                        return EncoderSelection {
                            backend,
                            logs,
                            using_hardware: backend.is_hardware(),
                        };
                    }
                }
            }
        }
    }

    // Should never reach here since OpenH264 is always available (when feature enabled)
    logs.push("No suitable encoder found, defaulting to OpenH264".to_string());
    EncoderSelection {
        backend: VideoEncoderBackend::OpenH264,
        logs,
        using_hardware: false,
    }
}

/// Create a video encoder instance
///
/// # Arguments
///
/// * `backend` - Encoder backend to use
/// * `width` - Video width
/// * `height` - Video height
/// * `fps` - Target FPS
///
/// # Returns
///
/// Returns a `Box<dyn VideoEncoder>` or an error.
pub fn create_encoder(
    backend: VideoEncoderBackend,
    width: usize,
    height: usize,
    fps: u32,
) -> Result<Box<dyn VideoEncoder>, PipelineError> {
    match backend {
        #[cfg(feature = "nvenc")]
        VideoEncoderBackend::Nvenc => Ok(Box::new(
            mrd_encode_nvenc::NvencH264Encoder::new(width, height, fps)
                .map_err(|e| PipelineError::message(format!("nvenc create failed: {}", e)))?,
        )),
        #[cfg(feature = "openh264")]
        VideoEncoderBackend::OpenH264 => Ok(Box::new(
            mrd_encode_openh264::OpenH264Encoder::new(width, height, fps)
                .map_err(|e| PipelineError::message(format!("openh264 create failed: {}", e)))?,
        )),
        #[cfg(not(feature = "nvenc"))]
        VideoEncoderBackend::Nvenc => Err(PipelineError::message("nvenc feature not enabled")),
        #[cfg(not(feature = "openh264"))]
        VideoEncoderBackend::OpenH264 => {
            Err(PipelineError::message("openh264 feature not enabled"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_selector_config_creates_valid_config() {
        let config = EncoderSelectorConfig::new(1920, 1080, 30);
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.fps, 30);
        assert!(config.allow_fallback);
    }

    #[test]
    fn encoder_selector_config_with_backend_sets_backend() {
        let config =
            EncoderSelectorConfig::new(1280, 720, 30).with_backend(VideoEncoderBackend::OpenH264);
        assert_eq!(
            config.requested_backend,
            Some(VideoEncoderBackend::OpenH264)
        );
    }

    #[test]
    fn encoder_selector_config_low_latency_creates_config() {
        let config = EncoderSelectorConfig::low_latency(1920, 1080, 60);
        assert_eq!(config.width, 1920);
        assert_eq!(config.fps, 60);
    }

    #[test]
    fn choose_encoder_backend_returns_selection() {
        let config =
            EncoderSelectorConfig::new(1920, 1080, 30).with_backend(VideoEncoderBackend::OpenH264);
        let selection = choose_encoder_backend(config);
        assert_eq!(selection.backend, VideoEncoderBackend::OpenH264);
        assert!(!selection.using_hardware);
        assert!(!selection.logs.is_empty());
    }

    #[test]
    fn available_encoder_descriptors_returns_valid_list() {
        let descriptors = available_encoder_descriptors();
        assert!(!descriptors.is_empty());
        for desc in &descriptors {
            assert!(!desc.name.is_empty());
        }
    }

    #[test]
    fn encoder_descriptor_is_available_checks() {
        let desc = EncoderDescriptor::new(
            VideoEncoderBackend::OpenH264,
            "OpenH264",
            "Software encoder",
        );
        // The result depends on feature flags, but it should not panic
        let _available = desc.is_available();
    }

    #[test]
    fn create_encoder_with_openh264_succeeds() {
        let result = create_encoder(VideoEncoderBackend::OpenH264, 640, 480, 30);
        #[cfg(feature = "openh264")]
        assert!(result.is_ok());
        #[cfg(not(feature = "openh264"))]
        assert!(result.is_err());
    }
}
