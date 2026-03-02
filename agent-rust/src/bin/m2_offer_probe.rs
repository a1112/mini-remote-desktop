use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;

#[tokio::main]
async fn main() -> Result<()> {
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

    let mut target_id = String::new();
    for _ in 0..10 {
        if let Some(msg) = read.next().await {
            let text = msg?.into_text()?;
            let v: Value = serde_json::from_str(&text)?;
            if v["type"] == "device" {
                if let Some(list) = v["payload"]["deviceList"].as_array() {
                    for d in list {
                        if let Some(id) = d["id"].as_str() {
                            target_id = id.to_string();
                            break;
                        }
                    }
                }
            }
            if !target_id.is_empty() {
                break;
            }
        }
    }
    if target_id.is_empty() {
        anyhow::bail!("no target device found");
    }
    println!("target_id={target_id}");

    let pc = build_pc(write.clone(), target_id.clone()).await?;
    let frame_count = Arc::new(AtomicU64::new(0));
    let packet_count = Arc::new(AtomicU64::new(0));
    {
        let frame_count = frame_count.clone();
        let packet_count = packet_count.clone();
        pc.on_track(Box::new(move |track, _, _| {
            let frame_count = frame_count.clone();
            let packet_count = packet_count.clone();
            Box::pin(async move {
                while let Ok((pkt, _)) = track.read_rtp().await {
                    packet_count.fetch_add(1, Ordering::Relaxed);
                    if pkt.header.marker {
                        frame_count.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        }));
    }

    pc.add_transceiver_from_kind(RTPCodecType::Video, None)
        .await?;
    let offer = pc.create_offer(None).await?;
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

    while Instant::now() < hard_deadline {
        if let Some(s) = media_start {
            if s.elapsed() >= Duration::from_secs(run_secs) {
                break;
            }
        }

        let msg = match timeout(Duration::from_millis(500), read.next()).await {
            Ok(Some(v)) => v?,
            Ok(None) => break,
            Err(_) => continue,
        };
        let text = msg.into_text()?;
        let v: Value = serde_json::from_str(&text)?;

        if v["type"] == "webrtc" && v["action"] == "answer" && media_start.is_none() {
            println!("answer_received=true");
            let sdp = v["payload"]["answer"]["sdp"]
                .as_str()
                .ok_or_else(|| anyhow!("answer.sdp missing"))?;
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
        "media_stats: seconds={:.2} frames={} packets={} estimated_fps={:.2}",
        secs, frames, packets, fps
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
        .await?,
    );

    {
        let write = write.clone();
        pc.on_ice_candidate(Box::new(move |cand| {
            let write = write.clone();
            let target_id = target_id.clone();
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
                    }
                }
            })
        }));
    }

    Ok(pc)
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
