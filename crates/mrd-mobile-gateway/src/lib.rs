//! Opt-in private-network gateway for an Android controller and target.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{
        header::{HOST, ORIGIN, X_FRAME_OPTIONS},
        uri::Authority,
        HeaderMap, StatusCode,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use bytes::Bytes;
#[cfg(any(windows, test))]
use serde::Deserialize;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
#[cfg(windows)]
use tokio::sync::mpsc;
use tokio::sync::{broadcast, watch, Mutex};
use tracing::info;
#[cfg(windows)]
use tracing::warn;

#[cfg(windows)]
mod video;

// DXGI permits only one duplication per output in a process. Keep ownership
// until the blocking capture worker has actually dropped its GPU resources.
#[cfg(windows)]
static DESKTOP_CAPTURE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Clone)]
struct MobileState {
    phone_frames: watch::Sender<Option<Bytes>>,
    phone_controls: broadcast::Sender<String>,
    phone_publisher_active: Arc<Mutex<bool>>,
}

#[cfg(any(windows, test))]
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DesktopControl {
    Pointer { x: f64, y: f64, action: String },
    Wheel { delta: i32 },
    Key { code: u16, pressed: bool },
    Text { value: String },
    RequestKeyframe,
    Ping { sent_us: u64 },
    Stop,
}

pub fn router_from_env() -> Router {
    let enabled = env::var("MRD_MOBILE_GATEWAY_ENABLED").is_ok_and(|value| value == "1");
    if !enabled {
        return Router::new();
    }
    router()
}

pub fn router() -> Router {
    let (phone_frames, _) = watch::channel(None);
    let (phone_controls, _) = broadcast::channel(32);
    let state = MobileState {
        phone_frames,
        phone_controls,
        phone_publisher_active: Arc::new(Mutex::new(false)),
    };
    info!("mobile gateway enabled on the web bridge listener");
    Router::new()
        .route("/mobile/desktop/ws", get(desktop_ws))
        .route("/mobile/desktop/video/ws", get(desktop_video_ws))
        .route("/mobile/desktop", get(desktop_page))
        .route("/mobile/phone/publish/ws", get(phone_publish_ws))
        .route("/mobile/phone/control/ws", get(phone_control_ws))
        .route("/mobile/phone", get(phone_page))
        .with_state(state)
}

