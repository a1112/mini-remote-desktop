use anyhow::{Context, Result};
use mrd_encode_openh264::OpenH264Encoder;
use mrd_ipc::MediaProfile;
use mrd_pipeline_core::ColorMode;
use mrd_pipeline_core::VideoEncoder;

use super::media_access_unit::LanAccessUnitCodec;
use super::media_profile::{
    default_media_profile, lan_color_mode_for_profile, lan_profile_requests_hevc_main10,
    missing_profile_receiver_media_capabilities,
};

pub(super) struct LanSenderEncoder {
    pub(super) codec: LanAccessUnitCodec,
    pub(super) backend: &'static str,
    pub(super) encoder: Box<dyn VideoEncoder + Send>,
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
}
