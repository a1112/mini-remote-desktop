use anyhow::{Context, Result};
use mrd_agent_ipc::{MediaAccessUnit, MediaCodec};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_ipc::MediaProfile;
use mrd_pipeline_core::ColorMode;
use mrd_pipeline_core::VideoEncoder;

use super::media_access_unit::LanAccessUnitCodec;
use super::media_profile::{
    default_media_profile, lan_color_mode_for_profile, lan_profile_requests_hevc_main10,
    missing_profile_receiver_media_capabilities,
};
use crate::agent_runtime::AgentMediaIngress;

pub(super) struct LanSenderEncoder {
    pub(super) codec: LanAccessUnitCodec,
    pub(super) backend: &'static str,
    pub(super) encoder: Box<dyn VideoEncoder + Send>,
}

/// Validated encoded payload received from the session agent boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentEncodedAccessUnit {
    pub(crate) resource_id: [u8; 16],
    pub(crate) session_id: String,
    pub(crate) sequence: u64,
    pub(crate) timestamp_us: u64,
    pub(crate) codec: LanAccessUnitCodec,
    pub(crate) is_keyframe: bool,
    pub(crate) bytes: Vec<u8>,
}

/// Selects the active media source while the migration keeps a local fallback.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaSourceSelection {
    Agent,
    LocalCapture,
}

/// Agent media takes precedence whenever a bounded batch is available.
#[allow(dead_code)]
pub(crate) fn select_media_source(agent_batch_len: usize) -> MediaSourceSelection {
    if agent_batch_len > 0 {
        MediaSourceSelection::Agent
    } else {
        MediaSourceSelection::LocalCapture
    }
}

/// Converts one authenticated agent message into the sender's transport form.
pub(crate) fn validate_agent_access_unit(unit: MediaAccessUnit) -> Option<AgentEncodedAccessUnit> {
    if !unit.is_valid() {
        return None;
    }
    let codec = match unit.codec {
        MediaCodec::H264 => LanAccessUnitCodec::H264,
        MediaCodec::Hevc => LanAccessUnitCodec::Hevc,
        MediaCodec::Av1 => LanAccessUnitCodec::Av1,
    };
    Some(AgentEncodedAccessUnit {
        resource_id: unit.resource_id,
        session_id: unit.session_id,
        sequence: unit.sequence,
        timestamp_us: unit.timestamp_us,
        codec,
        is_keyframe: unit.is_keyframe,
        bytes: unit.payload,
    })
}

/// Takes one bounded batch from the agent ingress and maps it for transport.
#[allow(dead_code)]
pub(crate) fn drain_agent_access_units(
    ingress: &mut AgentMediaIngress,
    limit: usize,
) -> Vec<AgentEncodedAccessUnit> {
    ingress
        .drain(limit)
        .into_iter()
        .filter_map(validate_agent_access_unit)
        .collect()
}

/// Takes a bounded batch belonging to one session for its sender loop.
pub(crate) fn drain_agent_access_units_for_session(
    ingress: &mut AgentMediaIngress,
    session_id: &str,
    limit: usize,
) -> Vec<AgentEncodedAccessUnit> {
    ingress
        .drain_session(session_id, limit)
        .into_iter()
        .filter_map(validate_agent_access_unit)
        .collect()
}

pub(super) fn create_lan_encoder(
    requested_codec: LanAccessUnitCodec,
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
    allow_h264_fallback: bool,
) -> Result<LanSenderEncoder> {
    match requested_codec {
        LanAccessUnitCodec::Hevc => {
            match create_lan_hevc_encoder(width, height, fps, bitrate, profile) {
                Ok((backend, encoder)) => Ok(LanSenderEncoder {
                    codec: LanAccessUnitCodec::Hevc,
                    backend,
                    encoder,
                }),
                Err(hevc_error) => {
                    if !allow_h264_fallback {
                        anyhow::bail!(
                            "HEVC unavailable ({hevc_error}); H.264 fallback blocked because the peer does not advertise H.264 receiver capability"
                        );
                    }
                    let (backend, encoder) =
                        create_lan_h264_encoder(width, height, fps, bitrate, profile)
                            .with_context(|| {
                                format!(
                                    "HEVC unavailable ({hevc_error}); H.264 fallback also failed"
                                )
                            })?;
                    Ok(LanSenderEncoder {
                        codec: LanAccessUnitCodec::H264,
                        backend,
                        encoder,
                    })
                }
            }
        }
        LanAccessUnitCodec::H264 => {
            let (backend, encoder) = create_lan_h264_encoder(width, height, fps, bitrate, profile)?;
            Ok(LanSenderEncoder {
                codec: LanAccessUnitCodec::H264,
                backend,
                encoder,
            })
        }
        LanAccessUnitCodec::Av1 => {
            let (backend, encoder) = create_lan_av1_encoder(width, height, fps, bitrate, profile)?;
            Ok(LanSenderEncoder {
                codec: LanAccessUnitCodec::Av1,
                backend,
                encoder,
            })
        }
    }
}

