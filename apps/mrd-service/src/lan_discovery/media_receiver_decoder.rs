use super::media_access_unit::LanAccessUnitCodec;
use super::media_profile::lan_runtime_media_profile;
use super::media_receiver::decode_lan_desktop_frame;
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

pub(super) async fn try_decode_keyframe_with_fallback(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    codec: LanAccessUnitCodec,
    failed_backend: &'static str,
    payload: &[u8],
    primary_error: &anyhow::Error,
) -> Result<(LanReceiverDecoder, Vec<DecodedFrame>)> {
    let result = decode_keyframe_with_candidates(
        codec,
        preferred_lan_receiver_decoder_candidates(codec)
            .into_iter()
            .filter(|backend| *backend != failed_backend),
        payload,
        create_lan_video_decoder,
    )
    .map_err(|error| anyhow::anyhow!("{failed_backend}: {primary_error:#} | {error:#}"))?;
    app_state
        .media_pipelines
        .lock()
        .await
        .set_active_decoder(session_id.clone(), result.0.backend);
    tracing::warn!(
        session_id = %session_id.0,
        failed_backend,
        fallback_backend = result.0.backend,
        primary_error = %primary_error,
        "LAN media receiver switched decoder after keyframe decode failure"
    );
    Ok(result)
}

fn decode_keyframe_with_candidates(
    codec: LanAccessUnitCodec,
    candidates: impl IntoIterator<Item = &'static str>,
    payload: &[u8],
    mut create: impl FnMut(&'static str) -> Result<Box<dyn VideoDecoder>>,
) -> Result<(LanReceiverDecoder, Vec<DecodedFrame>)> {
    let mut errors = Vec::new();
    for backend in candidates {
        let mut decoder = match create(backend) {
            Ok(decoder) => decoder,
            Err(error) => {
                errors.push(format!("{backend}: create failed: {error}"));
                continue;
            }
        };
        match decode_lan_desktop_frame(codec, decoder.as_mut(), payload) {
            Ok(frames) => {
                return Ok((
                    LanReceiverDecoder {
                        codec,
                        backend,
                        decoder,
                    },
                    frames,
                ));
            }
            Err(error) => errors.push(format!("{backend}: {error:#}")),
        }
    }
    anyhow::bail!(
        "all LAN {} receiver decoders failed for keyframe: {}",
        codec.display_name(),
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

#[cfg(test)]
mod fallback_tests {
    use super::*;
    use mrd_pipeline_core::PipelineError;

    struct FallbackDecoder {
        frames: Vec<DecodedFrame>,
    }
    impl VideoDecoder for FallbackDecoder {
        fn push_access_unit(&mut self, payload: &[u8]) -> std::result::Result<(), PipelineError> {
            assert_eq!(payload, b"keyframe");
            self.frames
                .push(DecodedFrame::from_cpu_rgb24(1, 1, 1, vec![1, 2, 3]));
            Ok(())
        }
        fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
            std::mem::take(&mut self.frames)
        }
    }

    #[test]
    fn keyframe_fallback_preserves_h264_hevc_and_av1_codec() {
        for codec in [
            LanAccessUnitCodec::H264,
            LanAccessUnitCodec::Hevc,
            LanAccessUnitCodec::Av1,
        ] {
            let mut attempts = Vec::new();
            let (selected, frames) =
                decode_keyframe_with_candidates(codec, ["shared", "cpu"], b"keyframe", |backend| {
                    attempts.push(backend);
                    if backend == "shared" {
                        anyhow::bail!("interop unavailable")
                    }
                    Ok(Box::new(FallbackDecoder { frames: Vec::new() }))
                })
                .unwrap();
            assert_eq!(attempts, ["shared", "cpu"]);
            assert_eq!(selected.codec, codec);
            assert_eq!(selected.backend, "cpu");
            assert_eq!(frames.len(), 1);
        }
    }

    #[test]
    fn keyframe_fallback_keeps_decoder_that_buffers_its_first_frame() {
        struct BufferedDecoder {
            accepted: usize,
        }
        impl VideoDecoder for BufferedDecoder {
            fn push_access_unit(&mut self, _: &[u8]) -> std::result::Result<(), PipelineError> {
                self.accepted += 1;
                Ok(())
            }
            fn drain_decoded_frames(&mut self) -> Vec<DecodedFrame> {
                if self.accepted >= 2 {
                    vec![DecodedFrame::from_cpu_rgb24(1, 1, 1, vec![1, 2, 3])]
                } else {
                    Vec::new()
                }
            }
        }

        for codec in [
            LanAccessUnitCodec::H264,
            LanAccessUnitCodec::Hevc,
            LanAccessUnitCodec::Av1,
        ] {
            let (mut selected, frames) =
                decode_keyframe_with_candidates(codec, ["buffered"], b"keyframe", |_| {
                    Ok(Box::new(BufferedDecoder { accepted: 0 }))
                })
                .expect("accepting an access unit does not require immediate output");
            assert!(frames.is_empty());
            assert_eq!(selected.codec, codec);
            let next = decode_lan_desktop_frame(codec, selected.decoder.as_mut(), b"next").unwrap();
            assert_eq!(next.len(), 1);
        }
    }
}
