use crate::net_adapt::{NetAdaptController, tier_reason_label};
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;

const LATENCY_WINDOW_SAMPLES: usize = 4096;

#[derive(Clone, Copy, Debug, Default)]
pub struct PercentilePairMs {
    pub p50: f64,
    pub p95: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransportLatencyPercentilesMs {
    pub capture: PercentilePairMs,
    pub encode: PercentilePairMs,
    pub queue_wait: PercentilePairMs,
    pub send: PercentilePairMs,
}

#[derive(Default)]
pub struct RuntimeStats {
    pub pli_count: AtomicU64,
    pub fir_count: AtomicU64,
    pub nack_count: AtomicU64,
    pub remb_count: AtomicU64,
    pub last_remb_kbps: AtomicU32,
    pub target_fps: AtomicU32,
    pub target_bitrate_kbps: AtomicU32,
    pub rtp_au_sent: AtomicU64,
    pub rtp_au_skipped: AtomicU64,
    pub encoded_au_total: AtomicU64,
    pub sent_au_total: AtomicU64,
    pub unique_sent_au_total: AtomicU64,
    pub repeated_sent_au_total: AtomicU64,
    pub tier_level: AtomicU32,
    pub tier_reason_code: AtomicU32,
    pub tier_switch_count: AtomicU64,
    pub native_direct_frames: AtomicU64,
    pub native_copy_frames: AtomicU64,
    pub native_scale_frames: AtomicU64,
    pub native_direct_register_failures: AtomicU64,
    pub native_acquire_ok: AtomicU64,
    pub native_acquire_timeout: AtomicU64,
    pub native_acquire_errors: AtomicU64,
    pub quic_au_sent: AtomicU64,
    pub quic_au_dropped: AtomicU64,
    pub quic_bytes_sent: AtomicU64,
    pub transport_enqueue_wait_us_total: AtomicU64,
    pub transport_capture_to_send_us_total: AtomicU64,
    pub transport_encode_approx_us_total: AtomicU64,
    pub transport_capture_encode_samples: AtomicU64,
    pub transport_send_us_total: AtomicU64,
    pub transport_send_samples: AtomicU64,
    transport_capture_us_window: Mutex<VecDeque<u32>>,
    transport_encode_us_window: Mutex<VecDeque<u32>>,
    transport_queue_wait_us_window: Mutex<VecDeque<u32>>,
    transport_send_us_window: Mutex<VecDeque<u32>>,
}

impl RuntimeStats {
    pub fn new(target_fps: u32, target_bitrate_kbps: u32) -> Self {
        Self {
            target_fps: AtomicU32::new(target_fps),
            target_bitrate_kbps: AtomicU32::new(target_bitrate_kbps),
            tier_level: AtomicU32::new(0),
            tier_reason_code: AtomicU32::new(0),
            tier_switch_count: AtomicU64::new(0),
            ..Default::default()
        }
    }

    fn push_window(window: &Mutex<VecDeque<u32>>, value_us: u64) {
        let mut guard = match window.lock() {
            Ok(v) => v,
            Err(_) => return,
        };
        guard.push_back(value_us.min(u32::MAX as u64) as u32);
        if guard.len() > LATENCY_WINDOW_SAMPLES {
            let drop_n = guard.len().saturating_sub(LATENCY_WINDOW_SAMPLES);
            guard.drain(..drop_n);
        }
    }

    fn percentile_pair_ms(window: &Mutex<VecDeque<u32>>) -> PercentilePairMs {
        let guard = match window.lock() {
            Ok(v) => v,
            Err(_) => return PercentilePairMs::default(),
        };
        if guard.is_empty() {
            return PercentilePairMs::default();
        }
        let mut sorted: Vec<u32> = guard.iter().copied().collect();
        sorted.sort_unstable();
        let idx = |p: f64| -> usize {
            ((sorted.len() as f64 * p).floor() as usize).min(sorted.len().saturating_sub(1))
        };
        PercentilePairMs {
            p50: sorted[idx(0.50)] as f64 / 1000.0,
            p95: sorted[idx(0.95)] as f64 / 1000.0,
        }
    }

    pub fn record_transport_queue_wait_us(&self, queue_wait_us: u64) {
        Self::push_window(&self.transport_queue_wait_us_window, queue_wait_us);
    }

    pub fn record_transport_send_us(&self, send_us: u64) {
        Self::push_window(&self.transport_send_us_window, send_us);
    }

    pub fn record_transport_capture_encode_us(&self, capture_us: u64, encode_us: u64) {
        Self::push_window(&self.transport_capture_us_window, capture_us);
        Self::push_window(&self.transport_encode_us_window, encode_us);
    }