fn is_trusted_peer(peer: IpAddr) -> bool {
    match peer {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

fn origin_matches_host(origin: Option<&str>, host: Option<&str>) -> bool {
    match origin {
        None => true,
        Some(origin) => host.is_some_and(|host| {
            origin.eq_ignore_ascii_case(&format!("http://{host}"))
                || origin.eq_ignore_ascii_case(&format!("https://{host}"))
        }),
    }
}

fn host_is_trusted(host: &str) -> bool {
    if host.contains('@') {
        return false;
    }
    let Ok(authority) = host.parse::<Authority>() else {
        return false;
    };
    let name = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(is_trusted_peer)
}

fn allowed_request(peer: SocketAddr, headers: &HeaderMap) -> bool {
    let origin = headers
        .get(ORIGIN)
        .map(|value| value.to_str().unwrap_or(""));
    let host = headers.get(HOST).and_then(|value| value.to_str().ok());
    is_trusted_peer(peer.ip())
        && host.is_some_and(host_is_trusted)
        && origin_matches_host(origin, host)
}

async fn phone_page(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    allowed_request(peer, &headers)
        .then_some((
            [(X_FRAME_OPTIONS, "DENY")],
            Html(include_str!("mobile_phone_page.html")),
        ))
        .ok_or(StatusCode::FORBIDDEN)
}

async fn desktop_ws(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    if !allowed_request(peer, &headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(desktop_session))
}

async fn desktop_page(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    allowed_request(peer, &headers)
        .then_some((
            [(X_FRAME_OPTIONS, "DENY")],
            Html(include_str!("desktop_video_page.html")),
        ))
        .ok_or(StatusCode::FORBIDDEN)
}

async fn desktop_video_ws(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    if !allowed_request(peer, &headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(|mut socket| async move {
        if !send_ready(&mut socket).await {
            return;
        }
        #[cfg(windows)]
        desktop_session_windows(socket, true).await;
        #[cfg(not(windows))]
        let _ = socket
            .send(Message::Text(
                "{\"type\":\"error\",\"message\":\"Windows capture is required\"}".into(),
            ))
            .await;
    }))
}

async fn phone_publish_ws(
    State(state): State<MobileState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    if !allowed_request(peer, &headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(move |socket| phone_publish_session(socket, state)))
}

async fn phone_control_ws(
    State(state): State<MobileState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    if !allowed_request(peer, &headers) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(move |socket| phone_control_session(socket, state)))
}

async fn send_ready(socket: &mut WebSocket) -> bool {
    socket
        .send(Message::Text("{\"type\":\"ready\"}".into()))
        .await
        .is_ok()
}

async fn phone_publish_session(mut socket: WebSocket, state: MobileState) {
    if !send_ready(&mut socket).await {
        return;
    }
    {
        let mut active = state.phone_publisher_active.lock().await;
        if *active {
            let _ = socket
                .send(Message::Text(
                    "{\"type\":\"error\",\"message\":\"phone publisher already active\"}".into(),
                ))
                .await;
            return;
        }
        *active = true;
    }
    let mut controls = state.phone_controls.subscribe();
    let mut last_frame_at = std::time::Instant::now() - Duration::from_secs(1);
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Binary(bytes))) if validate_jpeg(&bytes).is_ok() => {
                    if last_frame_at.elapsed() >= Duration::from_millis(100) {
                        last_frame_at = std::time::Instant::now();
                        state.phone_frames.send_replace(Some(bytes));
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            },
            control = controls.recv() => if let Ok(control) = control {
                if socket.send(Message::Text(control.into())).await.is_err() { break; }
            }
        }
    }
    state.phone_frames.send_replace(None);
    *state.phone_publisher_active.lock().await = false;
}

