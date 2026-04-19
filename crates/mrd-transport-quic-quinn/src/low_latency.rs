// Low-latency transmission optimizations
//
// Provides:
// - Paced transmission for network congestion avoidance
// - FEC (Forward Error Correction) for packet loss recovery
// - NACK (Negative Acknowledgement) for retransmission requests

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};

use bytes::Bytes;
use crate::QuinnDatagramEndpoint;

/// Pacing configuration for transmission rate control
#[derive(Debug, Clone)]
pub struct PacingConfig {
    /// Enable paced transmission
    pub enabled: bool,
    /// Target bitrate in bits per second
    pub target_bitrate_bps: u64,
    /// Maximum burst size in bytes
    pub max_burst_bytes: usize,
    /// Minimum interval between packets
    pub min_packet_interval: Duration,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_bitrate_bps: 20_000_000, // 20 Mbps default
            max_burst_bytes: 64 * 1024,     // 64 KB max burst
            min_packet_interval: Duration::from_micros(100), // 100us min
        }
    }
}

impl PacingConfig {
    /// Calculate pacing delay based on packet size
    pub fn calculate_delay(&self, packet_size: usize) -> Duration {
        if !self.enabled || self.target_bitrate_bps == 0 {
            return Duration::ZERO;
        }

        // Time to transmit this packet at target bitrate
        let tx_time = Duration::from_secs_f64(
            packet_size as f64 * 8.0 / self.target_bitrate_bps as f64
        );

        tx_time.max(self.min_packet_interval)
    }

    /// Calculate how many bytes can be sent in a burst
    pub fn burst_capacity(&self) -> usize {
        self.max_burst_bytes
    }
}

/// FEC configuration
#[derive(Debug, Clone)]
pub struct FecConfig {
    /// Enable FEC
    pub enabled: bool,
    /// FEC scheme type
    pub scheme: FecScheme,
    /// Number of data packets per FEC block
    pub block_size: usize,
    /// Number of parity packets per FEC block
    pub parity_count: usize,
}

impl Default for FecConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scheme: FecScheme::Xor,
            block_size: 10,
            parity_count: 2,
        }
    }
}

/// FEC encoding schemes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FecScheme {
    /// Simple XOR parity (low overhead, single recovery)
    Xor,
    /// Reed-Solomon (multiple recovery, higher overhead)
    ReedSolomon,
}

/// FEC encoder state
#[derive(Debug)]
struct FecEncoder {
    config: FecConfig,
    buffer: Vec<Bytes>,
    sequence: u32,
}

impl FecEncoder {
    fn new(config: FecConfig) -> Self {
        let block_size = config.block_size;
        Self {
            config,
            buffer: Vec::with_capacity(block_size),
            sequence: 0,
        }
    }

    /// Add a data packet and return any parity packets
    fn add_data_packet(&mut self, data: Bytes) -> Vec<FecPacket> {
        self.buffer.push(data);
        let seq = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);

        // Check if block is complete
        if self.buffer.len() < self.config.block_size {
            return Vec::new();
        }

        // Generate parity packets
        let parity = self.generate_parity();
        self.buffer.clear();

        parity.into_iter().map(|p| FecPacket {
            base_sequence: seq.wrapping_sub(self.config.block_size as u32),
            block_size: self.config.block_size as u16,
            parity_index: p.index,
            data: p.data,
        }).collect()
    }

    /// Flush remaining data with partial FEC
    fn flush(&mut self) -> Vec<FecPacket> {
        if self.buffer.is_empty() {
            return Vec::new();
        }

        let seq = self.sequence.wrapping_sub(self.buffer.len() as u32);
        let parity = self.generate_parity();
        self.buffer.clear();

        parity.into_iter().map(|p| FecPacket {
            base_sequence: seq,
            block_size: self.buffer.len() as u16,
            parity_index: p.index,
            data: p.data,
        }).collect()
    }

    /// Generate parity packets based on scheme
    fn generate_parity(&self) -> Vec<ParityPacket> {
        match self.config.scheme {
            FecScheme::Xor => self.generate_xor_parity(),
            FecScheme::ReedSolomon => self.generate_rs_parity(),
        }
    }

    /// Generate XOR parity packets
    fn generate_xor_parity(&self) -> Vec<ParityPacket> {
        let max_len = self.buffer.iter().map(|p| p.len()).max().unwrap_or(0);
        let mut parity = vec![0u8; max_len];

        for packet in &self.buffer {
            for (i, &byte) in packet.iter().enumerate() {
                parity[i] ^= byte;
            }
        }

        (0..self.config.parity_count)
            .map(|idx| ParityPacket {
                index: idx as u16,
                data: parity.clone(),
            })
            .collect()
    }

    /// Generate Reed-Solomon parity (simplified placeholder)
    fn generate_rs_parity(&self) -> Vec<ParityPacket> {
        // Placeholder: In production, use reed-solomon-rs
        self.generate_xor_parity()
    }
}

