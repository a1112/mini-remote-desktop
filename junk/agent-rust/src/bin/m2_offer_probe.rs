use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use interceptor::registry::Registry;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::setting_engine::SettingEngine;
use webrtc::dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let verbose = parse_verbose_flag();
    let ws_url = "ws://127.0.0.1:9527";
    let (ws, _) = connect_async(ws_url)
        .await
        .context("connect signaling failed")?;
    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));

    let connected = read
        .next()
        .await
        .ok_or_else(|| anyhow!("no connected message"))??;
    println!("connected={}", connected.into_text()?);

    ws_send_json(
        &write,
        &json!({"type":"device","action":"register","payload":{"type":"controller","name":"m2-probe"}}),
    )
    .await?;
    ws_send_json(
        &write,
        &json!({"type":"device","action":"getDeviceList","payload":{}}),
    )
    .await?;

    let target_id = discover_target_device(&write, &mut read).await?;
    println!("target_id={target_id}");

    let pc = build_pc(write.clone(), target_id.clone(), verbose).await?;
    let frame_count = Arc::new(AtomicU64::new(0));
    let packet_count = Arc::new(AtomicU64::new(0));
    {
        let frame_count = frame_count.clone();
        let packet_count = packet_count.clone();
        pc.on_track(Box::new(move |track, _, _| {
            let frame_count = frame_count.clone();
            let packet_count = packet_count.clone();
            Box::pin(async move {
                let mut last_marker_ts: Option<u32> = None;
                let mut media_ssrc: Option<u32> = None;
                while let Ok((pkt, _)) = track.read_rtp().await {
                    let ssrc = pkt.header.ssrc;
                    if media_ssrc.is_none() {
                        media_ssrc = Some(ssrc);
                        println!("probe_media_ssrc={ssrc}");
                    }
                    if media_ssrc != Some(ssrc) {
                        continue;
                    }

                    packet_count.fetch_add(1, Ordering::Relaxed);
                    if pkt.header.marker {
                        let ts = pkt.header.timestamp;
                        if last_marker_ts != Some(ts) {
                            frame_count.fetch_add(1, Ordering::Relaxed);
                            last_marker_ts = Some(ts);
                        }
                    }
                }
            })
        }));
    }

    pc.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit {
            direction: RTCRtpTransceiverDirection::Recvonly,
            send_encodings: vec![],
        }),
    )
    .await?;
    let offer = pc.create_offer(None).await?;
    if verbose {
        for line in offer.sdp.lines() {
            if line.starts_with("m=video")
                || line.starts_with("a=mid:")
                || line.starts_with("a=send")
                || line.starts_with("a=recv")
                || line.starts_with("a=inactive")
                || line.starts_with("a=rtpmap:")
                || line.starts_with("a=fmtp:")
                || line.starts_with("a=setup:")
                || line.starts_with("a=fingerprint:")
            {
                println!("probe_offer_sdp: {line}");
            }
        }
    }
    pc.set_local_description(offer.clone()).await?;
    ws_send_json(
        &write,
        &json!({
            "type":"webrtc",
            "action":"offer",
            "payload":{
                "targetDeviceId": target_id,
                "offer":{"type":"offer","sdp":offer.sdp}
            }
        }),
    )
    .await?;
    println!("offer_sent=true");

    let run_secs = std::env::var("PROBE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0 && *v <= 600)
        .unwrap_or(15);
    let hard_deadline = Instant::now() + Duration::from_secs(run_secs + 45);
    let mut media_start: Option<Instant> = None;
    let mut remote_ice_count: u64 = 0;
    let mut signaling_closed = false;

    while Instant::now() < hard_deadline {
        if let Some(s) = media_start {
            if s.elapsed() >= Duration::from_secs(run_secs) {
                break;
            }
        }

        if signaling_closed {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }

        let msg = match timeout(Duration::from_millis(500), read.next()).await {
            Ok(Some(v)) => match v {
                Ok(m) => m,
                Err(e) => {
                    if media_start.is_some() {
                        eprintln!("probe_warn: signaling read error after answer: {e}");
                        signaling_closed = true;
                        continue;
                    }
                    return Err(e.into());
                }
            },
            Ok(None) => {
                if media_start.is_some() {
                    eprintln!("probe_warn: signaling disconnected after answer");
                    signaling_closed = true;
                    continue;
                }
                return Err(anyhow!("signaling disconnected before answer"));
            }
            Err(_) => continue,
        };
        let text = msg.into_text()?;
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                if media_start.is_some() {
                    eprintln!("probe_warn: signaling json parse failed after answer: {e}");
                    continue;
                }
                return Err(e.into());
            }
        };

        if v["type"] == "webrtc" && v["action"] == "answer" && media_start.is_none() {
            println!("answer_received=true");
            let sdp = v["payload"]["answer"]["sdp"]
                .as_str()
                .ok_or_else(|| anyhow!("answer.sdp missing"))?;
            if verbose {
                for line in sdp.lines() {
                    if line.starts_with("m=video")
                        || line.starts_with("a=mid:")
                        || line.starts_with("a=send")
                        || line.starts_with("a=recv")
                        || line.starts_with("a=inactive")
                        || line.starts_with("a=rtpmap:")
                        || line.starts_with("a=fmtp:")
                        || line.starts_with("a=setup:")
                        || line.starts_with("a=fingerprint:")
                    {
                        println!("probe_answer_sdp: {line}");
                    }
                }
            }
            pc.set_remote_description(RTCSessionDescription::answer(sdp.to_string())?)
                .await
                .context("set remote answer failed")?;
            media_start = Some(Instant::now());
            continue;
        }

        if v["type"] == "webrtc" && v["action"] == "iceCandidate" {
            let cand_v = &v["payload"]["candidate"];
            if !cand_v.is_null() {
                let cand: RTCIceCandidateInit = serde_json::from_value(cand_v.clone())
                    .context("parse remote candidate failed")?;
                let _ = pc.add_ice_candidate(cand).await;
                remote_ice_count += 1;
                if verbose && remote_ice_count <= 6 {
                    println!("probe_remote_ice_recv={remote_ice_count}");
                }
            }
        }
    }

    let secs = media_start
        .map(|s| s.elapsed().as_secs_f64())
        .unwrap_or(0.0)
        .max(0.001);
    let frames = frame_count.load(Ordering::Relaxed);
    let packets = packet_count.load(Ordering::Relaxed);
    let fps = frames as f64 / secs;
    println!(
        "media_stats: seconds={:.2} frames={} packets={} estimated_fps={:.2} packets_per_frame={:.2} remote_ice={}",
        secs,
        frames,
        packets,
        fps,
        if frames > 0 {
            packets as f64 / frames as f64
        } else {
            0.0
        },
        remote_ice_count
    );

    Ok(())
}

