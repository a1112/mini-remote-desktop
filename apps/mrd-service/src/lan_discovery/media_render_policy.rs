use mrd_ipc::MediaProfile;

use super::{
    env_bool_override, fnv1a64, fnv1a64_media_metadata, LAN_MEDIA_PAYLOAD_HASH_ENV,
    LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES, LAN_RENDER_PACING_DEFAULT_MIN_FPS,
    LAN_RENDER_PACING_ENV, LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT,
    LAN_RENDER_QUEUE_CAPACITY_ENV, LAN_RENDER_QUEUE_POLICY_ENV,
};

#[cfg(any(windows, target_os = "macos"))]
use super::lan_local_render_refresh_hz;
#[cfg(windows)]
use super::{D3D11_RENDER_PRESENT_BLOCKING_ENV, D3D11_RENDER_WAITABLE_OBJECT_ENV};

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_pacing_enabled_for_profile(profile: &MediaProfile) -> bool {
    if let Some(enabled) =
        lan_render_pacing_from_env_value(std::env::var(LAN_RENDER_PACING_ENV).ok().as_deref())
    {
        return enabled;
    }

    profile.fps >= LAN_RENDER_PACING_DEFAULT_MIN_FPS
}

#[cfg(any(windows, target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanRenderQueuePolicy {
    PacedFifo,
    Latest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanMediaPayloadHashMode {
    Full,
    Metadata,
    Disabled,
}

#[cfg(any(windows, target_os = "macos"))]
impl LanRenderQueuePolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PacedFifo => "paced_fifo",
            Self::Latest => "latest",
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_policy_from_env_value(
    value: Option<&str>,
) -> Option<LanRenderQueuePolicy> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "latest" | "low_latency" | "low-latency" | "latency" => Some(LanRenderQueuePolicy::Latest),
        "paced_fifo" | "paced-fifo" | "fifo" => Some(LanRenderQueuePolicy::PacedFifo),
        "" => None,
        _ => None,
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_policy_for_profile(profile: &MediaProfile) -> LanRenderQueuePolicy {
    lan_render_queue_policy_for_profile_with_override(
        profile,
        lan_render_queue_policy_from_env_value(
            std::env::var(LAN_RENDER_QUEUE_POLICY_ENV).ok().as_deref(),
        ),
    )
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_policy_for_profile_with_override(
    _profile: &MediaProfile,
    override_policy: Option<LanRenderQueuePolicy>,
) -> LanRenderQueuePolicy {
    if let Some(policy) = override_policy {
        return policy;
    }
    #[cfg(target_os = "macos")]
    if _profile.fps >= LAN_RENDER_PACING_DEFAULT_MIN_FPS {
        return LanRenderQueuePolicy::Latest;
    }
    LanRenderQueuePolicy::PacedFifo
}

pub(crate) fn lan_media_payload_hash_mode_from_env_value(
    value: Option<&str>,
) -> Option<LanMediaPayloadHashMode> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "full" | "fnv" | "fnv1a64" => Some(LanMediaPayloadHashMode::Full),
        "metadata" | "meta" | "cheap" => Some(LanMediaPayloadHashMode::Metadata),
        "disabled" | "disable" | "off" | "none" | "0" | "false" => {
            Some(LanMediaPayloadHashMode::Disabled)
        }
        "" => None,
        _ => None,
    }
}

pub(crate) fn lan_media_payload_hash_mode_for_profile(
    profile: &MediaProfile,
) -> LanMediaPayloadHashMode {
    lan_media_payload_hash_mode_for_profile_with_override(
        profile,
        lan_media_payload_hash_mode_from_env_value(
            std::env::var(LAN_MEDIA_PAYLOAD_HASH_ENV).ok().as_deref(),
        ),
    )
}

pub(crate) fn lan_media_payload_hash_mode_for_profile_with_override(
    profile: &MediaProfile,
    override_mode: Option<LanMediaPayloadHashMode>,
) -> LanMediaPayloadHashMode {
    if let Some(mode) = override_mode {
        return mode;
    }
    if profile.fps >= LAN_RENDER_PACING_DEFAULT_MIN_FPS {
        return LanMediaPayloadHashMode::Metadata;
    }
    LanMediaPayloadHashMode::Full
}