pub(super) fn lan_sender_allows_h264_encoder_fallback(
    requested_codec: LanAccessUnitCodec,
    peer_media_capabilities: &[String],
) -> bool {
    requested_codec == LanAccessUnitCodec::Hevc
        && peer_can_receive_codec(peer_media_capabilities, LanAccessUnitCodec::H264)
}

fn peer_can_receive_codec(peer_media_capabilities: &[String], codec: LanAccessUnitCodec) -> bool {
    let mut profile = default_media_profile();
    profile.codec = codec.name().to_string();
    missing_profile_receiver_media_capabilities(&profile, peer_media_capabilities).is_empty()
}

#[cfg(windows)]
fn create_lan_hevc_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let color_mode = lan_color_mode_for_profile(profile)?;
    if lan_profile_requests_hevc_main10(profile) {
        return mrd_encode_nvenc::NvencHevcEncoder::new_main10_with_bitrate(
            width, height, fps, bitrate,
        )
        .map(|encoder| {
            (
                "nvenc_hevc_main10",
                Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>,
            )
        })
        .map_err(|error| anyhow::anyhow!(error.to_string()));
    }
    match mrd_encode_nvenc::NvencHevcEncoder::new_max_speed_with_bitrate(
        width, height, fps, bitrate,
    ) {
        Ok(encoder) => Ok((
            "nvenc_hevc_p1_ultra_low_latency",
            Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>,
        )),
        Err(max_speed_error) => {
            mrd_encode_nvenc::NvencHevcEncoder::new_main_with_bitrate(width, height, fps, bitrate)
                .map(|encoder| {
                    (
                        "nvenc_hevc",
                        Box::new(encoder.with_color_mode(color_mode))
                            as Box<dyn VideoEncoder + Send>,
                    )
                })
                .map_err(|error| {
                    anyhow::anyhow!(
                        "nvenc_hevc_p1_ultra_low_latency: {max_speed_error}; nvenc_hevc: {error}"
                    )
                })
        }
    }
}

#[cfg(target_os = "macos")]
fn create_lan_hevc_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    if lan_profile_requests_hevc_main10(profile) {
        anyhow::bail!("VideoToolbox HEVC Main10 LAN encoding is unavailable");
    }
    let color_mode = lan_color_mode_for_profile(profile)?;
    if color_mode != ColorMode::Full {
        anyhow::bail!(
            "VideoToolbox HEVC LAN encoding does not support color_mode={}",
            color_mode.as_str()
        );
    }
    mrd_codec_videotoolbox::VideoToolboxHevcEncoder::new_with_bitrate(width, height, fps, bitrate)
        .map(|encoder| {
            (
                "videotoolbox_hevc",
                Box::new(encoder) as Box<dyn VideoEncoder + Send>,
            )
        })
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn create_lan_hevc_encoder(
    _width: usize,
    _height: usize,
    _fps: u32,
    _bitrate: u32,
    _profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    anyhow::bail!("NVENC HEVC is unavailable on this platform")
}

fn create_lan_h264_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let color_mode = lan_color_mode_for_profile(profile)?;
    let mut last_error = None;
    for backend in preferred_lan_h264_encoder_backends() {
        let encoder: Result<Box<dyn VideoEncoder + Send>> = match *backend {
            #[cfg(windows)]
            "nvenc_h264" => mrd_encode_nvenc::NvencH264Encoder::new_max_speed_with_bitrate(
                width, height, fps, bitrate,
            )
            .map(|encoder| {
                Box::new(encoder.with_color_mode(color_mode)) as Box<dyn VideoEncoder + Send>
            })
            .map_err(|error| anyhow::anyhow!(error.to_string())),
            #[cfg(target_os = "macos")]
            "videotoolbox_h264" => {
                if color_mode != ColorMode::Full {
                    Err(anyhow::anyhow!(
                        "VideoToolbox H.264 LAN encoding does not support color_mode={}",
                        color_mode.as_str()
                    ))
                } else {
                    mrd_codec_videotoolbox::VideoToolboxH264Encoder::new_with_bitrate(
                        width, height, fps, bitrate,
                    )
                    .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                }
            }
            "openh264" => {
                if color_mode != ColorMode::Full {
                    Err(anyhow::anyhow!(
                        "OpenH264 LAN encoding does not support color_mode={}",
                        color_mode.as_str()
                    ))
                } else {
                    OpenH264Encoder::new_with_bitrate(width, height, fps, bitrate)
                        .map(|encoder| Box::new(encoder) as Box<dyn VideoEncoder + Send>)
                        .map_err(|error| anyhow::anyhow!(error.to_string()))
                }
            }
            _ => Err(anyhow::anyhow!(
                "unknown LAN H.264 encoder backend: {backend}"
            )),
        };
        match encoder {
            Ok(encoder) => return Ok((backend, encoder)),
            Err(error) => last_error = Some(format!("{backend}: {error}")),
        }
    }

    anyhow::bail!(
        "no LAN H.264 encoder available{}",
        last_error
            .map(|error| format!("; last error: {error}"))
            .unwrap_or_default()
    )
}