async fn build_pc(
    write: Arc<
        Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    target_id: String,
    verbose: bool,
) -> Result<Arc<RTCPeerConnection>> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let mut registry = Registry::new();
    registry = register_default_interceptors(registry, &mut m)?;
    let mut se = SettingEngine::default();
    se.set_srtp_protection_profiles(vec![
        SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80,
        SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_32,
    ]);
    se.set_include_loopback_candidate(true);
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .with_setting_engine(se)
        .build();
    let pc = Arc::new(
        api.new_peer_connection(RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        })
        .await?,
    );

    let verbose_ice = verbose;
    pc.on_ice_connection_state_change(Box::new(move |s: RTCIceConnectionState| {
        if verbose_ice {
            println!("probe_ice_state={s}");
        }
        Box::pin(async {})
    }));
    let verbose_pc = verbose;
    pc.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        if verbose_pc {
            println!("probe_pc_state={s}");
        }
        Box::pin(async {})
    }));

    {
        let write = write.clone();
        let local_ice_sent = Arc::new(AtomicU64::new(0));
        pc.on_ice_candidate(Box::new(move |cand| {
            let write = write.clone();
            let target_id = target_id.clone();
            let local_ice_sent = local_ice_sent.clone();
            Box::pin(async move {
                if let Some(c) = cand {
                    if let Ok(cjson) = c.to_json() {
                        let _ = ws_send_json(
                            &write,
                            &json!({
                                "type":"webrtc",
                                "action":"iceCandidate",
                                "payload":{
                                    "targetDeviceId": target_id,
                                    "candidate": cjson
                                }
                            }),
                        )
                        .await;
                        let n = local_ice_sent.fetch_add(1, Ordering::Relaxed) + 1;
                        if verbose && n <= 6 {
                            println!("probe_local_ice_sent={n}");
                        }
                    }
                }
            })
        }));
    }

    Ok(pc)
}

