use super::media_access_unit::LanAccessUnitCodec;
use super::media_profile::lan_runtime_media_profile;
use super::media_receiver::decode_h264_desktop_frame;
use super::media_receiver_decoder_candidates::{
    lan_receiver_decoder_candidates, preferred_lan_receiver_decoder_candidates,
};
use super::selected_media_profile;
use crate::app_state::AppState;
use anyhow::Result;
use mrd_pipeline_core::{DecodedFrame, VideoDecoder};
use mrd_proto::SessionId;
use std::sync::Arc;

pub(super) struct LanReceiverDecoder {
    pub(super) codec: LanAccessUnitCodec,
    pub(super) backend: &'static str,
    pub(super) decoder: Box<dyn VideoDecoder>,
}

pub(super) async fn create_lan_receiver_decoder(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
) -> Result<LanReceiverDecoder> {
    let profile = selected_media_profile(app_state, session_id).await;
    let requested_codec = LanAccessUnitCodec::from_profile(&profile);
    match create_lan_receiver_decoder_with_preference(app_state, session_id, requested_codec, None)
        .await
    {
        Ok(decoder) => Ok(decoder),
        Err(error) if requested_codec == LanAccessUnitCodec::Hevc => {
            app_state
                .media_pipelines
                .lock()
                .await
                .set_codec_fallback_reason(
                    session_id.clone(),
                    Some(format!(
                        "{} receiver unavailable; fell back to H.264: {error:#}",
                        requested_codec.display_name()
                    )),
                );
            create_lan_receiver_decoder_with_preference(
                app_state,
                session_id,
                LanAccessUnitCodec::H264,
                None,
            )
            .await
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn create_lan_receiver_decoder_with_preference(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    codec: LanAccessUnitCodec,
    preferred_backend: Option<&'static str>,
) -> Result<LanReceiverDecoder> {
    let mut last_error = None;
    let selected_profile = selected_media_profile(app_state, session_id).await;
    for backend in lan_receiver_decoder_candidates(codec, preferred_backend) {
        match create_lan_video_decoder(backend) {
            Ok(decoder) => {
                let mut pipelines = app_state.media_pipelines.lock().await;
                pipelines.set_active_decoder(session_id.clone(), backend);
                let runtime_profile = lan_runtime_media_profile(&selected_profile, codec);
                pipelines.set_active_media_profile(session_id.clone(), &runtime_profile);
                return Ok(LanReceiverDecoder {
                    codec,
                    backend,
                    decoder,
                });
            }
            Err(error) => {
                last_error = Some(format!("{backend}: {error}"));
            }
        }
    }

    anyhow::bail!(
        "no LAN {} receiver decoder available{}",
        codec.display_name(),
        last_error
            .map(|error| format!("; last error: {error}"))
            .unwrap_or_default()
    )
}

pub(super) async fn try_decode_h264_keyframe_with_fallback(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    failed_backend: &'static str,
    payload: &[u8],
    primary_error: &anyhow::Error,
) -> Result<(LanReceiverDecoder, Vec<DecodedFrame>)> {
    let mut errors = vec![format!("{failed_backend}: {primary_error:#}")];
    for backend in preferred_lan_receiver_decoder_candidates(LanAccessUnitCodec::H264)
        .into_iter()
        .filter(|backend| *backend != failed_backend)
    {
        let mut decoder = match create_lan_video_decoder(backend) {
            Ok(decoder) => decoder,
            Err(error) => {
                errors.push(format!("{backend}: create failed: {error}"));
                continue;
            }
        };
        match decode_h264_desktop_frame(decoder.as_mut(), payload) {
            Ok(decoded_frames) if !decoded_frames.is_empty() => {
                app_state
                    .media_pipelines
                    .lock()
                    .await
                    .set_active_decoder(session_id.clone(), backend);
                tracing::warn!(
                    session_id = %session_id.0,
                    failed_backend,
                    fallback_backend = backend,
                    primary_error = %primary_error,
                    "LAN media receiver switched decoder after keyframe decode failure"
                );
                return Ok((
                    LanReceiverDecoder {
                        codec: LanAccessUnitCodec::H264,
                        backend,
                        decoder,
                    },
                    decoded_frames,
                ));
            }
            Ok(_) => errors.push(format!("{backend}: decoded no frames")),
            Err(error) => errors.push(format!("{backend}: {error:#}")),
        }
    }

    anyhow::bail!(
        "all LAN H.264 receiver decoders failed for keyframe: {}",
        errors.join(" | ")
    )
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub(super) fn create_lan_video_decoder(backend: &str) -> Result<Box<dyn VideoDecoder>> {
    #[cfg(target_os = "macos")]
    if backend == "videotoolbox" {
        return mrd_codec_videotoolbox::VideoToolboxH264Decoder::new()
            .map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>)
            .map_err(|error| anyhow::anyhow!(error.to_string()));
    }
    #[cfg(target_os = "macos")]
    if backend == "videotoolbox_hevc" {
        return mrd_codec_videotoolbox::VideoToolboxHevcDecoder::new()
            .map(|decoder| Box::new(decoder) as Box<dyn VideoDecoder>)
            .map_err(|error| anyhow::anyhow!(error.to_string()));
    }

    mrd_decode::create_decoder(backend).map_err(|error| anyhow::anyhow!(error.to_string()))
}
