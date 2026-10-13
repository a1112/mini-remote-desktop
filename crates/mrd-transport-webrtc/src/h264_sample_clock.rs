//! H.264 RTP sampling clock, independent of packetization and send scheduling.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct H264SampleClock {
    origin_capture_us: u64,
    origin_rtp: u32,
    last_capture_us: u64,
    last_rtp: u32,
}

impl H264SampleClock {
    pub(super) fn next(
        previous: Option<Self>,
        capture_us: u64,
        packetizer_timestamp: u32,
        nominal_samples: u32,
    ) -> Option<Self> {
        let Some(previous) = previous else {
            return Some(Self::anchor(capture_us, packetizer_timestamp));
        };
        if capture_us == previous.last_capture_us {
            return Some(previous);
        }
        if capture_us < previous.last_capture_us {
            // DXGI capture currently uses SystemTime. A clock correction must
            // not become an unsigned multi-year gap or terminate the session.
            return Some(Self::anchor(
                capture_us,
                previous.last_rtp.wrapping_add(nominal_samples),
            ));
        }
        let elapsed_us = capture_us.checked_sub(previous.origin_capture_us)?;
        // 90kHz / 1MHz = 9/100. Round the entire origin-relative span;
        // rounding each delta separately would accumulate fractional drift.
        let ticks = u128::from(elapsed_us).checked_mul(9)?.checked_add(50)? / 100;
        let offset = u32::try_from(ticks % (1_u128 << 32)).ok()?;
        Some(Self {
            last_capture_us: capture_us,
            last_rtp: previous.origin_rtp.wrapping_add(offset),
            ..previous
        })
    }

    pub(super) fn timestamp(self) -> u32 {
        self.last_rtp
    }

    fn anchor(capture_us: u64, timestamp: u32) -> Self {
        Self {
            origin_capture_us: capture_us,
            origin_rtp: timestamp,
            last_capture_us: capture_us,
            last_rtp: timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::H264SampleClock;
    use crate::{H264RtpIngress, H264RtpSender};
    use mrd_pipeline_core::{EncodedAccessUnit, VideoCodec};

    fn au(timestamp_us: u64) -> EncodedAccessUnit {
        EncodedAccessUnit {
            codec: VideoCodec::H264,
            timestamp_us,
            is_keyframe: true,
            bytes: vec![0, 0, 0, 1, 0x65, 1, 2, 3],
        }
    }

    fn stamp(sender: &mut H264RtpSender, timestamp_us: u64) -> u32 {
        sender.packetize_access_unit(&au(timestamp_us)).unwrap()[0]
            .header
            .timestamp
    }

    #[test]
    fn h264_sample_clock_first_zero_preserves_the_first_packet_random_origin() {
        let first = H264SampleClock::next(None, 0, 123_456_789, 3000).unwrap();
        assert_eq!(first.timestamp(), 123_456_789);
        assert_eq!(
            H264SampleClock::next(Some(first), 33_000, 42, 3000)
                .unwrap()
                .timestamp(),
            123_459_759
        );
    }

    #[test]
    fn h264_sample_clock_forward_time_is_independent_of_configured_fps() {
        for fps in [30, 60, 120] {
            let mut sender = H264RtpSender::new("video", "stream", fps, 1200);
            let first = stamp(&mut sender, 0);
            let second = stamp(&mut sender, 16_000);
            let third = stamp(&mut sender, 60_000);
            assert_eq!(second.wrapping_sub(first), 1440);
            assert_eq!(third.wrapping_sub(second), 3960);
        }
    }

    #[test]
    fn h264_sample_clock_origin_rounding_preserves_fraction_across_many_frames() {
        let mut sender = H264RtpSender::new("video", "stream", 120, 1200);
        let origin = stamp(&mut sender, 1000);
        assert_eq!(stamp(&mut sender, 9333).wrapping_sub(origin), 750);
        for n in 2..=100_u64 {
            let timestamp = stamp(&mut sender, 1000 + n * 8333);
            if n == 100 {
                assert_eq!(timestamp.wrapping_sub(origin), 74_997);
            }
        }
    }

    #[test]
    fn h264_sample_clock_duplicate_and_subtick_samples_keep_the_same_time() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let first = stamp(&mut sender, 0);
        assert_eq!(stamp(&mut sender, 0), first);
        assert_eq!(stamp(&mut sender, 1), first);
        assert_eq!(stamp(&mut sender, 6), first.wrapping_add(1));
        assert_eq!(stamp(&mut sender, 100_000), first.wrapping_add(9000));
    }

    #[test]
    fn h264_sample_clock_backward_capture_rebases_once_then_retains_real_time() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let first = stamp(&mut sender, 0);
        let previous = stamp(&mut sender, 100_000);
        assert_eq!(previous.wrapping_sub(first), 9000);
        let second = stamp(&mut sender, 90_000);
        assert_eq!(second.wrapping_sub(previous), 3000);
        assert_eq!(stamp(&mut sender, 90_000), second);
        assert_eq!(stamp(&mut sender, 100_000).wrapping_sub(second), 900);
    }

