//! Video decoder selection and factory
//!
//! Provides automatic hardware decoder detection and fallback.

use crate::{DecoderSelection, VideoDecoderBackend};
use mrd_pipeline_core::PipelineError;
use std::fmt;

/// Decoder selector configuration
#[derive(Debug, Clone)]
pub struct DecoderSelectorConfig {
    /// Requested decoder backend (empty = auto-detect)
    pub requested_backend: Option<VideoDecoderBackend>,
    /// Whether to allow fallback to software decoder
    pub allow_fallback: bool,
}

impl DecoderSelectorConfig {
    /// Create a new decoder selector config
    pub fn new() -> Self {
        Self {
            requested_backend: None,
            allow_fallback: true,
        }
    }

    /// Set the requested backend
    pub fn with_backend(mut self, backend: VideoDecoderBackend) -> Self {
        self.requested_backend = Some(backend);
        self
    }

    /// Set whether to allow fallback
    pub fn with_fallback(mut self, allow: bool) -> Self {
        self.allow_fallback = allow;
        self
    }
}

impl Default for DecoderSelectorConfig {
    fn default() -> Self {
        Self {
            requested_backend: None,
            allow_fallback: true,
        }
    }
}

/// Decoder descriptor for capability detection
#[derive(Debug, Clone)]
pub struct DecoderDescriptor {
    /// Backend type
    pub backend: VideoDecoderBackend,
    /// Display name
    pub name: &'static str,
    /// Description
    pub description: &'static str,
}

impl DecoderDescriptor {
    /// Create a new decoder descriptor
    pub const fn new(
        backend: VideoDecoderBackend,
        name: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            backend,
            name,
            description,
        }
    }

    /// Check if this decoder is available
    pub fn is_available(&self) -> bool {
        match self.backend {
            #[cfg(feature = "nvdec")]
            VideoDecoderBackend::Nvdec => {
                mrd_decode_nvdec::probe_h264_available().is_ok()
            }
            #[cfg(not(feature = "nvdec"))]
            VideoDecoderBackend::Nvdec => false,
            #[cfg(feature = "software_decoder")]
            VideoDecoderBackend::Software => true, // Always available
            #[cfg(not(feature = "software_decoder"))]
            VideoDecoderBackend::Software => false,
        }
    }
}

impl fmt::Display for DecoderDescriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.backend)
    }
}

/// Get all available decoder descriptors
pub fn available_decoder_descriptors() -> Vec<DecoderDescriptor> {
    vec![
        DecoderDescriptor::new(
            VideoDecoderBackend::Nvdec,
            "NVDEC H.264",
            "NVIDIA hardware decoder",
        ),
        DecoderDescriptor::new(
            VideoDecoderBackend::Software,
            "Software H.264",
            "Software H.264 decoder",
        ),
    ]
}

/// Choose a decoder backend with automatic fallback
///
/// # Arguments
///
/// * `config` - Selector configuration
///
/// # Returns
///
/// Returns a `DecoderSelection` with the chosen backend and logs.
pub fn choose_decoder_backend(config: DecoderSelectorConfig) -> DecoderSelection {
    let mut logs = Vec::new();
    let gpu_caps = crate::gpu::detect_gpu_caps();

    logs.push(format!("GPU capabilities: {}", gpu_caps));

    // Determine the order of backends to try
    let backends_to_try = if let Some(requested) = config.requested_backend {
        logs.push(format!("Requested decoder backend: {}", requested));
        vec![requested]
    } else {
        // Auto-detect: try hardware first, then software
        logs.push("Auto-detecting decoder backend".to_string());
        let mut order = Vec::new();

        if gpu_caps.has_nvidia {
            order.push(VideoDecoderBackend::Nvdec);
        }

        // Always add software as fallback
        order.push(VideoDecoderBackend::Software);

        order
    };

    // Try each backend in order
    for backend in backends_to_try {
        let descriptor = DecoderDescriptor::new(
            backend,
            match backend {
                VideoDecoderBackend::Nvdec => "NVDEC H.264",
                VideoDecoderBackend::Software => "Software H.264",
            },
            "",
        );

        if !descriptor.is_available() {
            logs.push(format!(
                "Decoder '{}' is not available",
                backend
            ));

            // If this was explicitly requested and not available, check fallback
            if config.requested_backend == Some(backend) {
                if config.allow_fallback {
                    logs.push(format!(
                        "Requested decoder '{}' unavailable, allowing fallback",
                        backend
                    ));
                    continue;
                } else {
                    logs.push(format!(
                        "Requested decoder '{}' unavailable and fallback disabled",
                        backend
                    ));
                    // Return the requested backend even though it's not available
                    return DecoderSelection {
                        backend,
                        logs,
                        using_hardware: backend.is_hardware(),
                    };
                }
            }
            continue;
        }

        // Probe the decoder to ensure it actually works
        let probe_result = match backend {
            #[cfg(feature = "nvdec")]
            VideoDecoderBackend::Nvdec => mrd_decode_nvdec::probe_h264_available(),
            #[cfg(feature = "software_decoder")]
            VideoDecoderBackend::Software => Ok(()),
            #[cfg(not(feature = "nvdec"))]
            VideoDecoderBackend::Nvdec => {
                Err(PipelineError::message("nvdec feature not enabled"))
            }
            #[cfg(not(feature = "software_decoder"))]
            VideoDecoderBackend::Software => {
                Err(PipelineError::message("software decoder feature not enabled"))
            }
        };

        match probe_result {
            Ok(()) => {
                logs.push(format!("Decoder '{}' selected", backend));
                return DecoderSelection {
                    backend,
                    logs,
                    using_hardware: backend.is_hardware(),
                };
            }
            Err(e) => {
                logs.push(format!(
                    "Decoder '{}' probe failed: {}",
                    backend, e
                ));

                // If this was explicitly requested, check fallback
                if config.requested_backend == Some(backend) {
                    if config.allow_fallback {
                        logs.push("Allowing fallback to next backend".to_string());
                        continue;
                    } else {
                        logs.push("Fallback disabled, returning unavailable decoder".to_string());
                        return DecoderSelection {
                            backend,
                            logs,
                            using_hardware: backend.is_hardware(),
                        };
                    }
                }
            }
        }
    }

    // Should never reach here since Software decoder is always available (when feature enabled)
    logs.push("No suitable decoder found, defaulting to Software".to_string());
    DecoderSelection {
        backend: VideoDecoderBackend::Software,
        logs,
        using_hardware: false,
    }
}