    pub fn transport_latency_percentiles_ms(&self) -> TransportLatencyPercentilesMs {
        TransportLatencyPercentilesMs {
            capture: Self::percentile_pair_ms(&self.transport_capture_us_window),
            encode: Self::percentile_pair_ms(&self.transport_encode_us_window),
            queue_wait: Self::percentile_pair_ms(&self.transport_queue_wait_us_window),
            send: Self::percentile_pair_ms(&self.transport_send_us_window),
        }
    }
}

pub fn spawn_rtcp_feedback_loop(
    sender: Arc<RTCRtpSender>,
    keyframe_request: Arc<AtomicBool>,
    adapt: Arc<NetAdaptController>,
    stats: Arc<RuntimeStats>,
    enable_network_adapt: bool,
    force_idr_on_pli: bool,
) {
    tokio::spawn(async move {
        loop {
            let read = sender.read_rtcp().await;
            let (pkts, _) = match read {
                Ok(v) => v,
                Err(e) => {
                    let msg = e.to_string();
                    // Session switch or peer close can surface as transport/datachannel closed.
                    // Treat these as expected shutdown signals to keep logs actionable.
                    if msg.contains("DataChannel is not opened")
                        || msg.contains("SessionSRTP has been closed")
                        || msg.contains("closed")
                    {
                        tracing::info!(error = %e, "rtcp read stopped (session closed)");
                    } else {
                        tracing::warn!(error = %e, "rtcp read stopped");
                    }
                    break;
                }
            };
            for pkt in pkts {
                if pkt
                    .as_any()
                    .downcast_ref::<PictureLossIndication>()
                    .is_some()
                {
                    stats.pli_count.fetch_add(1, Ordering::Relaxed);
                    tracing::info!("rtcp pli");
                    if force_idr_on_pli {
                        keyframe_request.store(true, Ordering::Relaxed);
                    }
                    continue;
                }
                if pkt.as_any().downcast_ref::<FullIntraRequest>().is_some() {
                    stats.fir_count.fetch_add(1, Ordering::Relaxed);
                    tracing::info!("rtcp fir");
                    keyframe_request.store(true, Ordering::Relaxed);
                    continue;
                }
                if let Some(nack) = pkt.as_any().downcast_ref::<TransportLayerNack>() {
                    stats.nack_count.fetch_add(1, Ordering::Relaxed);
                    if enable_network_adapt {
                        let (target_fps, target_bitrate_kbps) = adapt.on_nack_burst();
                        stats.target_fps.store(target_fps, Ordering::Relaxed);
                        stats
                            .target_bitrate_kbps
                            .store(target_bitrate_kbps, Ordering::Relaxed);
                        stats
                            .tier_level
                            .store(adapt.current_tier_level(), Ordering::Relaxed);
                        stats
                            .tier_reason_code
                            .store(adapt.tier_reason_code(), Ordering::Relaxed);
                        stats
                            .tier_switch_count
                            .store(adapt.tier_switch_count(), Ordering::Relaxed);
                        tracing::info!(
                            sender_ssrc = nack.sender_ssrc,
                            media_ssrc = nack.media_ssrc,
                            target_fps,
                            target_bitrate_kbps,
                            "rtcp nack"
                        );
                    } else {
                        tracing::info!(
                            sender_ssrc = nack.sender_ssrc,
                            media_ssrc = nack.media_ssrc,
                            "rtcp nack"
                        );
                    }
                    continue;
                }
                if let Some(remb) = pkt
                    .as_any()
                    .downcast_ref::<ReceiverEstimatedMaximumBitrate>()
                {
                    stats.remb_count.fetch_add(1, Ordering::Relaxed);
                    stats
                        .last_remb_kbps
                        .store((remb.bitrate / 1000.0) as u32, Ordering::Relaxed);
                    if enable_network_adapt {
                        let (target_fps, target_bitrate_kbps) = adapt.on_remb_bps(remb.bitrate);
                        stats.target_fps.store(target_fps, Ordering::Relaxed);
                        stats
                            .target_bitrate_kbps
                            .store(target_bitrate_kbps, Ordering::Relaxed);
                        stats
                            .tier_level
                            .store(adapt.current_tier_level(), Ordering::Relaxed);
                        stats
                            .tier_reason_code
                            .store(adapt.tier_reason_code(), Ordering::Relaxed);
                        stats
                            .tier_switch_count
                            .store(adapt.tier_switch_count(), Ordering::Relaxed);
                        tracing::info!(
                            bitrate_bps = remb.bitrate,
                            target_fps,
                            target_bitrate_kbps,
                            "rtcp remb"
                        );
                    } else {
                        tracing::info!(bitrate_bps = remb.bitrate, "rtcp remb");
                    }
                }
            }
        }
    });
}

pub fn spawn_stats_panel(
    stats: Arc<RuntimeStats>,
    adapt: Arc<NetAdaptController>,
    interval_ms: u32,
    running: Arc<AtomicBool>,
) {
    let interval_ms = interval_ms.clamp(200, 10_000) as u64;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        let mut last_ts = std::time::Instant::now();
        let mut last_encoded = 0_u64;
        let mut last_sent = 0_u64;
        let mut last_unique_sent = 0_u64;
        let mut last_repeated_sent = 0_u64;
        let mut last_enqueue_wait_us_total = 0_u64;
        let mut last_capture_to_send_us_total = 0_u64;
        let mut last_encode_approx_us_total = 0_u64;
        let mut last_capture_encode_samples = 0_u64;
        let mut last_transport_send_us_total = 0_u64;
        let mut last_transport_send_samples = 0_u64;
        while running.load(Ordering::SeqCst) {
            ticker.tick().await;
            let now = std::time::Instant::now();
            let dt = (now - last_ts).as_secs_f64().max(0.001);
            last_ts = now;
            let pli = stats.pli_count.load(Ordering::Relaxed);
            let fir = stats.fir_count.load(Ordering::Relaxed);
            let nack = stats.nack_count.load(Ordering::Relaxed);
            let remb = stats.remb_count.load(Ordering::Relaxed);
            let remb_kbps = stats.last_remb_kbps.load(Ordering::Relaxed);
            let target_fps = adapt.current_fps();
            let target_bitrate_kbps = stats.target_bitrate_kbps.load(Ordering::Relaxed);
            let sent = stats.rtp_au_sent.load(Ordering::Relaxed);
            let skipped = stats.rtp_au_skipped.load(Ordering::Relaxed);
            let encoded_total = stats.encoded_au_total.load(Ordering::Relaxed);
            let sent_total = stats.sent_au_total.load(Ordering::Relaxed);
            let unique_sent_total = stats.unique_sent_au_total.load(Ordering::Relaxed);
            let repeated_sent_total = stats.repeated_sent_au_total.load(Ordering::Relaxed);
            let enqueue_wait_us_total = stats
                .transport_enqueue_wait_us_total
                .load(Ordering::Relaxed);
            let capture_to_send_us_total = stats
                .transport_capture_to_send_us_total
                .load(Ordering::Relaxed);
            let encode_approx_us_total = stats
                .transport_encode_approx_us_total
                .load(Ordering::Relaxed);
            let capture_encode_samples = stats
                .transport_capture_encode_samples
                .load(Ordering::Relaxed);
            let transport_send_us_total = stats.transport_send_us_total.load(Ordering::Relaxed);
            let transport_send_samples = stats.transport_send_samples.load(Ordering::Relaxed);
            let encode_fps = (encoded_total.saturating_sub(last_encoded) as f64 / dt) as f32;
            let send_fps = (sent_total.saturating_sub(last_sent) as f64 / dt) as f32;
            let unique_send_fps =
                (unique_sent_total.saturating_sub(last_unique_sent) as f64 / dt) as f32;
            let repeat_send_fps =
                (repeated_sent_total.saturating_sub(last_repeated_sent) as f64 / dt) as f32;
            let enqueue_wait_us_delta =
                enqueue_wait_us_total.saturating_sub(last_enqueue_wait_us_total);
            let capture_to_send_us_delta =
                capture_to_send_us_total.saturating_sub(last_capture_to_send_us_total);
            let encode_approx_us_delta =
                encode_approx_us_total.saturating_sub(last_encode_approx_us_total);
            let capture_encode_samples_delta =
                capture_encode_samples.saturating_sub(last_capture_encode_samples);
            let transport_send_us_delta =
                transport_send_us_total.saturating_sub(last_transport_send_us_total);
            let transport_send_samples_delta =
                transport_send_samples.saturating_sub(last_transport_send_samples);
            let enqueue_wait_avg_us = if encoded_total > last_encoded {
                enqueue_wait_us_delta as f64 / (encoded_total.saturating_sub(last_encoded)) as f64
            } else {
                0.0
            };
            let transport_send_avg_us = if transport_send_samples_delta > 0 {
                transport_send_us_delta as f64 / transport_send_samples_delta as f64
            } else {
                0.0
            };
            let p = stats.transport_latency_percentiles_ms();
            let capture_to_send_avg_us = if capture_encode_samples_delta > 0 {
                capture_to_send_us_delta as f64 / capture_encode_samples_delta as f64
            } else {
                0.0
            };
            let encode_approx_avg_us = if capture_encode_samples_delta > 0 {
                encode_approx_us_delta as f64 / capture_encode_samples_delta as f64
            } else {
                0.0
            };
            if let Some((new_fps, new_bitrate)) = adapt.on_quality_sample(unique_send_fps) {
                stats.target_fps.store(new_fps, Ordering::Relaxed);
                stats
                    .target_bitrate_kbps
                    .store(new_bitrate, Ordering::Relaxed);
            }
            let tier_level = adapt.current_tier_level();
            let tier_reason_code = adapt.tier_reason_code();
            let tier_reason = tier_reason_label(tier_reason_code);
            let tier_switch_count = adapt.tier_switch_count();
            stats.tier_level.store(tier_level, Ordering::Relaxed);
            stats
                .tier_reason_code
                .store(tier_reason_code, Ordering::Relaxed);
            stats
                .tier_switch_count
                .store(tier_switch_count, Ordering::Relaxed);
            last_encoded = encoded_total;
            last_sent = sent_total;
            last_unique_sent = unique_sent_total;
            last_repeated_sent = repeated_sent_total;
            last_enqueue_wait_us_total = enqueue_wait_us_total;
            last_capture_to_send_us_total = capture_to_send_us_total;
            last_encode_approx_us_total = encode_approx_us_total;
            last_capture_encode_samples = capture_encode_samples;
            last_transport_send_us_total = transport_send_us_total;
            last_transport_send_samples = transport_send_samples;
            tracing::info!(
                pli,
                fir,
                nack,
                remb,
                remb_kbps,
                target_fps,
                target_bitrate_kbps,
                au_sent = sent,
                au_skipped = skipped,
                encoded_total,
                sent_total,
                unique_sent_total,
                repeated_sent_total,
                encode_fps,
                send_fps,
                unique_send_fps,
                repeat_send_fps,
                tier_level,
                tier_reason,
                tier_switch_count,
                native_direct_frames = stats.native_direct_frames.load(Ordering::Relaxed),
                native_copy_frames = stats.native_copy_frames.load(Ordering::Relaxed),
                native_scale_frames = stats.native_scale_frames.load(Ordering::Relaxed),
                native_direct_register_failures = stats
                    .native_direct_register_failures
                    .load(Ordering::Relaxed),
                native_acquire_ok = stats.native_acquire_ok.load(Ordering::Relaxed),
                native_acquire_timeout = stats.native_acquire_timeout.load(Ordering::Relaxed),
                native_acquire_errors = stats.native_acquire_errors.load(Ordering::Relaxed),
                quic_au_sent = stats.quic_au_sent.load(Ordering::Relaxed),
                quic_au_dropped = stats.quic_au_dropped.load(Ordering::Relaxed),
                quic_bytes_sent = stats.quic_bytes_sent.load(Ordering::Relaxed),
                capture_ms = format!("{:.3}", capture_to_send_avg_us / 1000.0),
                capture_p50_ms = format!("{:.3}", p.capture.p50),
                capture_p95_ms = format!("{:.3}", p.capture.p95),
                encode_ms = format!("{:.3}", encode_approx_avg_us / 1000.0),
                encode_p50_ms = format!("{:.3}", p.encode.p50),
                encode_p95_ms = format!("{:.3}", p.encode.p95),
                queue_wait_ms = format!("{:.3}", enqueue_wait_avg_us / 1000.0),
                queue_wait_p50_ms = format!("{:.3}", p.queue_wait.p50),
                queue_wait_p95_ms = format!("{:.3}", p.queue_wait.p95),
                send_ms = format!("{:.3}", transport_send_avg_us / 1000.0),
                send_p50_ms = format!("{:.3}", p.send.p50),
                send_p95_ms = format!("{:.3}", p.send.p95),
                enqueue_wait_avg_us = format!("{enqueue_wait_avg_us:.1}"),
                transport_send_avg_us = format!("{transport_send_avg_us:.1}"),
                "[RTCP-PANEL]"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_latency_percentiles_track_recent_samples() {
        let stats = RuntimeStats::new(120, 12_000);
        for i in 1..=100 {
            stats.record_transport_queue_wait_us((i * 1000) as u64);
            stats.record_transport_send_us((i * 500) as u64);
            stats.record_transport_capture_encode_us((i * 2000) as u64, (i * 700) as u64);
        }
        let p = stats.transport_latency_percentiles_ms();
        assert!(p.queue_wait.p50 >= 50.0 && p.queue_wait.p95 >= 95.0);
        assert!(p.send.p50 >= 25.0 && p.send.p95 >= 47.5);
        assert!(p.capture.p50 >= 100.0 && p.capture.p95 >= 190.0);
        assert!(p.encode.p50 >= 35.0 && p.encode.p95 >= 66.5);
    }
}
