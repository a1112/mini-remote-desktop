use anyhow::{Result, anyhow};
use common_control_proto::{ChannelClass, ControlEvent, Frame};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::audio_control::{AudioControlManager, AudioSession};
use crate::clipboard::ClipboardManager;
use crate::control_plane::dispatcher::MountDispatcher;
use crate::file_transfer::FileTransferManager;
use crate::webdav_mount::envelope::MountEnvelope;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};

#[derive(Debug)]
struct QueuedFrame {
    class: ChannelClass,
    recv_us: u64,
    frame: Frame,
}

#[derive(Clone)]
pub struct InputInjector {
    rt_tx: mpsc::Sender<QueuedFrame>,
    rel_tx: mpsc::Sender<QueuedFrame>,
}

#[derive(Default)]
struct InjectorContext {
    clipboard: StdMutex<ClipboardManager>,
    file_transfer: StdMutex<FileTransferManager>,
    audio: StdMutex<AudioControlManager>,
    mount_dispatcher: StdMutex<MountDispatcher>,
}

impl InputInjector {
    pub fn new() -> Self {
        let (rt_tx, mut rt_rx) = mpsc::channel::<QueuedFrame>(8);
        let (rel_tx, mut rel_rx) = mpsc::channel::<QueuedFrame>(128);
        let stats = Arc::new(Mutex::new(LatencySamples::default()));
        let ctx = Arc::new(InjectorContext::default());

        let stats_rt = stats.clone();
        let ctx_rt = ctx.clone();
        tokio::spawn(async move {
            while let Some(msg) = rt_rx.recv().await {
                let t3 = unix_time_us();
                let t4 = unix_time_us();
                let _ = inject_event(&msg.frame.event, &ctx_rt);
                let t5 = unix_time_us();
                record_stats(&stats_rt, &msg, t3, t4, t5).await;
                debug!(
                    seq = msg.frame.seq,
                    ts_us = msg.frame.ts_us,
                    class = ?msg.class,
                    "received realtime control frame"
                );
            }
        });

        let stats_rel = stats.clone();
        let ctx_rel = ctx.clone();
        tokio::spawn(async move {
            while let Some(msg) = rel_rx.recv().await {
                let t3 = unix_time_us();
                let t4 = unix_time_us();
                let _ = inject_event(&msg.frame.event, &ctx_rel);
                let t5 = unix_time_us();
                record_stats(&stats_rel, &msg, t3, t4, t5).await;
                debug!(
                    seq = msg.frame.seq,
                    ts_us = msg.frame.ts_us,
                    class = ?msg.class,
                    "received reliable control frame"
                );
            }
        });

        let stats_panel = stats.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                ticker.tick().await;
                let snapshot = {
                    let mut s = stats_panel.lock().await;
                    s.take_snapshot()
                };
                if let Some(s) = snapshot {
                    info!(
                        one_way_p50_ms = format!("{:.3}", s.one_way_p50_ms),
                        one_way_p95_ms = format!("{:.3}", s.one_way_p95_ms),
                        one_way_p99_ms = format!("{:.3}", s.one_way_p99_ms),
                        recv_queue_p95_ms = format!("{:.3}", s.recv_queue_p95_ms),
                        inject_p95_ms = format!("{:.3}", s.inject_p95_ms),
                        samples = s.samples,
                        "[CTRL-LAT]"
                    );
                }
            }
        });

        Self { rt_tx, rel_tx }
    }

    pub async fn push_raw(&self, class: ChannelClass, data: &[u8]) -> Result<()> {
        let frame = Frame::decode(data)?;
        let q = QueuedFrame {
            class,
            recv_us: unix_time_us(),
            frame,
        };
        match class {
            ChannelClass::Realtime => {
                if self.rt_tx.try_send(q).is_err() {
                    // Realtime queue is lossy by design.
                    warn!("dropping realtime control frame due to full queue");
                }
            }
            ChannelClass::Reliable => {
                self.rel_tx.send(q).await?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct LatencySamples {
    one_way_us: VecDeque<u64>,
    recv_queue_us: VecDeque<u64>,
    inject_us: VecDeque<u64>,
}

struct LatencySnapshot {
    one_way_p50_ms: f64,
    one_way_p95_ms: f64,
    one_way_p99_ms: f64,
    recv_queue_p95_ms: f64,
    inject_p95_ms: f64,
    samples: usize,
}

impl LatencySamples {
    fn push_window(q: &mut VecDeque<u64>, v: u64) {
        const MAX_SAMPLES: usize = 2048;
        q.push_back(v);
        while q.len() > MAX_SAMPLES {
            let _ = q.pop_front();
        }
    }

    fn take_snapshot(&mut self) -> Option<LatencySnapshot> {
        if self.one_way_us.is_empty() {
            return None;
        }
        let one = percentile_snapshot(&self.one_way_us);
        let recv_q = percentile(&self.recv_queue_us, 95).unwrap_or(0.0);
        let inject = percentile(&self.inject_us, 95).unwrap_or(0.0);
        Some(LatencySnapshot {
            one_way_p50_ms: one.0 / 1000.0,
            one_way_p95_ms: one.1 / 1000.0,
            one_way_p99_ms: one.2 / 1000.0,
            recv_queue_p95_ms: recv_q / 1000.0,
            inject_p95_ms: inject / 1000.0,
            samples: self.one_way_us.len(),
        })
    }
}

async fn record_stats(
    stats: &Arc<Mutex<LatencySamples>>,
    msg: &QueuedFrame,
    t3: u64,
    t4: u64,
    t5: u64,
) {
    let mut s = stats.lock().await;
    let recv_queue_us = t3.saturating_sub(msg.recv_us);
    let inject_us = t5.saturating_sub(t4);
    LatencySamples::push_window(&mut s.recv_queue_us, recv_queue_us);
    LatencySamples::push_window(&mut s.inject_us, inject_us);
    if msg.frame.ts_us > 0 {
        let one_way = t5.saturating_sub(msg.frame.ts_us);
        LatencySamples::push_window(&mut s.one_way_us, one_way);
    }
}

fn unix_time_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

fn percentile_snapshot(v: &VecDeque<u64>) -> (f64, f64, f64) {
    (
        percentile(v, 50).unwrap_or(0.0),
        percentile(v, 95).unwrap_or(0.0),
        percentile(v, 99).unwrap_or(0.0),
    )
}

fn percentile(v: &VecDeque<u64>, p: usize) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut x: Vec<u64> = v.iter().copied().collect();
    x.sort_unstable();
    let idx = ((x.len() - 1) * p) / 100;
    Some(x[idx] as f64)
}

fn inject_event(event: &ControlEvent, ctx: &Arc<InjectorContext>) -> Result<()> {
    match event {
        ControlEvent::ClipboardSet { mime, bytes } => {
            let mut clipboard = ctx
                .clipboard
                .lock()
                .map_err(|_| anyhow!("clipboard mutex poisoned"))?;
            clipboard.set(*mime, bytes.clone());
            debug!(
                mime = *mime,
                len = bytes.len(),
                "clipboard set event handled"
            );
            Ok(())
        }
        ControlEvent::ClipboardGet {} => {
            let clipboard = ctx
                .clipboard
                .lock()
                .map_err(|_| anyhow!("clipboard mutex poisoned"))?;
            if let Some(item) = clipboard.latest() {
                debug!(
                    mime = item.mime,
                    len = item.bytes.len(),
                    "clipboard get event handled"
                );
            }
            Ok(())
        }
        ControlEvent::FileControl {
            op,
            transfer_id,
            arg0,
            arg1,
        } => {
            let mut transfer = ctx
                .file_transfer
                .lock()
                .map_err(|_| anyhow!("file transfer mutex poisoned"))?;
            if let Some(done) = transfer.handle_control(*op, *transfer_id, *arg0, *arg1)? {
                info!(
                    transfer_id = done.transfer_id,
                    bytes = done.bytes.len(),
                    "file transfer completed"
                );
                match MountEnvelope::from_bytes(&done.bytes) {
                    Ok(envelope) => {
                        let mut dispatcher = ctx
                            .mount_dispatcher
                            .lock()
                            .map_err(|_| anyhow!("mount dispatcher mutex poisoned"))?;
                        match dispatcher.on_mount_envelope(&envelope) {
                            Ok(resp) => {
                                debug!(
                                    mount_id = envelope.mount_id,
                                    request_id = envelope.request_id,
                                    kind = envelope.kind,
                                    response = ?resp,
                                    "mount envelope handled via file transfer"
                                );
                            }
                            Err(err) => {
                                warn!(
                                    transfer_id = done.transfer_id,
                                    error = %err,
                                    "mount envelope dispatch failed"
                                );
                            }
                        }
                    }
                    Err(_) => {
                        debug!(
                            transfer_id = done.transfer_id,
                            "completed payload is not a mount envelope"
                        );
                    }
                }
            }
            Ok(())
        }
        ControlEvent::FileChunk {
            transfer_id,
            chunk_idx,
            total_chunks,
            sha256_16,
            payload,
        } => {
            let mut transfer = ctx
                .file_transfer
                .lock()
                .map_err(|_| anyhow!("file transfer mutex poisoned"))?;
            transfer.handle_chunk(
                *transfer_id,
                *chunk_idx,
                *total_chunks,
                *sha256_16,
                payload.clone(),
            )?;
            Ok(())
        }
        ControlEvent::AudioControl {
            op,
            codec,
            sample_rate,
            channels,
            frame_ms,
        } => {
            let mut audio = ctx
                .audio
                .lock()
                .map_err(|_| anyhow!("audio mutex poisoned"))?;
            audio.apply(AudioSession {
                op: *op,
                codec: *codec,
                sample_rate: *sample_rate,
                channels: *channels,
                frame_ms: *frame_ms,
            });
            debug!(
                op = *op,
                codec = *codec,
                sample_rate = *sample_rate,
                channels = *channels,
                frame_ms = *frame_ms,
                "audio control event handled"
            );
            Ok(())
        }
        ControlEvent::FileMount {
            op,
            mount_id,
            flags,
            path,
        } => {
            let mut dispatcher = ctx
                .mount_dispatcher
                .lock()
                .map_err(|_| anyhow!("mount dispatcher mutex poisoned"))?;
            let resp = dispatcher.on_file_mount(*op, *mount_id, *flags, path.clone())?;
            debug!(mount_id = *mount_id, op = *op, response = ?resp, "file mount event handled");
            Ok(())
        }
        _ => {
            #[cfg(windows)]
            {
                inject_event_windows(event)
            }
            #[cfg(not(windows))]
            {
                let _ = event;
                Ok(())
            }
        }
    }
}

#[cfg(windows)]
fn inject_event_windows(event: &ControlEvent) -> Result<()> {
    match event {
        ControlEvent::MouseMove { x, y } => {
            send_mouse(*x, *y, MOUSEEVENTF_MOVE, 0)?;
        }
        ControlEvent::MouseButton { button, pressed } => {
            let flags = match (*button, *pressed) {
                (0, true) => MOUSEEVENTF_LEFTDOWN,
                (0, false) => MOUSEEVENTF_LEFTUP,
                (1, true) => MOUSEEVENTF_RIGHTDOWN,
                (1, false) => MOUSEEVENTF_RIGHTUP,
                (2, true) => MOUSEEVENTF_MIDDLEDOWN,
                (2, false) => MOUSEEVENTF_MIDDLEUP,
                _ => return Ok(()),
            };
            send_mouse(0, 0, flags, 0)?;
        }
        ControlEvent::MouseWheel { delta } => {
            send_mouse(0, 0, MOUSEEVENTF_WHEEL, *delta as u32)?;
        }
        ControlEvent::Key { key, pressed } => {
            send_key(*key as u16, *pressed)?;
        }
        ControlEvent::GamepadAxis { .. } | ControlEvent::GamepadButton { .. } => {
            // Stub for current phase: gamepad virtualization will be implemented next.
            warn!("gamepad event received but not yet implemented in SendInput path");
        }
        ControlEvent::ClipboardSet { .. }
        | ControlEvent::ClipboardGet {}
        | ControlEvent::FileControl { .. }
        | ControlEvent::FileChunk { .. }
        | ControlEvent::AudioControl { .. }
        | ControlEvent::FileMount { .. } => {}
    }
    Ok(())
}

#[cfg(windows)]
fn send_key(vk: u16, pressed: bool) -> Result<()> {
    let flags = if pressed {
        KEYBD_EVENT_FLAGS(0)
    } else {
        KEYEVENTF_KEYUP
    };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_input(&[input])
}

#[cfg(windows)]
fn send_mouse(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS, mouse_data: u32) -> Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send_input(&[input])
}

#[cfg(windows)]
fn send_input(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == 0 {
        return Err(anyhow!("SendInput returned 0"));
    }
    Ok(())
}
