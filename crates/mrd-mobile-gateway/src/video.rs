//! GPU desktop capture and paced, bounded H.264 delivery.
use axum::extract::ws::Message;
use bytes::Bytes;
use mrd_capture_dxgi::DxgiSharedTextureCapture;
use mrd_encode_nvenc::NvencH264Encoder;
use mrd_pipeline_core::{FrameCapture, VideoEncoder};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

pub const FPS: u32 = 60;
const INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);

pub struct StopCapture(pub Arc<AtomicBool>);
impl Drop for StopCapture {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

pub fn capture(
    tx: mpsc::Sender<(Message, u32, u32)>,
    running: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
) {
    if let Err(error) = run(&tx, &running, &keyframe) {
        tracing::warn!("desktop H.264 pipeline: {error}");
        let message = serde_json::json!({"type":"error","message":error}).to_string();
        let _ = tx.blocking_send((Message::Text(message.into()), 0, 0));
    }
}

fn run(
    tx: &mpsc::Sender<(Message, u32, u32)>,
    running: &AtomicBool,
    keyframe: &AtomicBool,
) -> Result<(), String> {
    let mut capture = DxgiSharedTextureCapture::new_primary().map_err(|e| e.to_string())?;
    // Preserve the full desktop. set_target_dimensions crops rather than scales.
    let (width, height) = (capture.width(), capture.height());
    let mut encoder = NvencH264Encoder::new_max_speed_with_bitrate(width, height, FPS, 16_000_000)
        .map_err(|e| format!("NVENC unavailable: {e}"))?;
    tracing::info!("mobile H.264: {width}x{height}, {FPS} FPS, GPU shared textures, 16 Mbps");
    let mut deadline = Instant::now();
    let mut sequence = 0u64;
    let mut recover = true;
    let mut codec = String::new();
    while running.load(Ordering::Relaxed) && !tx.is_closed() {
        wait_until(deadline);
        if !running.load(Ordering::Relaxed) {
            break;
        }
        let started = Instant::now();
        if started.saturating_duration_since(deadline) > INTERVAL {
            deadline = started;
        }
        deadline += INTERVAL;
        let capture_time = now_us();
        let mut frame = capture.capture_frame().map_err(|e| e.to_string())?;
        frame.timestamp_us = capture_time;
        let capture_us = started.elapsed().as_micros() as u64;
        if recover || keyframe.swap(false, Ordering::Relaxed) {
            encoder.request_keyframe();
        }
        let encode_start = Instant::now();
        let units = encoder.encode(&frame).map_err(|e| e.to_string())?;
        let encode_us = encode_start.elapsed().as_micros() as u64;
        for unit in units {
            sequence += 1;
            if let Some(actual_codec) = sps_codec(&unit.bytes) {
                codec = actual_codec;
            }
            // After queue pressure, reference frames are invalid until the next IDR.
            if recover && !unit.is_keyframe {
                continue;
            }
            let header = serde_json::json!({
                "type":"mrd.webcodecs.frame.v1",
                "sequence":sequence, "timestamp_us":unit.timestamp_us,
                "capture_unix_us":unit.timestamp_us, "duration_us":1_000_000/FPS,
                "width":width,"height":height, "keyframe":unit.is_keyframe,
                "codec":codec,"codec_format":"annexb",
                // Submission cost includes draining the previous asynchronous output.
                // It is not the hardware's complete encode latency for this frame.
                "capture_call_us":capture_us, "encode_call_us":encode_us
            });
            let json = serde_json::to_vec(&header).map_err(|e| e.to_string())?;
            let mut packet = Vec::with_capacity(12 + json.len() + unit.bytes.len());
            packet.extend_from_slice(b"MRDWC01\0");
            packet.extend_from_slice(&(json.len() as u32).to_le_bytes());
            packet.extend_from_slice(&json);
            packet.extend_from_slice(&unit.bytes);
            if !enqueue_frame(
                tx,
                &mut recover,
                unit.is_keyframe,
                (
                    Message::Binary(Bytes::from(packet)),
                    width as u32,
                    height as u32,
                ),
            ) {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn sps_codec(data: &[u8]) -> Option<String> {
    // Both Annex B start-code lengths end in 00 00 01.
    data.windows(7).find_map(|nal| {
        (nal[..3] == [0, 0, 1] && nal[3] & 31 == 7)
            .then(|| format!("avc1.{:02x}{:02x}{:02x}", nal[4], nal[5], nal[6]))
    })
}

fn enqueue_frame(
    tx: &mpsc::Sender<(Message, u32, u32)>,
    recover: &mut bool,
    keyframe: bool,
    packet: (Message, u32, u32),
) -> bool {
    if *recover && !keyframe {
        return !tx.is_closed();
    }
    match tx.try_send(packet) {
        Ok(()) => *recover = false,
        Err(mpsc::error::TrySendError::Full(_)) => *recover = true,
        Err(mpsc::error::TrySendError::Closed(_)) => return false,
    }
    true
}

fn wait_until(deadline: Instant) {
    loop {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if left > Duration::from_millis(2) {
            std::thread::sleep(left - Duration::from_millis(1));
        } else if left > Duration::from_micros(250) {
            std::thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
}

pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_is_read_from_the_actual_sps() {
        assert_eq!(
            sps_codec(&[0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x33]),
            Some("avc1.42c033".into())
        );
        assert_eq!(
            sps_codec(&[0, 0, 1, 0x67, 0x64, 0, 0x34]),
            Some("avc1.640034".into())
        );
        assert_eq!(sps_codec(&[0, 0, 1, 0x67, 0x42]), None);
    }

    #[test]
    fn queue_overflow_requires_idr_before_more_delta_frames() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut recover = false;
        let packet = || (Message::Binary(Bytes::from_static(b"frame")), 640, 480);
        assert!(enqueue_frame(&tx, &mut recover, false, packet()));
        assert!(enqueue_frame(&tx, &mut recover, false, packet()));
        assert!(recover);
        rx.try_recv().unwrap();
        assert!(enqueue_frame(&tx, &mut recover, false, packet()));
        assert!(rx.try_recv().is_err());
        assert!(enqueue_frame(&tx, &mut recover, true, packet()));
        assert!(!recover);
        rx.try_recv().unwrap();
        drop(rx);
        assert!(!enqueue_frame(&tx, &mut recover, true, packet()));
    }
}
