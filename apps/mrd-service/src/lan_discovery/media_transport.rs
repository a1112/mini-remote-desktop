use anyhow::{Context, Result};
use mrd_ipc::MediaProfile;
use mrd_transport_quic_quinn::{
    QuicAuReassemblerConfig, QuinnDatagramEndpoint, QUIC_AU_FRAGMENT_HEADER_LEN,
    QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN,
};
use std::time::Duration;

use super::{
    LAN_MEDIA_REASSEMBLER_FRAME_TIMEOUT_MS, LAN_MEDIA_REASSEMBLER_MAX_PENDING_FRAMES,
    LAN_QUIC_BEST_EFFORT_DATAGRAM_MAX_BITRATE_MBPS, LAN_QUIC_DATAGRAM_SEND_BUDGET,
    LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_BITRATE_MBPS, LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_FPS,
    LAN_QUIC_FALLBACK_DATAGRAM_BYTES, LAN_QUIC_LAN_HIGH_QUALITY_DATAGRAM_BYTES,
    LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_BITRATE_MBPS,
    LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_FPS, LAN_QUIC_RELIABLE_WHOLE_FRAME_MIN_BITRATE_MBPS,
    LAN_RELIABLE_WHOLE_FRAME_ENV,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanReliableMediaSendMode {
    Disabled,
    PerMessage,
    Persistent,
}

pub(crate) fn lan_media_reassembler_config() -> QuicAuReassemblerConfig {
    QuicAuReassemblerConfig {
        frame_timeout: Duration::from_millis(LAN_MEDIA_REASSEMBLER_FRAME_TIMEOUT_MS),
        max_pending_frames: LAN_MEDIA_REASSEMBLER_MAX_PENDING_FRAMES,
    }
}

pub(crate) fn should_send_access_unit_reliably(
    reliable_media_supported: bool,
    is_keyframe: bool,
    _payload_len: usize,
    _max_datagram_size: usize,
) -> bool {
    if !reliable_media_supported {
        return false;
    }

    is_keyframe
}

pub(crate) fn select_reliable_media_send_mode(
    reliable_media_supported: bool,
    persistent_media_supported: bool,
) -> LanReliableMediaSendMode {
    if persistent_media_supported {
        LanReliableMediaSendMode::Persistent
    } else if reliable_media_supported {
        LanReliableMediaSendMode::PerMessage
    } else {
        LanReliableMediaSendMode::Disabled
    }
}

pub(crate) fn select_reliable_media_send_mode_for_profile(
    reliable_media_supported: bool,
    persistent_media_supported: bool,
    profile: &MediaProfile,
) -> LanReliableMediaSendMode {
    if reliable_media_supported
        && (profile.fps <= 60
            || profile.bitrate_mbps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_MIN_BITRATE_MBPS)
    {
        LanReliableMediaSendMode::PerMessage
    } else {
        select_reliable_media_send_mode(reliable_media_supported, persistent_media_supported)
    }
}

pub(crate) fn use_best_effort_media_datagrams(profile: &MediaProfile) -> bool {
    profile.bitrate_mbps <= LAN_QUIC_BEST_EFFORT_DATAGRAM_MAX_BITRATE_MBPS
}

pub(crate) fn lan_media_datagram_size(
    negotiated_max_datagram_size: usize,
    profile: &MediaProfile,
    high_quality_datagram_supported: bool,
) -> usize {
    let minimum = QUIC_MEDIA_V3_FRAGMENT_HEADER_LEN.max(QUIC_AU_FRAGMENT_HEADER_LEN) + 1;
    let safe_cap = if high_quality_datagram_supported && !use_best_effort_media_datagrams(profile) {
        LAN_QUIC_LAN_HIGH_QUALITY_DATAGRAM_BYTES
    } else {
        LAN_QUIC_FALLBACK_DATAGRAM_BYTES
    };
    negotiated_max_datagram_size.min(safe_cap).max(minimum)
}

pub(crate) fn lan_datagram_frame_send_budget(
    profile: &MediaProfile,
    reliable_media_enabled: bool,
) -> Option<Duration> {
    if reliable_media_enabled
        && profile.fps >= LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_FPS
        && profile.bitrate_mbps >= LAN_QUIC_DATAGRAM_SEND_BUDGET_MIN_BITRATE_MBPS
    {
        Some(LAN_QUIC_DATAGRAM_SEND_BUDGET)
    } else {
        None
    }
}

pub(crate) enum LanDatagramSendOutcome {
    Sent,
    DroppedForCapacity,
}

pub(crate) async fn send_lan_media_datagram(
    endpoint: &QuinnDatagramEndpoint,
    fragment: bytes::Bytes,
    wait_for_capacity: bool,
    wait_timeout: Option<Duration>,
) -> Result<LanDatagramSendOutcome> {
    if !wait_for_capacity {
        endpoint
            .send_datagram(fragment)
            .context("failed to send LAN QUIC media datagram")?;
        return Ok(LanDatagramSendOutcome::Sent);
    }

    match endpoint.send_datagram(fragment.clone()) {
        Ok(()) => Ok(LanDatagramSendOutcome::Sent),
        Err(_) => {
            let Some(timeout) = wait_timeout else {
                endpoint
                    .send_datagram_wait(fragment)
                    .await
                    .context("failed to send LAN QUIC media datagram after waiting for capacity")?;
                return Ok(LanDatagramSendOutcome::Sent);
            };
            if timeout.is_zero() {
                return Ok(LanDatagramSendOutcome::DroppedForCapacity);
            }
            match tokio::time::timeout(timeout, endpoint.send_datagram_wait(fragment)).await {
                Ok(Ok(())) => Ok(LanDatagramSendOutcome::Sent),
                Ok(Err(error)) => Err(error)
                    .context("failed to send LAN QUIC media datagram after waiting for capacity"),
                Err(_) => Ok(LanDatagramSendOutcome::DroppedForCapacity),
            }
        }
    }
}

pub(crate) async fn send_lan_reliable_media_fragment(
    endpoint: &QuinnDatagramEndpoint,
    mode: LanReliableMediaSendMode,
    fragment: bytes::Bytes,
) -> Result<()> {
    match mode {
        LanReliableMediaSendMode::Disabled => {
            anyhow::bail!("LAN reliable media send requested while reliable media is disabled")
        }
        LanReliableMediaSendMode::PerMessage => {
            endpoint
                .send_reliable_message(fragment)
                .await
                .context("failed to send per-message reliable LAN media fragment")?;
        }
        LanReliableMediaSendMode::Persistent => {
            endpoint
                .send_reliable_message_persistent(fragment)
                .await
                .context("failed to send persistent reliable LAN media fragment")?;
        }
    }
    Ok(())
}

pub(crate) fn should_send_access_unit_as_reliable_frame(
    reliable_media_supported: bool,
    media_v3_supported: bool,
    _fragment_count: usize,
    profile: &MediaProfile,
    reliable_whole_frame_override: Option<bool>,
) -> bool {
    if !reliable_media_supported || !media_v3_supported {
        return false;
    }
    if let Some(enabled) = reliable_whole_frame_override {
        return enabled;
    }

    should_default_to_reliable_whole_frame(profile)
}

fn should_default_to_reliable_whole_frame(profile: &MediaProfile) -> bool {
    profile.fps <= 60
        || (profile.bitrate_mbps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_BITRATE_MBPS
            && profile.fps >= LAN_QUIC_RELIABLE_WHOLE_FRAME_DEFAULT_MIN_FPS)
}

pub(crate) fn reliable_whole_frame_media_override() -> Option<bool> {
    reliable_whole_frame_media_override_from_env_value(
        std::env::var(LAN_RELIABLE_WHOLE_FRAME_ENV).ok().as_deref(),
    )
}

pub(crate) fn reliable_whole_frame_media_override_from_env_value(
    value: Option<&str>,
) -> Option<bool> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        "" => None,
        _ => None,
    }
}
