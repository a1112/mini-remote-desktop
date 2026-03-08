use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
use rtp::{
    packet::Packet,
    packetizer::{new_packetizer, Packetizer},
    sequence::new_random_sequencer,
};
use std::sync::Arc;
use thiserror::Error;
use webrtc::{
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{track_local_static_rtp::TrackLocalStaticRTP, TrackLocalWriter},
};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("{0}")]
    Message(String),
}

pub struct H264RtpSender {
    track: Arc<TrackLocalStaticRTP>,
    packetizer: Box<dyn Packetizer + Send + Sync>,
    frame_samples: u32,
}

impl H264RtpSender {
    pub fn new(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        mtu: u16,
    ) -> Self {
        let payloader = Box::<rtp::codecs::h264::H264Payloader>::default();
        let packetizer = Box::new(new_packetizer(
            mtu.max(576) as usize,
            102,
            0,
            payloader,
            Box::new(new_random_sequencer()),
            90_000,
        ));
        Self {
            track: Arc::new(TrackLocalStaticRTP::new(
                h264_codec_capability(),
                track_id.into(),
                stream_id.into(),
            )),
            packetizer,
            frame_samples: (90_000 / fps.max(1)).max(1),
        }
    }

    pub fn track(&self) -> Arc<TrackLocalStaticRTP> {
        self.track.clone()
    }

    pub fn packetize_access_unit(
        &mut self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<Vec<Packet>, TransportError> {
        if access_unit.codec != VideoCodec::H264 {
            return Err(TransportError::Message(
                "H264 RTP sender only supports H264 access units".into(),
            ));
        }

        self.packetizer
            .packetize(
                &bytes::Bytes::copy_from_slice(access_unit.bytes.as_slice()),
                self.frame_samples,
            )
            .map_err(|error| TransportError::Message(format!("packetize failed: {error}")))
    }

    pub async fn send_access_unit(
        &mut self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<usize, TransportError> {
        let packets = self.packetize_access_unit(access_unit)?;
        let mut written = 0usize;
        for packet in packets {
            written += self
                .track
                .write_rtp(&packet)
                .await
                .map_err(|error| TransportError::Message(format!("write_rtp failed: {error}")))?;
        }
        Ok(written)
    }
}

pub fn h264_codec_capability() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability {
        mime_type: "video/H264".to_string(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
            .to_string(),
        rtcp_feedback: vec![],
    }
}
