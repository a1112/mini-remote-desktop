use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const PORT: u16 = 9527;

#[derive(Clone)]
struct Device {
    id: String,
    kind: String,
    name: String,
    tx: mpsc::UnboundedSender<Message>,
    last_seen: Instant,
}

#[derive(Default)]
struct State {
    devices: HashMap<String, Device>,
}

type SharedState = Arc<Mutex<State>>;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind(("0.0.0.0", PORT))
        .await
        .expect("bind signaling-rs failed");

    println!("[SIGNALING-RS] listening ws://0.0.0.0:{PORT}");

    let state = Arc::new(Mutex::new(State::default()));
    spawn_timeout_sweeper(state.clone());

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[SIGNALING-RS] accept error: {e}");
                continue;
            }
        };

        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, state).await {
                eprintln!("[SIGNALING-RS] {} error: {e}", addr);
            }
        });
    }
}

async fn handle_conn(stream: TcpStream, state: SharedState) -> Result<(), String> {
    let ws = accept_async(stream)
        .await
        .map_err(|e| format!("ws handshake failed: {e}"))?;

    let conn_id = Uuid::new_v4().to_string();
    let (mut ws_tx, mut ws_rx) = ws.split();

    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    let write_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    tx.send(Message::Text(
        json!({"type":"system","action":"connected","payload":{"deviceId":conn_id}})
            .to_string(),
    ))
    .map_err(|e| format!("send connected failed: {e}"))?;

    while let Some(msg) = ws_rx.next().await {
        let msg = match msg {
            Ok(v) => v,
            Err(e) => return Err(format!("ws read failed: {e}")),
        };

        if msg.is_close() {
            break;
        }

        if !msg.is_text() {
            continue;
        }

        let text = msg.into_text().map_err(|e| format!("text decode failed: {e}"))?;
        handle_message(&conn_id, &tx, &text, &state).await;
    }

    on_disconnect(&conn_id, &state).await;
    write_task.abort();
    Ok(())
}

async fn handle_message(conn_id: &str, tx: &mpsc::UnboundedSender<Message>, text: &str, state: &SharedState) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };

    let action = v["action"].as_str().unwrap_or("");

    match action {
        "register" => {
            let kind = v["payload"]["type"].as_str().unwrap_or("unknown").to_string();
            let name = v["payload"]["name"]
                .as_str()
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{kind}-{conn_id}"));

            {
                let mut s = state.lock().await;
                s.devices.insert(
                    conn_id.to_string(),
                    Device {
                        id: conn_id.to_string(),
                        kind: kind.clone(),
                        name: name.clone(),
                        tx: tx.clone(),
                        last_seen: Instant::now(),
                    },
                );
            }

            println!("[SUCCESS] {kind} registered: {name} ({conn_id})");
            send_registered(conn_id, tx, state).await;
            broadcast_device_list(state, Some(conn_id)).await;
        }
        "getDeviceList" => {
            send_device_list(tx, state).await;
        }
        "ping" => {
            touch(conn_id, state).await;
            let msg = json!({"type":"device","action":"pong"}).to_string();
            let _ = tx.send(Message::Text(msg));
        }
        "offer" => {
            let target = v["payload"]["targetDeviceId"].as_str().unwrap_or("");
            if target.is_empty() {
                return;
            }
            let offer = v["payload"]["offer"].clone();
            let msg = json!({
                "type":"webrtc",
                "action":"offer",
                "payload": {
                  "targetDeviceId": target,
                  "offer": offer,
                  "sessionId": Uuid::new_v4().to_string(),
                  "controllerId": conn_id
                }
            })
            .to_string();
            send_to_device(target, &msg, state).await;
        }
        "answer" => {
            let controller_id = v["payload"]["controllerId"].as_str().unwrap_or("");
            if controller_id.is_empty() {
                return;
            }
            let payload = v["payload"].clone();
            let msg = json!({"type":"webrtc","action":"answer","payload":payload}).to_string();
            send_to_device(controller_id, &msg, state).await;
        }
        "iceCandidate" => {
            let target = v["payload"]["targetDeviceId"].as_str().unwrap_or("");
            if target.is_empty() {
                return;
            }
            let candidate = v["payload"]["candidate"].clone();
            let msg = json!({
                "type":"webrtc",
                "action":"iceCandidate",
                "payload":{"candidate": candidate, "controllerId": conn_id}
            })
            .to_string();
            send_to_device(target, &msg, state).await;
        }
        _ => {}
    }
}

async fn send_registered(conn_id: &str, tx: &mpsc::UnboundedSender<Message>, state: &SharedState) {
    let list = build_device_list(state).await;
    let msg = json!({
        "type":"device",
        "action":"registered",
        "payload":{"deviceId": conn_id, "deviceList": list}
    })
    .to_string();
    let _ = tx.send(Message::Text(msg));
}

async fn send_device_list(tx: &mpsc::UnboundedSender<Message>, state: &SharedState) {
    let list = build_device_list(state).await;
    let msg = json!({"type":"device","action":"deviceList","payload":{"deviceList":list}}).to_string();
    let _ = tx.send(Message::Text(msg));
}

async fn build_device_list(state: &SharedState) -> Vec<Value> {
    let s = state.lock().await;
    s.devices
        .values()
        .filter(|d| d.kind == "agent" || d.kind == "agent-rust")
        .map(|d| json!({"id":d.id,"name":d.name,"online":true}))
        .collect()
}

async fn broadcast_device_list(state: &SharedState, exclude_id: Option<&str>) {
    let list = build_device_list(state).await;
    let msg = Message::Text(
        json!({"type":"device","action":"deviceList","payload":{"deviceList":list}})
            .to_string(),
    );

    let targets = {
        let s = state.lock().await;
        s.devices
            .values()
            .filter(|d| d.kind == "controller")
            .filter(|d| exclude_id.map(|x| x != d.id).unwrap_or(true))
            .map(|d| d.tx.clone())
            .collect::<Vec<_>>()
    };

    for tx in targets {
        let _ = tx.send(msg.clone());
    }
}

async fn send_to_device(target_id: &str, msg: &str, state: &SharedState) {
    let tx = {
        let s = state.lock().await;
        s.devices.get(target_id).map(|d| d.tx.clone())
    };
    if let Some(tx) = tx {
        let _ = tx.send(Message::Text(msg.to_string()));
    }
}

async fn touch(conn_id: &str, state: &SharedState) {
    let mut s = state.lock().await;
    if let Some(d) = s.devices.get_mut(conn_id) {
        d.last_seen = Instant::now();
    }
}

async fn on_disconnect(conn_id: &str, state: &SharedState) {
    let removed = {
        let mut s = state.lock().await;
        s.devices.remove(conn_id)
    };

    if let Some(d) = removed {
        println!("[INFO] device offline: {} ({})", d.name, d.id);

        let msg = Message::Text(
            json!({"type":"device","action":"offline","payload":{"deviceId":d.id}})
                .to_string(),
        );

        let targets = {
            let s = state.lock().await;
            s.devices
                .values()
                .filter(|v| v.kind == "controller")
                .map(|v| v.tx.clone())
                .collect::<Vec<_>>()
        };

        for tx in targets {
            let _ = tx.send(msg.clone());
        }
    }
}

fn spawn_timeout_sweeper(state: SharedState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let stale = {
                let s = state.lock().await;
                s.devices
                    .values()
                    .filter(|d| d.last_seen.elapsed() > Duration::from_secs(60))
                    .map(|d| d.id.clone())
                    .collect::<Vec<_>>()
            };

            for id in stale {
                on_disconnect(&id, &state).await;
            }
        }
    });
}
