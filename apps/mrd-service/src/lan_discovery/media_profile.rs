use anyhow::Result;
use mrd_ipc::{MediaProfile, MediaProfileNegotiation};
use mrd_pipeline_core::ColorMode;
use mrd_proto::DeviceId;

use super::{
    format_peer_capabilities, format_peer_transports, normalize_transport_kind, LanAccessUnitCodec,
    LAN_MEDIA_COLOR_MODE_CAPABILITY, LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY,
    LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY, LAN_MEDIA_MAX_FPS,
    LAN_MEDIA_PROFILE_CONTROL_TRANSPORT, LAN_MEDIA_TARGET_BITRATE_MBPS, LAN_MEDIA_TARGET_FPS,
    LAN_MEDIA_TARGET_HEIGHT, LAN_MEDIA_TARGET_WIDTH, LAN_QUIC_MEDIA_PROFILE_TRANSPORT,
    LAN_QUIC_MEDIA_TRANSPORT, LAN_QUIC_MEDIA_V2_TRANSPORT,
};

pub(crate) fn normalize_lan_codec_name(codec: &str) -> Option<&'static str> {
    let codec = codec.trim();
    if codec.eq_ignore_ascii_case("hevc")
        || codec.eq_ignore_ascii_case("h265")
        || codec.eq_ignore_ascii_case("h.265")
    {
        Some("hevc")
    } else if codec.eq_ignore_ascii_case("h264") || codec.eq_ignore_ascii_case("h.264") {
        Some("h264")
    } else {
        None
    }
}

pub(crate) fn default_media_profile() -> MediaProfile {
    let mut profile = MediaProfile {
        width: LAN_MEDIA_TARGET_WIDTH,
        height: LAN_MEDIA_TARGET_HEIGHT,
        fps: LAN_MEDIA_TARGET_FPS,
        bitrate_mbps: LAN_MEDIA_TARGET_BITRATE_MBPS,
        codec: "hevc".to_string(),
        ..MediaProfile::default()
    };
    apply_lan_media_profile_defaults(&mut profile);
    profile
}

pub(crate) fn default_media_profile_negotiation() -> MediaProfileNegotiation {
    let profile = default_media_profile();
    MediaProfileNegotiation {
        requested: profile.clone(),
        selected: profile,
        status: "accepted".to_string(),
        reason: None,
        selected_source_id: None,
        selected_width: None,
        selected_height: None,
        downgrade_reason: None,
    }
}

pub(crate) fn apply_lan_media_profile_defaults(profile: &mut MediaProfile) {
    if normalize_lan_codec_name(&profile.codec) == Some("hevc") {
        profile.codec = "hevc".to_string();
        if profile.codec_profile.is_none() {
            profile.codec_profile = Some("main".to_string());
        }
        if profile.bit_depth.is_none() {
            profile.bit_depth = Some(8);
        }
        if profile.chroma_subsampling.is_none() {
            profile.chroma_subsampling = Some("4:2:0".to_string());
        }
        if profile.pixel_format.is_none() {
            profile.pixel_format = Some("nv12".to_string());
        }
        if profile.hdr_enabled.is_none() {
            profile.hdr_enabled = Some(false);
        }
    }
}

pub(crate) fn clamp_media_profile_to_lan_capability(
    requested_profile: Option<MediaProfile>,
) -> Result<MediaProfileNegotiation> {
    let requested = requested_profile.unwrap_or_else(default_media_profile);
    validate_media_profile(&requested)?;

    let mut selected = requested.clone();
    selected.width = selected.width.min(LAN_MEDIA_TARGET_WIDTH);
    selected.height = selected.height.min(LAN_MEDIA_TARGET_HEIGHT);
    selected.fps = selected.fps.min(LAN_MEDIA_MAX_FPS);
    selected.bitrate_mbps = selected.bitrate_mbps.min(LAN_MEDIA_TARGET_BITRATE_MBPS);
    normalize_lan_media_profile(&mut selected);

    let changed = selected != requested;
    Ok(MediaProfileNegotiation {
        requested,
        selected: selected.clone(),
        status: if changed { "downgraded" } else { "accepted" }.to_string(),
        reason: if changed {
            Some("clamped to LAN QUIC media capability".to_string())
        } else {
            None
        },
        selected_source_id: None,
        selected_width: Some(selected.width),
        selected_height: Some(selected.height),
        downgrade_reason: if changed {
            Some("clamped to LAN QUIC media capability".to_string())
        } else {
            None
        },
    })
}