async fn phone_control_session(mut socket: WebSocket, state: MobileState) {
    if !send_ready(&mut socket).await {
        return;
    }
    let mut frames = state.phone_frames.subscribe();
    let first_frame = { frames.borrow_and_update().clone() };
    if let Some(frame) = first_frame {
        if socket.send(Message::Binary(frame)).await.is_err() {
            return;
        }
    }
    loop {
        tokio::select! {
            changed = frames.changed() => {
                if changed.is_err() { break; }
                let frame = { frames.borrow_and_update().clone() };
                if let Some(frame) = frame {
                    if socket.send(Message::Binary(frame)).await.is_err() { break; }
                } else if socket.send(Message::Text("{\"type\":\"offline\"}".into())).await.is_err() { break; }
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) if text.len() <= 4096 => {
                    if *state.phone_publisher_active.lock().await && is_valid_phone_control(&text) {
                        let _ = state.phone_controls.send(text.to_string());
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}

fn is_valid_phone_control(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    let kind = value.get("type").and_then(|v| v.as_str());
    match kind {
        Some("tap") => value
            .get("x")
            .and_then(|v| v.as_f64())
            .zip(value.get("y").and_then(|v| v.as_f64()))
            .is_some_and(|(x, y)| map_pointer(x, y, 1000, 1000).is_ok()),
        Some("swipe") => ["x", "y", "endX", "endY"].iter().all(|key| {
            value
                .get(*key)
                .and_then(|v| v.as_f64())
                .is_some_and(|v| v.is_finite() && (0.0..=1.0).contains(&v))
        }),
        Some("back" | "home") => true,
        Some("text") => value
            .get("text")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v.len() <= 512),
        _ => false,
    }
}

async fn desktop_session(mut socket: WebSocket) {
    if !send_ready(&mut socket).await {
        return;
    }
    #[cfg(not(windows))]
    {
        let _ = socket
            .send(Message::Text(
                "{\"type\":\"error\",\"message\":\"Windows capture is required\"}".into(),
            ))
            .await;
    }
    #[cfg(windows)]
    desktop_session_windows(socket, false).await;
}

#[cfg(windows)]
async fn desktop_session_windows(mut socket: WebSocket, h264: bool) {
    use mrd_input::{InputButton, InputEvent, InputInjector, InputKey};
    let permit = match tokio::time::timeout(Duration::from_secs(2), DESKTOP_CAPTURE.acquire()).await
    {
        Ok(Ok(permit)) => permit,
        _ => {
            let _ = socket.send(Message::Text(
                "{\"type\":\"error\",\"message\":\"当前电脑屏幕正在另一会话中使用，请先断开该会话\"}".into()
            )).await;
            return;
        }
    };
    let (tx, mut rx) = mpsc::channel::<(Message, u32, u32)>(2);
    let running = Arc::new(AtomicBool::new(true));
    let _capture_guard = video::StopCapture(running.clone());
    let keyframe = Arc::new(AtomicBool::new(false));
    let capture_keyframe = keyframe.clone();
    let capture_running = running.clone();
    let capture_task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if h264 {
            video::capture(tx, capture_running, capture_keyframe);
        } else {
            capture_desktop(tx, capture_running);
        }
    });
    let mut injector = mrd_input::windows::WindowsSendInputInjector::new();
    let (mut width, mut height) = (0, 0);
    let mut pressed = false;
    let mut keys = std::collections::HashSet::new();
    loop {
        tokio::select! {
            frame = rx.recv() => {
                let Some((message, w, h)) = frame else { break; };
                if w > 0 && (width, height) != (w, h) {
                    (width, height) = (w, h);
                    let dimensions = format!("{{\"type\":\"dimensions\",\"width\":{w},\"height\":{h}}}");
                    if socket.send(Message::Text(dimensions.into())).await.is_err() { break; }
                }
                if tokio::time::timeout(Duration::from_secs(2), socket.send(message)).await
                    .map_or(true, |result| result.is_err()) { break; }
            },
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Binary(_))) => continue,
                    _ => break,
                };
                if text.len() > 1024 { continue; }
                let Ok(control) = serde_json::from_str::<DesktopControl>(&text) else { continue; };
                match control {
                    DesktopControl::RequestKeyframe => keyframe.store(true, Ordering::Relaxed),
                    DesktopControl::Ping { sent_us } => {
                        let response = serde_json::json!({"type":"pong","sent_us":sent_us,"server_us":video::now_us()}).to_string();
                        if socket.send(Message::Text(response.into())).await.is_err() { break; }
                    }
                    DesktopControl::Pointer { x, y, action } => {
                        if let Ok((x, y)) = map_pointer(x, y, width, height) {
                            let _ = injector.inject(&InputEvent::MouseMove { x, y });
                            match action.as_str() {
                                "down" if !pressed => { let _ = injector.inject(&InputEvent::MouseButton { button: InputButton::Left, pressed: true }); pressed = true; },
                                "up" if pressed => { let _ = injector.inject(&InputEvent::MouseButton { button: InputButton::Left, pressed: false }); pressed = false; },
                                _ => {}
                            }
                        }
                    }
                    DesktopControl::Wheel { delta } if (-1200..=1200).contains(&delta) => { let _ = injector.inject(&InputEvent::MouseWheel { delta }); }
                    DesktopControl::Key { code, pressed: down } if code > 0 => {
                        let key = InputKey::VirtualKey(code);
                        let _ = injector.inject(&InputEvent::Key { key, pressed: down });
                        if down { keys.insert(key); } else { keys.remove(&key); }
                    }
                    DesktopControl::Text { value } if is_valid_desktop_text(&value) => {
                        if let Err(error) = inject_unicode_text(&value) { warn!("desktop text injection failed: {error}"); }
                    }
                    DesktopControl::Stop => break,
                    _ => {}
                }
            }
        }
    }
    running.store(false, Ordering::Relaxed);
    if pressed {
        let _ = injector.inject(&InputEvent::MouseButton {
            button: InputButton::Left,
            pressed: false,
        });
    }
    for key in keys {
        let _ = injector.inject(&InputEvent::Key {
            key,
            pressed: false,
        });
    }
    capture_task.abort();
}

