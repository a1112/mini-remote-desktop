mod capture_policy;
mod encoder_policy;
mod net_adapt;
mod nvenc_native;
mod rtp_send;

use crate::capture_policy::{CaptureBackend, choose_backend};
use crate::encoder_policy::{VideoEncoderBackend, choose_encoder_backend};
use crate::net_adapt::NetAdaptController;
use crate::nvenc_native::NativeNvencPipeline;
use crate::rtp_send::{RtpH264Sender, RtpH264SenderConfig};
use agent_rust::{load_config, register_message};
use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use image::ImageReader;
use openh264::OpenH264API;
use openh264::encoder::{Encoder, EncoderConfig, FrameRate, UsageType};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use serde_json::{Value, json};
use std::io::Cursor;
use std::io::Read;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::process::{Child, ChildStdin};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::media::Sample;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::rtp_transceiver::rtp_sender::RTCRtpSender;
use webrtc::rtp_transceiver::{RTCPFeedback, TYPE_RTCP_FB_CCM, TYPE_RTCP_FB_GOOG_REMB, TYPE_RTCP_FB_NACK, TYPE_RTCP_FB_TRANSPORT_CC};
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_rtp::TrackLocalStaticRTP;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

#[derive(Default)]
struct SessionState {
    controller_id: Option<String>,
    pc: Option<Arc<RTCPeerConnection>>,
}

#[derive(Default)]
struct RuntimeStats {
    pli_count: AtomicU64,
    fir_count: AtomicU64,
    nack_count: AtomicU64,
    remb_count: AtomicU64,
    last_remb_kbps: AtomicU32,
    target_fps: AtomicU32,
    target_bitrate_kbps: AtomicU32,
    rtp_au_sent: AtomicU64,
    rtp_au_skipped: AtomicU64,
}

impl RuntimeStats {
    fn new(target_fps: u32, target_bitrate_kbps: u32) -> Self {
        Self {
            target_fps: AtomicU32::new(target_fps),
            target_bitrate_kbps: AtomicU32::new(target_bitrate_kbps),
            ..Default::default()
        }
    }
}

type WsWrite = futures_util::stream::SplitSink<
    WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

enum RuntimeVideoEncoder {
    OpenH264(Encoder),
    HwFfmpeg {
        backend: VideoEncoderBackend,
        fps: u32,
        ffmpeg_bin: String,
        ffmpeg_cfg: agent_rust::CaptureConfig,
        pipe: Option<FfmpegPipeEncoder>,
        wh: Option<(u32, u32)>,
    },
}

struct FfmpegPipeEncoder {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    stream_buf: Vec<u8>,
    poll_wait_ms: u64,
}

struct RawFrame {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

enum FrameCapturer {
    Dxgi { screen: screenshots::Screen },
    Powershell,
    Dummy,
}

impl Drop for FfmpegPipeEncoder {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cfg = load_config(&PathBuf::from("config.json"));
    if cfg.device_name == "Rust Agent" {
        if let Ok(host) = std::env::var("COMPUTERNAME") {
            if !host.trim().is_empty() {
                cfg.device_name = format!("{host} - Rust Agent");
            }
        }
    }

