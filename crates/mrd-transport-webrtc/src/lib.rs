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

#[derive(Debug, Default, Clone)]
pub struct H264AccessUnitAssembler {
    annex_b_buffer: Vec<u8>,
    fua_active: bool,
}

impl H264AccessUnitAssembler {
    pub fn push_rtp_payload(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.is_empty() {
            return None;
        }

        let nal_type = payload[0] & 0x1f;
        match nal_type {
            1..=23 => {
                self.append_nal(payload);
                if marker {
                    self.take_access_unit()
                } else {
                    None
                }
            }
            24 => self.push_stap_a(payload, marker),
            28 => self.push_fua(payload, marker),
            _ => {
                if marker {
                    self.reset();
                }
                None
            }
        }
    }

    fn push_fua(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.len() < 2 {
            self.reset();
            return None;
        }

        let fu_indicator = payload[0];
        let fu_header = payload[1];
        let start = fu_header & 0x80 != 0;
        let end = fu_header & 0x40 != 0;
        let reconstructed_nal = (fu_indicator & 0xe0) | (fu_header & 0x1f);

        if start {
            if self.fua_active {
                self.reset();
            }
            self.annex_b_buffer
                .extend_from_slice(&[0, 0, 0, 1, reconstructed_nal]);
            self.annex_b_buffer.extend_from_slice(&payload[2..]);
            self.fua_active = true;
        } else if self.fua_active {
            self.annex_b_buffer.extend_from_slice(&payload[2..]);
        } else {
            self.reset();
            return None;
        }

        if end || marker {
            self.fua_active = false;
            return self.take_access_unit();
        }

        None
    }

    fn push_stap_a(&mut self, payload: &[u8], marker: bool) -> Option<Vec<u8>> {
        if payload.len() < 3 {
            self.reset();
            return None;
        }

        let mut offset = 1usize;
        while offset + 2 <= payload.len() {
            let nal_len = u16::from_be_bytes([payload[offset], payload[offset + 1]]) as usize;
            offset += 2;
            if offset + nal_len > payload.len() {
                self.reset();
                return None;
            }
            self.append_nal(&payload[offset..offset + nal_len]);
            offset += nal_len;
        }

        if marker {
            return self.take_access_unit();
        }

        None
    }

    fn append_nal(&mut self, nal: &[u8]) {
        self.annex_b_buffer.extend_from_slice(&[0, 0, 0, 1]);
        self.annex_b_buffer.extend_from_slice(nal);
    }

    fn take_access_unit(&mut self) -> Option<Vec<u8>> {
        if self.annex_b_buffer.is_empty() {
            return None;
        }
        let mut complete = Vec::new();
        std::mem::swap(&mut complete, &mut self.annex_b_buffer);
        Some(complete)
    }

    fn reset(&mut self) {
        self.annex_b_buffer.clear();
        self.fua_active = false;
    }
}

#[derive(Debug, Default, Clone)]
pub struct H264RtpIngress {
    assembler: H264AccessUnitAssembler,
}

impl H264RtpIngress {
    pub fn push_payload(
        &mut self,
        payload: &[u8],
        marker: bool,
        timestamp_us: u64,
    ) -> Option<EncodedAccessUnit> {
        self.assembler
            .push_rtp_payload(payload, marker)
            .map(|bytes| EncodedAccessUnit {
                codec: VideoCodec::H264,
                timestamp_us,
                is_keyframe: annex_b_contains_keyframe(&bytes),
                bytes,
            })
    }
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

fn annex_b_contains_keyframe(access_unit: &[u8]) -> bool {
    let mut offset = 0usize;
    while offset + 4 < access_unit.len() {
        if access_unit[offset..].starts_with(&[0, 0, 0, 1]) {
            let nal_header = access_unit[offset + 4];
            let nal_type = nal_header & 0x1f;
            if matches!(nal_type, 5 | 7 | 8) {
                return true;
            }
            offset += 4;
        } else {
            offset += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{annex_b_contains_keyframe, H264AccessUnitAssembler, H264RtpIngress};
    use mrd_pipeline_core::VideoCodec;

    #[test]
    fn single_nal_emits_annex_b_access_unit_on_marker() {
        let mut assembler = H264AccessUnitAssembler::default();

        let access_unit = assembler
            .push_rtp_payload(&[0x65, 0x88, 0x99], true)
            .expect("single nal access unit");

        assert_eq!(access_unit, vec![0, 0, 0, 1, 0x65, 0x88, 0x99]);
    }

    #[test]
    fn fua_fragments_emit_single_access_unit() {
        let mut assembler = H264AccessUnitAssembler::default();

        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x85, 0xaa, 0xbb], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x45, 0xcc, 0xdd], true),
            Some(vec![0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc, 0xdd])
        );
    }

    #[test]
    fn stap_a_then_fua_preserves_full_access_unit_until_marker() {
        let mut assembler = H264AccessUnitAssembler::default();

        assert_eq!(
            assembler.push_rtp_payload(&[24, 0, 2, 0x67, 0x42, 0, 2, 0x68, 0xce], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x85, 0xaa, 0xbb], false),
            None
        );
        assert_eq!(
            assembler.push_rtp_payload(&[0x7c, 0x45, 0xcc, 0xdd], true),
            Some(vec![
                0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1, 0x65, 0xaa,
                0xbb, 0xcc, 0xdd,
            ])
        );
    }

    #[test]
    fn ingress_wraps_annex_b_payload_as_h264_access_unit() {
        let mut ingress = H264RtpIngress::default();

        assert!(ingress.push_payload(&[0x7c, 0x85, 0xaa, 0xbb], false, 33_000).is_none());
        let access_unit = ingress
            .push_payload(&[0x7c, 0x45, 0xcc, 0xdd], true, 33_000)
            .expect("completed access unit");

        assert_eq!(access_unit.codec, VideoCodec::H264);
        assert_eq!(access_unit.timestamp_us, 33_000);
        assert!(access_unit.is_keyframe);
    }

    #[test]
    fn keyframe_detector_marks_idr_annex_b_payloads() {
        assert!(annex_b_contains_keyframe(&[0, 0, 0, 1, 0x65, 0xaa]));
    }
}
