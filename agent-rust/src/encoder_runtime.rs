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
        transport_hint: String,
        ffmpeg_bin: String,
        ffmpeg_cfg: agent_rust::CaptureConfig,
        applied_bitrate_kbps: u32,
        pipe: Option<FfmpegPipeEncoder>,
        wh: Option<(u32, u32)>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveVideoCodec {
    H264,
    Hevc,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveTransportHint {
    WebRtc,
    QuicLike,
    Unknown,
}

impl EffectiveTransportHint {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "webrtc" => EffectiveTransportHint::WebRtc,
            "quic" | "webtransport" => EffectiveTransportHint::QuicLike,
            _ => EffectiveTransportHint::Unknown,
        }
    }
}

impl EffectiveVideoCodec {
    fn from_env() -> Self {
        match std::env::var("AGENT_VIDEO_CODEC_EFFECTIVE")
            .ok()
            .unwrap_or_else(|| "h264".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "hevc" | "h265" => EffectiveVideoCodec::Hevc,
            "av1" => EffectiveVideoCodec::Av1,
            _ => EffectiveVideoCodec::H264,
        }
    }
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
    transport_hint: &str,
) -> Result<RuntimeVideoEncoder> {
    let selected_codec = EffectiveVideoCodec::from_env();
    if let Some(codec) = ffmpeg_codec_name(backend, selected_codec) {
        let ffmpeg_bin = resolve_ffmpeg_bin();
        match probe_ffmpeg_encoder(&ffmpeg_bin, codec) {
            Ok(()) => {
                return Ok(RuntimeVideoEncoder::HwFfmpeg {
                    backend,
                    fps,
                    transport_hint: transport_hint.to_string(),
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
            transport_hint,
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
                    *backend,
                    *fps,
                    transport_hint,
                    ffmpeg_bin,
                    ffmpeg_cfg,
                    width,
                    height,
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
                    tracing::warn!(
                        backend = backend.as_str(),
                        transport = transport_hint,
                        error = %e,
                        "ffmpeg_pipe_restart reason=encode_error"
                    );
                    *pipe = Some(start_ffmpeg_pipe(
                        *backend,
                        *fps,
                        transport_hint,
                        ffmpeg_bin,
                        ffmpeg_cfg,
                        width,
                        height,
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
    transport_hint_raw: &str,
    ffmpeg_bin: &str,
    cfg: &agent_rust::CaptureConfig,
    width: u32,
    height: u32,
) -> Result<FfmpegPipeEncoder> {
    let selected_codec = EffectiveVideoCodec::from_env();
    let transport_hint = EffectiveTransportHint::parse(transport_hint_raw);
    let codec =
        ffmpeg_codec_name(backend, selected_codec).ok_or_else(|| anyhow!("not a ffmpeg hw backend"))?;
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
    ];
    if backend == VideoEncoderBackend::Nvenc {
        apply_nvenc_transport_template(&mut args, cfg, selected_codec, transport_hint);
    }
    let roi_requested = std::env::var("AGENT_ROI_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let roi_filter = ffmpeg_roi_filter_expr(width, height);
    if let Some(vf) = roi_filter.as_ref() {
        args.push("-vf".to_string());
        args.push(vf.clone());
    }
    match selected_codec {
        EffectiveVideoCodec::H264 => {
            args.push("-bsf:v".to_string());
            args.push("h264_metadata=aud=insert".to_string());
            args.push("-f".to_string());
            args.push("h264".to_string());
        }
        EffectiveVideoCodec::Hevc => {
            args.push("-f".to_string());
            args.push("hevc".to_string());
        }
        EffectiveVideoCodec::Av1 => {
            args.push("-f".to_string());
            args.push("obu".to_string());
        }
    }
    args.push("-".to_string());
    tracing::info!(
        backend = backend.as_str(),
        codec = codec,
        transport = transport_hint_raw,
        roi_requested,
        roi_applied = roi_filter.is_some(),
        "ffmpeg_pipe_start"
    );
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

fn apply_nvenc_transport_template(
    args: &mut Vec<String>,
    cfg: &agent_rust::CaptureConfig,
    codec: EffectiveVideoCodec,
    transport: EffectiveTransportHint,
) {
    if transport == EffectiveTransportHint::Unknown {
        return;
    }
    let ll_enable = std::env::var("AGENT_NVENC_LL_TEMPLATE_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(true);
    if !ll_enable {
        return;
    }
    let strict_quic_like = matches!(transport, EffectiveTransportHint::QuicLike);
    if strict_quic_like {
        args.push("-preset".to_string());
        args.push("p1".to_string());
        args.push("-tune".to_string());
        args.push("ll".to_string());
        args.push("-rc".to_string());
        args.push("cbr".to_string());
        args.push("-rc-lookahead".to_string());
        args.push("0".to_string());
        args.push("-zerolatency".to_string());
        args.push("1".to_string());
        args.push("-bf".to_string());
        args.push("0".to_string());
    }
    // Keep AV1/HEVC GOP bounded for low-tail latency transports.
    if strict_quic_like && (codec == EffectiveVideoCodec::Hevc || codec == EffectiveVideoCodec::Av1)
    {
        let max_gop = cfg.fps.max(1) * 2;
        args.push("-g".to_string());
        args.push(cfg.gop.max(1).min(max_gop).to_string());
    }
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
            cwd.join("tools")
                .join("ffmpeg_full_build")
                .join("bin")
                .join("ffmpeg.exe"),
            cwd.join("..")
                .join("tools")
                .join("ffmpeg_full_build")
                .join("bin")
                .join("ffmpeg.exe"),
            cwd.join("..")
                .join("..")
                .join("tools")
                .join("ffmpeg_full_build")
                .join("bin")
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

fn ffmpeg_codec_name(
    backend: VideoEncoderBackend,
    codec: EffectiveVideoCodec,
) -> Option<&'static str> {
    match backend {
        VideoEncoderBackend::Nvenc => match codec {
            EffectiveVideoCodec::H264 => Some("h264_nvenc"),
            EffectiveVideoCodec::Hevc => Some("hevc_nvenc"),
            EffectiveVideoCodec::Av1 => Some("av1_nvenc"),
        },
        VideoEncoderBackend::Qsv => match codec {
            EffectiveVideoCodec::H264 => Some("h264_qsv"),
            EffectiveVideoCodec::Hevc => Some("hevc_qsv"),
            EffectiveVideoCodec::Av1 => Some("av1_qsv"),
        },
        VideoEncoderBackend::Amf => match codec {
            EffectiveVideoCodec::H264 => Some("h264_amf"),
            EffectiveVideoCodec::Hevc => Some("hevc_amf"),
            EffectiveVideoCodec::Av1 => Some("av1_amf"),
        },
        VideoEncoderBackend::OpenH264 => None,
    }
}

fn ffmpeg_roi_filter_expr(width: u32, height: u32) -> Option<String> {
    let enabled = std::env::var("AGENT_ROI_ENABLE")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let rect = std::env::var("AGENT_ROI_RECT").ok()?;
    let parts: Vec<&str> = rect.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    if width == 0 || height == 0 {
        return None;
    }
    let x_raw = parts.first()?.trim().parse::<f64>().ok()?;
    let y_raw = parts.get(1)?.trim().parse::<f64>().ok()?;
    let w_raw = parts.get(2)?.trim().parse::<f64>().ok()?;
    let h_raw = parts.get(3)?.trim().parse::<f64>().ok()?;
    if !x_raw.is_finite() || !y_raw.is_finite() || !w_raw.is_finite() || !h_raw.is_finite() {
        return None;
    }
    // Accept both normalized rect (0..1) and absolute pixel rect.
    let normalized = x_raw <= 1.0 && y_raw <= 1.0 && w_raw <= 1.0 && h_raw <= 1.0;
    let (x, y, w, h) = if normalized {
        (
            (x_raw.max(0.0) * width as f64).round() as u32,
            (y_raw.max(0.0) * height as f64).round() as u32,
            (w_raw.max(0.0) * width as f64).round() as u32,
            (h_raw.max(0.0) * height as f64).round() as u32,
        )
    } else {
        (
            x_raw.max(0.0).round() as u32,
            y_raw.max(0.0).round() as u32,
            w_raw.max(0.0).round() as u32,
            h_raw.max(0.0).round() as u32,
        )
    };
    if w == 0 || h == 0 {
        return None;
    }
    let nx = (x.min(width.saturating_sub(1)) as f64) / (width as f64);
    let ny = (y.min(height.saturating_sub(1)) as f64) / (height as f64);
    let nw = (w.min(width.saturating_sub(x).max(1)) as f64) / (width as f64);
    let nh = (h.min(height.saturating_sub(y).max(1)) as f64) / (height as f64);
    let area = nw * nh;
    let min_area = std::env::var("AGENT_ROI_MIN_AREA_PCT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    if area <= min_area {
        return None;
    }
    // Full-frame ROI is effectively no-op and adds filter overhead.
    if nw >= 0.995 && nh >= 0.995 {
        return None;
    }
    let qoffset = std::env::var("AGENT_ROI_QOFFSET")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or_else(|| {
            let boost = std::env::var("AGENT_ROI_BOOST_PCT")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(15.0)
                .clamp(0.0, 200.0);
            -(boost / 250.0).clamp(0.0, 0.8)
        })
        .clamp(-1.0, 1.0);
    // NOTE: some ffmpeg builds reject addroi with an `enable=` clause, which can
    // cause encoder subprocess crash loops. Keep ROI expression conservative here.
    Some(format!(
        "addroi=x={nx:.6}:y={ny:.6}:w={nw:.6}:h={nh:.6}:qoffset={qoffset:.3}"
    ))
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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn ffmpeg_codec_mapping_is_correct() {
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Nvenc, EffectiveVideoCodec::H264),
            Some("h264_nvenc")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Nvenc, EffectiveVideoCodec::Hevc),
            Some("hevc_nvenc")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Nvenc, EffectiveVideoCodec::Av1),
            Some("av1_nvenc")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Qsv, EffectiveVideoCodec::H264),
            Some("h264_qsv")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::Amf, EffectiveVideoCodec::H264),
            Some("h264_amf")
        );
        assert_eq!(
            ffmpeg_codec_name(VideoEncoderBackend::OpenH264, EffectiveVideoCodec::H264),
            None
        );
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
            .join("ffmpeg_full_build")
            .join("bin")
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

    #[test]
    fn roi_expr_accepts_normalized_rect() {
        let _guard = env_lock().lock().expect("env lock");
        unsafe {
            std::env::set_var("AGENT_ROI_ENABLE", "1");
            std::env::set_var("AGENT_ROI_RECT", "0.25,0.25,0.5,0.5");
            std::env::set_var("AGENT_ROI_QOFFSET", "-0.2");
        }
        let vf = ffmpeg_roi_filter_expr(1920, 1080);
        assert!(vf.is_some());
        unsafe {
            std::env::remove_var("AGENT_ROI_ENABLE");
            std::env::remove_var("AGENT_ROI_RECT");
            std::env::remove_var("AGENT_ROI_QOFFSET");
        }
    }

    #[test]
    fn roi_expr_skips_full_frame_rect() {
        let _guard = env_lock().lock().expect("env lock");
        unsafe {
            std::env::set_var("AGENT_ROI_ENABLE", "1");
            std::env::set_var("AGENT_ROI_RECT", "0,0,1,1");
        }
        let vf = ffmpeg_roi_filter_expr(1280, 720);
        assert!(vf.is_none());
        unsafe {
            std::env::remove_var("AGENT_ROI_ENABLE");
            std::env::remove_var("AGENT_ROI_RECT");
        }
    }

    #[test]
    fn roi_expr_does_not_append_enable_clause_for_frame_interval() {
        let _guard = env_lock().lock().expect("env lock");
        unsafe {
            std::env::set_var("AGENT_ROI_ENABLE", "1");
            std::env::set_var("AGENT_ROI_RECT", "0.25,0.25,0.5,0.5");
            std::env::set_var("AGENT_ROI_FRAME_INTERVAL", "4");
        }
        let vf = ffmpeg_roi_filter_expr(1920, 1080).expect("roi expr");
        assert!(!vf.contains(":enable="), "unexpected enable clause: {vf}");
        unsafe {
            std::env::remove_var("AGENT_ROI_ENABLE");
            std::env::remove_var("AGENT_ROI_RECT");
            std::env::remove_var("AGENT_ROI_FRAME_INTERVAL");
        }
    }
}