pub(crate) fn lan_media_payload_hash_for_profile(
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload: &[u8],
) -> String {
    lan_media_payload_hash_for_mode(
        lan_media_payload_hash_mode_for_profile(profile),
        profile,
        sequence,
        timestamp_us,
        encoded_payload,
    )
}

pub(crate) fn lan_media_payload_hash_for_mode(
    mode: LanMediaPayloadHashMode,
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload: &[u8],
) -> String {
    match mode {
        LanMediaPayloadHashMode::Full => {
            format!("fnv1a64:{:016x}", fnv1a64(encoded_payload))
        }
        LanMediaPayloadHashMode::Metadata => format!(
            "fnv1a64:meta:{:016x}",
            fnv1a64_media_metadata(profile, sequence, timestamp_us, encoded_payload.len())
        ),
        LanMediaPayloadHashMode::Disabled => "fnv1a64:disabled".to_string(),
    }
}

#[cfg(windows)]
pub(crate) fn lan_render_waitable_swapchain_pacing_enabled() -> bool {
    env_bool_override(
        std::env::var(D3D11_RENDER_PRESENT_BLOCKING_ENV)
            .ok()
            .as_deref(),
    ) != Some(true)
        && env_bool_override(
            std::env::var(D3D11_RENDER_WAITABLE_OBJECT_ENV)
                .ok()
                .as_deref(),
        ) == Some(true)
}

#[cfg(windows)]
pub(crate) fn native_render_waitable_swapchain_pacing_enabled() -> bool {
    lan_render_waitable_swapchain_pacing_enabled()
}

#[cfg(target_os = "macos")]
pub(crate) fn native_render_waitable_swapchain_pacing_enabled() -> bool {
    false
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_policy_allows_service_pacing(
    policy: LanRenderQueuePolicy,
    profile: &MediaProfile,
    waitable_swapchain_pacing: bool,
) -> bool {
    policy == LanRenderQueuePolicy::PacedFifo
        && !waitable_swapchain_pacing
        && lan_render_pacing_enabled_for_profile(profile)
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_capacity_for_profile(profile: &MediaProfile) -> usize {
    if lan_render_pacing_enabled_for_profile(profile) {
        lan_render_queue_capacity_from_env_value(
            std::env::var(LAN_RENDER_QUEUE_CAPACITY_ENV).ok().as_deref(),
        )
    } else {
        1
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_capacity_for_policy(
    profile: &MediaProfile,
    policy: LanRenderQueuePolicy,
) -> usize {
    match policy {
        LanRenderQueuePolicy::Latest => 1,
        LanRenderQueuePolicy::PacedFifo => lan_render_queue_capacity_for_profile(profile),
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_queue_capacity_from_env_value(value: Option<&str>) -> usize {
    value
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(LAN_RENDER_PACING_MAX_PENDING_FRAMES_LIMIT))
        .unwrap_or(LAN_RENDER_PACING_DEFAULT_MAX_PENDING_FRAMES)
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_pacing_target_fps(profile: &MediaProfile) -> u32 {
    lan_render_pacing_target_fps_from_values(profile.fps, lan_local_render_refresh_hz())
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_cap_target_fps_for_profile(profile: &MediaProfile) -> Option<u32> {
    if profile.fps >= LAN_RENDER_PACING_DEFAULT_MIN_FPS {
        Some(lan_render_pacing_target_fps(profile))
    } else {
        None
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn lan_render_pacing_target_fps_from_values(
    profile_fps: u32,
    local_refresh_hz: Option<u32>,
) -> u32 {
    let profile_fps = profile_fps.max(1);
    match local_refresh_hz.filter(|refresh_hz| *refresh_hz > 0) {
        Some(refresh_hz) => profile_fps.min(refresh_hz),
        None => profile_fps,
    }
}

pub(crate) fn lan_render_pacing_from_env_value(value: Option<&str>) -> Option<bool> {
    env_bool_override(value)
}
