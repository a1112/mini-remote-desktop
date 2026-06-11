use super::media_access_unit::{describe_lan_access_unit, LanAccessUnitCodec};
use anyhow::Result;
#[cfg(any(test, target_os = "macos"))]
use mrd_ipc::MediaProfile;
use mrd_pipeline_core::{DecodedFrame, VideoDecoder};
#[cfg(any(test, windows, target_os = "macos"))]
use mrd_render::RendererSnapshot;
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

#[cfg(any(test, windows, target_os = "macos"))]
pub(super) fn renderer_snapshot_uses_render_proxy(snapshot: &RendererSnapshot) -> bool {
    snapshot
        .swap_chain_present_mode
        .as_deref()
        .is_some_and(|mode| mode.starts_with("render_proxy"))
}

#[cfg(any(test, windows, target_os = "macos"))]
pub(super) fn renderer_snapshot_render_queue_replacement_delta(
    before: &RendererSnapshot,
    after: &RendererSnapshot,
) -> u64 {
    after
        .render_queue_replacements
        .unwrap_or_default()
        .saturating_sub(before.render_queue_replacements.unwrap_or_default())
}

#[cfg(any(test, windows, target_os = "macos"))]
#[derive(Default)]
pub(super) struct RendererWaitableDelta {
    pub(super) wait_ms: f64,
    pub(super) waits: u64,
    pub(super) timeouts: u64,
}

#[cfg(any(test, windows, target_os = "macos"))]
pub(super) fn renderer_snapshot_waitable_delta(
    before: &RendererSnapshot,
    after: &RendererSnapshot,
) -> RendererWaitableDelta {
    let before_waits = before.waitable_wait_count.unwrap_or_default();
    let after_waits = after.waitable_wait_count.unwrap_or_default();
    let before_total = before.waitable_wait_total_ms.unwrap_or_default();
    let after_total = after.waitable_wait_total_ms.unwrap_or_default();
    let before_timeouts = before.waitable_timeout_count.unwrap_or_default();
    let after_timeouts = after.waitable_timeout_count.unwrap_or_default();
    RendererWaitableDelta {
        wait_ms: (after_total - before_total).max(0.0),
        waits: after_waits.saturating_sub(before_waits),
        timeouts: after_timeouts.saturating_sub(before_timeouts),
    }
}

pub(super) fn decode_h264_desktop_frame(
    decoder: &mut dyn VideoDecoder,
    payload: &[u8],
) -> Result<Vec<DecodedFrame>> {
    decode_lan_desktop_frame(LanAccessUnitCodec::H264, decoder, payload)
}

pub(super) fn decode_lan_desktop_frame(
    codec: LanAccessUnitCodec,
    decoder: &mut dyn VideoDecoder,
    payload: &[u8],
) -> Result<Vec<DecodedFrame>> {
    if let Err(error) = decoder.push_access_unit(payload) {
        anyhow::bail!(
            "failed to decode LAN {} access unit: {error}; {}",
            codec.display_name(),
            describe_lan_access_unit(codec, payload)
        );
    }
    Ok(decoder.drain_decoded_frames())
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
    use mrd_pipeline_core::{DecodedFrame, PipelineError, VideoDecoder};
    use mrd_render::RendererSnapshot;

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
    fn decode_error_message_uses_selected_access_unit_codec() {
        struct RejectingDecoder;

        impl VideoDecoder for RejectingDecoder {
            fn push_access_unit(&mut self, _access_unit: &[u8]) -> Result<(), PipelineError> {
                Err(PipelineError::message("synthetic failure"))
            }

            fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
                Vec::new()
            }
        }

        let mut decoder = RejectingDecoder;
        let error = super::decode_lan_desktop_frame(
            super::super::media_access_unit::LanAccessUnitCodec::Hevc,
            &mut decoder,
            &[0, 0, 1, 0x26],
        )
        .expect_err("decode should fail")
        .to_string();

        assert!(error.contains("HEVC access unit"));
        assert!(!error.contains("H.264"));
        assert!(!error.contains("invalid magic"));
        assert!(!error.contains("probe fallback"));
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

    #[test]
    fn renderer_snapshot_detects_render_proxy_present_modes() {
        assert!(super::renderer_snapshot_uses_render_proxy(
            &renderer_snapshot(Some("render_proxy_metal_immediate"), None, None, None, None,)
        ));
        assert!(!super::renderer_snapshot_uses_render_proxy(
            &renderer_snapshot(Some("waitable"), None, None, None, None)
        ));
        assert!(!super::renderer_snapshot_uses_render_proxy(
            &renderer_snapshot(None, None, None, None, None)
        ));
    }

    #[test]
    fn renderer_snapshot_render_queue_replacement_delta_is_saturating() {
        assert_eq!(
            super::renderer_snapshot_render_queue_replacement_delta(
                &renderer_snapshot(None, Some(2), None, None, None),
                &renderer_snapshot(None, Some(5), None, None, None),
            ),
            3
        );
        assert_eq!(
            super::renderer_snapshot_render_queue_replacement_delta(
                &renderer_snapshot(None, Some(5), None, None, None),
                &renderer_snapshot(None, Some(2), None, None, None),
            ),
            0
        );
        assert_eq!(
            super::renderer_snapshot_render_queue_replacement_delta(
                &renderer_snapshot(None, None, None, None, None),
                &renderer_snapshot(None, Some(1), None, None, None),
            ),
            1
        );
    }

    #[test]
    fn renderer_snapshot_waitable_delta_clamps_time_and_saturates_counts() {
        let delta = super::renderer_snapshot_waitable_delta(
            &renderer_snapshot(None, None, Some(2), Some(5.0), Some(1)),
            &renderer_snapshot(None, None, Some(5), Some(9.5), Some(3)),
        );
        assert_eq!(delta.wait_ms, 4.5);
        assert_eq!(delta.waits, 3);
        assert_eq!(delta.timeouts, 2);

        let saturated = super::renderer_snapshot_waitable_delta(
            &renderer_snapshot(None, None, Some(5), Some(9.5), Some(3)),
            &renderer_snapshot(None, None, Some(2), Some(5.0), Some(1)),
        );
        assert_eq!(saturated.wait_ms, 0.0);
        assert_eq!(saturated.waits, 0);
        assert_eq!(saturated.timeouts, 0);
    }

    fn renderer_snapshot(
        swap_chain_present_mode: Option<&str>,
        render_queue_replacements: Option<u64>,
        waitable_wait_count: Option<u64>,
        waitable_wait_total_ms: Option<f64>,
        waitable_timeout_count: Option<u64>,
    ) -> RendererSnapshot {
        RendererSnapshot {
            attached_to_target: true,
            uploaded_frame_count: 0,
            presented_frame_count: 0,
            present_skipped_count: 0,
            render_queue_replacements,
            last_present_status: None,
            low_latency_frame_latency_target: None,
            swap_chain_max_frame_latency: None,
            swap_chain_allow_tearing: None,
            swap_chain_waitable_object: None,
            swap_chain_present_mode: swap_chain_present_mode.map(ToString::to_string),
            display_refresh_hz: None,
            render_thread_priority: None,
            waitable_wait_count,
            waitable_wait_total_ms,
            waitable_timeout_count,
            last_waitable_wait_ms: None,
            last_render_prepare_wait_ms: None,
            last_render_shared_resource_ms: None,
            last_render_wait_for_drawable_ms: None,
            last_render_encode_commit_ms: None,
            last_render_draw_present_ms: None,
            last_width: 2560,
            last_height: 1440,
            last_pixel_format: None,
        }
    }
}
