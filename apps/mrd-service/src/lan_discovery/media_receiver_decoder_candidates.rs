use super::LanAccessUnitCodec;

pub(super) fn preferred_lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
) -> Vec<&'static str> {
    let preferred = std::env::var("MRD_LAN_RECEIVER_DECODER")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    preferred_lan_receiver_decoder_candidates_from_preference(codec, preferred.as_str())
}

pub(super) fn preferred_lan_receiver_decoder_candidates_from_preference(
    codec: LanAccessUnitCodec,
    preferred: &str,
) -> Vec<&'static str> {
    match (codec, preferred) {
        (LanAccessUnitCodec::H264, "software" | "h264_software" | "openh264") => {
            vec!["h264_software"]
        }
        #[cfg(target_os = "macos")]
        (LanAccessUnitCodec::H264, "videotoolbox" | "videotoolbox_h264") => {
            vec!["videotoolbox", "h264_software"]
        }
        (LanAccessUnitCodec::H264, "nvdec" | "nvdec_d3d11_shared" | "d3d11_shared") => {
            vec!["nvdec_d3d11_shared", "nvdec"]
        }
        (LanAccessUnitCodec::H264, "nvdec_cpu" | "nvdec_cpu_nv12") => vec!["nvdec"],
        (LanAccessUnitCodec::H264, "ffmpeg" | "ffmpeg_h264" | "h264_ffmpeg") => {
            vec!["ffmpeg_h264", "h264_software"]
        }
        #[cfg(target_os = "macos")]
        (LanAccessUnitCodec::Hevc, "videotoolbox" | "videotoolbox_hevc" | "hevc") => {
            vec!["videotoolbox_hevc", "ffmpeg_hevc"]
        }
        (
            LanAccessUnitCodec::Hevc,
            "nvdec" | "nvdec_hevc_d3d11_shared" | "nvdec_d3d11_shared_hevc" | "d3d11_shared",
        ) => {
            vec!["nvdec_hevc_d3d11_shared", "nvdec_hevc"]
        }
        (LanAccessUnitCodec::Hevc, "nvdec_cpu" | "nvdec_cpu_nv12" | "nvdec_hevc") => {
            vec!["nvdec_hevc"]
        }
        (LanAccessUnitCodec::Hevc, "ffmpeg" | "ffmpeg_hevc" | "hevc_ffmpeg" | "h265_ffmpeg") => {
            vec!["ffmpeg_hevc"]
        }
        _ => default_lan_receiver_decoder_candidates(codec).to_vec(),
    }
}

pub(super) fn lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
    preferred_backend: Option<&'static str>,
) -> Vec<&'static str> {
    prioritize_lan_receiver_decoder_candidates(
        preferred_lan_receiver_decoder_candidates(codec),
        preferred_backend,
    )
}

pub(super) fn prioritize_lan_receiver_decoder_candidates(
    candidates: Vec<&'static str>,
    preferred_backend: Option<&'static str>,
) -> Vec<&'static str> {
    let Some(preferred_backend) = preferred_backend else {
        return candidates;
    };
    if !candidates.contains(&preferred_backend) {
        return candidates;
    }

    let mut prioritized = vec![preferred_backend];
    prioritized.extend(
        candidates
            .into_iter()
            .filter(|backend| *backend != preferred_backend),
    );
    prioritized
}

#[cfg(windows)]
pub(super) fn default_lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &[
            "nvdec_d3d11_shared",
            "nvdec",
            "ffmpeg_h264",
            "h264_software",
        ],
        LanAccessUnitCodec::Hevc => &["nvdec_hevc_d3d11_shared", "nvdec_hevc", "ffmpeg_hevc"],
    }
}

#[cfg(target_os = "linux")]
pub(super) fn default_lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["linux_h264", "ffmpeg_h264", "h264_software"],
        LanAccessUnitCodec::Hevc => &["linux_hevc", "ffmpeg_hevc"],
    }
}

#[cfg(target_os = "macos")]
pub(super) fn default_lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["videotoolbox", "ffmpeg_h264", "h264_software"],
        LanAccessUnitCodec::Hevc => &["videotoolbox_hevc", "ffmpeg_hevc"],
    }
}

#[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
pub(super) fn default_lan_receiver_decoder_candidates(
    codec: LanAccessUnitCodec,
) -> &'static [&'static str] {
    match codec {
        LanAccessUnitCodec::H264 => &["ffmpeg_h264", "h264_software"],
        LanAccessUnitCodec::Hevc => &["ffmpeg_hevc"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_h264_software_aliases_use_software_only() {
        assert_eq!(
            preferred_lan_receiver_decoder_candidates_from_preference(
                LanAccessUnitCodec::H264,
                "openh264"
            ),
            vec!["h264_software"]
        );
    }

    #[test]
    fn preferred_hevc_ffmpeg_aliases_use_hevc_ffmpeg_only() {
        assert_eq!(
            preferred_lan_receiver_decoder_candidates_from_preference(
                LanAccessUnitCodec::Hevc,
                "h265_ffmpeg"
            ),
            vec!["ffmpeg_hevc"]
        );
    }

    #[test]
    fn preferred_backend_is_promoted_when_available() {
        assert_eq!(
            prioritize_lan_receiver_decoder_candidates(
                vec!["nvdec", "ffmpeg_h264", "h264_software"],
                Some("h264_software"),
            ),
            vec!["h264_software", "nvdec", "ffmpeg_h264"]
        );
    }

    #[test]
    fn unknown_preferred_backend_keeps_original_order() {
        assert_eq!(
            prioritize_lan_receiver_decoder_candidates(
                vec!["nvdec", "ffmpeg_h264", "h264_software"],
                Some("missing"),
            ),
            vec!["nvdec", "ffmpeg_h264", "h264_software"]
        );
    }
}