/// FEC packet for transmission
#[derive(Debug, Clone, PartialEq, Eq)]
struct FecPacket {
    base_sequence: u32,
    block_size: u16,
    parity_index: u16,
    data: Vec<u8>,
}

/// FEC decoder state
#[derive(Debug)]
struct FecDecoder {
    config: FecConfig,
    buffer: Vec<Option<Bytes>>,
    parity_packets: Vec<Option<FecPacket>>,
    base_sequence: Option<u32>,
}

impl FecDecoder {
    fn new(config: FecConfig) -> Self {
        Self {
            config,
            buffer: Vec::new(),
            parity_packets: Vec::new(),
            base_sequence: None,
        }
    }

    /// Process a received data packet
    fn process_data(&mut self, seq: u32, data: Bytes) -> Vec<Bytes> {
        // Initialize or reset buffer for new block
        if self.base_sequence.is_none() {
            self.base_sequence = Some(seq);
            self.buffer = vec![None; self.config.block_size];
            self.parity_packets = vec![None; self.config.parity_count];
        }

        let base = self.base_sequence.unwrap();
        let offset = seq.wrapping_sub(base) as usize;

        if offset >= self.config.block_size {
            // Old packet, discard
            return Vec::new();
        }

        self.buffer[offset] = Some(data);

        self.try_recover()
    }

    /// Process a received FEC packet
    fn process_fec(&mut self, fec: FecPacket) -> Vec<Bytes> {
        if self.base_sequence.is_none() {
            self.base_sequence = Some(fec.base_sequence);
            self.buffer = vec![None; fec.block_size as usize];
            self.parity_packets = vec![None; self.config.parity_count];
        }

        let fec_idx = fec.parity_index as usize;
        if fec_idx >= self.parity_packets.len() {
            return Vec::new();
        }

        self.parity_packets[fec_idx] = Some(fec);

        self.try_recover()
    }

    /// Attempt to recover missing packets
    fn try_recover(&mut self) -> Vec<Bytes> {
        // Check if we have enough FEC to recover
        let missing_count = self.buffer.iter().filter(|p| p.is_none()).count();
        let fec_count = self.parity_packets.iter().filter(|p| p.is_some()).count();

        // Check if all packets received
        let all_received = missing_count == 0;
        if all_received {
            let recovered = self.buffer.drain(..)
                .map(|p| p.unwrap())
                .collect();
            self.reset();
            return recovered;
        }

        // Try FEC recovery
        if missing_count > 0 && missing_count <= fec_count {
            if let Some(parity) = self.parity_packets.first().and_then(|p| p.as_ref()) {
                // Simple XOR recovery - recover missing packets and return all
                let parity_data = parity.data.clone();
                self.recover_with_xor_internal(&parity_data);

                // Return all packets in order
                let recovered = self.buffer.drain(..)
                    .map(|p| p.unwrap())
                    .collect();
                self.reset();
                return recovered;
            }
        }

        Vec::new()
    }