#[cfg(windows)]
fn capture_desktop(tx: mpsc::Sender<(Message, u32, u32)>, running: Arc<AtomicBool>) {
    use mrd_capture_dxgi::DxgiDesktopCapture;
    use mrd_pipeline_core::FrameCapture;
    let Ok(mut capture) = DxgiDesktopCapture::new_primary() else {
        return;
    };
    while running.load(Ordering::Relaxed) {
        let started = std::time::Instant::now();
        let Ok(frame) = capture.capture_frame() else {
            break;
        };
        if frame.pixel_format == mrd_pipeline_core::FramePixelFormat::Bgra32 {
            if let Ok(jpeg) =
                encode_desktop_jpeg(&frame.data, frame.width as u32, frame.height as u32)
            {
                if matches!(
                    tx.try_send((
                        Message::Binary(Bytes::from(jpeg)),
                        frame.width as u32,
                        frame.height as u32
                    )),
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
                ) {
                    break;
                }
            }
        }
        let remaining = Duration::from_millis(150).saturating_sub(started.elapsed());
        std::thread::sleep(remaining);
    }
}

#[cfg(windows)]
fn encode_desktop_jpeg(bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || bgra.len() != width as usize * height as usize * 4 {
        return Err("invalid desktop frame".into());
    }
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let image = image::RgbImage::from_raw(width, height, rgb).ok_or("invalid RGB frame")?;
    let scale = (1280.0 / width as f64).min(720.0 / height as f64).min(1.0);
    let output = if scale < 1.0 {
        image::imageops::resize(
            &image,
            (width as f64 * scale) as u32,
            (height as f64 * scale) as u32,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 68)
        .encode_image(&output)
        .map_err(|error| error.to_string())?;
    if validate_jpeg(&bytes).is_err() {
        return Err("encoded JPEG exceeds size limit".into());
    }
    Ok(bytes)
}

#[cfg(any(windows, test))]
fn is_valid_desktop_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(|character| character == '\0')
}

#[cfg(windows)]
fn inject_unicode_text(value: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        VIRTUAL_KEY,
    };
    let mut inputs = Vec::with_capacity(value.encode_utf16().count() * 2);
    for code in value.encode_utf16() {
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(format!(
            "SendInput accepted {sent}/{} Unicode events",
            inputs.len()
        ))
    }
}

pub const MAX_JPEG_BYTES: usize = 2 * 1024 * 1024;

pub fn map_pointer(x: f64, y: f64, width: u32, height: u32) -> Result<(i32, i32), &'static str> {
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=1.0).contains(&x)
        || !(0.0..=1.0).contains(&y)
        || width == 0
        || height == 0
    {
        return Err("invalid pointer coordinates");
    }
    Ok((
        ((width - 1) as f64 * x).round() as i32,
        ((height - 1) as f64 * y).round() as i32,
    ))
}

pub fn validate_jpeg(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() < 4
        || bytes.len() > MAX_JPEG_BYTES
        || !bytes.starts_with(&[0xff, 0xd8])
        || !bytes.ends_with(&[0xff, 0xd9])
    {
        return Err("invalid or oversized JPEG frame");
    }
    Ok(())
}