    println!("[RustAgent-M2] ws_url={}", cfg.ws_url);
    println!(
        "[RustAgent-M2] capture cfg: fps={} backend={} allow_fallback={} encoder={} allow_encoder_fallback={}",
        cfg.capture.fps,
        cfg.capture.backend,
        cfg.capture.allow_fallback,
        cfg.capture.encoder,
        cfg.capture.allow_encoder_fallback
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
                eprintln!("[RustAgent-M2] ws read error: {e}");
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
            println!("[RustAgent-M2] registered as {}", cfg.device_name);
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
                eprintln!("[RustAgent-M2] invalid offer payload");
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
            println!("[RustAgent-M2] answer sent");

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
                    eprintln!("[RustAgent-M2] add remote ice failed: {e}");
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
                    match c.to_json() {
                        Ok(cjson) => {
                            let msg = json!({
                                "type": "webrtc",
                                "action": "iceCandidate",
                                "payload": {
                                    "targetDeviceId": controller_id,
                                    "candidate": cjson
                                }
                            });
                            if let Err(e) = ws_send_json(&ws_write, &msg).await {
                                eprintln!("[RustAgent-M2] send local ice failed: {e}");
                            }
                        }
                        Err(e) => eprintln!("[RustAgent-M2] candidate to_json failed: {e}"),
                    }
                }
            })
        }));
    }

    pc.on_peer_connection_state_change(Box::new(|s: RTCPeerConnectionState| {
        println!("[RustAgent-M2] pc state: {s}");
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
        println!("[RustAgent-M2] {line}");
    }
    let (encoder_backend, logs) = choose_encoder_backend(&effective_cfg);
    for line in logs {
        println!("[RustAgent-M2] {line}");
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
        effective_cfg.network_adapt_enable,
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
                println!(
                    "[RustAgent-M2] native nvenc pipeline attached: input={}x{} target={}x{} @{}fps profile={} queue={}",
                    input_w,
                    input_h,
                    target_w,
                    target_h,
                    effective_cfg.fps.max(1),
                    effective_cfg.performance_profile,
                    effective_cfg.queue_strategy
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
                                eprintln!("[RustAgent-M2] native nvenc encode failed: {e}");
                                std::thread::sleep(Duration::from_millis(2));
                            }
                        }
                    }
                });
                if let Some(track) = rtp_track.clone() {
                    let mut sender = RtpH264Sender::new(
                        track,
                        &RtpH264SenderConfig {
                            fps: effective_cfg.fps.max(1),
                            mtu: effective_cfg.rtp_mtu,
                            frame_pacing_enable: effective_cfg.frame_pacing_enable,
                            frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
                        },
                    );
                    let adapt2 = adapt.clone();
                    let stats2 = stats.clone();
                    tokio::spawn(async move {
                        let mut next_due = Instant::now();
                        while let Some(encoded) = encoded_rx.recv().await {
                            if let Some(v) = adapt2.tick_recover() {
                                println!("[RustAgent-M2] net-adapt recover target_fps={v}");
                            }
                            let target_fps = adapt2.current_fps().max(1);
                            stats2.target_fps.store(target_fps, Ordering::Relaxed);
                            let frame_gap =
                                Duration::from_millis((1000.0 / target_fps as f64).max(1.0) as u64);
                            if Instant::now() < next_due {
                                stats2.rtp_au_skipped.fetch_add(1, Ordering::Relaxed);
                                continue;
                            }
                            next_due = Instant::now() + frame_gap;
                            if let Err(e) = sender.send_access_unit(&encoded).await {
                                eprintln!("[RustAgent-M2] write rtp failed: {e}");
                                break;
                            }
                            stats2.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
                        }
                    });
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
                                eprintln!("[RustAgent-M2] write sample failed: {e}");
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
                eprintln!("[RustAgent-M2] native nvenc init failed, fallback enabled: {e}");
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
                    eprintln!("[RustAgent-M2] capture init failed: {e}");
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
                    Err(e) => eprintln!("[RustAgent-M2] capture frame failed: {e}"),
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
        std::thread::spawn(move || {
            let mut encoder = match build_video_encoder(
                fps,
                &encode_cfg,
                encoder_backend,
                allow_encoder_fallback,
            ) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[RustAgent-M2] h264 encoder init failed: {e}");
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

                let encoded =
                    match encode_rgba_frame(&mut encoder, &frame.rgba, frame.width, frame.height) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("[RustAgent-M2] h264 encode failed: {e}");
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
        let mut sender = RtpH264Sender::new(
            track,
            &RtpH264SenderConfig {
                fps,
                mtu: effective_cfg.rtp_mtu,
                frame_pacing_enable: effective_cfg.frame_pacing_enable,
                frame_pacing_batch_packets: effective_cfg.frame_pacing_batch_packets,
            },
        );
        let running2 = running.clone();
        let adapt2 = adapt.clone();
        let stats2 = stats.clone();
        tokio::spawn(async move {
            let mut next_due = Instant::now();
            while let Some(encoded) = encoded_rx.recv().await {
                if let Some(v) = adapt2.tick_recover() {
                    println!("[RustAgent-M2] net-adapt recover target_fps={v}");
                }
                let target_fps = adapt2.current_fps().max(1);
                stats2.target_fps.store(target_fps, Ordering::Relaxed);
                let frame_gap = Duration::from_millis((1000.0 / target_fps as f64).max(1.0) as u64);
                if Instant::now() < next_due {
                    stats2.rtp_au_skipped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                next_due = Instant::now() + frame_gap;
                if let Err(e) = sender.send_access_unit(&encoded).await {
                    eprintln!("[RustAgent-M2] write rtp failed: {e}");
                    running2.store(false, Ordering::Relaxed);
                    break;
                }
                stats2.rtp_au_sent.fetch_add(1, Ordering::Relaxed);
            }
        });
    } else if let Some(track) = sample_track {
        tokio::spawn(async move {
            while let Some(encoded) = encoded_rx.recv().await {
                let sample = Sample {
                    data: Bytes::from(encoded),
                    duration: frame_duration,
                    ..Default::default()
                };
                if let Err(e) = track.write_sample(&sample).await {
                    eprintln!("[RustAgent-M2] write sample failed: {e}");
                    running.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });
    }

    Ok(())
}

fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

fn request_keyframe(encoder: &mut RuntimeVideoEncoder) {
    if let RuntimeVideoEncoder::HwFfmpeg { pipe, .. } = encoder {
        *pipe = None;
    }
}

fn spawn_rtcp_feedback_loop(
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
                    eprintln!("[RustAgent-M2] rtcp read stopped: {e}");
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
                    println!("[RustAgent-M2] rtcp pli");
                    if force_idr_on_pli {
                        keyframe_request.store(true, Ordering::Relaxed);
                    }
                    continue;
                }
                if pkt.as_any().downcast_ref::<FullIntraRequest>().is_some() {
                    stats.fir_count.fetch_add(1, Ordering::Relaxed);
                    println!("[RustAgent-M2] rtcp fir");
                    keyframe_request.store(true, Ordering::Relaxed);
                    continue;
                }
                if let Some(nack) = pkt.as_any().downcast_ref::<TransportLayerNack>() {
                    stats.nack_count.fetch_add(1, Ordering::Relaxed);
                    if enable_network_adapt {
                        let target = adapt.on_nack_burst();
                        println!(
                            "[RustAgent-M2] rtcp nack sender_ssrc={} media_ssrc={} target_fps={}",
                            nack.sender_ssrc, nack.media_ssrc, target
                        );
                    } else {
                        println!(
                            "[RustAgent-M2] rtcp nack sender_ssrc={} media_ssrc={}",
                            nack.sender_ssrc, nack.media_ssrc
                        );
                    }
                    continue;
                }
                if let Some(remb) = pkt
                    .as_any()
                    .downcast_ref::<ReceiverEstimatedMaximumBitrate>()
                    && enable_network_adapt
                {
                    stats.remb_count.fetch_add(1, Ordering::Relaxed);
                    stats
                        .last_remb_kbps
                        .store((remb.bitrate / 1000.0) as u32, Ordering::Relaxed);
                    let target = adapt.on_remb_bps(remb.bitrate);
                    println!(
                        "[RustAgent-M2] rtcp remb bitrate_bps={:.0} target_fps={}",
                        remb.bitrate, target
                    );
                }
            }
        }
    });
}

