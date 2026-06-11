pub(super) fn format_peer_transports(peer_transports: &[String]) -> String {
    format_peer_list(peer_transports)
}

pub(super) fn format_peer_capabilities(peer_media_capabilities: &[String]) -> String {
    format_peer_list(peer_media_capabilities)
}

pub(super) fn normalize_transport_kind(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() || normalized == "quic_quinn" {
        "quic".to_string()
    } else {
        normalized
    }
}

fn format_peer_list(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_empty_peer_lists_as_none() {
        assert_eq!(format_peer_transports(&[]), "none");
        assert_eq!(format_peer_capabilities(&[]), "none");
    }

    #[test]
    fn formats_peer_lists_in_advertised_order() {
        let transports = vec![
            "quic_datagram".to_string(),
            "media_profile_control_v1".to_string(),
        ];
        let capabilities = vec!["nvenc_h264".to_string(), "media.color_mode_v1".to_string()];

        assert_eq!(
            format_peer_transports(&transports),
            "quic_datagram, media_profile_control_v1"
        );
        assert_eq!(
            format_peer_capabilities(&capabilities),
            "nvenc_h264, media.color_mode_v1"
        );
    }

    #[test]
    fn normalizes_transport_aliases_and_case() {
        assert_eq!(normalize_transport_kind(""), "quic");
        assert_eq!(normalize_transport_kind("  QUIC_QUINN  "), "quic");
        assert_eq!(normalize_transport_kind(" WebRTC "), "webrtc");
    }
}
