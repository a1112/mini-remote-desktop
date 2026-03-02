mod capture_policy;
mod capture_runtime;
mod encoder_policy;
mod encoder_runtime;
mod net_adapt;
mod nvenc_native;
mod profile;
mod rtp_send;
mod runtime_stats;

use crate::capture_policy::choose_backend;
use crate::capture_runtime::{
    RawFrame, build_frame_capturer, detect_input_resolution, resize_rgba_fast, sleep_until,
};
use crate::encoder_policy::{VideoEncoderBackend, choose_encoder_backend};
use crate::encoder_runtime::{build_video_encoder, encode_rgba_frame, request_keyframe};
use crate::net_adapt::NetAdaptController;
use crate::nvenc_native::NativeNvencPipeline;
use crate::profile::apply_capture_profile;
use crate::rtp_send::{RtpH264Sender, RtpH264SenderConfig};
use crate::runtime_stats::{RuntimeStats, spawn_rtcp_feedback_loop, spawn_stats_panel};
use agent_rust::{load_config, register_message};
use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use tracing::{error, info, warn};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::ice_transport::ice_candidate_pair::RTCIceCandidatePair;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::{
    RTCPFeedback, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK,
    TYPE_RTCP_FB_TRANSPORT_CC,
};
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

#[derive(Default)]
struct SessionState {
    controller_id: Option<String>,
    pc: Option<Arc<RTCPeerConnection>>,
}

type WsWrite = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "agent_rust=info,tokio=warn,webrtc=warn".to_string()),
        )
        .init();

    let mut cfg = load_config(&PathBuf::from("config.json"));
    if cfg.device_name == "Rust Agent" {
        if let Ok(host) = std::env::var("COMPUTERNAME") {
            if !host.trim().is_empty() {
                cfg.device_name = format!("{host} - Rust Agent");
            }
        }
    }

    info!(ws_url = %cfg.ws_url, "connecting to signaling server");
    info!(
        fps = cfg.capture.fps,
        backend = %cfg.capture.backend,
        encoder = %cfg.capture.encoder,
        allow_fallback = cfg.capture.allow_fallback,
        allow_encoder_fallback = cfg.capture.allow_encoder_fallback,
        "capture configuration"
    );

    let (ws, _) = connect_async(&cfg.ws_url)
        .await
        .with_context(|| format!("connect signaling failed: {}", cfg.ws_url))?;

    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));
    let session = Arc::new(Mutex::new(SessionState::default()));

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(v) => v,
            Err(e) => {
                error!(error = %e, "websocket read error");
                break;
            }
        };

        if !msg.is_text() {
            continue;
        }

        let text = msg.into_text().context("ws message not text")?;
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let typ = v["type"].as_str().unwrap_or("");
        let action = v["action"].as_str().unwrap_or("");

        if typ == "system" && action == "connected" {
            let reg = register_message(&cfg.device_name);
            ws_send_json(&write, &serde_json::from_str::<Value>(&reg)?).await?;
            info!(device_name = %cfg.device_name, "registered with signaling server");
            continue;
        }

        if typ == "webrtc" && action == "offer" {
            let payload = &v["payload"];
            let controller_id = payload["controllerId"].as_str().unwrap_or("").to_string();
            let offer_type = payload["offer"]["type"]
                .as_str()
                .unwrap_or("offer")
                .to_string();
            let offer_sdp = payload["offer"]["sdp"].as_str().unwrap_or("").to_string();
            if controller_id.is_empty() || offer_sdp.is_empty() {
                warn!("received invalid offer payload");
                continue;
            }

            let pc = create_peer_connection(write.clone(), controller_id.clone()).await?;
            attach_video_track_with_policy(pc.clone(), &cfg.capture).await?;

            pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?)
                .await
                .context("set remote offer failed")?;

            let answer = pc
                .create_answer(None)
                .await
                .context("create answer failed")?;
            pc.set_local_description(answer.clone())
                .await
                .context("set local answer failed")?;

            let msg = json!({
                "type": "webrtc",
                "action": "answer",
                "payload": {
                    "answer": { "type": offer_type.replace("offer", "answer"), "sdp": answer.sdp },
                    "controllerId": controller_id
                }
            });
            ws_send_json(&write, &msg).await?;
            info!("WebRTC answer sent");

            let mut s = session.lock().await;
            s.controller_id = Some(controller_id);
            s.pc = Some(pc);
            continue;
        }

        if typ == "webrtc" && action == "iceCandidate" {
            let candidate = &v["payload"]["candidate"];
            if candidate.is_null() {
                continue;
            }
            let mut s = session.lock().await;
            if let Some(pc) = &mut s.pc {
                let cand: webrtc::ice_transport::ice_candidate::RTCIceCandidateInit =
                    serde_json::from_value(candidate.clone()).context("parse remote ice failed")?;
                if let Err(e) = pc.add_ice_candidate(cand).await {
                    warn!(error = %e, "failed to add remote ice candidate");
                }
            }
        }
    }

    Ok(())
}