    /// Internal XOR recovery (separate to avoid borrow issues)
    fn recover_with_xor_internal(&mut self, parity_data: &[u8]) -> Vec<Bytes> {
        let mut recovered = Vec::new();

        for i in 0..self.buffer.len() {
            if self.buffer[i].is_none() {
                let mut recovered_data = parity_data.to_vec();

                for (j, p) in self.buffer.iter().enumerate() {
                    if i != j {
                        if let Some(data) = p {
                            for (k, &byte) in data.iter().enumerate() {
                                if k < recovered_data.len() {
                                    recovered_data[k] ^= byte;
                                }
                            }
                        }
                    }
                }

                self.buffer[i] = Some(Bytes::from(recovered_data.clone()));
                recovered.push(Bytes::from(recovered_data));
            }
        }

        recovered
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.parity_packets.clear();
        self.base_sequence = None;
    }
}

#[derive(Debug)]
struct ParityPacket {
    index: u16,
    data: Vec<u8>,
}

/// NACK configuration
#[derive(Debug, Clone)]
pub struct NackConfig {
    /// Enable NACK
    pub enabled: bool,
    /// Maximum sequence numbers per NACK packet
    pub max_nacked_seqs: usize,
    /// NACK retransmission timeout
    pub nack_timeout: Duration,
    /// Maximum number of retransmission requests
    pub max_nack_retries: u32,
}

impl Default for NackConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_nacked_seqs: 20,
            nack_timeout: Duration::from_millis(50),
            max_nack_retries: 3,
        }
    }
}

/// NACK receiver state
#[derive(Debug)]
struct NackReceiver {
    config: NackConfig,
    expected_sequence: u32,
    missing_sequences: HashSet<u32>,
    nack_retries: HashMap<u32, u32>,
    last_nack_time: Option<Instant>,
}

impl NackReceiver {
    fn new(config: NackConfig, initial_sequence: u32) -> Self {
        Self {
            config,
            expected_sequence: initial_sequence,
            missing_sequences: HashSet::new(),
            nack_retries: HashMap::new(),
            last_nack_time: None,
        }
    }

    /// Process received packet and track missing sequences
    fn process_packet(&mut self, seq: u32, _data: &[u8]) -> Option<NackMessage> {
        // Remove from missing if it was lost
        self.missing_sequences.remove(&seq);
        self.nack_retries.remove(&seq);

        // Check for gap
        let seq_delta = seq.wrapping_sub(self.expected_sequence);
        if seq_delta > 0 && seq_delta < 1000 { // Reasonable gap
            // Mark missing packets
            for missing in self.expected_sequence..seq {
                self.missing_sequences.insert(missing);
                self.nack_retries.insert(missing, 0);
            }
        }

        self.expected_sequence = seq.wrapping_add(1);

        // Generate NACK if needed
        self.should_send_nack().then(|| {
            NackMessage {
                sequence_numbers: self.missing_sequences.iter().copied().collect(),
            }
        })
    }

    /// Check if NACK should be sent
    fn should_send_nack(&self) -> bool {
        if !self.config.enabled || self.missing_sequences.is_empty() {
            return false;
        }

        // Check timeout
        if let Some(last) = self.last_nack_time {
            if last.elapsed() < self.config.nack_timeout {
                return false;
            }
        }

        true
    }

    /// Update last NACK time
    fn update_nack_sent(&mut self) {
        self.last_nack_time = Some(Instant::now());
    }

    /// Increment NACK retry count for exhausted retransmissions
    #[allow(dead_code)]
    fn increment_retry(&mut self, seq: u32) {
        *self.nack_retries.entry(seq).or_insert(0) += 1;
    }

    /// Check if sequence has exceeded retry limit
    #[allow(dead_code)]
    fn is_retry_exhausted(&self, seq: u32) -> bool {
        self.nack_retries.get(&seq)
            .copied()
            .unwrap_or(0) >= self.config.max_nack_retries
    }
}

/// NACK message for requesting retransmission
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NackMessage {
    pub sequence_numbers: Vec<u32>,
}

/// Low-latency transmission manager
#[allow(dead_code)]
pub struct LowLatencyTx {
    endpoint: QuinnDatagramEndpoint,
    pacing_config: PacingConfig,
    #[allow(dead_code)]
    fec_config: FecConfig,
    #[allow(dead_code)]
    nack_config: NackConfig,
    fec_encoder: FecEncoder,
    send_queue: Arc<Mutex<VecDeque<TransmissionTask>>>,
    notify_send: Arc<Notify>,
}