fn spawn_stats_panel(stats: Arc<RuntimeStats>, adapt: Arc<NetAdaptController>, interval_ms: u32) {
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
            let sent = stats.rtp_au_sent.load(Ordering::Relaxed);
            let skipped = stats.rtp_au_skipped.load(Ordering::Relaxed);
            println!(
                "[RustAgent-M2][RTCP-PANEL] pli={} fir={} nack={} remb={} remb_kbps={} target_fps={} au_sent={} au_skipped={}",
                pli, fir, nack, remb, remb_kbps, target_fps, sent, skipped
            );
        }
    });
}

fn apply_capture_profile(cfg: &mut agent_rust::CaptureConfig) {
    let mut template = cfg.profile_template.to_ascii_lowercase();
    if template.is_empty() {
        template = match cfg.performance_profile.as_str() {
            "smooth" | "latency_first" => "latency_first".to_string(),
            "quality" | "quality_first" => "quality_first".to_string(),
            _ => "balanced".to_string(),
        };
    }
    if template == "custom" && !cfg.enable_template_overlay {
        return;
    }
    match template.as_str() {
        "latency_first" => {
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p1".to_string();
                cfg.encoder_tune = "ull".to_string();
                cfg.rc_mode = "cbr".to_string();
                cfg.bframes = cfg.bframes.min(0);
                cfg.gop = cfg.gop.clamp(30, 60);
                cfg.queue_depth = cfg.queue_depth.clamp(2, 6);
                cfg.queue_strategy = "drop".to_string();
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(16000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps);
                cfg.min_fps = cfg.min_fps.max(30);
            }
            cfg.performance_profile = "smooth".to_string();
        }
        "quality_first" => {
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p5".to_string();
                cfg.encoder_tune = "hq".to_string();
                cfg.rc_mode = "vbr".to_string();
                cfg.bframes = cfg.bframes.max(2).min(3);
                cfg.gop = cfg.gop.max(120);
                cfg.queue_depth = cfg.queue_depth.clamp(12, 32);
                cfg.queue_strategy = "block".to_string();
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(28000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps + 12000);
            }
            cfg.performance_profile = "quality".to_string();
        }
        "custom" => {}
        _ => {
            cfg.profile_template = "balanced".to_string();
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p3".to_string();
                cfg.encoder_tune = "ll".to_string();
                cfg.rc_mode = "cbr".to_string();
                cfg.bframes = cfg.bframes.min(1);
                cfg.gop = cfg.gop.clamp(45, 90);
                cfg.queue_depth = cfg.queue_depth.clamp(4, 16);
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(20000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps + 8000);
                cfg.queue_strategy = "drop".to_string();
            }
            cfg.performance_profile = "balanced".to_string();
        }
    }
    if cfg.max_bitrate_kbps < cfg.bitrate_kbps {
        cfg.max_bitrate_kbps = cfg.bitrate_kbps;
    }
    if cfg.network_adapt_ceiling_bitrate_kbps < cfg.network_adapt_floor_bitrate_kbps {
        cfg.network_adapt_ceiling_bitrate_kbps = cfg.network_adapt_floor_bitrate_kbps;
    }
    if !matches!(cfg.queue_strategy.as_str(), "drop" | "block") {
        cfg.queue_strategy = "drop".to_string();
    }
}

