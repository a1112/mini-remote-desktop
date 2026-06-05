// Encoder configuration and enhancements
//
// Provides advanced encoder features including:
// - Multi-codec support (H.264, HEVC, AV1)
// - Adaptive bitrate control
// - ROI (Region of Interest) encoding
// - Quality presets

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// GPU-side color transform applied before encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    /// Preserve source color.
    #[default]
    Full,
    /// Convert source color to luma grayscale on the GPU.
    Grayscale,
    /// Convert source color to thresholded black/white on the GPU.
    Monochrome,
    /// Reduce chroma while preserving some source color.
    LowChroma,
}

impl ColorMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Grayscale => "grayscale",
            Self::Monochrome => "monochrome",
            Self::LowChroma => "low_chroma",
        }
    }
}

/// Bit-depth and transfer pipeline used to carry encoded frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorPipeline {
    /// 8-bit SDR media path.
    #[default]
    Sdr8,
    /// 10-bit HEVC Main10 media path.
    HdrMain10,
}

impl ColorPipeline {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sdr8 => "sdr8",
            Self::HdrMain10 => "hdr_main10",
        }
    }
}

/// Video codec type with feature support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EnhancedCodec {
    /// H.264/AVC - Widely supported, good compression
    H264,
    /// HEVC/H.265 - Better compression, newer hardware
    HEVC,
    /// AV1 - Royalty-free, excellent compression
    AV1,
}

impl EnhancedCodec {
    /// Get codec name for NVENC
    pub fn nvenc_name(&self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::HEVC => "HEVC",
            Self::AV1 => "AV1",
        }
    }

    /// Check if codec requires specific hardware generation
    pub fn min_gpu_architecture(&self) -> GPUArchitecture {
        match self {
            Self::H264 => GPUArchitecture::Maxwell, // GTX 900 series
            Self::HEVC => GPUArchitecture::Pascal,  // GTX 1000 series
            Self::AV1 => GPUArchitecture::Ampere,   // RTX 3000 series
        }
    }
}

/// Minimum GPU architecture for codec support
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GPUArchitecture {
    Maxwell, // GTX 900 series (GM2xx)
    Pascal,  // GTX 1000 series (GP1xx)
    Turing,  // RTX 2000 series (TU1xx)
    Ampere,  // RTX 3000 series (GA1xx)
    Ada,     // RTX 4000 series (AD1xx)
}

/// Encoding quality preset
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityPreset {
    /// Lowest quality, fastest encoding
    Performance,
    /// Balanced quality and speed
    Balanced,
    /// Higher quality, slower encoding
    Quality,
    /// Best quality, slowest encoding
    MaxQuality,
}

impl QualityPreset {
    /// Get NVENC preset GUID name
    pub fn nvenc_preset(&self) -> &'static str {
        match self {
            Self::Performance => "P1",
            Self::Balanced => "P3",
            Self::Quality => "P5",
            Self::MaxQuality => "P7",
        }
    }

    /// Get relative quality factor (1.0 = baseline)
    pub fn quality_factor(&self) -> f32 {
        match self {
            Self::Performance => 0.7,
            Self::Balanced => 1.0,
            Self::Quality => 1.3,
            Self::MaxQuality => 1.5,
        }
    }
}

/// Rate control mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateControlMode {
    /// Constant bitrate
    CBR,
    /// Variable bitrate (peak capped)
    VBR,
    /// Constant quality (target quality instead of bitrate)
    CQ,
    /// Variable bitrate with relaxed constraints
    VbrRelaxed,
}

impl RateControlMode {
    /// Get NVENC rate control mode string
    pub fn nvenc_mode(&self) -> &'static str {
        match self {
            Self::CBR => "CBR",
            Self::VBR => "VBR",
            Self::CQ => "CQ",
            Self::VbrRelaxed => "VbrRelaxed",
        }
    }
}

/// Bitrate configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitrateConfig {
    /// Target bitrate in bits per second
    pub target_bitrate_bps: u64,
    /// Peak bitrate (for VBR modes)
    pub peak_bitrate_bps: Option<u64>,
    /// Minimum bitrate
    pub min_bitrate_bps: Option<u64>,
    /// Rate control mode
    pub mode: RateControlMode,
    /// Target quality (for CQ mode, 0-51, lower is better)
    pub target_quality: Option<u8>,
}

