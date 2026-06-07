use mrd_ipc::MediaProfile;

const HIGH_FPS_METADATA_MIN_FPS: u32 = 120;
const FNV1A64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV1A64_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Full,
    Metadata,
    Disabled,
}

pub(super) fn mode_from_env_value(value: Option<&str>) -> Option<Mode> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "full" | "fnv" | "fnv1a64" => Some(Mode::Full),
        "metadata" | "meta" | "cheap" => Some(Mode::Metadata),
        "disabled" | "disable" | "off" | "none" | "0" | "false" => Some(Mode::Disabled),
        "" => None,
        _ => None,
    }
}

pub(super) fn mode_for_profile_with_override(
    profile: &MediaProfile,
    override_mode: Option<Mode>,
) -> Mode {
    if let Some(mode) = override_mode {
        return mode;
    }
    if profile.fps >= HIGH_FPS_METADATA_MIN_FPS {
        return Mode::Metadata;
    }
    Mode::Full
}

pub(super) fn for_profile(
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload: &[u8],
    env_override: Option<&str>,
) -> String {
    for_mode(
        mode_for_profile_with_override(profile, mode_from_env_value(env_override)),
        profile,
        sequence,
        timestamp_us,
        encoded_payload,
    )
}

pub(super) fn for_mode(
    mode: Mode,
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload: &[u8],
) -> String {
    match mode {
        Mode::Full => {
            format!("fnv1a64:{:016x}", fnv1a64(encoded_payload))
        }
        Mode::Metadata => format!(
            "fnv1a64:meta:{:016x}",
            metadata_hash(profile, sequence, timestamp_us, encoded_payload.len())
        ),
        Mode::Disabled => "fnv1a64:disabled".to_string(),
    }
}

pub(super) fn fnv1a64(bytes: &[u8]) -> u64 {
    extend(FNV1A64_OFFSET_BASIS, bytes)
}

pub(super) fn metadata_hash(
    profile: &MediaProfile,
    sequence: u64,
    timestamp_us: u64,
    encoded_payload_len: usize,
) -> u64 {
    let mut hash = FNV1A64_OFFSET_BASIS;
    hash = extend(hash, &profile.width.to_le_bytes());
    hash = extend(hash, &profile.height.to_le_bytes());
    hash = extend(hash, &profile.fps.to_le_bytes());
    hash = extend(hash, &profile.bitrate_mbps.to_le_bytes());
    hash = extend(hash, profile.codec.as_bytes());
    hash = extend(
        hash,
        profile.color_mode.as_deref().unwrap_or("full").as_bytes(),
    );
    hash = extend(
        hash,
        profile
            .color_pipeline
            .as_deref()
            .unwrap_or("sdr8")
            .as_bytes(),
    );
    hash = extend(hash, &sequence.to_le_bytes());
    hash = extend(hash, &timestamp_us.to_le_bytes());
    hash = extend(hash, &(encoded_payload_len as u64).to_le_bytes());
    hash
}

fn extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
    }
    hash
}