/// Task for transmission
#[derive(Debug)]
enum TransmissionTask {
    Data { #[allow(dead_code)] sequence: u32, data: Vec<u8> },
    Fec { packet: FecPacket },
    Retransmit { #[allow(dead_code)] sequence: u32, data: Vec<u8> },
}

impl LowLatencyTx {
    /// Create a new low-latency transmitter
    pub fn new(
        endpoint: QuinnDatagramEndpoint,
        pacing_config: PacingConfig,
        fec_config: FecConfig,
        nack_config: NackConfig,
    ) -> Self {
        Self {
            endpoint,
            fec_encoder: FecEncoder::new(fec_config.clone()),
            pacing_config,
            fec_config,
            nack_config,
            send_queue: Arc::new(Mutex::new(VecDeque::new())),
            notify_send: Arc::new(Notify::new()),
        }
    }

    /// Start the transmission pump
    pub async fn start(&self) {
        let endpoint = self.endpoint.clone();
        let queue = self.send_queue.clone();
        let notify = self.notify_send.clone();
        let pacing = self.pacing_config.clone();

        tokio::spawn(async move {
            let mut send_time = Instant::now();

            loop {
                // Wait for data
                notify.notified().await;

                let mut guard = queue.lock().await;
                let mut burst_bytes = 0usize;

                while let Some(task) = guard.pop_front() {
                    // Check pacing
                    let now = Instant::now();
                    if send_time > now {
                        tokio::time::sleep(send_time - now).await;
                        send_time = Instant::now();
                    }

                    let data = match task {
                        TransmissionTask::Data { data, .. } |
                        TransmissionTask::Retransmit { data, .. } => data,
                        TransmissionTask::Fec { packet } => {
                            // Encode FEC packet
                            Self::encode_fec_packet(&packet)
                        }
                    };

                    let size = data.len();
                    match endpoint.send_datagram(data.into()) {
                        Ok(_) => {
                            send_time = Instant::now();
                            burst_bytes += size;

                            if burst_bytes >= pacing.burst_capacity() {
                                let delay = pacing.calculate_delay(size);
                                tokio::time::sleep(delay).await;
                                burst_bytes = 0;
                            }
                        }
                        Err(e) => {
                            eprintln!("Send failed: {}", e);
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Send a data packet with FEC
    pub async fn send(&mut self, sequence: u32, data: Bytes) -> Result<(), TxError> {
        // Add data to FEC encoder
        let fec_packets = self.fec_encoder.add_data_packet(data.clone());

        // Queue data packet
        {
            let mut queue = self.send_queue.lock().await;
            queue.push_back(TransmissionTask::Data {
                sequence,
                data: data.to_vec(),
            });
        }

        // Queue FEC packets
        for fec in fec_packets {
            let mut queue = self.send_queue.lock().await;
            queue.push_back(TransmissionTask::Fec { packet: fec });
        }

        self.notify_send.notify_one();
        Ok(())
    }

    /// Request retransmission via NACK
    pub async fn request_retransmit(&self, message: NackMessage) -> Result<(), TxError> {
        if !self.nack_config.enabled {
            return Ok(());
        }

        // Queue retransmission request (simplified)
        for seq in message.sequence_numbers {
            let mut queue = self.send_queue.lock().await;
            // In production, would request from remote
            queue.push_back(TransmissionTask::Retransmit {
                sequence: seq,
                data: vec![],
            });
        }

        self.notify_send.notify_one();
        Ok(())
    }

    /// Flush pending FEC packets
    pub async fn flush(&mut self) -> Result<(), TxError> {
        let fec_packets = self.fec_encoder.flush();
        for fec in fec_packets {
            let mut queue = self.send_queue.lock().await;
            queue.push_back(TransmissionTask::Fec { packet: fec });
        }
        self.notify_send.notify_one();
        Ok(())
    }

    fn encode_fec_packet(packet: &FecPacket) -> Vec<u8> {
        // FEC packet format: [base_seq(4) | block_size(2) | parity_idx(2) | data(n)]
        let mut buffer = Vec::with_capacity(8 + packet.data.len());
        buffer.extend_from_slice(&packet.base_sequence.to_le_bytes());
        buffer.extend_from_slice(&packet.block_size.to_le_bytes());
        buffer.extend_from_slice(&packet.parity_index.to_le_bytes());
        buffer.extend_from_slice(&packet.data);
        buffer
    }
}

/// Low-latency reception manager
#[allow(dead_code)]
pub struct LowLatencyRx {
    endpoint: QuinnDatagramEndpoint,
    #[allow(dead_code)]
    fec_config: FecConfig,
    #[allow(dead_code)]
    nack_config: NackConfig,
    fec_decoder: FecDecoder,
    nack_receiver: NackReceiver,
    #[allow(dead_code)]
    received_sequence: u32,
}

impl LowLatencyRx {
    /// Create a new low-latency receiver
    pub fn new(
        endpoint: QuinnDatagramEndpoint,
        fec_config: FecConfig,
        nack_config: NackConfig,
    ) -> Self {
        Self {
            endpoint,
            fec_decoder: FecDecoder::new(fec_config.clone()),
            nack_receiver: NackReceiver::new(nack_config.clone(), 0),
            fec_config,
            nack_config,
            received_sequence: 0,
        }
    }

    /// Receive and process packets
    pub async fn recv(&mut self) -> Result<Vec<Bytes>, RxError> {
        loop {
            let datagram = self.endpoint.read_datagram().await
                .map_err(|e| RxError::ReceiveFailed(e.to_string()))?;

            // Try to parse as FEC packet first
            if let Ok(fec) = Self::parse_fec_packet(&datagram) {
                let recovered = self.fec_decoder.process_fec(fec);
                if !recovered.is_empty() {
                    return Ok(recovered.into_iter().map(|b| b.to_vec().into()).collect());
                }
                continue;
            }

            // Treat as data packet with sequence
            // Simplified: assume first 4 bytes are sequence
            if datagram.len() >= 4 {
                let seq = u32::from_le_bytes([datagram[0], datagram[1], datagram[2], datagram[3]]);
                let data = Bytes::copy_from_slice(&datagram[4..]);

                // Process NACK tracking (borrow data for tracking)
                if self.nack_receiver.process_packet(seq, &data).is_some() {
                    // Send NACK (would queue for transmission)
                    self.nack_receiver.update_nack_sent();
                }

                // Process FEC decoder (move data)
                let recovered = self.fec_decoder.process_data(seq, data);
                if !recovered.is_empty() {
                    return Ok(recovered.into_iter().map(|b| b.to_vec().into()).collect());
                }

                // If no recovery needed, return empty (data was consumed)
                // In practice, would return something here
            }
        }
    }

    fn parse_fec_packet(datagram: &[u8]) -> Result<FecPacket, ()> {
        if datagram.len() < 8 {
            return Err(());
        }

        let base_sequence = u32::from_le_bytes([datagram[0], datagram[1], datagram[2], datagram[3]]);
        let block_size = u16::from_le_bytes([datagram[4], datagram[5]]);
        let parity_index = u16::from_le_bytes([datagram[6], datagram[7]]);
        let data = datagram[8..].to_vec();

        Ok(FecPacket {
            base_sequence,
            block_size,
            parity_index,
            data,
        })
    }
}

/// Transmission errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxError {
    SendFailed(String),
    InvalidSequence,
}

/// Reception errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RxError {
    ReceiveFailed(String),
    InvalidFecPacket,
    DecoderError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacing_config_calculates_delay_for_packet() {
        let config = PacingConfig {
            enabled: true,
            target_bitrate_bps: 10_000_000, // 10 Mbps
            max_burst_bytes: 64 * 1024,
            min_packet_interval: Duration::from_micros(100),
        };

        // 1500 byte packet at 10 Mbps = ~1.2ms
        let delay = config.calculate_delay(1500);
        assert!(delay >= Duration::from_micros(100));
        assert!(delay <= Duration::from_millis(2));
    }

    #[test]
    fn pacing_disabled_returns_zero_delay() {
        let config = PacingConfig {
            enabled: false,
            ..Default::default()
        };

        let delay = config.calculate_delay(1500);
        assert_eq!(delay, Duration::ZERO);
    }

    #[test]
    fn fec_encoder_generates_parity_after_block_full() {
        let config = FecConfig {
            enabled: true,
            scheme: FecScheme::Xor,
            block_size: 3,
            parity_count: 1,
        };

        let mut encoder = FecEncoder::new(config);

        // First two packets don't generate parity
        let p1 = encoder.add_data_packet(Bytes::from(&[1, 2, 3][..]));
        assert!(p1.is_empty());

        let p2 = encoder.add_data_packet(Bytes::from(&[4, 5, 6][..]));
        assert!(p2.is_empty());

        // Third packet completes block
        let p3 = encoder.add_data_packet(Bytes::from(&[7, 8, 9][..]));
        assert_eq!(p3.len(), 1);

        // Verify XOR: [1^4^7, 2^5^8, 3^6^9] = [2, 15, 12]
        assert_eq!(p3[0].data, vec![2, 15, 12]);
    }

    #[test]
    fn fec_decoder_recovers_missing_packet_with_xor() {
        let config = FecConfig {
            enabled: true,
            scheme: FecScheme::Xor,
            block_size: 3,
            parity_count: 1,
        };

        let mut decoder = FecDecoder::new(config);

        // Receive packet 0 and 2, missing 1
        decoder.process_data(0, Bytes::from(&[1, 2, 3][..]));
        let r1 = decoder.process_data(2, Bytes::from(&[7, 8, 9][..]));
        assert!(r1.is_empty()); // Not enough yet

        // Receive FEC packet
        let fec = FecPacket {
            base_sequence: 0,
            block_size: 3,
            parity_index: 0,
            data: vec![2, 15, 12], // XOR of all three
        };

        let recovered = decoder.process_fec(fec);
        assert_eq!(recovered.len(), 3);

        // Should have original packets in order
        assert_eq!(recovered[0].to_vec(), vec![1, 2, 3]);
        assert_eq!(recovered[1].to_vec(), vec![4, 5, 6]); // Recovered
        assert_eq!(recovered[2].to_vec(), vec![7, 8, 9]);
    }

    #[test]
    fn nack_receiver_tracks_missing_sequences() {
        let config = NackConfig {
            enabled: true,
            ..Default::default()
        };

        let mut receiver = NackReceiver::new(config, 0);

        // Receive seq 0, then seq 3 (gap of 1, 2)
        let n0 = receiver.process_packet(0, &[1, 2, 3]);
        assert!(n0.is_none());

        let n3 = receiver.process_packet(3, &[4, 5, 6]);
        assert!(n3.is_some());

        // Should have missing sequences 1 and 2
        assert!(receiver.missing_sequences.contains(&1));
        assert!(receiver.missing_sequences.contains(&2));
    }

    #[test]
    fn nack_receiver_removes_recovered_sequence_from_missing() {
        let config = NackConfig::default();
        let mut receiver = NackReceiver::new(config, 0);

        // Create gap
        receiver.process_packet(0, &[1, 2, 3]);
        receiver.process_packet(3, &[4, 5, 6]);

        assert!(receiver.missing_sequences.contains(&1));

        // Later receive missing packet
        let _n1 = receiver.process_packet(1, &[7, 8, 9]);
        assert!(!receiver.missing_sequences.contains(&1));
    }

    #[test]
    fn fec_config_defaults_are_sensible() {
        let config = FecConfig::default();

        assert!(config.enabled);
        assert!(config.block_size >= 5);
        assert!(config.parity_count >= 1);
        assert!(config.parity_count < config.block_size);
    }

    #[test]
    fn nack_config_defaults_are_sensible() {
        let config = NackConfig::default();

        assert!(config.enabled);
        assert!(config.max_nacked_seqs > 0);
        assert!(config.nack_timeout >= Duration::from_millis(10));
        assert!(config.max_nack_retries > 0);
    }

    #[test]
    fn nack_message_stores_sequence_numbers() {
        let msg = NackMessage {
            sequence_numbers: vec![1, 5, 10],
        };

        assert_eq!(msg.sequence_numbers.len(), 3);
        assert!(msg.sequence_numbers.contains(&5));
    }

    #[test]
    fn fec_scheme_variants_exist() {
        let _ = FecScheme::Xor;
        let _ = FecScheme::ReedSolomon;
    }
}