impl BitrateConfig {
    /// Create CBR bitrate config
    pub fn cbr(bitrate_mbps: f64) -> Self {
        Self {
            target_bitrate_bps: (bitrate_mbps * 1_000_000.0) as u64,
            peak_bitrate_bps: None,
            min_bitrate_bps: None,
            mode: RateControlMode::CBR,
            target_quality: None,
        }
    }

    /// Create VBR bitrate config
    pub fn vbr(target_mbps: f64, peak_mbps: f64) -> Self {
        Self {
            target_bitrate_bps: (target_mbps * 1_000_000.0) as u64,
            peak_bitrate_bps: Some((peak_mbps * 1_000_000.0) as u64),
            min_bitrate_bps: None,
            mode: RateControlMode::VBR,
            target_quality: None,
        }
    }

    /// Create CQ bitrate config
    pub fn cq(target_quality: u8, max_bitrate_mbps: Option<f64>) -> Self {
        Self {
            target_bitrate_bps: max_bitrate_mbps
                .map(|mbps| (mbps * 1_000_000.0) as u64)
                .unwrap_or(20_000_000),
            peak_bitrate_bps: None,
            min_bitrate_bps: None,
            mode: RateControlMode::CQ,
            target_quality: Some(target_quality.min(51)),
        }
    }

    /// Get target bitrate in Mbps
    pub fn target_mbps(&self) -> f64 {
        self.target_bitrate_bps as f64 / 1_000_000.0
    }

    /// Get peak bitrate in Mbps (if set)
    pub fn peak_mbps(&self) -> Option<f64> {
        self.peak_bitrate_bps.map(|bps| bps as f64 / 1_000_000.0)
    }
}

impl Default for BitrateConfig {
    fn default() -> Self {
        Self::cbr(10.0)
    }
}

/// Region of Interest for quality weighting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionOfInterest {
    /// X coordinate (normalized 0-1)
    pub x: f32,
    /// Y coordinate (normalized 0-1)
    pub y: f32,
    /// Width (normalized 0-1)
    pub width: f32,
    /// Height (normalized 0-1)
    pub height: f32,
    /// Quality priority multiplier (1.0 = normal, >1.0 = higher quality)
    pub priority: f32,
}

// Implement PartialEq manually for f32 comparisons with epsilon
impl PartialEq for RegionOfInterest {
    fn eq(&self, other: &Self) -> bool {
        (self.x - other.x).abs() < 1e-6
            && (self.y - other.y).abs() < 1e-6
            && (self.width - other.width).abs() < 1e-6
            && (self.height - other.height).abs() < 1e-6
            && (self.priority - other.priority).abs() < 1e-6
    }
}

impl Eq for RegionOfInterest {}

// Implement Hash for HashMap key
impl std::hash::Hash for RegionOfInterest {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Convert to integers for stable hashing
        let x_bits = self.x.to_bits();
        let y_bits = self.y.to_bits();
        let width_bits = self.width.to_bits();
        let height_bits = self.height.to_bits();
        let priority_bits = self.priority.to_bits();

        x_bits.hash(state);
        y_bits.hash(state);
        width_bits.hash(state);
        height_bits.hash(state);
        priority_bits.hash(state);
    }
}

impl RegionOfInterest {
    /// Create a new ROI
    pub fn new(x: f32, y: f32, width: f32, height: f32, priority: f32) -> Self {
        // Clamp coordinates first
        let x_clamped = x.clamp(0.0, 1.0);
        let y_clamped = y.clamp(0.0, 1.0);

        Self {
            x: x_clamped,
            y: y_clamped,
            width: width.clamp(0.0, 1.0 - x_clamped),
            height: height.clamp(0.0, 1.0 - y_clamped),
            priority: priority.max(0.1),
        }
    }

    /// Create a center ROI (useful for screen sharing focus area)
    pub fn center(size: f32, priority: f32) -> Self {
        let half = size / 2.0;
        Self::new(0.5 - half, 0.5 - half, size, size, priority)
    }

    /// Check if ROI is valid
    pub fn is_valid(&self) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && self.x + self.width <= 1.0
            && self.y + self.height <= 1.0
            && self.priority > 0.0
    }
}