#[cfg(windows)]
fn create_lan_av1_encoder(
    width: usize,
    height: usize,
    fps: u32,
    bitrate: u32,
    profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    let color_mode = lan_color_mode_for_profile(profile)?;
    if color_mode != ColorMode::Full {
        anyhow::bail!(
            "NVENC AV1 LAN encoding does not support color_mode={}",
            color_mode.as_str()
        );
    }
    mrd_encode_nvenc_av1::NvencAv1Encoder::new_high_refresh_rate_with_bitrate(
        width, height, fps, bitrate,
    )
    .map(|encoder| {
        (
            "nvenc_av1_high_refresh",
            Box::new(encoder) as Box<dyn VideoEncoder + Send>,
        )
    })
    .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[cfg(not(windows))]
fn create_lan_av1_encoder(
    _width: usize,
    _height: usize,
    _fps: u32,
    _bitrate: u32,
    _profile: &MediaProfile,
) -> Result<(&'static str, Box<dyn VideoEncoder + Send>)> {
    anyhow::bail!("NVENC AV1 LAN encoding is unavailable on this platform")
}

#[cfg(windows)]
pub(super) fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["nvenc_h264", "openh264"]
}

#[cfg(all(not(windows), not(target_os = "macos")))]
pub(super) fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["openh264"]
}

#[cfg(target_os = "macos")]
pub(super) fn preferred_lan_h264_encoder_backends() -> &'static [&'static str] {
    &["videotoolbox_h264", "openh264"]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_agent_ipc::AgentEventContext;

    #[test]
    fn hevc_sender_h264_fallback_requires_peer_h264_receiver_capability() {
        assert!(!lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.videotoolbox_hevc".to_string()],
        ));
        assert!(lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.videotoolbox_h264".to_string()],
        ));
        assert!(lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::Hevc,
            &["decode.nvdec".to_string()],
        ));
        assert!(!lan_sender_allows_h264_encoder_fallback(
            LanAccessUnitCodec::H264,
            &["decode.videotoolbox_h264".to_string()],
        ));
    }

    #[test]
    fn agent_access_unit_is_mapped_without_copying_raw_desktop_pixels() {
        let mapped = validate_agent_access_unit(MediaAccessUnit {
            context: AgentEventContext {
                registration_id: [1; 16],
                registration_epoch: 1,
                windows_session_id: 3,
                desktop_epoch: 2,
                sequence: 4,
                observed_at_ms: 5,
            },
            resource_id: [9; 16],
            session_id: "session-1".to_string(),
            sequence: 7,
            timestamp_us: 8,
            codec: MediaCodec::H264,
            is_keyframe: true,
            payload: vec![1, 2, 3],
        })
        .expect("valid agent unit");
        assert_eq!(mapped.codec, LanAccessUnitCodec::H264);
        assert_eq!(mapped.resource_id, [9; 16]);
        assert_eq!(mapped.bytes, vec![1, 2, 3]);
    }

    #[test]
    fn sender_drains_only_one_bounded_agent_batch() {
        let mut ingress = AgentMediaIngress::new(4).unwrap();
        let make = |sequence| MediaAccessUnit {
            context: AgentEventContext {
                registration_id: [1; 16],
                registration_epoch: 1,
                windows_session_id: 1,
                desktop_epoch: 1,
                sequence,
                observed_at_ms: sequence,
            },
            resource_id: [9; 16],
            session_id: "session-1".to_string(),
            sequence,
            timestamp_us: sequence,
            codec: MediaCodec::H264,
            is_keyframe: sequence == 1,
            payload: vec![1],
        };
        assert!(ingress.push(make(1)));
        assert!(ingress.push(make(2)));
        let batch = drain_agent_access_units(&mut ingress, 1);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].sequence, 1);
        assert_eq!(ingress.len(), 1);
    }

    #[test]
    fn agent_media_source_precedes_local_capture_when_available() {
        assert_eq!(select_media_source(1), MediaSourceSelection::Agent);
        assert_eq!(select_media_source(0), MediaSourceSelection::LocalCapture);
    }
}
