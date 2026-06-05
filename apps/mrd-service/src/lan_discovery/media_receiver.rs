#[cfg(any(test, target_os = "macos"))]
pub(super) fn compressed_proxy_enabled_for_values(
    codec: &str,
    width: u32,
    height: u32,
    fps: u32,
    override_value: Option<bool>,
) -> bool {
    if let Some(enabled) = override_value {
        return enabled;
    }

    !(codec.trim().eq_ignore_ascii_case("hevc")
        && high_throughput_media_profile(width, height, fps))
}

#[cfg(any(test, target_os = "macos"))]
fn high_throughput_media_profile(width: u32, height: u32, fps: u32) -> bool {
    fps >= 120 && u64::from(width).saturating_mul(u64::from(height)) >= 2_560_u64 * 1_440
}

#[cfg(test)]
mod tests {
    #[test]
    fn compressed_proxy_policy_defaults_away_from_high_throughput_hevc() {
        assert!(!super::compressed_proxy_enabled_for_values(
            "hevc", 2560, 1440, 144, None
        ));
        assert!(super::compressed_proxy_enabled_for_values(
            "h264", 2560, 1440, 144, None
        ));
        assert!(super::compressed_proxy_enabled_for_values(
            "hevc", 1920, 1080, 144, None
        ));
        assert!(super::compressed_proxy_enabled_for_values(
            "hevc", 2560, 1440, 60, None
        ));
        assert!(super::compressed_proxy_enabled_for_values(
            "hevc",
            2560,
            1440,
            144,
            Some(true)
        ));
        assert!(!super::compressed_proxy_enabled_for_values(
            "h264",
            2560,
            1440,
            144,
            Some(false)
        ));
    }
}