pub(crate) fn validate_media_profile(profile: &MediaProfile) -> Result<()> {
    if profile.width == 0 || profile.height == 0 || profile.fps == 0 || profile.bitrate_mbps == 0 {
        anyhow::bail!("media profile width, height, fps and bitrate must be greater than zero");
    }
    Ok(())
}

pub(crate) fn normalize_lan_media_profile(profile: &mut MediaProfile) {
    match normalize_lan_codec_name(&profile.codec) {
        Some(codec) => {
            profile.codec = codec.to_string();
            apply_lan_media_profile_defaults(profile);
        }
        None => {
            profile.codec = "h264".to_string();
            profile.codec_profile = None;
            profile.bit_depth = None;
            profile.chroma_subsampling = None;
            profile.pixel_format = None;
            profile.hdr_enabled = None;
        }
    }
}

pub(super) fn lan_runtime_media_profile(
    selected_profile: &MediaProfile,
    codec: LanAccessUnitCodec,
) -> MediaProfile {
    let mut profile = selected_profile.clone();
    profile.codec = codec.name().to_string();
    if codec == LanAccessUnitCodec::H264 {
        profile.codec_profile = Some("high".to_string());
        profile.bit_depth = Some(8);
        profile.chroma_subsampling = Some("4:2:0".to_string());
        profile.pixel_format = Some("nv12".to_string());
        profile.hdr_enabled = Some(false);
    } else {
        apply_lan_media_profile_defaults(&mut profile);
    }
    profile
}

pub(crate) fn ensure_peer_supports_requested_media(
    target_device_id: &DeviceId,
    transport_kind: &str,
    peer_transports: &[String],
    requested_profile: Option<&MediaProfile>,
    peer_media_capabilities: &[String],
) -> Result<()> {
    let transport = normalize_transport_kind(transport_kind);
    if transport == "quic" {
        let required = [
            LAN_QUIC_MEDIA_TRANSPORT,
            LAN_QUIC_MEDIA_PROFILE_TRANSPORT,
            LAN_QUIC_MEDIA_V2_TRANSPORT,
            LAN_MEDIA_PROFILE_CONTROL_TRANSPORT,
        ];
        let missing = required
            .iter()
            .filter(|required_transport| {
                !peer_transports
                    .iter()
                    .any(|peer_transport| peer_transport.eq_ignore_ascii_case(required_transport))
            })
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            anyhow::bail!(
                "LAN peer does not advertise required media controls [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
                missing.join(", "),
                target_device_id.0,
                format_peer_transports(peer_transports)
            );
        }
        let profile = requested_profile
            .cloned()
            .unwrap_or_else(default_media_profile);
        let missing_capabilities =
            missing_profile_media_capabilities(&profile, peer_media_capabilities);
        if !missing_capabilities.is_empty() {
            anyhow::bail!(
                "LAN peer does not advertise required media capabilities for {} [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
                format_media_profile(&profile),
                missing_capabilities.join(", "),
                target_device_id.0,
                format_peer_capabilities(peer_media_capabilities)
            );
        }
    }
    Ok(())
}

fn missing_profile_media_capabilities(
    profile: &MediaProfile,
    peer_media_capabilities: &[String],
) -> Vec<String> {
    let mut groups = match LanAccessUnitCodec::from_profile(profile) {
        LanAccessUnitCodec::H264 => vec![(
            "h264 encoder",
            vec![
                "encode.nvenc_h264",
                "nvenc_h264",
                "encode.videotoolbox_h264",
                "videotoolbox_h264",
                "encode.openh264",
                "openh264_fallback",
                "openh264",
            ],
        )],
        LanAccessUnitCodec::Hevc if lan_profile_requests_hevc_main10(profile) => vec![
            (
                "hevc main10 encoder",
                vec!["encode.nvenc_hevc_main10", "nvenc_hevc_main10"],
            ),
            (
                LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY,
                vec![LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY],
            ),
        ],
        LanAccessUnitCodec::Hevc => vec![
            (
                "hevc encoder",
                vec![
                    "encode.nvenc_hevc",
                    "nvenc_hevc",
                    "encode.videotoolbox_hevc",
                    "videotoolbox_hevc",
                ],
            ),
            (
                LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY,
                vec![LAN_MEDIA_HEVC_MAIN_420_8BIT_CAPABILITY],
            ),
        ],
    };
    if profile_requests_non_full_color(profile) {
        groups.push((
            LAN_MEDIA_COLOR_MODE_CAPABILITY,
            vec![LAN_MEDIA_COLOR_MODE_CAPABILITY],
        ));
    }
    missing_capability_groups(peer_media_capabilities, &groups)
}

