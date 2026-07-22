use mrd_proto::SessionId;
use std::collections::HashMap;

/// Probe telemetry accumulated from LAN data-plane probe frames.
#[derive(Debug, Default)]
pub struct ProbeRegistry {
    probes: HashMap<SessionId, SessionProbeStats>,
}

#[derive(Debug, Clone, Default)]
struct SessionProbeStats {
    frames_received: u64,
    frames_decoded: u64,
    frames_dropped: u64,
    sequence_gap_drops: u64,
    decode_error_drops: u64,
    transient_drops: u64,
    bytes_received: u64,
    first_seen_ms: Option<u64>,
    last_seen_ms: Option<u64>,
    media_probe_valid: bool,
    media_probe_format: Option<String>,
    media_probe_width: Option<u32>,
    media_probe_height: Option<u32>,
    media_probe_target_fps: Option<u32>,
    media_probe_target_bitrate_mbps: Option<u32>,
    media_probe_payload_bytes: Option<u32>,
    last_media_sequence: Option<u64>,
    last_media_timestamp_us: Option<u64>,
    last_media_payload_hash: Option<String>,
    latest_frame: Option<DecodedPreviewFrame>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct DecodedPreviewFrame {
    width: u32,
    height: u32,
    pixel_format: String,
}

#[derive(Debug, Clone)]
pub struct MediaProbeFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub payload_bytes: u32,
    pub format: String,
    pub payload_hash: String,
}

#[derive(Debug, Clone)]
pub struct DecodedVideoFrameStats {
    pub bytes_received: u64,
    pub sequence: u64,
    pub timestamp_us: u64,
    pub width: u32,
    pub height: u32,
    pub target_fps: u32,
    pub target_bitrate_mbps: u32,
    pub encoded_bytes: u32,
    pub format: String,
    pub pixel_format: String,
    pub payload_hash: String,
    pub preview_width: Option<u32>,
    pub preview_height: Option<u32>,
    pub rgb24: Option<Vec<u8>>,
}

