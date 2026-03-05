use crate::audio_control::AudioControlManager;
use crate::audio_quic_tx::AudioQuicPacket;
use anyhow::{Context, Result, anyhow};
use audiopus::coder::Encoder as OpusEncoder;
use audiopus::{Application, Channels, SampleRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::convert::TryFrom;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Clone)]
struct CaptureState {
    tx: mpsc::Sender<AudioQuicPacket>,
    seq: Arc<AtomicU64>,
    frame_samples: usize,
    sample_rate: u32,
    channels: u16,
    pending: Arc<Mutex<Vec<f32>>>,
    encoder: Arc<Mutex<OpusEncoder>>,
}

fn unix_time_us() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_micros().min(u64::MAX as u128) as u64,
        Err(_) => 0,
    }
}

fn normalize_frame_ms(raw: u32) -> u32 {
    match raw {
        2 | 3 => 2,
        5 => 5,
        10 => 10,
        20 => 20,
        40 => 40,
        60 => 60,
        _ => 10,
    }
}

fn nearest_sample_rate(min: u32, max: u32, target: u32) -> u32 {
    if target < min {
        return min;
    }
    if target > max {
        return max;
    }
    target
}

fn score_config(channels: u16, sample_rate: u32) -> i64 {
    let ch_penalty = if channels == 2 { 0 } else { 1_000 };
    let sr_penalty = (sample_rate as i64 - 48_000_i64).abs();
    ch_penalty + sr_penalty
}

fn pick_input_config(
    device: &cpal::Device,
) -> Option<(cpal::StreamConfig, cpal::SampleFormat, &'static str)> {
    let mut best: Option<(cpal::StreamConfig, cpal::SampleFormat, &'static str, i64)> = None;

    if let Ok(configs) = device.supported_input_configs() {
        for c in configs {
            let min = c.min_sample_rate();
            let max = c.max_sample_rate();
            let picked = nearest_sample_rate(min, max, 48_000);
            let cfg = c.with_sample_rate(picked).config();
            let score = score_config(cfg.channels, cfg.sample_rate);
            match &best {
                Some((_, _, _, cur_score)) if score >= *cur_score => {}
                _ => {
                    best = Some((cfg, c.sample_format(), "supported_input_configs", score));
                }
            }
        }
    }

    if let Some((cfg, fmt, src, _)) = best {
        return Some((cfg, fmt, src));
    }

    if let Ok(v) = device.default_input_config() {
        return Some((v.config(), v.sample_format(), "default_input_config"));
    }
    None
}

fn upmix_to_stereo(input: &[f32], input_channels: usize) -> Vec<f32> {
    if input_channels == 0 {
        return Vec::new();
    }
    if input_channels == 2 {
        return input.to_vec();
    }
    if input_channels == 1 {
        let mut out = Vec::with_capacity(input.len() * 2);
        for s in input {
            out.push(*s);
            out.push(*s);
        }
        return out;
    }
    let frames = input.len() / input_channels;
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let base = i * input_channels;
        out.push(input[base]);
        out.push(input[base + 1]);
    }
    out
}

fn send_frames(state: &CaptureState, input: &[f32]) {
    let mut pending = match state.pending.lock() {
        Ok(v) => v,
        Err(_) => return,
    };
    pending.extend_from_slice(input);

    let channels = state.channels as usize;
    let samples_per_packet = state.frame_samples.saturating_mul(channels);
    while pending.len() >= samples_per_packet && samples_per_packet > 0 {
        let chunk: Vec<f32> = pending.drain(..samples_per_packet).collect();
        let encoded = {
            let mut enc = match state.encoder.lock() {
                Ok(v) => v,
                Err(_) => return,
            };
            let mut out = vec![0_u8; 4000];
            match enc.encode_float(chunk.as_slice(), out.as_mut_slice()) {
                Ok(n) => out[..n].to_vec(),
                Err(_) => continue,
            }
        };
        let pkt = AudioQuicPacket {
            sequence: state.seq.fetch_add(1, Ordering::Relaxed).saturating_add(1),
            capture_unix_us: unix_time_us(),
            codec: 1, // 1=opus
            sample_rate: state.sample_rate,
            channels: state.channels,
            frame_samples: state.frame_samples as u16,
            payload: encoded,
        };
        if state.tx.try_send(pkt).is_err() {
            // Drop stale audio when the queue is full to keep low latency.
        }
    }
}

