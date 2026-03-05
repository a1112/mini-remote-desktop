use crate::webrtc::peer::VideoFrame;
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub enabled: bool,
    pub output_dir: String,
    pub ffmpeg_path: String,
    pub segment_seconds: u32,
    pub input_fps: u32,
    pub container: String,
    pub video_codec: String,
    pub video_preset: String,
    pub video_crf: i32,
    pub video_bitrate_kbps: u32,
    pub audio_enabled: bool,
    pub audio_codec: String,
    pub audio_bitrate_kbps: u32,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    pub queue_depth: usize,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output_dir: "recordings".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
            segment_seconds: 60,
            input_fps: 60,
            container: "mp4".to_string(),
            video_codec: "copy".to_string(),
            video_preset: "p4".to_string(),
            video_crf: 23,
            video_bitrate_kbps: 0,
            audio_enabled: true,
            audio_codec: "aac".to_string(),
            audio_bitrate_kbps: 160,
            audio_sample_rate: 48_000,
            audio_channels: 2,
            queue_depth: 512,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecordingStats {
    pub in_frames: u64,
    pub written_frames: u64,
    pub dropped_frames: u64,
    pub write_failures: u64,
    pub bytes_written: u64,
    pub avg_write_us: f64,
}

enum RecorderMsg {
    Frame(bytes::Bytes),
    AudioPcm(bytes::Bytes),
    Stop,
}

#[derive(Default)]
struct RecordingStatsAtomic {
    in_frames: AtomicU64,
    written_frames: AtomicU64,
    dropped_frames: AtomicU64,
    write_failures: AtomicU64,
    bytes_written: AtomicU64,
    write_total_us: AtomicU64,
    audio_packets: AtomicU64,
    audio_bytes_written: AtomicU64,
    audio_write_failures: AtomicU64,
}

pub struct Recorder {
    enabled: bool,
    tx: Option<SyncSender<RecorderMsg>>,
    worker: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    stats: Arc<RecordingStatsAtomic>,
}

