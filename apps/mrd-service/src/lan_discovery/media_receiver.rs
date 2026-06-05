#[cfg(any(test, target_os = "macos"))]
use mrd_ipc::MediaProfile;
#[cfg(any(test, target_os = "macos"))]
use mrd_transport_quic_quinn::{QuicMediaCodec, QuicMediaPayloadType};

#[cfg(target_os = "macos")]
const COMPRESSED_MEDIA_ENV: &str = "MRD_MACOS_RENDER_PROXY_COMPRESSED_MEDIA";

#[cfg(target_os = "macos")]
pub(super) fn compressed_proxy_enabled() -> bool {
    compressed_proxy_env_override().unwrap_or(true)
}

#[cfg(target_os = "macos")]
pub(super) fn compressed_proxy_enabled_for_profile(profile: &MediaProfile) -> bool {
    compressed_proxy_enabled_for_profile_values(profile, compressed_proxy_env_override())
}

#[cfg(target_os = "macos")]
fn compressed_proxy_env_override() -> Option<bool> {
    compressed_proxy_env_override_from_value(std::env::var(COMPRESSED_MEDIA_ENV).ok().as_deref())
}

#[cfg(any(test, target_os = "macos"))]
fn compressed_proxy_env_override_from_value(value: Option<&str>) -> Option<bool> {
    super::runtime_flags::env_bool_override(value)
}

#[cfg(any(test, target_os = "macos"))]
pub(super) fn compressed_proxy_enabled_for_profile_values(
    profile: &MediaProfile,
    override_value: Option<bool>,
) -> bool {
    compressed_proxy_enabled_for_values(
        profile.codec.as_str(),
        profile.width,
        profile.height,
        profile.fps,
        override_value,
    )
}

#[cfg(any(test, target_os = "macos"))]
pub(super) fn compressed_direct_render_candidate(
    proxy_enabled: bool,
    payload_type: QuicMediaPayloadType,
    codec: QuicMediaCodec,
) -> bool {
    proxy_enabled
        && payload_type == QuicMediaPayloadType::AccessUnit
        && matches!(codec, QuicMediaCodec::H264 | QuicMediaCodec::Hevc)
}

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
    use super::{QuicMediaCodec, QuicMediaPayloadType};
    use mrd_ipc::MediaProfile;

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

    #[test]
    fn compressed_direct_render_candidate_requires_access_unit_and_compressed_video_codec() {
        assert!(super::compressed_direct_render_candidate(
            true,
            QuicMediaPayloadType::AccessUnit,
            QuicMediaCodec::H264
        ));
        assert!(super::compressed_direct_render_candidate(
            true,
            QuicMediaPayloadType::AccessUnit,
            QuicMediaCodec::Hevc
        ));
        assert!(!super::compressed_direct_render_candidate(
            false,
            QuicMediaPayloadType::AccessUnit,
            QuicMediaCodec::H264
        ));
        assert!(!super::compressed_direct_render_candidate(
            true,
            QuicMediaPayloadType::Probe,
            QuicMediaCodec::H264
        ));
        assert!(!super::compressed_direct_render_candidate(
            true,
            QuicMediaPayloadType::AccessUnit,
            QuicMediaCodec::Av1
        ));
    }

    #[test]
    fn compressed_proxy_env_override_parses_bool_aliases() {
        assert_eq!(super::compressed_proxy_env_override_from_value(None), None);
        assert_eq!(
            super::compressed_proxy_env_override_from_value(Some("")),
            None
        );
        assert_eq!(
            super::compressed_proxy_env_override_from_value(Some("YES")),
            Some(true)
        );
        assert_eq!(
            super::compressed_proxy_env_override_from_value(Some("off")),
            Some(false)
        );
        assert_eq!(
            super::compressed_proxy_env_override_from_value(Some("invalid")),
            None
        );
    }

    #[test]
    fn compressed_proxy_profile_gate_uses_override_and_high_throughput_policy() {
        let high_throughput_hevc = MediaProfile {
            width: 2560,
            height: 1440,
            fps: 144,
            codec: "hevc".to_string(),
            ..MediaProfile::default()
        };
        let high_throughput_h264 = MediaProfile {
            codec: "h264".to_string(),
            ..high_throughput_hevc.clone()
        };

        assert!(!super::compressed_proxy_enabled_for_profile_values(
            &high_throughput_hevc,
            None
        ));
        assert!(super::compressed_proxy_enabled_for_profile_values(
            &high_throughput_h264,
            None
        ));
        assert!(super::compressed_proxy_enabled_for_profile_values(
            &high_throughput_hevc,
            Some(true)
        ));
        assert!(!super::compressed_proxy_enabled_for_profile_values(
            &high_throughput_h264,
            Some(false)
        ));
    }
}