/// Create a video decoder instance
///
/// # Arguments
///
/// * `backend` - Decoder backend to use
///
/// # Returns
///
/// Returns a `Box<dyn mrd_decode::VideoDecoder>` or an error.
pub fn create_decoder(
    backend: VideoDecoderBackend,
) -> Result<Box<dyn mrd_decode::VideoDecoder>, PipelineError> {
    match backend {
        #[cfg(feature = "nvdec")]
        VideoDecoderBackend::Nvdec => Ok(Box::new(
            mrd_decode::NvdecVideoDecoder::new()
                .map_err(|e| PipelineError::message(format!("nvdec create failed: {}", e)))?,
        )),
        #[cfg(feature = "software_decoder")]
        VideoDecoderBackend::Software => Ok(Box::new(
            mrd_decode::H264SoftwareDecoder::new()
                .map_err(|e| PipelineError::message(format!("software decoder create failed: {}", e)))?,
        )),
        #[cfg(not(feature = "nvdec"))]
        VideoDecoderBackend::Nvdec => Err(PipelineError::message("nvdec feature not enabled")),
        #[cfg(not(feature = "software_decoder"))]
        VideoDecoderBackend::Software => {
            Err(PipelineError::message("software decoder feature not enabled"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_selector_config_creates_valid_config() {
        let config = DecoderSelectorConfig::new();
        assert!(config.allow_fallback);
        assert!(config.requested_backend.is_none());
    }

    #[test]
    fn decoder_selector_config_with_backend_sets_backend() {
        let config = DecoderSelectorConfig::new()
            .with_backend(VideoDecoderBackend::Software);
        assert_eq!(
            config.requested_backend,
            Some(VideoDecoderBackend::Software)
        );
    }

    #[test]
    fn decoder_selector_config_with_fallback_sets_fallback() {
        let config = DecoderSelectorConfig::new().with_fallback(false);
        assert!(!config.allow_fallback);
    }

    #[test]
    fn choose_decoder_backend_returns_selection() {
        let config = DecoderSelectorConfig::new()
            .with_backend(VideoDecoderBackend::Software);
        let selection = choose_decoder_backend(config);
        assert_eq!(selection.backend, VideoDecoderBackend::Software);
        assert!(!selection.using_hardware);
        assert!(!selection.logs.is_empty());
    }

    #[test]
    fn available_decoder_descriptors_returns_valid_list() {
        let descriptors = available_decoder_descriptors();
        assert!(!descriptors.is_empty());
        for desc in &descriptors {
            assert!(!desc.name.is_empty());
        }
    }

    #[test]
    fn decoder_descriptor_is_available_checks() {
        let desc = DecoderDescriptor::new(
            VideoDecoderBackend::Software,
            "Software H.264",
            "Software decoder",
        );
        // The result depends on feature flags, but it should not panic
        let _available = desc.is_available();
    }

    #[test]
    fn create_decoder_with_software_succeeds() {
        let result = create_decoder(VideoDecoderBackend::Software);
        #[cfg(feature = "software_decoder")]
        assert!(result.is_ok());
        #[cfg(not(feature = "software_decoder"))]
        assert!(result.is_err());
    }
}