/// Adaptive bitrate configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveBitrateConfig {
    /// Enable adaptive bitrate
    pub enabled: bool,
    /// Minimum bitrate in Mbps
    pub min_bitrate_mbps: f32,
    /// Maximum bitrate in Mbps
    pub max_bitrate_mbps: f32,
    /// Target packet loss percentage (below this, increase bitrate)
    pub target_packet_loss_pct: f32,
    /// Target latency in ms (above this, decrease bitrate)
    pub target_latency_ms: u32,
    /// Adjustment step size (percentage of current bitrate)
    pub adjustment_step_pct: f32,
    /// Minimum interval between adjustments
    pub adjustment_interval_ms: u32,
}

impl AdaptiveBitrateConfig {
    /// Calculate target bitrate based on network conditions
    pub fn calculate_target_bitrate(
        &self,
        current_bitrate_mbps: f32,
        packet_loss_pct: f32,
        latency_ms: u32,
    ) -> f32 {
        if !self.enabled {
            return current_bitrate_mbps;
        }

        let mut target = current_bitrate_mbps;

        // Increase bitrate if conditions are good
        if packet_loss_pct < self.target_packet_loss_pct / 2.0
            && latency_ms < self.target_latency_ms / 2
        {
            target = (target * (1.0 + self.adjustment_step_pct / 100.0)).min(self.max_bitrate_mbps);
        }
        // Decrease bitrate if conditions are poor
        else if packet_loss_pct > self.target_packet_loss_pct
            || latency_ms > self.target_latency_ms
        {
            let reduction_factor = if packet_loss_pct > self.target_packet_loss_pct * 2.0 {
                0.8 // Severe packet loss - reduce more
            } else if f64::from(latency_ms) > f64::from(self.target_latency_ms) * 2.0 {
                0.9 // High latency - reduce moderately
            } else {
                0.95 // Slight degradation - reduce slightly
            };
            target = (target * reduction_factor).max(self.min_bitrate_mbps);
        }

        target
    }
}

impl Default for AdaptiveBitrateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_bitrate_mbps: 2.0,
            max_bitrate_mbps: 20.0,
            target_packet_loss_pct: 1.0,
            target_latency_ms: 100,
            adjustment_step_pct: 10.0,
            adjustment_interval_ms: 500,
        }
    }
}

/// Enhanced encoder configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedEncoderConfig {
    /// Video codec
    pub codec: EnhancedCodec,
    /// Quality preset
    pub quality_preset: QualityPreset,
    /// Bitrate configuration
    pub bitrate: BitrateConfig,
    /// Regions of interest (higher quality areas)
    pub regions_of_interest: Vec<RegionOfInterest>,
    /// Adaptive bitrate configuration
    pub adaptive_bitrate: AdaptiveBitrateConfig,
    /// Keyframe interval (frames)
    pub keyframe_interval: u32,
    /// Enable temporal layering for SVC
    pub enable_temporal_layers: bool,
    /// Number of temporal layers (1-4)
    pub temporal_layers: u32,
    /// Enable spatial scalability
    pub enable_spatial_layers: bool,
    /// Number of spatial layers (1-3)
    pub spatial_layers: u32,
}

impl EnhancedEncoderConfig {
    /// Create a low-latency config for real-time streaming
    pub fn low_latency(width: usize, height: usize, fps: u32) -> Self {
        // Calculate target bitrate based on resolution and FPS
        let pixels = width * height;
        let base_bitrate = (pixels as f64 * fps as f64 * 0.07) as u64; // ~0.07 bpp

        Self {
            codec: EnhancedCodec::H264,
            quality_preset: QualityPreset::Performance,
            bitrate: BitrateConfig {
                target_bitrate_bps: base_bitrate,
                peak_bitrate_bps: Some((base_bitrate as f64 * 1.5) as u64),
                min_bitrate_bps: Some((base_bitrate as f64 * 0.5) as u64),
                mode: RateControlMode::VBR,
                target_quality: None,
            },
            regions_of_interest: Vec::new(),
            adaptive_bitrate: AdaptiveBitrateConfig {
                max_bitrate_mbps: 20.0,
                target_latency_ms: 50,
                ..Default::default()
            },
            keyframe_interval: fps,
            enable_temporal_layers: true,
            temporal_layers: 3,
            enable_spatial_layers: false,
            spatial_layers: 1,
        }
    }