impl FrameCapturer {
    fn capture(&mut self) -> Result<(Vec<u8>, u32, u32)> {
        match self {
            FrameCapturer::Dxgi { screen } => {
                let img = screen.capture().context("dxgi capture failed")?;
                Ok((img.as_raw().to_vec(), img.width(), img.height()))
            }
            FrameCapturer::Powershell => capture_via_powershell(),
            FrameCapturer::Dummy => {
                let w = 640_u32;
                let h = 360_u32;
                let mut rgba = vec![0_u8; (w * h * 4) as usize];
                for px in rgba.chunks_exact_mut(4) {
                    px[0] = 16;
                    px[1] = 16;
                    px[2] = 16;
                    px[3] = 255;
                }
                Ok((rgba, w, h))
            }
        }
    }
}

fn build_frame_capturer(backend: CaptureBackend) -> Result<FrameCapturer> {
    match backend {
        CaptureBackend::Dxgi => {
            let screens = screenshots::Screen::all().context("list screens failed")?;
            let screen = screens
                .first()
                .ok_or_else(|| anyhow!("no screen found"))?
                .clone();
            Ok(FrameCapturer::Dxgi { screen })
        }
        CaptureBackend::Powershell => Ok(FrameCapturer::Powershell),
        CaptureBackend::Dummy => Ok(FrameCapturer::Dummy),
    }
}

fn detect_input_resolution() -> Result<(u32, u32)> {
    let screens = screenshots::Screen::all().context("list screens failed")?;
    let screen = screens.first().ok_or_else(|| anyhow!("no screen found"))?;
    let img = screen
        .capture()
        .context("capture for resolution detect failed")?;
    Ok((img.width(), img.height()))
}

fn resize_rgba_fast(
    rgba: &[u8],
    width: u32,
    height: u32,
    target_width: u32,
    target_height: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::RgbaImage::from_raw(width, height, rgba.to_vec())?;
    let resized = image::imageops::resize(
        &img,
        target_width,
        target_height,
        image::imageops::FilterType::Triangle,
    );
    Some((resized.into_raw(), target_width, target_height))
}