impl ProbeRegistry {
    pub fn record_probe_frame(&mut self, session_id: &SessionId, bytes_received: u64, now_ms: u64) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = None;
    }

    pub fn record_media_probe_frame(
        &mut self,
        session_id: &SessionId,
        frame: MediaProbeFrameStats,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        if let Some(last_sequence) = stats.last_media_sequence {
            if frame.sequence > last_sequence.saturating_add(1) {
                let missing = frame.sequence.saturating_sub(last_sequence + 1);
                stats.frames_dropped = stats.frames_dropped.saturating_add(missing);
                stats.sequence_gap_drops = stats.sequence_gap_drops.saturating_add(missing);
            }
        }

        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(frame.bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.media_probe_valid = true;
        stats.media_probe_format = Some(frame.format);
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.payload_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        stats.last_error = None;
    }

    pub fn record_decoded_video_frame(
        &mut self,
        session_id: &SessionId,
        frame: DecodedVideoFrameStats,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        if let Some(last_sequence) = stats.last_media_sequence {
            if frame.sequence > last_sequence.saturating_add(1) {
                let missing = frame.sequence.saturating_sub(last_sequence + 1);
                stats.frames_dropped = stats.frames_dropped.saturating_add(missing);
                stats.sequence_gap_drops = stats.sequence_gap_drops.saturating_add(missing);
            }
        }

        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_decoded = stats.frames_decoded.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(frame.bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.media_probe_valid = true;
        stats.media_probe_format = Some(frame.format);
        stats.media_probe_width = Some(frame.width);
        stats.media_probe_height = Some(frame.height);
        stats.media_probe_target_fps = Some(frame.target_fps);
        stats.media_probe_target_bitrate_mbps = Some(frame.target_bitrate_mbps);
        stats.media_probe_payload_bytes = Some(frame.encoded_bytes);
        stats.last_media_sequence = Some(frame.sequence);
        stats.last_media_timestamp_us = Some(frame.timestamp_us);
        stats.last_media_payload_hash = Some(frame.payload_hash);
        stats.latest_frame = Some(DecodedPreviewFrame {
            width: frame.preview_width.unwrap_or(frame.width),
            height: frame.preview_height.unwrap_or(frame.height),
            pixel_format: frame.pixel_format,
        });
        stats.last_error = None;
    }

    pub fn record_probe_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
        error: impl Into<String>,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.decode_error_drops = stats.decode_error_drops.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
        stats.last_error = Some(error.into());
    }

    pub fn record_transient_frame_drop(
        &mut self,
        session_id: &SessionId,
        bytes_received: u64,
        now_ms: u64,
    ) {
        let stats = self.probes.entry(session_id.clone()).or_default();
        stats.frames_received = stats.frames_received.saturating_add(1);
        stats.frames_dropped = stats.frames_dropped.saturating_add(1);
        stats.transient_drops = stats.transient_drops.saturating_add(1);
        stats.bytes_received = stats.bytes_received.saturating_add(bytes_received);
        stats.first_seen_ms.get_or_insert(now_ms);
        stats.last_seen_ms = Some(now_ms);
    }

    pub fn snapshot(&self, session_id: &SessionId) -> mrd_ipc::ProbeSnapshot {
        let Some(stats) = self.probes.get(session_id) else {
            return mrd_ipc::ProbeSnapshot {
                session_id: session_id.clone(),
                frames_received: 0,
                frames_decoded: 0,
                frames_dropped: 0,
                sequence_gap_drops: 0,
                decode_error_drops: 0,
                transient_drops: 0,
                current_fps: None,
                bitrate_mbps: None,
                media_probe_valid: false,
                media_probe_format: None,
                media_probe_width: None,
                media_probe_height: None,
                media_probe_target_fps: None,
                media_probe_target_bitrate_mbps: None,
                media_probe_payload_bytes: None,
                last_media_sequence: None,
                last_media_timestamp_us: None,
                last_media_payload_hash: None,
                latest_frame_data_url: None,
                latest_frame_width: None,
                latest_frame_height: None,
                latest_frame_pixel_format: None,
                last_error: None,
            };
        };

        let elapsed_ms = match (stats.first_seen_ms, stats.last_seen_ms) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        };
        let current_fps = if elapsed_ms > 0 {
            Some((stats.frames_decoded as f32 * 1000.0) / elapsed_ms as f32)
        } else {
            Some(0.0)
        };
        let bitrate_mbps = if elapsed_ms > 0 {
            Some((stats.bytes_received as f32 * 8.0) / elapsed_ms as f32 / 1000.0)
        } else {
            Some(0.0)
        };

        mrd_ipc::ProbeSnapshot {
            session_id: session_id.clone(),
            frames_received: stats.frames_received,
            frames_decoded: stats.frames_decoded,
            frames_dropped: stats.frames_dropped,
            sequence_gap_drops: stats.sequence_gap_drops,
            decode_error_drops: stats.decode_error_drops,
            transient_drops: stats.transient_drops,
            current_fps,
            bitrate_mbps,
            media_probe_valid: stats.media_probe_valid,
            media_probe_format: stats.media_probe_format.clone(),
            media_probe_width: stats.media_probe_width,
            media_probe_height: stats.media_probe_height,
            media_probe_target_fps: stats.media_probe_target_fps,
            media_probe_target_bitrate_mbps: stats.media_probe_target_bitrate_mbps,
            media_probe_payload_bytes: stats.media_probe_payload_bytes,
            last_media_sequence: stats.last_media_sequence,
            last_media_timestamp_us: stats.last_media_timestamp_us,
            last_media_payload_hash: stats.last_media_payload_hash.clone(),
            latest_frame_data_url: None,
            latest_frame_width: stats.latest_frame.as_ref().map(|frame| frame.width),
            latest_frame_height: stats.latest_frame.as_ref().map(|frame| frame.height),
            latest_frame_pixel_format: stats
                .latest_frame
                .as_ref()
                .map(|frame| frame.pixel_format.clone()),
            last_error: stats.last_error.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::SessionId;

    #[test]
    fn decoded_video_sequence_gap_counts_missing_frames() {
        let mut registry = ProbeRegistry::default();
        let session_id = SessionId("probe-gap-session".to_string());

        registry.record_decoded_video_frame(&session_id, decoded_frame(1), 1_000);
        registry.record_decoded_video_frame(&session_id, decoded_frame(4), 1_030);

        let snapshot = registry.snapshot(&session_id);
        assert_eq!(snapshot.frames_received, 2);
        assert_eq!(snapshot.frames_dropped, 2);
        assert_eq!(snapshot.sequence_gap_drops, 2);
        assert_eq!(snapshot.last_media_sequence, Some(4));
    }

    fn decoded_frame(sequence: u64) -> DecodedVideoFrameStats {
        DecodedVideoFrameStats {
            bytes_received: 512,
            sequence,
            timestamp_us: sequence * 1_000,
            width: 1280,
            height: 720,
            target_fps: 60,
            target_bitrate_mbps: 20,
            encoded_bytes: 256,
            format: "h264_desktop_frame".to_string(),
            pixel_format: "nv12".to_string(),
            payload_hash: format!("hash-{sequence}"),
            preview_width: None,
            preview_height: None,
            rgb24: None,
        }
    }
}
