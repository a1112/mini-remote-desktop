use crate::audio_quic_rx::AudioFrame;
use crate::recorder::Recorder;
use anyhow::{anyhow, Context, Result};
use audiopus::coder::Decoder as OpusDecoder;
use audiopus::{packet::Packet, Channels, MutSignals, SampleRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::convert::TryFrom;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Default)]
struct AudioStats {
    frames: u64,
    dropped: u64,
    latency_ms_sum: f64,
    latency_ms_max: f64,
    av_skew_abs_ms: VecDeque<f64>,
    av_skew_signed_ms: VecDeque<f64>,
}

fn unix_time_us() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_micros().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

fn p95(vals: &VecDeque<f64>) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f64> = vals.iter().copied().collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((s.len() - 1) * 95) / 100;
    s[idx]
}

fn avg(vals: &VecDeque<f64>) -> f64 {
    if vals.is_empty() {
        return 0.0;
    }
    vals.iter().sum::<f64>() / vals.len() as f64
}

pub fn spawn_audio_playback(
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<AudioFrame>>>,
    latest_video_tx_us: Arc<AtomicU64>,
    recorder: Option<Arc<Recorder>>,
) -> Result<()> {
    let avsync_enable = std::env::var("MRD_AUDIO_AVSYNC_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    let avsync_target_ms = std::env::var("MRD_AUDIO_AVSYNC_TARGET_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(2.0)
        .clamp(0.0, 50.0);
    let avsync_deadzone_ms = std::env::var("MRD_AUDIO_AVSYNC_DEADZONE_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.5)
        .clamp(0.0, 20.0);
    let avsync_max_delay_ms = std::env::var("MRD_AUDIO_AVSYNC_MAX_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(80.0)
        .clamp(0.0, 500.0);
    let avsync_pass_p95_ms = std::env::var("MRD_AUDIO_AVSYNC_PASS_P95_MS")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(20.0)
        .clamp(0.0, 500.0);

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("audio playback device unavailable")?;
    let default_cfg = device
        .default_output_config()
        .context("audio playback config unavailable")?;

    let queue = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(48_000)));
    let stats = Arc::new(Mutex::new(AudioStats::default()));

    let q_consume = queue.clone();
    let err_fn = |err| warn!(error = %err, "audio output stream error");
    let stream = match default_cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let cfg = default_cfg.config();
            device.build_output_stream(
                &cfg,
                move |output: &mut [f32], _| {
                    let mut q = match q_consume.lock() {
                        Ok(v) => v,
                        Err(_) => return,
                    };
                    for sample in output.iter_mut() {
                        *sample = q.pop_front().unwrap_or(0.0);
                    }
                },
                err_fn,
                None,
            )?
        }
        other => {
            return Err(anyhow!("unsupported playback sample format: {other:?}"));
        }
    };
    stream
        .play()
        .context("start audio playback stream failed")?;
    info!(
        avsync_enable,
        avsync_target_ms = format!("{:.3}", avsync_target_ms),
        avsync_deadzone_ms = format!("{:.3}", avsync_deadzone_ms),
        avsync_max_delay_ms = format!("{:.3}", avsync_max_delay_ms),
        avsync_pass_p95_ms = format!("{:.3}", avsync_pass_p95_ms),
        "audio playback started"
    );

    let stats_log = stats.clone();
    let avsync_enable_for_log = avsync_enable;
    let avsync_target_ms_for_log = avsync_target_ms;
    let avsync_deadzone_ms_for_log = avsync_deadzone_ms;
    let avsync_max_delay_ms_for_log = avsync_max_delay_ms;
    let avsync_pass_p95_ms_for_log = avsync_pass_p95_ms;
    tokio::spawn(async move {
        let mut log_every = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            log_every.tick().await;
            if let Ok(mut s) = stats_log.lock() {
                let playback_avg = if s.frames > 0 {
                    s.latency_ms_sum / s.frames as f64
                } else {
                    0.0
                };
                let av_abs_p95 = p95(&s.av_skew_abs_ms);
                let av_signed_avg = avg(&s.av_skew_signed_ms);
                info!(
                    side = "controller_audio",
                    window_s = 2,
                    frames = s.frames,
                    dropped = s.dropped,
                    stage_audio_playback_avg_ms = format!("{:.3}", playback_avg),
                    stage_audio_playback_max_ms = format!("{:.3}", s.latency_ms_max),
                    av_sync_signed_avg_ms = format!("{:.3}", av_signed_avg),
                    av_sync_abs_p95_ms = format!("{:.3}", av_abs_p95),
                    av_sync_align_enabled = avsync_enable_for_log,
                    av_sync_target_ms = format!("{:.3}", avsync_target_ms_for_log),
                    av_sync_deadzone_ms = format!("{:.3}", avsync_deadzone_ms_for_log),
                    av_sync_max_delay_ms = format!("{:.3}", avsync_max_delay_ms_for_log),
                    av_sync_pass_p95_ms = format!("{:.3}", avsync_pass_p95_ms_for_log),
                    "[AUDIO-PIPELINE-STATS]"
                );
                info!(
                    window_s = 2,
                    av_sync_signed_avg_ms = format!("{:.3}", av_signed_avg),
                    av_sync_abs_p95_ms = format!("{:.3}", av_abs_p95),
                    target_p95_ms = format!("{:.3}", avsync_pass_p95_ms_for_log),
                    av_sync_align_enabled = avsync_enable_for_log,
                    verdict = if av_abs_p95 <= avsync_pass_p95_ms_for_log {
                        "pass"
                    } else {
                        "fail"
                    },
                    "[AVSYNC-STATS]"
                );
                *s = AudioStats::default();
            }
        }
    });

    let q_produce = queue.clone();
    let stats_produce = stats.clone();
    tokio::spawn(async move {
        let mut decoder: Option<OpusDecoder> = None;
        loop {
            let frame = {
                let mut guard = rx.lock().await;
                guard.recv().await
            };
            let Some(frame) = frame else {
                break;
            };

            // Audio follows video: if audio timestamp is ahead, delay audio playback.
            let mut skew_after_align_ms: Option<f64> = None;
            if frame.capture_unix_us > 0 {
                let vts = latest_video_tx_us.load(Ordering::Relaxed);
                if vts > 0 {
                    let raw_skew_ms = (frame.capture_unix_us as i64 - vts as i64) as f64 / 1000.0;
                    if avsync_enable && raw_skew_ms > (avsync_target_ms + avsync_deadzone_ms) {
                        let delay_ms = (raw_skew_ms - avsync_target_ms - avsync_deadzone_ms)
                            .min(avsync_max_delay_ms);
                        if delay_ms > 0.0 {
                            tokio::time::sleep(std::time::Duration::from_secs_f64(
                                delay_ms / 1000.0,
                            ))
                            .await;
                        }
                        skew_after_align_ms = Some(raw_skew_ms - delay_ms);
                    } else {
                        skew_after_align_ms = Some(raw_skew_ms);
                    }
                }
            }

            if frame.codec != 1 {
                continue;
            }
            if frame.channels != 2 {
                continue;
            }
            if decoder.is_none() {
                let sr = SampleRate::try_from(frame.sample_rate as i32).ok();
                if let Some(sr) = sr {
                    decoder = OpusDecoder::new(sr, Channels::Stereo).ok();
                }
            }
            let Some(dec) = decoder.as_mut() else {
                continue;
            };
            let mut out =
                vec![0_f32; (frame.sample_rate as usize * frame.channels as usize).max(9600)];
            let pkt = match Packet::try_from(frame.payload.as_slice()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let sig = match MutSignals::try_from(out.as_mut_slice()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let decoded_samples_per_ch = match dec.decode_float(Some(pkt), sig, false) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let decoded_len = decoded_samples_per_ch * frame.channels as usize;
            out.truncate(decoded_len);
            if let Some(rec) = &recorder {
                rec.record_audio_f32(&out);
            }

            if let Ok(mut q) = q_produce.lock() {
                let max_samples =
                    (frame.sample_rate as usize * frame.channels as usize / 2).max(9600);
                let room_needed = out.len();
                if q.len().saturating_add(room_needed) > max_samples {
                    let drop_count = q
                        .len()
                        .saturating_add(room_needed)
                        .saturating_sub(max_samples);
                    for _ in 0..drop_count {
                        let _ = q.pop_front();
                    }
                    if let Ok(mut s) = stats_produce.lock() {
                        s.dropped = s.dropped.saturating_add(1);
                    }
                }
                q.extend(out);
            }

            let latency_ms = if frame.capture_unix_us > 0 {
                (unix_time_us().saturating_sub(frame.capture_unix_us) as f64) / 1000.0
            } else {
                0.0
            };
            if let Ok(mut s) = stats_produce.lock() {
                s.frames = s.frames.saturating_add(1);
                s.latency_ms_sum += latency_ms;
                if latency_ms > s.latency_ms_max {
                    s.latency_ms_max = latency_ms;
                }
                if let Some(skew_ms) = skew_after_align_ms {
                    if s.av_skew_signed_ms.len() >= 4096 {
                        let _ = s.av_skew_signed_ms.pop_front();
                    }
                    if s.av_skew_abs_ms.len() >= 4096 {
                        let _ = s.av_skew_abs_ms.pop_front();
                    }
                    s.av_skew_signed_ms.push_back(skew_ms);
                    s.av_skew_abs_ms.push_back(skew_ms.abs());
                }
            }
        }
    });

    std::mem::forget(stream);
    Ok(())
}
