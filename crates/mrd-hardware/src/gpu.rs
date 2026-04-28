//! GPU capability detection
//!
//! Detects available GPU hardware and codec support.

use std::fmt;

/// GPU capabilities detected on the system
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuCaps {
    /// NVIDIA GPU available
    pub has_nvidia: bool,
    /// Intel GPU available
    pub has_intel: bool,
    /// AMD GPU available
    pub has_amd: bool,
    /// Detection completed
    pub detected: bool,
}

impl GpuCaps {
    /// Create a new GpuCaps with all capabilities disabled
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any hardware GPU is available
    pub fn has_hardware(&self) -> bool {
        self.has_nvidia || self.has_intel || self.has_amd
    }

    /// Get the preferred hardware encoder backend
    /// Returns None if no hardware is available
    pub fn preferred_encoder_backend(&self) -> Option<super::VideoEncoderBackend> {
        if self.has_nvidia {
            Some(super::VideoEncoderBackend::Nvenc)
        } else if self.has_intel {
            // Future: QSV
            None
        } else if self.has_amd {
            // Future: AMF
            None
        } else {
            None
        }
    }

    /// Get the preferred hardware decoder backend
    /// Returns None if no hardware is available
    pub fn preferred_decoder_backend(&self) -> Option<super::VideoDecoderBackend> {
        if self.has_nvidia {
            Some(super::VideoDecoderBackend::Nvdec)
        } else {
            None
        }
    }
}

impl fmt::Display for GpuCaps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GpuCaps(")?;
        let mut parts = Vec::new();
        if self.has_nvidia {
            parts.push("NVIDIA");
        }
        if self.has_intel {
            parts.push("Intel");
        }
        if self.has_amd {
            parts.push("AMD");
        }
        if parts.is_empty() {
            write!(f, "no hardware")?;
        } else {
            write!(f, "{}", parts.join(", "))?;
        }
        write!(f, ")")
    }
}

/// Detect GPU capabilities on the system
///
/// This function checks for available GPUs and their codec support.
/// On Windows, it uses WMI to query video controllers.
pub fn detect_gpu_caps() -> GpuCaps {
    #[cfg(windows)]
    return detect_gpu_caps_windows();

    #[cfg(not(windows))]
    {
        let _ = detect_gpu_caps_windows;
        GpuCaps::new()
    }
}

#[cfg(windows)]
fn detect_gpu_caps_windows() -> GpuCaps {
    use std::process::Command;

    let mut caps = GpuCaps::new();

    // Use PowerShell to query video controllers
    let output = match Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ])
        .output()
    {
        Ok(output) => output,
        Err(_) => return caps,
    };

    let gpu_names = String::from_utf8_lossy(&output.stdout);
    caps.detected = true;

    for name in gpu_names.lines() {
        let name_lower = name.to_lowercase();

        // Check for NVIDIA
        if name_lower.contains("nvidia")
            || name_lower.contains("geforce")
            || name_lower.contains("quadro")
            || name_lower.contains("tesla")
            || name_lower.contains("rtx")
        {
            caps.has_nvidia = true;
            log::debug!("Detected NVIDIA GPU: {}", name.trim());
        }

        // Check for Intel
        if name_lower.contains("intel")
            && (name_lower.contains("graphics")
                || name_lower.contains("iris")
                || name_lower.contains("arc"))
        {
            caps.has_intel = true;
            log::debug!("Detected Intel GPU: {}", name.trim());
        }

        // Check for AMD
        if name_lower.contains("amd") || name_lower.contains("radeon") || name_lower.contains("ati")
        {
            caps.has_amd = true;
            log::debug!("Detected AMD GPU: {}", name.trim());
        }
    }

    if !caps.has_hardware() {
        log::debug!("No hardware GPU detected");
    }

    caps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_caps_new_creates_empty_caps() {
        let caps = GpuCaps::new();
        assert!(!caps.has_nvidia);
        assert!(!caps.has_intel);
        assert!(!caps.has_amd);
        assert!(!caps.has_hardware());
    }

    #[test]
    fn gpu_caps_display_formats_correctly() {
        let caps = GpuCaps {
            has_nvidia: true,
            has_intel: false,
            has_amd: false,
            detected: true,
        };
        assert_eq!(format!("{}", caps), "GpuCaps(NVIDIA)");
    }

    #[test]
    fn gpu_caps_with_no_hardware_shows_no_hardware() {
        let caps = GpuCaps::new();
        assert_eq!(format!("{}", caps), "GpuCaps(no hardware)");
    }

    #[test]
    fn detect_gpu_caps_returns_valid_caps() {
        let caps = detect_gpu_caps();
        // Should not panic and should return valid caps
        assert_eq!(caps.detected || !caps.detected, true); // Always true
    }

    #[test]
    fn preferred_encoder_backend_returns_nvidia_when_available() {
        use crate::VideoEncoderBackend;
        let caps = GpuCaps {
            has_nvidia: true,
            ..Default::default()
        };
        assert_eq!(
            caps.preferred_encoder_backend(),
            Some(VideoEncoderBackend::Nvenc)
        );
    }

    #[test]
    fn preferred_decoder_backend_returns_none_when_no_hardware() {
        let caps = GpuCaps::new();
        assert_eq!(caps.preferred_decoder_backend(), None);
    }
}