pub(crate) fn ensure_peer_can_receive_selected_media(
    peer_label: &str,
    profile: &MediaProfile,
    peer_media_capabilities: &[String],
) -> Result<()> {
    let missing_capabilities =
        missing_profile_receiver_media_capabilities(profile, peer_media_capabilities);
    if !missing_capabilities.is_empty() {
        anyhow::bail!(
            "LAN peer cannot receive selected media profile {} [{}]: {} supports {}. Rebuild and restart the peer mrd-service/Rdesk from the latest main branch",
            format_media_profile(profile),
            missing_capabilities.join(", "),
            peer_label,
            format_peer_capabilities(peer_media_capabilities)
        );
    }
    Ok(())
}

pub(crate) fn missing_profile_receiver_media_capabilities(
    profile: &MediaProfile,
    peer_media_capabilities: &[String],
) -> Vec<String> {
    let groups = match LanAccessUnitCodec::from_profile(profile) {
        LanAccessUnitCodec::H264 => vec![(
            "h264 decoder",
            vec![
                "decode.nvdec",
                "nvdec",
                "decode.videotoolbox_h264",
                "decode.videotoolbox",
                "videotoolbox",
                "decode.linux_h264",
                "linux_h264",
                "decode.ffmpeg_h264",
                "ffmpeg_h264",
                "decode.software",
                "h264_software",
                "software_decode",
            ],
        )],
        LanAccessUnitCodec::Hevc if lan_profile_requests_hevc_main10(profile) => vec![
            (
                "hevc main10 decoder",
                vec![
                    "decode.nvdec_hevc_main10",
                    "nvdec_hevc_main10",
                    "nvdec_hevc_main10_d3d11_shared",
                    "decode.ffmpeg_hevc_main10",
                    "ffmpeg_hevc_main10",
                    "software_hevc_main10",
                ],
            ),
            (
                LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY,
                vec![LAN_MEDIA_HEVC_MAIN10_420_10BIT_CAPABILITY],
            ),
        ],
        LanAccessUnitCodec::Hevc => vec![(
            "hevc decoder",
            vec![
                "decode.nvdec_hevc",
                "nvdec_hevc",
                "nvdec_hevc_d3d11_shared",
                "decode.videotoolbox_hevc",
                "decode.linux_hevc",
                "linux_hevc",
                "decode.ffmpeg_hevc",
                "ffmpeg_hevc",
                "software_decode",
            ],
        )],
    };
    missing_capability_groups(peer_media_capabilities, &groups)
}

fn missing_capability_groups(
    peer_media_capabilities: &[String],
    groups: &[(&'static str, Vec<&'static str>)],
) -> Vec<String> {
    groups
        .iter()
        .filter(|(_, aliases)| {
            !aliases.iter().any(|alias| {
                peer_media_capabilities
                    .iter()
                    .any(|capability| capability.eq_ignore_ascii_case(alias))
            })
        })
        .map(|(label, aliases)| {
            if aliases.len() == 1 && aliases[0] == *label {
                (*label).to_string()
            } else {
                format!("{label} ({})", aliases.join(" | "))
            }
        })
        .collect()
}

fn profile_requests_non_full_color(profile: &MediaProfile) -> bool {
    match profile.color_mode.as_deref() {
        None => false,
        Some(mode) => !mode.eq_ignore_ascii_case(ColorMode::Full.as_str()),
    }
}

pub(crate) fn lan_color_mode_for_profile(profile: &MediaProfile) -> Result<ColorMode> {
    match profile.color_mode.as_deref() {
        None => Ok(ColorMode::Full),
        Some(mode) if mode.eq_ignore_ascii_case(ColorMode::Full.as_str()) => Ok(ColorMode::Full),
        Some(mode) if mode.eq_ignore_ascii_case(ColorMode::Grayscale.as_str()) => {
            Ok(ColorMode::Grayscale)
        }
        Some(mode) if mode.eq_ignore_ascii_case(ColorMode::Monochrome.as_str()) => {
            Ok(ColorMode::Monochrome)
        }
        Some(mode) if mode.eq_ignore_ascii_case(ColorMode::LowChroma.as_str()) => {
            Ok(ColorMode::LowChroma)
        }
        Some(mode) => anyhow::bail!("unsupported LAN color_mode: {mode}"),
    }
}

