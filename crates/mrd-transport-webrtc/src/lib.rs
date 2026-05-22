use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};
use rtp::{
    packet::Packet,
    packetizer::{new_packetizer, Packetizer},
    sequence::new_random_sequencer,
};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use webrtc::{
    media::Sample,
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{
        track_local_static_rtp::TrackLocalStaticRTP,
        track_local_static_sample::TrackLocalStaticSample, TrackLocalWriter,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264RtpSendReport {
    pub bytes_written: usize,
    pub rtp_timestamp: u32,
}

pub struct H264SampleSender {
    track: Arc<TrackLocalStaticSample>,
    frame_duration: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264SampleSendReport {
    pub bytes_written: usize,
}

pub struct Av1RtpSender {
    track: Arc<TrackLocalStaticRTP>,
    packetizer: Box<dyn Packetizer + Send + Sync>,
    frame_samples: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    Baseline,
    High,
}

#[derive(Debug, Default, Clone)]
pub struct H264AccessUnitAssembler {
    annex_b_buffer: Vec<u8>,
    fua_active: bool,
    last_sequence_number: Option<u16>,
    drop_until_marker: bool,
}

impl H264AccessUnitAssembler {
    pub fn push_rtp_packet(
        &mut self,
        payload: &[u8],
        marker: bool,
        sequence_number: u16,
    ) -> Option<Vec<u8>> {
        if let Some(previous) = self.last_sequence_number {
            let expected = previous.wrapping_add(1);
            if sequence_number != expected && self.has_incomplete_access_unit() {
                self.reset();
                self.drop_until_marker = true;
            }
        }
        self.last_sequence_number = Some(sequence_number);

        if self.drop_until_marker {
            if marker {
                self.drop_until_marker = false;
            }
            return None;
        }

        self.push_rtp_payload(payload, marker)
    }

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

    fn has_incomplete_access_unit(&self) -> bool {
        self.fua_active || !self.annex_b_buffer.is_empty()
    }
}

#[derive(Debug, Default, Clone)]
pub struct H264RtpIngress {
    assembler: H264AccessUnitAssembler,
}

#[derive(Debug, Default, Clone)]
pub struct Av1AccessUnitAssembler {
    temporal_unit: Vec<u8>,
    fragmented_obu: Vec<u8>,
    last_sequence_number: Option<u16>,
    drop_until_marker: bool,
    has_new_sequence: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Av1RtpIngress {
    assembler: Av1AccessUnitAssembler,
}

impl H264RtpIngress {
    pub fn push_packet(
        &mut self,
        payload: &[u8],
        marker: bool,
        sequence_number: u16,
        timestamp_us: u64,
    ) -> Option<EncodedAccessUnit> {
        self.assembler
            .push_rtp_packet(payload, marker, sequence_number)
            .map(|bytes| EncodedAccessUnit {
                codec: VideoCodec::H264,
                timestamp_us,
                is_keyframe: annex_b_contains_keyframe(&bytes),
                bytes,
            })
    }

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

impl Av1AccessUnitAssembler {
    pub fn push_rtp_packet(
        &mut self,
        payload: &[u8],
        marker: bool,
        sequence_number: u16,
    ) -> Option<(Vec<u8>, bool)> {
        if let Some(previous) = self.last_sequence_number {
            let expected = previous.wrapping_add(1);
            if sequence_number != expected && self.has_incomplete_temporal_unit() {
                self.reset();
                self.drop_until_marker = true;
            }
        }
        self.last_sequence_number = Some(sequence_number);

        if self.drop_until_marker {
            if marker {
                self.drop_until_marker = false;
            }
            return None;
        }

        self.push_rtp_payload(payload, marker)
    }

    pub fn push_rtp_payload(&mut self, payload: &[u8], marker: bool) -> Option<(Vec<u8>, bool)> {
        if payload.is_empty() {
            return None;
        }

        let aggregation_header = payload[0];
        let z = aggregation_header & 0x80 != 0;
        let y = aggregation_header & 0x40 != 0;
        let w = ((aggregation_header >> 4) & 0x03) as usize;
        let n = aggregation_header & 0x08 != 0;
        if n {
            self.has_new_sequence = true;
        }

        let mut offset = 1usize;
        let element_count = if w == 0 { None } else { Some(w) };
        let mut element_index = 0usize;

        while offset < payload.len() {
            let is_last_by_count = element_count
                .map(|count| element_index + 1 == count)
                .unwrap_or(false);
            let element_len = if element_count.is_some() && is_last_by_count {
                payload.len() - offset
            } else {
                match read_leb128(payload, &mut offset) {
                    Some(len) => len,
                    None => {
                        self.reset();
                        return None;
                    }
                }
            };

            if offset + element_len > payload.len() {
                self.reset();
                return None;
            }

            let element = &payload[offset..offset + element_len];
            offset += element_len;

            let is_first_element = element_index == 0;
            let is_last_element = element_count
                .map(|count| element_index + 1 == count)
                .unwrap_or(offset >= payload.len());
            let is_continuation = is_first_element && z;
            let continues = is_last_element && y;

            if is_continuation {
                if self.fragmented_obu.is_empty() {
                    self.reset();
                    return None;
                }
                self.fragmented_obu.extend_from_slice(element);
                if !continues {
                    let obu = std::mem::take(&mut self.fragmented_obu);
                    if !self.append_complete_obu(&obu) {
                        self.reset();
                        return None;
                    }
                }
            } else if continues {
                if !self.fragmented_obu.is_empty() {
                    self.reset();
                    return None;
                }
                self.fragmented_obu.extend_from_slice(element);
            } else if !self.append_complete_obu(element) {
                self.reset();
                return None;
            }

            element_index += 1;
        }

        if marker {
            if self.fragmented_obu.is_empty() {
                return self.take_temporal_unit();
            }
            self.reset();
        }

        None
    }

    fn append_complete_obu(&mut self, element: &[u8]) -> bool {
        if element.is_empty() {
            return false;
        }

        let header = element[0] | 0x02;
        let has_extension = header & 0x04 != 0;
        let header_size = if has_extension { 2 } else { 1 };
        if element.len() < header_size {
            return false;
        }

        self.temporal_unit.push(header);
        if has_extension {
            self.temporal_unit.push(element[1]);
        }
        write_leb128(
            (element.len() - header_size) as u32,
            &mut self.temporal_unit,
        );
        self.temporal_unit
            .extend_from_slice(&element[header_size..]);
        true
    }

    fn take_temporal_unit(&mut self) -> Option<(Vec<u8>, bool)> {
        if self.temporal_unit.is_empty() {
            return None;
        }
        let keyframe = self.has_new_sequence;
        self.has_new_sequence = false;
        let mut complete = Vec::new();
        std::mem::swap(&mut complete, &mut self.temporal_unit);
        Some((complete, keyframe))
    }

    fn reset(&mut self) {
        self.temporal_unit.clear();
        self.fragmented_obu.clear();
        self.has_new_sequence = false;
    }

    fn has_incomplete_temporal_unit(&self) -> bool {
        !self.temporal_unit.is_empty() || !self.fragmented_obu.is_empty()
    }
}

impl Av1RtpIngress {
    pub fn push_packet(
        &mut self,
        payload: &[u8],
        marker: bool,
        sequence_number: u16,
        timestamp_us: u64,
    ) -> Option<EncodedAccessUnit> {
        self.assembler
            .push_rtp_packet(payload, marker, sequence_number)
            .map(|(bytes, is_keyframe)| EncodedAccessUnit {
                codec: VideoCodec::Av1,
                timestamp_us,
                is_keyframe,
                bytes,
            })
    }

    pub fn push_payload(
        &mut self,
        payload: &[u8],
        marker: bool,
        timestamp_us: u64,
    ) -> Option<EncodedAccessUnit> {
        self.assembler
            .push_rtp_payload(payload, marker)
            .map(|(bytes, is_keyframe)| EncodedAccessUnit {
                codec: VideoCodec::Av1,
                timestamp_us,
                is_keyframe,
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
        Self::new_with_profile(track_id, stream_id, fps, mtu, H264Profile::Baseline)
    }

    pub fn new_with_profile(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        mtu: u16,
        profile: H264Profile,
    ) -> Self {
        Self::new_with_profile_level_id(
            track_id,
            stream_id,
            fps,
            mtu,
            profile,
            h264_profile_level_id(profile),
        )
    }

    pub fn new_with_profile_level_id(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        mtu: u16,
        profile: H264Profile,
        profile_level_id: impl Into<String>,
    ) -> Self {
        let payload_type = match profile {
            H264Profile::Baseline => 102,
            H264Profile::High => 123,
        };
        let profile_level_id = profile_level_id.into();
        let payloader = Box::<rtp::codecs::h264::H264Payloader>::default();
        let packetizer = Box::new(new_packetizer(
            mtu.max(576) as usize,
            payload_type,
            0,
            payloader,
            Box::new(new_random_sequencer()),
            90_000,
        ));
        Self {
            track: Arc::new(TrackLocalStaticRTP::new(
                h264_codec_capability_for_profile_level_id(&profile_level_id),
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
        self.send_access_unit_with_report(access_unit)
            .await
            .map(|report| report.bytes_written)
    }

    pub async fn send_access_unit_with_report(
        &mut self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<H264RtpSendReport, TransportError> {
        let packets = self.packetize_access_unit(access_unit)?;
        let rtp_timestamp = packets
            .first()
            .map(|packet| packet.header.timestamp)
            .unwrap_or_default();
        let mut written = 0usize;
        for packet in packets {
            written +=
                self.track.write_rtp(&packet).await.map_err(|error| {
                    TransportError::Message(format!("write_rtp failed: {error}"))
                })?;
        }
        Ok(H264RtpSendReport {
            bytes_written: written,
            rtp_timestamp,
        })
    }
}

impl H264SampleSender {
    pub fn new_with_profile_level_id(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        profile_level_id: impl Into<String>,
    ) -> Self {
        let profile_level_id = profile_level_id.into();
        let frame_duration = Duration::from_nanos(1_000_000_000u64 / fps.max(1) as u64);
        Self {
            track: Arc::new(TrackLocalStaticSample::new(
                h264_codec_capability_for_profile_level_id(&profile_level_id),
                track_id.into(),
                stream_id.into(),
            )),
            frame_duration,
        }
    }

    pub fn track(&self) -> Arc<TrackLocalStaticSample> {
        self.track.clone()
    }

    pub async fn send_access_unit(
        &self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<usize, TransportError> {
        self.send_access_unit_with_report(access_unit)
            .await
            .map(|report| report.bytes_written)
    }

    pub async fn send_access_unit_with_report(
        &self,
        access_unit: &EncodedAccessUnit,
    ) -> Result<H264SampleSendReport, TransportError> {
        if access_unit.codec != VideoCodec::H264 {
            return Err(TransportError::Message(
                "H264 sample sender only supports H264 access units".into(),
            ));
        }

        let sample = Sample {
            data: bytes::Bytes::copy_from_slice(access_unit.bytes.as_slice()),
            duration: self.frame_duration,
            ..Default::default()
        };
        self.track
            .write_sample(&sample)
            .await
            .map_err(|error| TransportError::Message(format!("write_sample failed: {error}")))?;
        Ok(H264SampleSendReport {
            bytes_written: access_unit.bytes.len(),
        })
    }
}

impl Av1RtpSender {
    pub fn new(
        track_id: impl Into<String>,
        stream_id: impl Into<String>,
        fps: u32,
        mtu: u16,
    ) -> Self {
        let payload_type = 104;
        let payloader = Box::<rtp::codecs::av1::Av1Payloader>::default();
        let packetizer = Box::new(new_packetizer(
            mtu.max(576) as usize,
            payload_type,
            0,
            payloader,
            Box::new(new_random_sequencer()),
            90_000,
        ));
        Self {
            track: Arc::new(TrackLocalStaticRTP::new(
                av1_codec_capability(),
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
        if access_unit.codec != VideoCodec::Av1 {
            return Err(TransportError::Message(
                "AV1 RTP sender only supports AV1 access units".into(),
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
            written +=
                self.track.write_rtp(&packet).await.map_err(|error| {
                    TransportError::Message(format!("write_rtp failed: {error}"))
                })?;
        }
        Ok(written)
    }
}

fn h264_profile_level_id(profile: H264Profile) -> &'static str {
    match profile {
        // The browser preview path can send 1440p/120 from the local DXGI source.
        // Advertise level 5.2 so the browser does not negotiate a low-level H.264
        // receiver and then reject the first high-rate access units.
        H264Profile::Baseline => "42e034",
        H264Profile::High => "640034",
    }
}

pub fn h264_codec_capability(profile: H264Profile) -> RTCRtpCodecCapability {
    h264_codec_capability_for_profile_level_id(h264_profile_level_id(profile))
}

pub fn h264_codec_capability_for_profile_level_id(profile_level_id: &str) -> RTCRtpCodecCapability {
    RTCRtpCodecCapability {
        mime_type: "video/H264".to_string(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: format!(
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={profile_level_id}"
        ),
        rtcp_feedback: vec![],
    }
}

pub fn av1_codec_capability() -> RTCRtpCodecCapability {
    RTCRtpCodecCapability {
        mime_type: "video/AV1".to_string(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: String::new(),
        rtcp_feedback: vec![],
    }
}

pub fn annex_b_contains_keyframe(access_unit: &[u8]) -> bool {
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

fn read_leb128(data: &[u8], offset: &mut usize) -> Option<usize> {
    let mut value = 0usize;
    let mut shift = 0usize;
    while *offset < data.len() {
        let byte = data[*offset];
        *offset += 1;
        value |= ((byte & 0x7f) as usize) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
    None
}

fn write_leb128(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        annex_b_contains_keyframe, h264_codec_capability, Av1AccessUnitAssembler, Av1RtpIngress,
        Av1RtpSender, H264AccessUnitAssembler, H264Profile, H264RtpIngress,
    };
    use mrd_encode_nvenc::NvencH264Encoder;
    use mrd_pipeline_core::{CapturedFrame, FramePixelFormat, VideoCodec, VideoEncoder};

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
                0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x68, 0xce, 0, 0, 0, 1, 0x65, 0xaa, 0xbb, 0xcc,
                0xdd,
            ])
        );
    }

    #[test]
    fn ingress_wraps_annex_b_payload_as_h264_access_unit() {
        let mut ingress = H264RtpIngress::default();

        assert!(ingress
            .push_payload(&[0x7c, 0x85, 0xaa, 0xbb], false, 33_000)
            .is_none());
        let access_unit = ingress
            .push_payload(&[0x7c, 0x45, 0xcc, 0xdd], true, 33_000)
            .expect("completed access unit");

        assert_eq!(access_unit.codec, VideoCodec::H264);
        assert_eq!(access_unit.timestamp_us, 33_000);
        assert!(access_unit.is_keyframe);
    }

    #[test]
    fn sequence_gap_drops_incomplete_fua_access_unit() {
        let mut ingress = H264RtpIngress::default();

        assert!(ingress
            .push_packet(&[0x7c, 0x85, 0xaa, 0xbb], false, 10, 33_000)
            .is_none());
        assert!(ingress
            .push_packet(&[0x7c, 0x45, 0xcc, 0xdd], true, 12, 33_000)
            .is_none());
    }

    #[test]
    fn sequence_gap_recovers_on_next_complete_access_unit() {
        let mut ingress = H264RtpIngress::default();

        assert!(ingress
            .push_packet(&[0x7c, 0x85, 0xaa, 0xbb], false, 21, 33_000)
            .is_none());
        assert!(ingress
            .push_packet(&[0x7c, 0x45, 0xcc, 0xdd], true, 23, 33_000)
            .is_none());

        let access_unit = ingress
            .push_packet(&[0x65, 0x11, 0x22, 0x33], true, 24, 66_000)
            .expect("recovered single NAL access unit");
        assert!(annex_b_contains_keyframe(&access_unit.bytes));
    }

    #[test]
    fn keyframe_detector_marks_idr_annex_b_payloads() {
        assert!(annex_b_contains_keyframe(&[0, 0, 0, 1, 0x65, 0xaa]));
    }

    #[test]
    fn keyframe_detector_accepts_idr_with_lower_nri() {
        assert!(annex_b_contains_keyframe(&[0, 0, 0, 1, 0x25, 0xaa]));
    }

    #[test]
    fn av1_ingress_rebuilds_single_obu_with_size_field() {
        let mut assembler = Av1AccessUnitAssembler::default();
        let (access_unit, is_keyframe) = assembler
            .push_rtp_payload(&[0b0001_1000, 0x08, 0xaa, 0xbb], true)
            .expect("complete av1 temporal unit");

        assert!(is_keyframe);
        assert_eq!(access_unit, vec![0x0a, 0x02, 0xaa, 0xbb]);
    }

    #[test]
    fn av1_ingress_rebuilds_fragmented_obu() {
        let mut assembler = Av1AccessUnitAssembler::default();
        assert!(assembler
            .push_rtp_payload(&[0b0101_0000, 0x30, 1, 2], false)
            .is_none());

        let (access_unit, is_keyframe) = assembler
            .push_rtp_payload(&[0b1001_0000, 3, 4], true)
            .expect("fragmented av1 temporal unit");

        assert!(!is_keyframe);
        assert_eq!(access_unit, vec![0x32, 0x04, 1, 2, 3, 4]);
    }

    #[test]
    fn av1_packetizer_roundtrips_through_ingress() {
        let mut sender = Av1RtpSender::new("track", "stream", 60, 8);
        let input = mrd_pipeline_core::EncodedAccessUnit {
            codec: VideoCodec::Av1,
            timestamp_us: 42,
            is_keyframe: true,
            bytes: vec![0x0a, 0x02, 0xaa, 0xbb, 0x32, 0x03, 1, 2, 3],
        };

        let packets = sender.packetize_access_unit(&input).expect("packetize av1");
        let mut ingress = Av1RtpIngress::default();
        let mut output = None;
        for packet in packets {
            output = ingress.push_packet(
                &packet.payload,
                packet.header.marker,
                packet.header.sequence_number,
                input.timestamp_us,
            );
        }

        let output = output.expect("reassembled av1 access unit");
        assert_eq!(output.codec, VideoCodec::Av1);
        assert_eq!(output.timestamp_us, input.timestamp_us);
        assert_eq!(output.bytes, input.bytes);
    }

    #[test]
    fn h264_browser_capability_advertises_2k120_safe_level() {
        let baseline = h264_codec_capability(H264Profile::Baseline);
        let high = h264_codec_capability(H264Profile::High);

        assert!(baseline.sdp_fmtp_line.contains("profile-level-id=42e034"));
        assert!(high.sdp_fmtp_line.contains("profile-level-id=640034"));
    }

    #[test]
    fn h264_packetizer_exposes_stable_rtp_timestamp_per_access_unit() {
        let mut sender = super::H264RtpSender::new("video", "stream", 120, 1200);
        let first = mrd_pipeline_core::EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us: 1_000,
            is_keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x65, 1, 2, 3],
        };
        let second = mrd_pipeline_core::EncodedAccessUnit {
            timestamp_us: 9_333,
            bytes: vec![0, 0, 0, 1, 0x41, 4, 5, 6],
            ..first.clone()
        };

        let first_packets = sender
            .packetize_access_unit(&first)
            .expect("packetize first access unit");
        let second_packets = sender
            .packetize_access_unit(&second)
            .expect("packetize second access unit");
        let first_timestamp = first_packets[0].header.timestamp;

        assert!(first_packets
            .iter()
            .all(|packet| packet.header.timestamp == first_timestamp));
        assert_eq!(second_packets[0].header.timestamp, first_timestamp + 750);
    }

    #[test]
    fn nvenc_access_unit_survives_rtp_packetize_and_ingress_reassembly() {
        let Ok(mut encoder) = NvencH264Encoder::new(16, 16, 30) else {
            return;
        };
        let frame = CapturedFrame::from_cpu(
            16,
            16,
            FramePixelFormat::Bgra32,
            33_000,
            vec![0x55; 16 * 16 * 4],
        );
        let access_unit = encoder
            .encode(&frame)
            .expect("encode nvenc frame")
            .into_iter()
            .next()
            .expect("single access unit");
        let mut sender = super::H264RtpSender::new("video", "stream", 30, 1200);
        let packets = sender
            .packetize_access_unit(&access_unit)
            .expect("packetize nvenc access unit");
        let mut ingress = H264RtpIngress::default();
        let mut reassembled = None;
        for packet in packets {
            reassembled = ingress.push_payload(
                &packet.payload,
                packet.header.marker,
                access_unit.timestamp_us,
            );
        }

        let reassembled = reassembled.expect("reassembled access unit");
        assert_eq!(reassembled.codec, VideoCodec::H264);
        assert!(annex_b_contains_keyframe(&reassembled.bytes));
        assert!(!reassembled.bytes.is_empty());
    }
}