fn parse_verbose_flag() -> bool {
    if std::env::args().any(|a| a == "--verbose" || a == "-v") {
        return true;
    }
    std::env::var("PROBE_VERBOSE")
        .ok()
        .map(|v| {
            let s = v.trim().to_ascii_lowercase();
            matches!(s.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

async fn ws_send_json(
    ws: &Arc<
        Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    v: &Value,
) -> Result<()> {
    let mut w = ws.lock().await;
    w.send(Message::Text(v.to_string())).await?;
    Ok(())
}

async fn discover_target_device(
    write: &Arc<
        Mutex<
            futures_util::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> Result<String> {
    let timeout_secs = std::env::var("DISCOVERY_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0 && *v <= 120)
        .unwrap_or(20);
    let preferred_name = std::env::var("PROBE_TARGET_NAME").ok();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_pull = Instant::now() - Duration::from_secs(3);

    let mut seen_messages = 0_u64;
    let mut seen_lists = 0_u64;
    let mut last_list_summary = String::new();

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!(
                "device discovery timeout after {}s: seen_messages={} seen_device_lists={} last_device_list={}",
                timeout_secs,
                seen_messages,
                seen_lists,
                if last_list_summary.is_empty() {
                    "<empty>".to_string()
                } else {
                    last_list_summary
                }
            ));
        }

        if Instant::now().duration_since(last_pull) >= Duration::from_secs(1) {
            ws_send_json(
                write,
                &json!({"type":"device","action":"getDeviceList","payload":{}}),
            )
            .await
            .context("send getDeviceList failed")?;
            last_pull = Instant::now();
        }

        let msg = match timeout(Duration::from_millis(600), read.next()).await {
            Ok(Some(v)) => v?,
            Ok(None) => return Err(anyhow!("signaling disconnected during device discovery")),
            Err(_) => continue,
        };
        if !msg.is_text() {
            continue;
        }
        seen_messages += 1;
        let text = msg.into_text()?;
        let v: Value = serde_json::from_str(&text).context("parse signaling message failed")?;

        if v["type"] != "device" {
            continue;
        }

        let Some(list) = v["payload"]["deviceList"].as_array() else {
            continue;
        };
        seen_lists += 1;
        let mut lines = Vec::new();
        for d in list {
            let id = d["id"].as_str().unwrap_or("");
            let name = d["name"].as_str().unwrap_or("");
            let online = d["online"].as_bool().unwrap_or(false);
            lines.push(format!("{name}({id}) online={online}"));
        }
        last_list_summary = lines.join(", ");

        if let Some(ref preferred_name) = preferred_name {
            if let Some(id) = list.iter().find_map(|d| {
                let name = d["name"].as_str().unwrap_or("");
                let online = d["online"].as_bool().unwrap_or(true);
                let id = d["id"].as_str().unwrap_or("");
                if online && name == preferred_name && !id.is_empty() {
                    Some(id.to_string())
                } else {
                    None
                }
            }) {
                println!("device_discovery=matched_preferred_name");
                return Ok(id);
            }
        }

        if let Some(id) = list.iter().find_map(|d| {
            let online = d["online"].as_bool().unwrap_or(true);
            let id = d["id"].as_str().unwrap_or("");
            let name = d["name"].as_str().unwrap_or("");
            if online
                && !id.is_empty()
                && !name.is_empty()
                && !name.eq_ignore_ascii_case("m2-probe")
                && !name.eq_ignore_ascii_case("Web 控制端")
            {
                Some(id.to_string())
            } else {
                None
            }
        }) {
            println!("device_discovery=matched_first_online_agent_like");
            return Ok(id);
        }
    }
}