async fn create_peer_connection(
    ws_write: Arc<Mutex<WsWrite>>,
    controller_id: String,
) -> Result<Arc<RTCPeerConnection>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let api = APIBuilder::new().with_media_engine(m).build();

    let pc = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await
        .context("new peer connection failed")?,
    );

    {
        let ws_write = ws_write.clone();
        let controller_id = controller_id.clone();
        pc.on_ice_candidate(Box::new(move |cand| {
            let ws_write = ws_write.clone();
            let controller_id = controller_id.clone();
            Box::pin(async move {
                if let Some(c) = cand {
                    if let Ok(cjson) = c.to_json() {
                        let msg = json!({
                            "type": "webrtc",
                            "action": "iceCandidate",
                            "payload": {
                                "targetDeviceId": controller_id,
                                "candidate": cjson
                            }
                        });
                        if let Err(e) = ws_send_json(&ws_write, &msg).await {
                            warn!(error = %e, "failed to send local ICE candidate");
                        }
                    }
                }
            })
        }));
    }

    pc.on_peer_connection_state_change(Box::new(|s: RTCPeerConnectionState| {
        info!(state = %s, "peer connection state changed");
        Box::pin(async {})
    }));
    pc.on_ice_connection_state_change(Box::new(|s: RTCIceConnectionState| {
        info!(state = %s, "ice connection state changed");
        Box::pin(async {})
    }));
    pc.sctp()
        .transport()
        .ice_transport()
        .on_selected_candidate_pair_change(Box::new(move |p: RTCIceCandidatePair| {
            info!(pair = %p, "selected ICE candidate pair changed");
            Box::pin(async {})
        }));

    Ok(pc)
}