    /// Create a high-quality config for recording
    pub fn high_quality(width: usize, height: usize, fps: u32) -> Self {
        let pixels = width * height;
        let base_bitrate = (pixels as f64 * fps as f64 * 0.15) as u64; // Higher quality

        Self {
            codec: EnhancedCodec::HEVC,
            quality_preset: QualityPreset::Quality,
            bitrate: BitrateConfig {
                target_bitrate_bps: base_bitrate,
                peak_bitrate_bps: Some((base_bitrate as f64 * 2.0) as u64),
                min_bitrate_bps: Some(base_bitrate / 2),
                mode: RateControlMode::VBR,
                target_quality: None,
            },
            regions_of_interest: Vec::new(),
            adaptive_bitrate: AdaptiveBitrateConfig {
                enabled: false, // Fixed bitrate for recording
                ..Default::default()
            },
            keyframe_interval: fps * 2, // Longer GOP for quality
            enable_temporal_layers: false,
            temporal_layers: 1,
            enable_spatial_layers: false,
            spatial_layers: 1,
        }
    }

    /// Add a region of interest
    pub fn with_roi(mut self, roi: RegionOfInterest) -> Self {
        self.regions_of_interest.push(roi);
        self
    }

    /// Add center ROI for screen sharing focus
    pub fn with_center_roi(mut self, size: f32, priority: f32) -> Self {
        self.regions_of_interest
            .push(RegionOfInterest::center(size, priority));
        self
    }

    /// Check if configuration is valid
    pub fn validate(&self) -> Result<(), String> {
        if self.keyframe_interval == 0 {
            return Err("keyframe_interval must be > 0".to_string());
        }

        if self.temporal_layers < 1 || self.temporal_layers > 4 {
            return Err("temporal_layers must be 1-4".to_string());
        }

        if self.spatial_layers < 1 || self.spatial_layers > 3 {
            return Err("spatial_layers must be 1-3".to_string());
        }

        if self.bitrate.target_bitrate_bps == 0 {
            return Err("target_bitrate_bps must be > 0".to_string());
        }

        for roi in &self.regions_of_interest {
            if !roi.is_valid() {
                return Err(format!("invalid ROI: {:?}", roi));
            }
        }

        Ok(())
    }

    /// Get estimated bitrate in Mbps
    pub fn estimated_bitrate_mbps(&self) -> f64 {
        self.bitrate.target_bitrate_bps as f64 / 1_000_000.0
    }

    /// Calculate quality-adjusted bitrate for ROI areas
    pub fn calculate_roi_bitrate(&self, base_bitrate_bps: u64) -> HashMap<RegionOfInterest, u64> {
        let mut result = HashMap::new();
        let total_priority: f32 = self.regions_of_interest.iter().map(|r| r.priority).sum();

        for roi in &self.regions_of_interest {
            let area = roi.width * roi.height;
            let share = if total_priority > 0.0 {
                roi.priority / total_priority
            } else {
                1.0 / self.regions_of_interest.len() as f32
            };
            let roi_bitrate = (base_bitrate_bps as f64 * f64::from(area) * f64::from(share)) as u64;
            result.insert(roi.clone(), roi_bitrate);
        }

        result
    }
}

