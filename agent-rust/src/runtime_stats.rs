use crate::net_adapt::NetAdaptController;
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
}

impl RuntimeStats {
    pub fn new(target_fps: u32, target_bitrate_kbps: u32) -> Self {
        Self {
            target_fps: AtomicU32::new(target_fps),
            target_bitrate_kbps: AtomicU32::new(target_bitrate_kbps),
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
                    tracing::error!(error = %e, "rtcp read stopped");
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
) {
    let interval_ms = interval_ms.clamp(200, 10_000) as u64;
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        loop {
            ticker.tick().await;
            let pli = stats.pli_count.load(Ordering::Relaxed);
            let fir = stats.fir_count.load(Ordering::Relaxed);
            let nack = stats.nack_count.load(Ordering::Relaxed);
            let remb = stats.remb_count.load(Ordering::Relaxed);
            let remb_kbps = stats.last_remb_kbps.load(Ordering::Relaxed);
            let target_fps = adapt.current_fps();
            let target_bitrate_kbps = stats.target_bitrate_kbps.load(Ordering::Relaxed);
            let sent = stats.rtp_au_sent.load(Ordering::Relaxed);
            let skipped = stats.rtp_au_skipped.load(Ordering::Relaxed);
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
                "[RTCP-PANEL]"
            );
        }
    });
}