impl Recorder {
    pub fn new(config: RecordingConfig) -> Result<Self> {
        if !config.enabled {
            return Ok(Self {
                enabled: false,
                tx: None,
                worker: None,
                stop_flag: Arc::new(AtomicBool::new(false)),
                stats: Arc::new(RecordingStatsAtomic::default()),
            });
        }

        let output_dir = PathBuf::from(&config.output_dir);
        fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "failed to create record output dir: {}",
                output_dir.display()
            )
        })?;

        let mut container = config.container.to_ascii_lowercase();
        if container != "mp4" && container != "mkv" {
            container = "mp4".to_string();
        }
        let output_pattern = output_dir.join(format!("record_%Y%m%d_%H%M%S.{}", container));
        let segment_seconds = config.segment_seconds.clamp(5, 24 * 3600);
        let input_fps = config.input_fps.clamp(1, 240);
        let queue_depth = config.queue_depth.clamp(16, 4096);
        let video_codec = normalize_codec(&config.video_codec);
        let video_preset = resolve_preset(&video_codec, &config.video_preset);
        let use_copy = video_codec == "copy";
        let audio_enabled = config.audio_enabled;
        let audio_sample_rate = config.audio_sample_rate.clamp(8_000, 192_000);
        let audio_channels = config.audio_channels.clamp(1, 8);
        let audio_codec = resolve_audio_codec(&config.audio_codec);
        let audio_bitrate_kbps = config.audio_bitrate_kbps.clamp(16, 1536);
        let audio_udp_port = if audio_enabled {
            Some(pick_local_udp_port().context("pick local udp port for recorder audio failed")?)
        } else {
            None
        };

        let mut cmd = Command::new(&config.ffmpeg_path);
        cmd.arg("-hide_banner")
            .arg("-loglevel")
            .arg("warning")
            .arg("-y")
            .arg("-fflags")
            .arg("+genpts")
            .arg("-f")
            .arg("h264")
            .arg("-r")
            .arg(input_fps.to_string())
            .arg("-i")
            .arg("pipe:0");
        if let Some(port) = audio_udp_port {
            let audio_input =
                format!("udp://127.0.0.1:{port}?fifo_size=1048576&overrun_nonfatal=1");
            cmd.arg("-thread_queue_size")
                .arg("512")
                .arg("-f")
                .arg("f32le")
                .arg("-ar")
                .arg(audio_sample_rate.to_string())
                .arg("-ac")
                .arg(audio_channels.to_string())
                .arg("-i")
                .arg(audio_input);
        } else {
            cmd.arg("-an");
        }

        if use_copy {
            cmd.arg("-c:v").arg("copy");
        } else {
            cmd.arg("-c:v").arg(&video_codec);
            if let Some(preset) = video_preset.as_deref() {
                cmd.arg("-preset").arg(preset);
            }
            if config.video_bitrate_kbps > 0 {
                cmd.arg("-b:v")
                    .arg(format!("{}k", config.video_bitrate_kbps));
            }
            if supports_crf(&video_codec) && config.video_crf >= 0 {
                cmd.arg("-crf").arg(config.video_crf.to_string());
            }
            cmd.arg("-g").arg((segment_seconds * input_fps).to_string());
            cmd.arg("-force_key_frames")
                .arg(format!("expr:gte(t,n_forced*{})", segment_seconds));
        }

        if audio_udp_port.is_some() {
            cmd.arg("-map").arg("0:v:0").arg("-map").arg("1:a:0");
            cmd.arg("-c:a").arg(&audio_codec);
            if audio_codec != "copy" {
                cmd.arg("-b:a").arg(format!("{audio_bitrate_kbps}k"));
            }
        }

        cmd.arg("-f")
            .arg("segment")
            .arg("-segment_time")
            .arg(segment_seconds.to_string());
        if use_copy {
            cmd.arg("-break_non_keyframes").arg("1");
        }
        cmd.arg("-reset_timestamps")
            .arg("1")
            .arg("-strftime")
            .arg("1")
            .arg(output_pattern.to_string_lossy().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "failed to spawn ffmpeg recorder: path={}",
                config.ffmpeg_path
            )
        })?;
        let Some(mut stdin) = child.stdin.take() else {
            return Err(anyhow::anyhow!("ffmpeg recorder stdin not available"));
        };
        let audio_socket = if let Some(port) = audio_udp_port {
            let sock = std::net::UdpSocket::bind("127.0.0.1:0")
                .context("bind recorder audio udp sender failed")?;
            sock.connect(("127.0.0.1", port))
                .context("connect recorder audio udp sender failed")?;
            Some(sock)
        } else {
            None
        };

        let (tx, rx): (SyncSender<RecorderMsg>, Receiver<RecorderMsg>) =
            mpsc::sync_channel(queue_depth);
        let stats = Arc::new(RecordingStatsAtomic::default());
        let stats_worker = stats.clone();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let stop_flag_worker = stop_flag.clone();

        let worker = thread::Builder::new()
            .name("mrd-recorder".to_string())
            .spawn(move || {
                while !stop_flag_worker.load(Ordering::Relaxed) {
                    match rx.recv() {
                        Ok(RecorderMsg::Frame(data)) => {
                            let t0 = Instant::now();
                            match stdin.write_all(data.as_ref()) {
                                Ok(_) => {
                                    stats_worker.written_frames.fetch_add(1, Ordering::Relaxed);
                                    stats_worker
                                        .bytes_written
                                        .fetch_add(data.len() as u64, Ordering::Relaxed);
                                    let elapsed =
                                        t0.elapsed().as_micros().min(u64::MAX as u128) as u64;
                                    stats_worker
                                        .write_total_us
                                        .fetch_add(elapsed, Ordering::Relaxed);
                                }
                                Err(e) => {
                                    stats_worker.write_failures.fetch_add(1, Ordering::Relaxed);
                                    error!(error = %e, "recorder write to ffmpeg failed");
                                    break;
                                }
                            }
                        }
                        Ok(RecorderMsg::AudioPcm(data)) => {
                            if let Some(sock) = audio_socket.as_ref() {
                                match sock.send(data.as_ref()) {
                                    Ok(n) => {
                                        stats_worker.audio_packets.fetch_add(1, Ordering::Relaxed);
                                        stats_worker
                                            .audio_bytes_written
                                            .fetch_add(n as u64, Ordering::Relaxed);
                                    }
                                    Err(e) => {
                                        stats_worker
                                            .audio_write_failures
                                            .fetch_add(1, Ordering::Relaxed);
                                        warn!(error = %e, "recorder audio udp send failed");
                                    }
                                }
                            }
                        }
                        Ok(RecorderMsg::Stop) => break,
                        Err(_) => break,
                    }
                }
                let _ = stdin.flush();
                match child.wait() {
                    Ok(status) => {
                        info!(?status, "recorder ffmpeg exited");
                    }
                    Err(e) => {
                        warn!(error = %e, "recorder ffmpeg wait failed");
                    }
                }
            })
            .context("failed to spawn recorder worker thread")?;

        info!(
            output_dir = %output_dir.display(),
            segment_seconds,
            input_fps,
            container,
            video_codec,
            video_preset = ?video_preset,
            video_crf = config.video_crf,
            video_bitrate_kbps = config.video_bitrate_kbps,
            audio_enabled,
            audio_codec,
            audio_bitrate_kbps,
            audio_sample_rate,
            audio_channels,
            queue_depth,
            "receiver recording enabled"
        );
        Ok(Self {
            enabled: true,
            tx: Some(tx),
            worker: Some(worker),
            stop_flag,
            stats,
        })
    }

    pub fn record_frame(&self, frame: &VideoFrame) {
        if !self.enabled {
            return;
        }
        self.stats.in_frames.fetch_add(1, Ordering::Relaxed);
        if let Some(tx) = &self.tx {
            match tx.try_send(RecorderMsg::Frame(frame.data.clone())) {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    self.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.stats.write_failures.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn record_audio_f32(&self, samples: &[f32]) {
        if !self.enabled || samples.is_empty() {
            return;
        }
        let mut buf = Vec::<u8>::with_capacity(samples.len() * std::mem::size_of::<f32>());
        for s in samples {
            buf.extend_from_slice(&s.to_le_bytes());
        }
        if let Some(tx) = &self.tx {
            match tx.try_send(RecorderMsg::AudioPcm(bytes::Bytes::from(buf))) {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    self.stats.dropped_frames.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.stats
                        .audio_write_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn snapshot(&self) -> RecordingStats {
        let in_frames = self.stats.in_frames.load(Ordering::Relaxed);
        let written_frames = self.stats.written_frames.load(Ordering::Relaxed);
        let write_total_us = self.stats.write_total_us.load(Ordering::Relaxed);
        RecordingStats {
            in_frames,
            written_frames,
            dropped_frames: self.stats.dropped_frames.load(Ordering::Relaxed),
            write_failures: self.stats.write_failures.load(Ordering::Relaxed),
            bytes_written: self.stats.bytes_written.load(Ordering::Relaxed),
            avg_write_us: if written_frames > 0 {
                write_total_us as f64 / written_frames as f64
            } else {
                0.0
            },
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.enabled {
            return;
        }
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(RecorderMsg::Stop);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn normalize_codec(codec: &str) -> String {
    let v = codec.trim().to_ascii_lowercase();
    if v.is_empty() {
        "copy".to_string()
    } else {
        v
    }
}

fn supports_crf(codec: &str) -> bool {
    matches!(codec, "libx264" | "libx265" | "libsvtav1")
}

fn resolve_preset(codec: &str, configured: &str) -> Option<String> {
    let normalized = configured.trim().to_ascii_lowercase();
    let preset = if normalized.is_empty() || normalized == "auto" {
        match codec {
            "libx264" => "veryfast".to_string(),
            "libx265" => "medium".to_string(),
            "h264_nvenc" | "hevc_nvenc" => "p4".to_string(),
            _ => return None,
        }
    } else {
        normalized
    };

    if matches!(codec, "libx264" | "libx265")
        && matches!(
            preset.as_str(),
            "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7"
        )
    {
        return Some(if codec == "libx264" {
            "veryfast".to_string()
        } else {
            "medium".to_string()
        });
    }
    if matches!(codec, "h264_nvenc" | "hevc_nvenc")
        && !matches!(
            preset.as_str(),
            "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7"
        )
    {
        return Some("p4".to_string());
    }
    Some(preset)
}

fn resolve_audio_codec(codec: &str) -> String {
    match codec.trim().to_ascii_lowercase().as_str() {
        "aac" | "libopus" | "opus" => codec.trim().to_ascii_lowercase(),
        "copy" => "aac".to_string(),
        _ => "aac".to_string(),
    }
}

fn pick_local_udp_port() -> Result<u16> {
    let sock =
        std::net::UdpSocket::bind("127.0.0.1:0").context("bind temporary udp socket failed")?;
    let port = sock
        .local_addr()
        .context("query local udp addr failed")?
        .port();
    Ok(port)
}