pub fn discovery_reply(probe: &[u8], name: &str, port: u16) -> Option<String> {
    if probe != b"MRD_DISCOVER_V1" || port == 0 {
        return None;
    }
    let safe_name: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .take(48)
        .collect();
    Some(
        serde_json::json!({ "type": "rdesk_gateway", "name": safe_name, "port": port }).to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_gateway_rejects_out_of_bounds_pointer() {
        assert!(map_pointer(0.5, 0.5, 1920, 1080).is_ok());
        assert!(map_pointer(-0.1, 0.5, 1920, 1080).is_err());
        assert!(map_pointer(0.5, 1.1, 1920, 1080).is_err());
    }

    #[test]
    fn mobile_gateway_rejects_oversized_or_invalid_jpeg() {
        assert!(validate_jpeg(&[0xff, 0xd8, 0xff, 0xd9]).is_ok());
        assert!(validate_jpeg(&[1, 2, 3]).is_err());
        assert!(validate_jpeg(&vec![0xff; MAX_JPEG_BYTES + 1]).is_err());
    }

    #[test]
    fn mobile_gateway_accepts_unicode_desktop_text_and_bounds_its_length() {
        let message =
            serde_json::from_str::<DesktopControl>(r#"{"type":"text","value":"你好"}"#).unwrap();
        assert!(matches!(message, DesktopControl::Text { value } if value == "你好"));
        assert!(is_valid_desktop_text("你好"));
        assert!(!is_valid_desktop_text(&"a".repeat(513)));
    }

    #[test]
    fn mobile_gateway_deserializes_desktop_control_fields() {
        let parse = |text| serde_json::from_str::<DesktopControl>(text).unwrap();
        assert!(matches!(
            parse(r#"{"type":"pointer","x":0.25,"y":0.75,"action":"down"}"#),
            DesktopControl::Pointer { x, y, action }
                if map_pointer(x, y, 5, 5) == Ok((1, 3)) && action == "down"
        ));
        let DesktopControl::Wheel { delta } = parse(r#"{"type":"wheel","delta":-120}"#) else {
            panic!("expected wheel control");
        };
        assert_eq!(delta, -120);
        let DesktopControl::Key { code, pressed } =
            parse(r#"{"type":"key","code":65,"pressed":true}"#)
        else {
            panic!("expected key control");
        };
        assert_eq!(code, 65);
        assert!(pressed);
        let DesktopControl::Ping { sent_us } = parse(r#"{"type":"ping","sent_us":123}"#) else {
            panic!("expected ping control");
        };
        assert_eq!(sent_us, 123);
        assert!(matches!(
            parse(r#"{"type":"request_keyframe"}"#),
            DesktopControl::RequestKeyframe
        ));
        assert!(matches!(parse(r#"{"type":"stop"}"#), DesktopControl::Stop));
    }

    #[test]
    fn mobile_gateway_rejects_invalid_phone_touch_coordinates() {
        assert!(!is_valid_phone_control(r#"{"type":"tap","x":1.4,"y":0.5}"#));
        assert!(is_valid_phone_control(r#"{"type":"tap","x":0.4,"y":0.5}"#));
    }

    #[test]
    fn discovery_only_answers_expected_probe_without_secret() {
        assert!(discovery_reply(b"MRD_DISCOVER_V1", "Office-PC", 9534).is_some());
        assert!(discovery_reply(b"other", "Office-PC", 9534).is_none());
        let reply = discovery_reply(b"MRD_DISCOVER_V1", "Office-PC", 9534).unwrap();
        assert!(reply.contains("Office-PC"));
        assert!(reply.contains("9534"));
        assert!(!reply.contains("token"));
    }

    #[test]
    fn direct_connection_only_accepts_private_or_loopback_peers() {
        assert!(is_trusted_peer("192.168.10.102".parse().unwrap()));
        assert!(is_trusted_peer("192.168.1.253".parse().unwrap()));
        assert!(is_trusted_peer("127.0.0.1".parse().unwrap()));
        assert!(!is_trusted_peer("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn browser_origin_must_match_requested_host() {
        assert!(origin_matches_host(
            Some("http://127.0.0.1:9534"),
            Some("127.0.0.1:9534")
        ));
        assert!(origin_matches_host(None, Some("192.168.1.253:9534")));
        assert!(!origin_matches_host(
            Some("https://evil.example"),
            Some("127.0.0.1:9534")
        ));
        assert!(!origin_matches_host(Some("null"), Some("127.0.0.1:9534")));
    }

    #[test]
    fn browser_host_must_be_a_private_ip_or_localhost() {
        assert!(host_is_trusted("192.168.1.253:9534"));
        assert!(host_is_trusted("localhost:9534"));
        assert!(!host_is_trusted("evil.example:9534"));
        assert!(!host_is_trusted("8.8.8.8:9534"));
    }
}
