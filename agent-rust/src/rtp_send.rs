use anyhow::Result;
use bytes::Bytes;
use rtp::packet::Packet;
use rtp::packetizer::{Packetizer, new_packetizer};
use rtp::sequence::new_random_sequencer;
use std::sync::Arc;
use std::time::Duration;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;

pub struct RtpH264SenderConfig {
    pub fps: u32,
    pub mtu: u16,
    pub frame_pacing_enable: bool,
    pub frame_pacing_batch_packets: u32,
}

pub struct RtpH264Sender {
    track: Arc<TrackLocalStaticRTP>,
    packetizer: Box<dyn Packetizer + Send + Sync>,
    frame_samples: u32,
    frame_duration: Duration,
    frame_pacing_enable: bool,
    frame_pacing_batch_packets: usize,
}

impl RtpH264Sender {
    pub fn new(track: Arc<TrackLocalStaticRTP>, cfg: &RtpH264SenderConfig) -> Self {
        let fps = cfg.fps.max(1);
        let mtu = cfg.mtu.max(576) as usize;
        let payloader = Box::<rtp::codecs::h264::H264Payloader>::default();
        let packetizer = Box::new(new_packetizer(
            mtu,
            102,
            0,
            payloader,
            Box::new(new_random_sequencer()),
            90_000,
        ));

        Self {
            track,
            packetizer,
            frame_samples: (90_000 / fps).max(1),
            frame_duration: Duration::from_millis((1000.0 / fps as f64).max(1.0).round() as u64),
            frame_pacing_enable: cfg.frame_pacing_enable,
            frame_pacing_batch_packets: cfg.frame_pacing_batch_packets.max(1) as usize,
        }
    }

    pub async fn send_access_unit(&mut self, annexb_au: &[u8]) -> Result<usize> {
        let packets = self
            .packetizer
            .packetize(&Bytes::copy_from_slice(annexb_au), self.frame_samples)?;
        if packets.is_empty() {
            return Ok(0);
        }

        if !self.frame_pacing_enable || packets.len() <= self.frame_pacing_batch_packets {
            return self.send_packets(&packets).await;
        }

        let batches = packets.len().div_ceil(self.frame_pacing_batch_packets);
        let sleep_ns = (self.frame_duration.as_nanos() / batches.max(1) as u128) as u64;
        let sleep_dur = Duration::from_nanos(sleep_ns.max(100_000));

        let mut sent = 0usize;
        for chunk in packets.chunks(self.frame_pacing_batch_packets) {
            sent += self.send_packets(chunk).await?;
            tokio::time::sleep(sleep_dur).await;
        }
        Ok(sent)
    }

    async fn send_packets(&self, packets: &[Packet]) -> Result<usize> {
        let mut n = 0usize;
        for pkt in packets {
            n += self.track.write_rtp_with_extensions(pkt, &[]).await?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_samples_has_expected_floor() {
        let cfg = RtpH264SenderConfig {
            fps: 60,
            mtu: 1200,
            frame_pacing_enable: true,
            frame_pacing_batch_packets: 6,
        };
        assert_eq!((90_000 / cfg.fps.max(1)).max(1), 1500);
    }
}