pub fn spawn_loopback_capture(
    tx: mpsc::Sender<AudioQuicPacket>,
    audio_ctrl: Arc<std::sync::Mutex<AudioControlManager>>,
) -> Result<cpal::Stream> {
    let host = cpal::default_host();

    let route = audio_ctrl
        .lock()
        .ok()
        .and_then(|m| m.latest())
        .unwrap_or_default();
    let route_mode = route.route_mode;
    let route_scope = route.route_scope;
    let target_pid = route.target_pid;
    let follow_children = route.follow_children;

    let output_device = host.default_output_device();
    let input_device = host.default_input_device();

    let mut selected: Option<(
        cpal::Device,
        cpal::StreamConfig,
        cpal::SampleFormat,
        &'static str,
    )> = None;

    if let Some(dev) = output_device {
        if let Some((cfg, fmt, src)) = pick_input_config(&dev) {
            selected = Some((dev, cfg, fmt, src));
        } else {
            warn!(
                "default output device does not expose usable input config; fallback to input device"
            );
        }
    }
    if selected.is_none() {
        if let Some(dev) = input_device {
            if let Some((cfg, fmt, src)) = pick_input_config(&dev) {
                selected = Some((dev, cfg, fmt, src));
            }
        }
    }
    let (device, cfg, sample_format, config_source) = selected
        .context("audio capture config unavailable: no usable input stream config found")?;

    let input_channels = cfg.channels;
    if input_channels != 2 {
        warn!(
            input_channels,
            "audio capture input is not stereo; converting to stereo for transport"
        );
    }

    let sample_rate = cfg.sample_rate;
    let channels = 2_u16;
    let frame_ms = normalize_frame_ms(
        std::env::var("MRD_AUDIO_FRAME_MS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .or_else(|| {
                audio_ctrl
                    .lock()
                    .ok()
                    .and_then(|m| m.latest())
                    .map(|s| s.frame_ms as u32)
            })
            .unwrap_or(10),
    );
    let frame_samples = ((sample_rate as u64) * (frame_ms as u64) / 1000) as usize;

    let opus_rate = SampleRate::try_from(sample_rate as i32)
        .map_err(|_| anyhow!("unsupported opus sample rate: {sample_rate}"))?;
    let encoder = OpusEncoder::new(opus_rate, Channels::Stereo, Application::LowDelay)
        .map_err(|e| anyhow!("create opus encoder failed: {e}"))?;

    info!(
        sample_rate,
        input_channels,
        channels,
        frame_ms,
        config_source,
        route_mode,
        route_scope,
        target_pid,
        follow_children,
        "audio capture configured (opus)"
    );

    let state = CaptureState {
        tx,
        seq: Arc::new(AtomicU64::new(0)),
        frame_samples,
        sample_rate,
        channels,
        pending: Arc::new(Mutex::new(Vec::with_capacity(
            frame_samples * channels as usize * 4,
        ))),
        encoder: Arc::new(Mutex::new(encoder)),
    };

    let err_fn = |err| {
        warn!(error = %err, "audio input stream error");
    };

    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            let st = state.clone();
            let in_ch = input_channels as usize;
            device.build_input_stream(
                &cfg,
                move |data: &[f32], _| {
                    if in_ch == 2 {
                        send_frames(&st, data);
                    } else {
                        let buf = upmix_to_stereo(data, in_ch);
                        send_frames(&st, &buf);
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::I16 => {
            let st = state.clone();
            let in_ch = input_channels as usize;
            device.build_input_stream(
                &cfg,
                move |data: &[i16], _| {
                    let mut buf = Vec::with_capacity(data.len());
                    for x in data {
                        buf.push(*x as f32 / i16::MAX as f32);
                    }
                    if in_ch == 2 {
                        send_frames(&st, &buf);
                    } else {
                        let stereo = upmix_to_stereo(&buf, in_ch);
                        send_frames(&st, &stereo);
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let st = state.clone();
            let in_ch = input_channels as usize;
            device.build_input_stream(
                &cfg,
                move |data: &[u16], _| {
                    let mut buf = Vec::with_capacity(data.len());
                    for x in data {
                        buf.push((*x as f32 / u16::MAX as f32) * 2.0 - 1.0);
                    }
                    if in_ch == 2 {
                        send_frames(&st, &buf);
                    } else {
                        let stereo = upmix_to_stereo(&buf, in_ch);
                        send_frames(&st, &stereo);
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::U8 => {
            let st = state.clone();
            let in_ch = input_channels as usize;
            device.build_input_stream(
                &cfg,
                move |data: &[u8], _| {
                    let mut buf = Vec::with_capacity(data.len());
                    for x in data {
                        buf.push((*x as f32 / u8::MAX as f32) * 2.0 - 1.0);
                    }
                    if in_ch == 2 {
                        send_frames(&st, &buf);
                    } else {
                        let stereo = upmix_to_stereo(&buf, in_ch);
                        send_frames(&st, &stereo);
                    }
                },
                err_fn,
                None,
            )?
        }
        cpal::SampleFormat::I8 => {
            let st = state.clone();
            let in_ch = input_channels as usize;
            device.build_input_stream(
                &cfg,
                move |data: &[i8], _| {
                    let mut buf = Vec::with_capacity(data.len());
                    for x in data {
                        buf.push(*x as f32 / i8::MAX as f32);
                    }
                    if in_ch == 2 {
                        send_frames(&st, &buf);
                    } else {
                        let stereo = upmix_to_stereo(&buf, in_ch);
                        send_frames(&st, &stereo);
                    }
                },
                err_fn,
                None,
            )?
        }
        other => {
            return Err(anyhow!("unsupported audio sample format: {other:?}"));
        }
    };

    stream.play().context("start audio capture stream failed")?;
    Ok(stream)
}