    #[test]
    fn h264_sample_clock_wrap_and_large_u64_are_checked_without_narrowing_first() {
        let origin = H264SampleClock::next(None, 0, u32::MAX - 1000, 3000).unwrap();
        assert_eq!(
            H264SampleClock::next(Some(origin), 20_000, 42, 3000)
                .unwrap()
                .timestamp(),
            799
        );
        let origin = H264SampleClock::next(None, 0, 40, 3000).unwrap();
        assert_eq!(
            H264SampleClock::next(Some(origin), u64::MAX, 42, 3000)
                .unwrap()
                .timestamp(),
            2_748_779_109
        );
        let origin = H264SampleClock::next(None, u64::MAX - 100_000, 40, 3000).unwrap();
        assert_eq!(
            H264SampleClock::next(Some(origin), u64::MAX, 42, 3000)
                .unwrap()
                .timestamp(),
            9040
        );
    }

    #[test]
    fn h264_sample_clock_empty_and_parameter_only_units_do_not_anchor_or_double_count() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let mut empty = au(u64::MAX);
        empty.bytes.clear();
        assert!(sender.packetize_access_unit(&empty).unwrap().is_empty());
        assert!(sender.sample_clock.is_none());
        let first_packets = sender.packetize_access_unit(&au(0)).unwrap();
        let first_rtp = first_packets[0].header.timestamp;
        let first_sequence = first_packets.last().unwrap().header.sequence_number;
        let sps_bytes = vec![0, 0, 0, 1, 0x67, 0x42, 0, 0x1e];
        let pps_bytes = vec![0, 0, 0, 1, 0x68, 1];
        let before = sender.sample_clock;
        for bytes in [sps_bytes.clone(), pps_bytes.clone(), vec![]] {
            let parameter = EncodedAccessUnit {
                timestamp_us: 200_000,
                bytes,
                ..au(0)
            };
            assert!(sender.packetize_access_unit(&parameter).unwrap().is_empty());
            assert_eq!(sender.sample_clock, before);
        }
        let input = au(300_000);
        let packets = sender.packetize_access_unit(&input).unwrap();
        assert_eq!(packets[0].header.timestamp.wrapping_sub(first_rtp), 27_000);
        assert_eq!(
            packets[0]
                .header
                .sequence_number
                .wrapping_sub(first_sequence),
            1
        );
        assert_eq!(
            packets[0].payload[0] & 0x1f,
            24,
            "cached SPS/PPS must still be emitted as STAP-A"
        );
        let mut ingress = H264RtpIngress::default();
        let mut rebuilt = None;
        for packet in packets {
            rebuilt =
                ingress.push_payload(&packet.payload, packet.header.marker, input.timestamp_us);
        }
        let expected = [sps_bytes, pps_bytes, input.bytes].concat();
        assert_eq!(rebuilt.unwrap().bytes, expected);
    }

    #[test]
    fn h264_sample_clock_all_fragments_keep_payload_marker_and_sequence() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let first = stamp(&mut sender, 0);
        let mut input = au(100_000);
        input.bytes.extend(vec![0x11; 4000]);
        let packets = sender.packetize_access_unit(&input).unwrap();
        assert!(packets.len() >= 3);
        assert!(packets
            .iter()
            .all(|p| p.header.timestamp == first.wrapping_add(9000)));
        assert!(packets.last().unwrap().header.marker);
        assert!(packets[..packets.len() - 1]
            .iter()
            .all(|p| !p.header.marker));
        assert!(packets.windows(2).all(|p| p[1]
            .header
            .sequence_number
            .wrapping_sub(p[0].header.sequence_number)
            == 1));
        let mut ingress = H264RtpIngress::default();
        let mut rebuilt = None;
        for packet in packets {
            rebuilt =
                ingress.push_payload(&packet.payload, packet.header.marker, input.timestamp_us);
        }
        assert_eq!(rebuilt.unwrap().bytes, input.bytes);
    }

    #[test]
    fn h264_sample_clock_rejected_codec_does_not_commit_emitted_state() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let first = stamp(&mut sender, 0);
        let before = sender.sample_clock;
        let invalid = EncodedAccessUnit {
            codec: VideoCodec::Hevc,
            timestamp_us: 200_000,
            ..au(0)
        };
        assert!(sender.packetize_access_unit(&invalid).is_err());
        assert_eq!(sender.sample_clock, before);
        assert_eq!(stamp(&mut sender, 100_000).wrapping_sub(first), 9000);
    }

    #[test]
    fn h264_sample_clock_initial_sps_pps_cache_does_not_create_a_picture_anchor() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let sps = vec![0, 0, 0, 1, 0x67, 0x42, 0, 0x1e];
        let pps = vec![0, 0, 0, 1, 0x68, 1];
        for (timestamp_us, bytes) in [(200_000, sps), (90_000, pps)] {
            let parameter = EncodedAccessUnit {
                timestamp_us,
                bytes,
                ..au(0)
            };
            assert!(sender.packetize_access_unit(&parameter).unwrap().is_empty());
            assert!(sender.sample_clock.is_none());
        }
        let first = sender.packetize_access_unit(&au(0)).unwrap();
        assert_eq!(first[0].payload[0] & 0x1f, 24);
        assert_eq!(
            sender.sample_clock.unwrap().timestamp(),
            first[0].header.timestamp
        );
        assert_eq!(
            stamp(&mut sender, 100_000).wrapping_sub(first[0].header.timestamp),
            9000
        );
    }

    // Fault injection is limited to the packetizer error boundary. It does not
    // model the clock, payload, transport or decoder under test.
    #[derive(Debug)]
    struct RejectingPacketizer;
    impl rtp::packetizer::Packetizer for RejectingPacketizer {
        fn enable_abs_send_time(&mut self, _: u8) {}
        fn packetize(
            &mut self,
            _: &bytes::Bytes,
            _: u32,
        ) -> Result<Vec<rtp::packet::Packet>, rtp::Error> {
            Err(rtp::Error::ErrShortPacket)
        }
        fn skip_samples(&mut self, _: u32) {}
        fn clone_to(&self) -> Box<dyn rtp::packetizer::Packetizer + Send + Sync> {
            Box::new(Self)
        }
    }

    #[test]
    fn h264_sample_clock_packetizer_failure_does_not_commit_an_emitted_anchor() {
        let mut sender = H264RtpSender::new("video", "stream", 30, 1200);
        let first = stamp(&mut sender, 0);
        let before = sender.sample_clock;
        let packetizer = std::mem::replace(&mut sender.packetizer, Box::new(RejectingPacketizer));
        assert!(sender.packetize_access_unit(&au(200_000)).is_err());
        assert_eq!(sender.sample_clock, before);
        sender.packetizer = packetizer;
        assert_eq!(stamp(&mut sender, 100_000).wrapping_sub(first), 9000);
    }
}