async fn attach_video_track_with_policy(
    pc: Arc<RTCPeerConnection>,
    capture_cfg: &agent_rust::CaptureConfig,
) -> Result<()> {
    let mut effective_cfg = capture_cfg.clone();
    apply_capture_profile(&mut effective_cfg);

    let (backend, logs) = choose_backend(capture_cfg);
    for line in logs {
        info!("{}", line);
    }
    let (encoder_backend, logs) = choose_encoder_backend(&effective_cfg);
    for line in logs {
        info!("{}", line);
    }

    let codec_cap = RTCRtpCodecCapability {
        mime_type: "video/H264".to_string(),
        clock_rate: 90_000,
        channels: 0,
        sdp_fmtp_line: "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
            .to_string(),
        rtcp_feedback: vec![
            RTCPFeedback {
                typ: TYPE_RTCP_FB_NACK.to_string(),
                parameter: "".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_NACK.to_string(),
                parameter: "pli".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_CCM.to_string(),
                parameter: "fir".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_GOOG_REMB.to_string(),
                parameter: "".to_string(),
            },
            RTCPFeedback {
                typ: TYPE_RTCP_FB_TRANSPORT_CC.to_string(),
                parameter: "".to_string(),
            },
        ],
    };

    let use_manual_packetizer = effective_cfg.rtp_use_manual_packetizer;
    let sample_track = if use_manual_packetizer {
        None
    } else {
        Some(Arc::new(TrackLocalStaticSample::new(
            codec_cap.clone(),
            "video".to_string(),
            "rust-agent".to_string(),
        )))
    };
    let rtp_track = if use_manual_packetizer {
        Some(Arc::new(TrackLocalStaticRTP::new(
            codec_cap,
            "video".to_string(),
            "rust-agent".to_string(),
        )))
    } else {
        None
    };
    let track: Arc<dyn TrackLocal + Send + Sync> = if let Some(t) = &rtp_track {
        t.clone()
    } else if let Some(t) = &sample_track {
        t.clone()
    } else {
        return Err(anyhow!("invalid track mode"));
    };

    let sender = pc
        .add_track(track)
        .await
        .context("add local video track failed")?;

    let keyframe_request = Arc::new(AtomicBool::new(false));
    let enable_network_adapt = effective_cfg.network_adapt_enable;
    let adapt_min_fps = effective_cfg.min_fps.max(1);
    let adapt_max_fps = effective_cfg
        .fps
        .max(1)
        .clamp(adapt_min_fps, effective_cfg.max_fps.max(1));
    let adapt = Arc::new(NetAdaptController::new(
        adapt_min_fps,
        adapt_max_fps,
        effective_cfg.fps.max(1),
        effective_cfg.network_adapt_floor_bitrate_kbps.max(100),
        effective_cfg
            .network_adapt_ceiling_bitrate_kbps
            .max(effective_cfg.network_adapt_floor_bitrate_kbps.max(100)),
        effective_cfg.bitrate_kbps.max(100),
    ));
    let stats = Arc::new(RuntimeStats::new(
        adapt.current_fps(),
        adapt.current_bitrate_kbps(),
    ));

    spawn_rtcp_feedback_loop(
        sender.clone(),
        keyframe_request.clone(),
        adapt.clone(),
        stats.clone(),
        enable_network_adapt,
        effective_cfg.force_idr_on_pli,
    );
    spawn_stats_panel(
        stats.clone(),
        adapt.clone(),
        effective_cfg.stats_interval_ms,
    );

    if encoder_backend == VideoEncoderBackend::Nvenc {
        let (input_w, input_h) = detect_input_resolution()?;
        let target_w = if effective_cfg.target_width > 0 {
            effective_cfg.target_width
        } else {
            input_w
        };
        let target_h = if effective_cfg.target_height > 0 {
            effective_cfg.target_height
        } else {
            input_h
        };
        match NativeNvencPipeline::new(target_w, target_h, &effective_cfg) {
            Ok(mut native) => {
                info!(
                    input_w,
                    input_h,
                    target_w,
                    target_h,
                    fps = effective_cfg.fps.max(1),
                    "native NVENC pipeline attached"
                );
                let queue_depth = effective_cfg.queue_depth.clamp(1, 64) as usize;
                let block_queue = effective_cfg.queue_strategy == "block";
                let (encoded_tx, mut encoded_rx) =
                    tokio::sync::mpsc::channel::<Vec<u8>>(queue_depth);
                let keyframe_request2 = keyframe_request.clone();
                let idr_interval_frames =
                    effective_cfg.fps.max(1) * effective_cfg.idr_interval_sec.max(1);
                std::thread::spawn(move || {
                    let mut encoded_frames: u32 = 0;
                    loop {
                        let force_idr = keyframe_request2.swap(false, Ordering::Relaxed)
                            || (idr_interval_frames > 0
                                && encoded_frames > 0
                                && encoded_frames.is_multiple_of(idr_interval_frames));
                        match native.encode_next(force_idr) {
                            Ok(Some(v)) if !v.is_empty() => {
                                encoded_frames = encoded_frames.saturating_add(1);
                                if block_queue {
                                    let _ = encoded_tx.blocking_send(v);
                                } else {
                                    let _ = encoded_tx.try_send(v);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                error!(error = %e, "native NVENC encode failed");
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                });

                if let Some(track) = rtp_track.clone() {
                    let sender = RtpH264Sender::new(
                        track,
                        &RtpH264SenderConfig {
                            fps: effective_cfg.fps.max(1),
                            mtu: effective_cfg.rtp_mtu,
                            frame_pacing_enable: effective_cfg.frame_pacing_enable,
                            frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
                        },
                    );
                    tokio::spawn(spawn_send_loop_rtp(
                        sender,
                        encoded_rx,
                        adapt,
                        stats,
                        enable_network_adapt,
                    ));
                } else if let Some(track) = sample_track.clone() {
                    let frame_duration = Duration::from_millis(
                        (1000.0 / effective_cfg.fps.max(1) as f64).max(1.0).round() as u64,
                    );
                    tokio::spawn(async move {
                        while let Some(encoded) = encoded_rx.recv().await {
                            let sample = Sample {
                                data: Bytes::from(encoded),
                                duration: frame_duration,
                                ..Default::default()
                            };
                            if let Err(e) = track.write_sample(&sample).await {
                                error!(error = %e, "sample write failed");
                                break;
                            }
                        }
                    });
                }
                return Ok(());
            }
            Err(e) => {
                if !effective_cfg.allow_encoder_fallback {
                    return Err(anyhow!(
                        "native nvenc init failed and fallback disabled: {e}"
                    ));
                }
                warn!(error = %e, "native NVENC init failed, using fallback");
            }
        }
    }

    let fps = effective_cfg
        .fps
        .clamp(effective_cfg.min_fps.max(1), effective_cfg.max_fps.max(1));
    let frame_ms = (1000.0 / fps as f64).max(1.0).round() as u64;
    let frame_duration = Duration::from_millis(frame_ms);
    let allow_encoder_fallback = effective_cfg.allow_encoder_fallback;
    let block_queue = effective_cfg.queue_strategy == "block";
    let running = Arc::new(AtomicBool::new(true));
    let latest = Arc::new(std::sync::Mutex::new(None::<RawFrame>));
    let queue_depth = effective_cfg.queue_depth.clamp(1, 64) as usize;
    let (encoded_tx, mut encoded_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(queue_depth);

    {
        let running = running.clone();
        let latest = latest.clone();
        let target_width = effective_cfg.target_width;
        let target_height = effective_cfg.target_height;
        std::thread::spawn(move || {
            let mut capturer = match build_frame_capturer(backend) {
                Ok(v) => v,
                Err(e) => {
                    error!(error = %e, "capture initialization failed");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            let mut next_tick = Instant::now();
            while running.load(Ordering::Relaxed) {
                match capturer.capture() {
                    Ok((mut rgba, mut width, mut height)) => {
                        if target_width > 0
                            && target_height > 0
                            && (target_width != width || target_height != height)
                        {
                            if let Some((resized, rw, rh)) =
                                resize_rgba_fast(&rgba, width, height, target_width, target_height)
                            {
                                rgba = resized;
                                width = rw;
                                height = rh;
                            }
                        }
                        if let Ok(mut slot) = latest.lock() {
                            *slot = Some(RawFrame {
                                rgba,
                                width,
                                height,
                            });
                        }
                    }
                    Err(e) => error!(error = %e, "capture frame failed"),
                }
                next_tick += frame_duration;
                sleep_until(next_tick);
            }
        });
    }

    {
        let running = running.clone();
        let latest = latest.clone();
        let encode_cfg = effective_cfg.clone();
        let keyframe_request2 = keyframe_request.clone();
        let adapt2 = adapt.clone();
        std::thread::spawn(move || {
            let mut encoder = match build_video_encoder(
                fps,
                &encode_cfg,
                encoder_backend,
                allow_encoder_fallback,
            ) {
                Ok(e) => e,
                Err(e) => {
                    error!(error = %e, "H264 encoder initialization failed");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };

            while running.load(Ordering::Relaxed) {
                let frame = match latest.lock() {
                    Ok(mut slot) => slot.take(),
                    Err(_) => None,
                };
                let Some(frame) = frame else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                if keyframe_request2.swap(false, Ordering::Relaxed) {
                    request_keyframe(&mut encoder);
                }

                let target_bitrate_kbps = if enable_network_adapt {
                    Some(adapt2.current_bitrate_kbps())
                } else {
                    None
                };

                let encoded = match encode_rgba_frame(
                    &mut encoder,
                    &frame.rgba,
                    frame.width,
                    frame.height,
                    target_bitrate_kbps,
                    enable_network_adapt,
                ) {
                    Ok(v) => v,
                    Err(e) => {
                        error!(error = %e, "H264 encode failed");
                        continue;
                    }
                };
                if encoded.is_empty() {
                    continue;
                }
                if block_queue {
                    let _ = encoded_tx.blocking_send(encoded);
                } else {
                    let _ = encoded_tx.try_send(encoded);
                }
            }
        });
    }

    if let Some(track) = rtp_track {
        let sender = RtpH264Sender::new(
            track,
            &RtpH264SenderConfig {
                fps,
                mtu: effective_cfg.rtp_mtu,
                frame_pacing_enable: effective_cfg.frame_pacing_enable,
                frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
            },
        );
        tokio::spawn(spawn_send_loop_rtp(
            sender,
            encoded_rx,
            adapt,
            stats,
            enable_network_adapt,
        ));
    } else if let Some(track) = sample_track {
        tokio::spawn(async move {
            while let Some(encoded) = encoded_rx.recv().await {
                let sample = Sample {
                    data: Bytes::from(encoded),
                    duration: frame_duration,
                    ..Default::default()
                };
                if let Err(e) = track.write_sample(&sample).await {
                    error!(error = %e, "sample write failed");
                    running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    Ok(())
}

async fn spawn_send_loop_rtp(
    mut sender: RtpH264Sender,
    mut encoded_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    adapt: Arc<NetAdaptController>,
    stats: Arc<RuntimeStats>,
    enable_network_adapt: bool,
) {
    let mut next_due = Instant::now();
    let mut next_recover_tick = Instant::now();
    while let Some(encoded) = encoded_rx.recv().await {
        if enable_network_adapt && Instant::now() >= next_recover_tick {
            if let Some((fps_v, br_v)) = adapt.tick_recover() {
                info!(
                    target_fps = fps_v,
                    target_bitrate_kbps = br_v,
                    "network adapt recovered"
                );
            }
            next_recover_tick = Instant::now() + Duration::from_secs(1);
        }

        let target_fps = adapt.current_fps().max(1);
        let target_bitrate = adapt.current_bitrate_kbps().max(100);
        stats.target_fps.store(target_fps, Ordering::Relaxed);
        stats
            .target_bitrate_kbps
            .store(target_bitrate, Ordering::Relaxed);

        let frame_gap = Duration::from_millis((1000.0 / target_fps as f64).max(1.0) as u64);
        let now = Instant::now();
        if now < next_due {
            tokio::time::sleep(next_due - now).await;
        }
        next_due = Instant::now() + frame_gap;

        if let Err(e) = sender.send_access_unit(&encoded).await {
            error!(error = %e, "RTP write failed");
            break;
        }
        stats.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
    }
}

async fn ws_send_json(ws: &Arc<Mutex<WsWrite>>, v: &Value) -> Result<()> {
    let text = v.to_string();
    let mut w = ws.lock().await;
    w.send(Message::Text(text))
        .await
        .map_err(|e| anyhow!("ws send failed: {e}"))
}
