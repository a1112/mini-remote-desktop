use crate::encoder_policy::VideoEncoderBackend;
use anyhow::{Context, Result, anyhow};
use openh264::OpenH264API;
use openh264::encoder::{Encoder, EncoderConfig, FrameRate, UsageType};
use openh264::formats::{RgbaSliceU8, YUVBuffer};
use std::io::Read;
use std::io::Write;
use std::process::{Child, ChildStdin, Command};
use std::time::{Duration, Instant};

pub enum RuntimeVideoEncoder {
    OpenH264(Encoder),
    HwFfmpeg {
        backend: VideoEncoderBackend,
        fps: u32,
        ffmpeg_bin: String,
        ffmpeg_cfg: agent_rust::CaptureConfig,
        applied_bitrate_kbps: u32,
        pipe: Option<FfmpegPipeEncoder>,
        wh: Option<(u32, u32)>,
    },
}

pub struct FfmpegPipeEncoder {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: std::sync::mpsc::Receiver<Vec<u8>>,
    stream_buf: Vec<u8>,
    poll_wait_ms: u64,
}

impl Drop for FfmpegPipeEncoder {
    fn drop(&mut self) {
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn request_keyframe(encoder: &mut RuntimeVideoEncoder) {
    if let RuntimeVideoEncoder::HwFfmpeg { pipe, .. } = encoder {
        *pipe = None;
    }
}

pub fn build_video_encoder(
    fps: u32,
    cfg: &agent_rust::CaptureConfig,
    backend: VideoEncoderBackend,
    allow_fallback: bool,
) -> Result<RuntimeVideoEncoder> {
    if let Some(codec) = ffmpeg_codec_name(backend) {
        let ffmpeg_bin = resolve_ffmpeg_bin();
        match probe_ffmpeg_encoder(&ffmpeg_bin, codec) {
            Ok(()) => {
                return Ok(RuntimeVideoEncoder::HwFfmpeg {
                    backend,
                    fps,
                    ffmpeg_bin,
                    ffmpeg_cfg: cfg.clone(),
                    applied_bitrate_kbps: cfg.bitrate_kbps.max(100),
                    pipe: None,
                    wh: None,
                });
            }
            Err(e) if allow_fallback => {
                tracing::warn!(
                    backend = backend.as_str(),
                    error = %e,
                    "hardware encoder backend unavailable, fallback to openh264"
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

pub fn encode_rgba_frame(
    encoder: &mut RuntimeVideoEncoder,
    rgba: &[u8],
    width: u32,
    height: u32,
    target_bitrate_kbps: Option<u32>,
    enable_network_adapt: bool,
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
            applied_bitrate_kbps,
            pipe,
            wh,
        } => {
            if enable_network_adapt && let Some(target) = target_bitrate_kbps {
                let target = target.max(100);
                let drift = applied_bitrate_kbps.abs_diff(target);
                if drift >= 800 {
                    ffmpeg_cfg.bitrate_kbps = target;
                    ffmpeg_cfg.max_bitrate_kbps = ffmpeg_cfg.max_bitrate_kbps.max(target);
                    *applied_bitrate_kbps = target;
                    *pipe = None;
                }
            }
            if pipe.is_none() || wh != &Some((width, height)) {
                *pipe = Some(start_ffmpeg_pipe(
                    *backend, *fps, ffmpeg_bin, ffmpeg_cfg, width, height,
                )?);
                *wh = Some((width, height));
            }
            match pipe
                .as_mut()
                .expect("pipe initialized")
                .encode_one_frame(rgba)
            {
                Ok(v) => Ok(v),
                Err(e) => {
                    *pipe = Some(start_ffmpeg_pipe(
                        *backend, *fps, ffmpeg_bin, ffmpeg_cfg, width, height,
                    )?);
                    pipe.as_mut()
                        .expect("pipe initialized after restart")
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
    resolve_ffmpeg_bin_from(
        std::env::var("AGENT_FFMPEG_PATH").ok(),
        std::env::current_dir().ok(),
    )
}

fn resolve_ffmpeg_bin_from(env_ffmpeg: Option<String>, cwd: Option<std::path::PathBuf>) -> String {
    if let Some(v) = env_ffmpeg {
        let v = v.trim();
        if !v.is_empty() {
            return v.to_string();
        }
    }

    if let Some(cwd) = cwd {
        let candidates = [
            cwd.join("tools").join("ffmpeg-min").join("ffmpeg.exe"),
            cwd.join("..")
                .join("tools")
                .join("ffmpeg-min")
                .join("ffmpeg.exe"),
            cwd.join("..")
                .join("..")
                .join("tools")
                .join("ffmpeg-min")
                .join("ffmpeg.exe"),
        ];
        for p in candidates {
            if p.is_file() {
                return p.to_string_lossy().to_string();
            }
        }
    }

    "ffmpeg".to_string()
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

pub fn ffmpeg_codec_name(backend: VideoEncoderBackend) -> Option<&'static str> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    #[test]
    fn resolve_ffmpeg_prefers_env_path() {
        let got = resolve_ffmpeg_bin_from(
            Some("C:/custom/ffmpeg.exe".to_string()),
            Some(PathBuf::from("J:/tmp/agent-rust")),
        );
        assert_eq!(got, "C:/custom/ffmpeg.exe");
    }

    #[test]
    fn resolve_ffmpeg_uses_repo_tools_when_present() {
        let base = std::env::temp_dir().join(format!(
            "mini-rd-ffmpeg-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let agent_dir = base.join("mini-remote-desktop").join("agent-rust");
        let ffmpeg_exe = base
            .join("mini-remote-desktop")
            .join("tools")
            .join("ffmpeg-min")
            .join("ffmpeg.exe");
        std::fs::create_dir_all(ffmpeg_exe.parent().expect("ffmpeg parent")).expect("mkdir");
        std::fs::create_dir_all(&agent_dir).expect("agent dir");
        std::fs::write(&ffmpeg_exe, b"fake").expect("touch ffmpeg");

        let got = resolve_ffmpeg_bin_from(None, Some(agent_dir));
        let got_canon = PathBuf::from(got).canonicalize().expect("canonical got");
        let expect_canon = ffmpeg_exe.canonicalize().expect("canonical expect");
        assert_eq!(got_canon, expect_canon);

        let _ = std::fs::remove_dir_all(base);
    }
}
