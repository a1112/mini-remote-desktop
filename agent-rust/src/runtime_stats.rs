use crate::net_adapt::{NetAdaptController, tier_reason_label};
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;

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
            let encode_fps = (encoded_total.saturating_sub(last_encoded) as f64 / dt) as f32;
            let send_fps = (sent_total.saturating_sub(last_sent) as f64 / dt) as f32;
            let unique_send_fps =
                (unique_sent_total.saturating_sub(last_unique_sent) as f64 / dt) as f32;
            let repeat_send_fps =
                (repeated_sent_total.saturating_sub(last_repeated_sent) as f64 / dt) as f32;
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
                "[RTCP-PANEL]"
            );
        }
    });
}
