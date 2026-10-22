//! Bounded PCM system-audio transport. Like R-ShareMouse, realtime callbacks
//! never wait for network I/O, and stream identity is bound to an authorization.
use anyhow::{bail, Result};
use std::collections::{BTreeMap, VecDeque};

#[cfg(target_os = "macos")]
pub mod macos;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
pub const FRAMES_PER_PACKET: usize = 240;
pub const SAMPLES_PER_PACKET: usize = FRAMES_PER_PACKET * CHANNELS;
pub const HEADER_BYTES: usize = 40;
pub const PACKET_BYTES: usize = HEADER_BYTES + SAMPLES_PER_PACKET * 2;
const MAGIC: &[u8; 8] = b"MRDAUD01";
const MAX_BUFFER_PACKETS: usize = 12;
const PREBUFFER_PACKETS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct AudioPacket {
    pub stream_id: [u8; 16],
    pub sequence: u64,
    pub samples: Vec<i16>,
}
impl AudioPacket {
    pub fn is_audio(bytes: &[u8]) -> bool {
        bytes.starts_with(MAGIC)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.samples.len() != SAMPLES_PER_PACKET {
            bail!("invalid audio sample count");
        }
        let mut bytes = Vec::with_capacity(PACKET_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.stream_id);
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        bytes.extend_from_slice(&(FRAMES_PER_PACKET as u16).to_le_bytes());
        bytes.push(CHANNELS as u8);
        bytes.push(1); // Signed PCM16 LE.
        for sample in &self.samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8], stream_id: [u8; 16]) -> Result<Self> {
        if bytes.len() != PACKET_BYTES
            || !Self::is_audio(bytes)
            || bytes[8..24] != stream_id
            || u32::from_le_bytes(bytes[32..36].try_into()?) != SAMPLE_RATE
            || u16::from_le_bytes(bytes[36..38].try_into()?) as usize != FRAMES_PER_PACKET
            || bytes[38] as usize != CHANNELS
            || bytes[39] != 1
        {
            bail!("invalid, unsupported or stale audio packet");
        }
        Ok(Self {
            stream_id,
            sequence: u64::from_le_bytes(bytes[24..32].try_into()?),
            samples: bytes[HEADER_BYTES..]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect(),
        })
    }
}

/// Twenty milliseconds of initial jitter tolerance, capped at sixty milliseconds.
/// Missing packets become silence; duplicates and late packets cannot replay.
#[derive(Default)]
pub struct PlayoutBuffer {
    packets: BTreeMap<u64, Vec<i16>>,
    current: VecDeque<i16>,
    next_sequence: Option<u64>,
    primed: bool,
    pub received_packets: u64,
    pub dropped_packets: u64,
    pub underrun_packets: u64,
    pub rendered_samples: u64,
}
impl PlayoutBuffer {
    pub fn push(&mut self, packet: AudioPacket) {
        if packet.samples.len() != SAMPLES_PER_PACKET
            || self
                .next_sequence
                .is_some_and(|next| packet.sequence < next)
            || self.packets.contains_key(&packet.sequence)
        {
            self.dropped_packets += 1;
            return;
        }
        self.received_packets += 1;
        self.packets.insert(packet.sequence, packet.samples);
        while self.packets.len() > MAX_BUFFER_PACKETS {
            self.packets.pop_first();
            self.dropped_packets += 1;
            self.next_sequence = self.packets.keys().next().copied();
            self.current.clear();
        }
    }
    pub fn next_sample(&mut self) -> f32 {
        if self.current.is_empty() {
            if !self.primed {
                if self.packets.len() < PREBUFFER_PACKETS {
                    return 0.0;
                }
                self.next_sequence = self.packets.keys().next().copied();
                self.primed = true;
            }
            let sequence = self.next_sequence.unwrap_or(0);
            // After a long outage, resume at current audio instead of producing
            // seconds of silence while incrementing through missing sequence IDs.
            if self.packets.first_key_value().is_some_and(|(&first, _)| {
                first.saturating_sub(sequence) > MAX_BUFFER_PACKETS as u64
            }) {
                self.primed = false;
                self.next_sequence = self.packets.keys().next().copied();
                return 0.0;
            }
            if let Some(samples) = self.packets.remove(&sequence) {
                self.current.extend(samples);
            } else {
                self.underrun_packets += 1;
                self.current.resize(SAMPLES_PER_PACKET, 0);
                if self.packets.is_empty() {
                    self.primed = false;
                }
            }
            self.next_sequence = sequence.checked_add(1);
        }
        self.rendered_samples += 1;
        f32::from(self.current.pop_front().unwrap_or(0)) / 32768.0
    }
    pub fn clear(&mut self) {
        self.packets.clear();
        self.current.clear();
        self.primed = false;
    }
    pub fn buffered_frames(&self) -> usize {
        self.packets.len() * FRAMES_PER_PACKET + self.current.len() / CHANNELS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(sequence: u64) -> AudioPacket {
        AudioPacket {
            stream_id: [7; 16],
            sequence,
            samples: vec![123; SAMPLES_PER_PACKET],
        }
    }
    #[test]
    fn wire_format_is_mtu_bounded_and_bound_to_the_grant() {
        let p = packet(9);
        let bytes = p.encode().unwrap();
        assert!(bytes.len() < 1200);
        assert_eq!(AudioPacket::decode(&bytes, [7; 16]).unwrap(), p);
        assert!(AudioPacket::decode(&bytes, [8; 16]).is_err());
        for length in 0..bytes.len() {
            assert!(AudioPacket::decode(&bytes[..length], [7; 16]).is_err());
        }
        let mut malformed = bytes;
        malformed[38] = 255;
        assert!(AudioPacket::decode(&malformed, [7; 16]).is_err());
    }
    #[test]
    fn playout_reorders_rejects_replay_and_fills_packet_loss() {
        let mut queue = PlayoutBuffer::default();
        for sequence in [2, 0, 3, 1] {
            queue.push(packet(sequence));
        }
        assert!(queue.next_sample() > 0.0);
        queue.push(packet(0));
        assert_eq!(queue.dropped_packets, 1);
        for _ in 1..SAMPLES_PER_PACKET * 4 {
            assert!(queue.next_sample() > 0.0);
        }
        assert_eq!(queue.next_sample(), 0.0);
        assert_eq!(queue.underrun_packets, 1);
    }
    #[test]
    fn slow_playback_has_a_fixed_memory_and_latency_bound() {
        let mut queue = PlayoutBuffer::default();
        for sequence in 0..1000 {
            queue.push(packet(sequence));
        }
        assert_eq!(
            queue.buffered_frames(),
            MAX_BUFFER_PACKETS * FRAMES_PER_PACKET
        );
        assert_eq!(queue.dropped_packets, 988);
        queue.clear();
        assert_eq!(queue.next_sample(), 0.0);
    }

    #[test]
    fn long_network_outage_rebuffers_current_audio_without_replaying_old_audio() {
        let mut queue = PlayoutBuffer::default();
        for sequence in 0..4 {
            queue.push(packet(sequence));
        }
        for _ in 0..SAMPLES_PER_PACKET * 4 {
            queue.next_sample();
        }
        for sequence in 1000..1004 {
            queue.push(packet(sequence));
        }
        assert_eq!(queue.next_sample(), 0.0);
        assert!(queue.next_sample() > 0.0);
        queue.push(packet(3));
        assert_eq!(queue.dropped_packets, 1);
    }
}