fn capture_via_powershell() -> Result<(Vec<u8>, u32, u32)> {
    let temp_path = std::env::temp_dir().join("mini-rust-agent-ps-capture.jpg");
    let path = temp_path
        .to_str()
        .ok_or_else(|| anyhow!("temp path invalid"))?
        .replace('\'', "''");

    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $b=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
         $bmp=New-Object System.Drawing.Bitmap $b.Width,$b.Height; \
         $g=[System.Drawing.Graphics]::FromImage($bmp); \
         $g.CopyFromScreen($b.Location,[System.Drawing.Point]::Empty,$b.Size); \
         $bmp.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Jpeg); \
         $g.Dispose(); $bmp.Dispose(); \
         Write-Output ($b.Width.ToString() + ',' + $b.Height.ToString());"
    );

    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .context("powershell capture spawn failed")?;
    if !out.status.success() {
        return Err(anyhow!(
            "powershell capture failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let size_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = size_str.split(',');
    let width = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse width failed"))?;
    let height = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("parse height failed"))?;

    let jpg = std::fs::read(&temp_path).context("read captured jpeg failed")?;
    let img = ImageReader::new(Cursor::new(jpg))
        .with_guessed_format()
        .context("guess image format failed")?
        .decode()
        .context("decode jpeg failed")?
        .to_rgba8();

    Ok((img.as_raw().to_vec(), width, height))
}

fn build_video_encoder(
    fps: u32,
    cfg: &agent_rust::CaptureConfig,
    backend: VideoEncoderBackend,
    allow_fallback: bool,
) -> Result<RuntimeVideoEncoder> {
    if let Some(codec) = ffmpeg_codec_name(backend) {
        let ffmpeg_bin = resolve_ffmpeg_bin();
        match probe_ffmpeg_encoder(&ffmpeg_bin, codec) {
            Ok(()) => {
                println!(
                    "[RustAgent-M2] encoder backend {} attached via ffmpeg codec {}",
                    backend.as_str(),
                    codec
                );
                return Ok(RuntimeVideoEncoder::HwFfmpeg {
                    backend,
                    fps,
                    ffmpeg_bin,
                    ffmpeg_cfg: cfg.clone(),
                    pipe: None,
                    wh: None,
                });
            }
            Err(e) if allow_fallback => {
                eprintln!(
                    "[RustAgent-M2] encoder backend {} unavailable ({}), fallback to openh264",
                    backend.as_str(),
                    e
                );
            }
            Err(e) => {
                return Err(anyhow!(
                    "encoder backend {} unavailable and fallback disabled: {}",
                    backend.as_str(),
                    e
                ));
            }
        }
    }

    let cfg = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .skip_frames(false);
    let api = OpenH264API::from_source();
    let enc = Encoder::with_api_config(api, cfg).context("create openh264 encoder failed")?;
    Ok(RuntimeVideoEncoder::OpenH264(enc))
}

fn encode_rgba_frame(
    encoder: &mut RuntimeVideoEncoder,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    match encoder {
        RuntimeVideoEncoder::OpenH264(enc) => {
            let rgb = RgbaSliceU8::new(rgba, (width as usize, height as usize));
            let yuv = YUVBuffer::from_rgb_source(rgb);
            let bitstream = enc.encode(&yuv).context("openh264 encode failed")?;
            Ok(bitstream.to_vec())
        }
        RuntimeVideoEncoder::HwFfmpeg {
            backend,
            fps,
            ffmpeg_bin,
            ffmpeg_cfg,
            pipe,
            wh,
        } => {
            if pipe.is_none() || wh != &Some((width, height)) {
                *pipe = Some(start_ffmpeg_pipe(
                    *backend, *fps, ffmpeg_bin, ffmpeg_cfg, width, height,
                )?);
                *wh = Some((width, height));
            }
            match pipe.as_mut().unwrap().encode_one_frame(rgba) {
                Ok(v) => Ok(v),
                Err(e) => {
                    *pipe = Some(start_ffmpeg_pipe(
                        *backend, *fps, ffmpeg_bin, ffmpeg_cfg, width, height,
                    )?);
                    pipe.as_mut()
                        .unwrap()
                        .encode_one_frame(rgba)
                        .with_context(|| format!("ffmpeg reinit encode failed: {e}"))
                }
            }
        }
    }
}

fn start_ffmpeg_pipe(
    backend: VideoEncoderBackend,
    fps: u32,
    ffmpeg_bin: &str,
    cfg: &agent_rust::CaptureConfig,
    width: u32,
    height: u32,
) -> Result<FfmpegPipeEncoder> {
    let codec = ffmpeg_codec_name(backend).ok_or_else(|| anyhow!("not a ffmpeg hw backend"))?;
    let size = format!("{width}x{height}");
    let fps_s = fps.to_string();

    let tune = if cfg.encoder_tune == "balanced" {
        "ll"
    } else {
        cfg.encoder_tune.as_str()
    };
    let mut args = vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-f".to_string(),
        "rawvideo".to_string(),
        "-pix_fmt".to_string(),
        "rgba".to_string(),
        "-s".to_string(),
        size,
        "-r".to_string(),
        fps_s,
        "-i".to_string(),
        "-".to_string(),
        "-an".to_string(),
        "-c:v".to_string(),
        codec.to_string(),
        "-preset".to_string(),
        cfg.encoder_preset.clone(),
        "-tune".to_string(),
        tune.to_string(),
        "-g".to_string(),
        cfg.gop.max(1).to_string(),
        "-bf".to_string(),
        cfg.bframes.to_string(),
        "-rc".to_string(),
        cfg.rc_mode.clone(),
        "-b:v".to_string(),
        format!("{}k", cfg.bitrate_kbps.max(100)),
        "-maxrate".to_string(),
        format!("{}k", cfg.max_bitrate_kbps.max(cfg.bitrate_kbps.max(100))),
        "-bufsize".to_string(),
        format!(
            "{}k",
            (cfg.max_bitrate_kbps.max(cfg.bitrate_kbps.max(100)) * 2)
        ),
        "-bsf:v".to_string(),
        "h264_metadata=aud=insert".to_string(),
        "-f".to_string(),
        "h264".to_string(),
        "-".to_string(),
    ];
    let mut cmd = Command::new(ffmpeg_bin);
    cmd.args(args.drain(..));
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().context("spawn ffmpeg failed")?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stdin unavailable"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stdout unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("ffmpeg stderr unavailable"))?;

    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    std::thread::spawn(move || {
        let mut sink = [0_u8; 4096];
        loop {
            match stderr.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    Ok(FfmpegPipeEncoder {
        child,
        stdin,
        stdout_rx: rx,
        stream_buf: Vec::with_capacity(256 * 1024),
        poll_wait_ms: (1000_u64 / fps.max(1) as u64).clamp(1, 8),
    })
}

impl FfmpegPipeEncoder {
    fn encode_one_frame(&mut self, rgba: &[u8]) -> Result<Vec<u8>> {
        self.stdin
            .write_all(rgba)
            .context("write raw frame to ffmpeg failed")?;
        self.stdin.flush().ok();

        let deadline = Instant::now() + Duration::from_millis(self.poll_wait_ms);
        loop {
            while let Ok(chunk) = self.stdout_rx.try_recv() {
                self.stream_buf.extend_from_slice(&chunk);
            }
            if let Some(au) = take_one_access_unit_by_aud(&mut self.stream_buf) {
                return Ok(au);
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }

        // Fallback for encoders/bitstreams without parseable AU boundaries.
        if let Some(status) = self.child.try_wait().ok().flatten() {
            return Err(anyhow!("ffmpeg exited unexpectedly: {status}"));
        }
        if self.stream_buf.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        std::mem::swap(&mut out, &mut self.stream_buf);
        Ok(out)
    }
}

fn resolve_ffmpeg_bin() -> String {
    std::env::var("AGENT_FFMPEG_PATH").unwrap_or_else(|_| "ffmpeg".to_string())
}

fn probe_ffmpeg_encoder(ffmpeg_bin: &str, codec: &str) -> Result<()> {
    let out = Command::new(ffmpeg_bin)
        .args(["-hide_banner", "-encoders"])
        .output()
        .with_context(|| format!("spawn {ffmpeg_bin} failed"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "{} -encoders failed: {}",
            ffmpeg_bin,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if !text.contains(codec) {
        return Err(anyhow!("encoder {codec} not found in ffmpeg -encoders"));
    }
    Ok(())
}

fn ffmpeg_codec_name(backend: VideoEncoderBackend) -> Option<&'static str> {
    match backend {
        VideoEncoderBackend::Nvenc => Some("h264_nvenc"),
        VideoEncoderBackend::Qsv => Some("h264_qsv"),
        VideoEncoderBackend::Amf => Some("h264_amf"),
        VideoEncoderBackend::OpenH264 => None,
    }
}

fn take_one_access_unit_by_aud(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let nals = parse_annexb_nals(buf);
    if nals.len() < 2 {
        return None;
    }

    // Preferred boundary: AUD (nal type 9).
    let aud_positions: Vec<usize> = nals
        .iter()
        .filter_map(|n| if n.nal_type == 9 { Some(n.start) } else { None })
        .collect();
    if aud_positions.len() >= 2 {
        let cut = aud_positions[1];
        let out = buf[..cut].to_vec();
        buf.drain(..cut);
        return if out.is_empty() { None } else { Some(out) };
    }

    // Fallback boundary: second VCL with first_mb_in_slice == 0.
    let mut frame_starts = Vec::new();
    for (idx, nal) in nals.iter().enumerate() {
        if !(1..=5).contains(&nal.nal_type) {
            continue;
        }
        let end = nals.get(idx + 1).map(|n| n.start).unwrap_or(buf.len());
        if nal.header_idx + 1 >= end {
            continue;
        }
        let first_mb_zero = h264_slice_first_mb_is_zero(&buf[nal.header_idx + 1..end]);
        if first_mb_zero {
            frame_starts.push(nal.start);
        }
    }
    if frame_starts.len() >= 2 {
        let cut = frame_starts[1];
        let out = buf[..cut].to_vec();
        buf.drain(..cut);
        return if out.is_empty() { None } else { Some(out) };
    }
    None
}

#[derive(Clone, Copy)]
struct NalPos {
    start: usize,
    header_idx: usize,
    nal_type: u8,
}

fn parse_annexb_nals(buf: &[u8]) -> Vec<NalPos> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 3 < buf.len() {
        let (is_start, sc_len) =
            if i + 2 < buf.len() && buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
                (true, 3usize)
            } else if i + 3 < buf.len()
                && buf[i] == 0
                && buf[i + 1] == 0
                && buf[i + 2] == 0
                && buf[i + 3] == 1
            {
                (true, 4usize)
            } else {
                (false, 0usize)
            };
        if !is_start {
            i += 1;
            continue;
        }
        let header_idx = i + sc_len;
        if header_idx < buf.len() {
            out.push(NalPos {
                start: i,
                header_idx,
                nal_type: buf[header_idx] & 0x1f,
            });
        }
        i = header_idx.saturating_add(1);
    }
    out
}

fn h264_slice_first_mb_is_zero(ebsp: &[u8]) -> bool {
    let rbsp = remove_emulation_prevention(ebsp);
    let mut br = BitReader::new(&rbsp);
    matches!(br.read_ue(), Some(0))
}

fn remove_emulation_prevention(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len());
    let mut i = 0usize;
    while i < src.len() {
        if i + 2 < src.len() && src[i] == 0 && src[i + 1] == 0 && src[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3;
            continue;
        }
        out.push(src[i]);
        i += 1;
    }
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bit(&mut self) -> Option<u8> {
        let byte_idx = self.bit_pos / 8;
        if byte_idx >= self.data.len() {
            return None;
        }
        let shift = 7 - (self.bit_pos % 8);
        self.bit_pos += 1;
        Some((self.data[byte_idx] >> shift) & 1)
    }

    fn read_bits(&mut self, n: usize) -> Option<u32> {
        let mut v = 0_u32;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.read_bit()?);
        }
        Some(v)
    }

    fn read_ue(&mut self) -> Option<u32> {
        let mut zeros = 0usize;
        while self.read_bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        if zeros == 0 {
            return Some(0);
        }
        let suffix = self.read_bits(zeros)?;
        Some(((1_u32 << zeros) - 1) + suffix)
    }
}

async fn ws_send_json(ws: &Arc<Mutex<WsWrite>>, v: &Value) -> Result<()> {
    let text = v.to_string();
    let mut w = ws.lock().await;
    w.send(Message::Text(text))
        .await
        .map_err(|e| anyhow!("ws send failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffmpeg_codec_mapping_is_correct() {
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Nvenc),
            Some("h264_nvenc")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Qsv),
            Some("h264_qsv")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Amf),
            Some("h264_amf")
        );
        assert_eq!(ffmpeg_codec_name(VideoEncoderBackend::OpenH264), None);
    }
}