impl Default for EnhancedEncoderConfig {
    fn default() -> Self {
        Self::low_latency(1920, 1080, 30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_mode_defaults_to_full() {
        assert_eq!(ColorMode::default(), ColorMode::Full);
    }

    #[test]
    fn color_mode_uses_stable_snake_case_json_values() {
        let cases = [
            (ColorMode::Full, r#""full""#),
            (ColorMode::Grayscale, r#""grayscale""#),
            (ColorMode::Monochrome, r#""monochrome""#),
            (ColorMode::LowChroma, r#""low_chroma""#),
        ];

        for (mode, expected_json) in cases {
            assert_eq!(serde_json::to_string(&mode).unwrap(), expected_json);
            assert_eq!(
                serde_json::from_str::<ColorMode>(expected_json).unwrap(),
                mode
            );
        }
    }

    #[test]
    fn color_pipeline_defaults_to_sdr8() {
        assert_eq!(ColorPipeline::default(), ColorPipeline::Sdr8);
    }

    #[test]
    fn color_pipeline_uses_stable_snake_case_json_values() {
        let cases = [
            (ColorPipeline::Sdr8, r#""sdr8""#),
            (ColorPipeline::HdrMain10, r#""hdr_main10""#),
        ];

        for (pipeline, expected_json) in cases {
            assert_eq!(serde_json::to_string(&pipeline).unwrap(), expected_json);
            assert_eq!(
                serde_json::from_str::<ColorPipeline>(expected_json).unwrap(),
                pipeline
            );
        }
    }

    #[test]
    fn quality_preset_has_correct_factors() {
        assert_eq!(QualityPreset::Performance.quality_factor(), 0.7);
        assert_eq!(QualityPreset::Balanced.quality_factor(), 1.0);
        assert_eq!(QualityPreset::Quality.quality_factor(), 1.3);
        assert_eq!(QualityPreset::MaxQuality.quality_factor(), 1.5);
    }

    #[test]
    fn bitrate_config_cbr_creates_correct_config() {
        let config = BitrateConfig::cbr(10.0);
        assert_eq!(config.target_bitrate_bps, 10_000_000);
        assert_eq!(config.mode, RateControlMode::CBR);
        assert_eq!(config.target_mbps(), 10.0);
    }

    #[test]
    fn bitrate_config_vbr_creates_correct_config() {
        let config = BitrateConfig::vbr(8.0, 12.0);
        assert_eq!(config.target_bitrate_bps, 8_000_000);
        assert_eq!(config.peak_bitrate_bps, Some(12_000_000));
        assert_eq!(config.mode, RateControlMode::VBR);
    }

    #[test]
    fn roi_is_clamped_to_valid_range() {
        let roi = RegionOfInterest::new(1.5, 0.5, 1.0, 1.0, 2.0);
        assert!(roi.x <= 1.0);
        assert!(roi.y <= 1.0);
    }

    #[test]
    fn roi_center_creates_valid_region() {
        let roi = RegionOfInterest::center(0.5, 2.0);
        assert!(roi.is_valid());
        assert_eq!(roi.priority, 2.0);
    }

    #[test]
    fn adaptive_bitrate_increases_for_good_conditions() {
        let config = AdaptiveBitrateConfig::default();
        let new_bitrate = config.calculate_target_bitrate(10.0, 0.1, 20);
        assert!(new_bitrate > 10.0);
        assert!(new_bitrate <= config.max_bitrate_mbps);
    }

    #[test]
    fn adaptive_bitrate_decreases_for_bad_conditions() {
        let config = AdaptiveBitrateConfig::default();
        let new_bitrate = config.calculate_target_bitrate(10.0, 3.0, 50);
        assert!(new_bitrate < 10.0);
        assert!(new_bitrate >= config.min_bitrate_mbps);
    }

    #[test]
    fn low_latency_config_creates_valid_config() {
        let config = EnhancedEncoderConfig::low_latency(1920, 1080, 30);
        assert!(config.validate().is_ok());
        assert_eq!(config.codec, EnhancedCodec::H264);
        assert_eq!(config.quality_preset, QualityPreset::Performance);
        assert!(config.adaptive_bitrate.enabled);
    }

    #[test]
    fn high_quality_config_creates_valid_config() {
        let config = EnhancedEncoderConfig::high_quality(1920, 1080, 30);
        assert!(config.validate().is_ok());
        assert_eq!(config.codec, EnhancedCodec::HEVC);
        assert_eq!(config.quality_preset, QualityPreset::Quality);
        assert!(!config.adaptive_bitrate.enabled);
    }

    #[test]
    fn config_with_roi_adds_region() {
        let config = EnhancedEncoderConfig::low_latency(1920, 1080, 30).with_center_roi(0.3, 2.0);
        assert_eq!(config.regions_of_interest.len(), 1);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn config_rejects_invalid_keyframe_interval() {
        let config = EnhancedEncoderConfig {
            keyframe_interval: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn codec_min_architecture_is_correct() {
        assert!(
            EnhancedCodec::H264.min_gpu_architecture()
                <= EnhancedCodec::HEVC.min_gpu_architecture()
        );
        assert!(
            EnhancedCodec::HEVC.min_gpu_architecture() <= EnhancedCodec::AV1.min_gpu_architecture()
        );
    }
}