pub(crate) fn lan_profile_requests_hevc_main10(profile: &MediaProfile) -> bool {
    if LanAccessUnitCodec::from_profile(profile) != LanAccessUnitCodec::Hevc {
        return false;
    }
    profile
        .codec_profile
        .as_deref()
        .map(|profile| {
            let profile = profile.to_ascii_lowercase();
            profile == "main10" || profile == "main_10"
        })
        .unwrap_or(false)
        || profile.bit_depth.map(|depth| depth >= 10).unwrap_or(false)
        || profile
            .pixel_format
            .as_deref()
            .map(|format| {
                format.eq_ignore_ascii_case("p010") || format.eq_ignore_ascii_case("p016")
            })
            .unwrap_or(false)
        || profile
            .color_pipeline
            .as_deref()
            .map(|pipeline| pipeline.eq_ignore_ascii_case("hdr_main10"))
            .unwrap_or(false)
}

pub(crate) fn format_media_profile(profile: &MediaProfile) -> String {
    let color_suffix = match (
        profile.color_mode.as_deref(),
        profile.color_pipeline.as_deref(),
    ) {
        (None, None) => String::new(),
        (mode, pipeline) => format!(
            " / color={} pipeline={}",
            mode.unwrap_or("full"),
            pipeline.unwrap_or("sdr8")
        ),
    };
    format!(
        "{}x{} @ {} FPS / {} Mbps / {}{}",
        profile.width,
        profile.height,
        profile.fps,
        profile.bitrate_mbps,
        profile.codec,
        color_suffix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_lan_codec_aliases() {
        assert_eq!(normalize_lan_codec_name("hevc"), Some("hevc"));
        assert_eq!(normalize_lan_codec_name(" H.265 "), Some("hevc"));
        assert_eq!(normalize_lan_codec_name("h264"), Some("h264"));
        assert_eq!(normalize_lan_codec_name("AV1"), None);
    }

    #[test]
    fn default_profile_uses_hevc_lan_defaults() {
        let profile = default_media_profile();

        assert_eq!(profile.codec, "hevc");
        assert_eq!(profile.codec_profile.as_deref(), Some("main"));
        assert_eq!(profile.bit_depth, Some(8));
        assert_eq!(profile.chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(profile.pixel_format.as_deref(), Some("nv12"));
        assert_eq!(profile.hdr_enabled, Some(false));
    }

    #[test]
    fn runtime_h264_profile_uses_h264_encoder_defaults() {
        let mut selected = default_media_profile();
        selected.codec_profile = Some("main10".to_string());
        selected.bit_depth = Some(10);
        selected.pixel_format = Some("p010".to_string());
        selected.hdr_enabled = Some(true);

        let runtime = lan_runtime_media_profile(&selected, LanAccessUnitCodec::H264);

        assert_eq!(runtime.codec, "h264");
        assert_eq!(runtime.codec_profile.as_deref(), Some("high"));
        assert_eq!(runtime.bit_depth, Some(8));
        assert_eq!(runtime.chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(runtime.pixel_format.as_deref(), Some("nv12"));
        assert_eq!(runtime.hdr_enabled, Some(false));
    }

    #[test]
    fn runtime_hevc_profile_preserves_selected_main10_fields() {
        let mut selected = default_media_profile();
        selected.codec_profile = Some("main10".to_string());
        selected.bit_depth = Some(10);
        selected.pixel_format = Some("p010".to_string());
        selected.hdr_enabled = Some(true);

        let runtime = lan_runtime_media_profile(&selected, LanAccessUnitCodec::Hevc);

        assert_eq!(runtime.codec, "hevc");
        assert_eq!(runtime.codec_profile.as_deref(), Some("main10"));
        assert_eq!(runtime.bit_depth, Some(10));
        assert_eq!(runtime.pixel_format.as_deref(), Some("p010"));
        assert_eq!(runtime.hdr_enabled, Some(true));
    }
}
