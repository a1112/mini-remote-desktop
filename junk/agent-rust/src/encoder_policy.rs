use agent_rust::CaptureConfig;
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoderBackend {
    Nvenc,
    Qsv,
    Amf,
    OpenH264,
}

impl VideoEncoderBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            VideoEncoderBackend::Nvenc => "nvenc",
            VideoEncoderBackend::Qsv => "qsv",
            VideoEncoderBackend::Amf => "amf",
            VideoEncoderBackend::OpenH264 => "openh264",
        }
    }
}

#[derive(Default)]
struct GpuCaps {
    has_nvidia: bool,
    has_intel: bool,
    has_amd: bool,
    detect_unknown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveCodec {
    H264,
    Hevc,
    Av1,
}

impl EffectiveCodec {
    fn from_env() -> Self {
        match std::env::var("AGENT_VIDEO_CODEC_EFFECTIVE")
            .ok()
            .unwrap_or_else(|| "h264".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "hevc" | "h265" => EffectiveCodec::Hevc,
            "av1" => EffectiveCodec::Av1,
            _ => EffectiveCodec::H264,
        }
    }
}

pub fn choose_encoder_backend(cfg: &CaptureConfig) -> (VideoEncoderBackend, Vec<String>) {
    let mut logs = Vec::new();
    let requested = cfg.encoder.to_ascii_lowercase();
    let caps = detect_gpu_caps(&mut logs);
    let codec = EffectiveCodec::from_env();
    let fallback_allowed = cfg.allow_encoder_fallback && !cfg.strict_gpu_direct;

    let mut order = match requested.as_str() {
        "nvenc" => vec![VideoEncoderBackend::Nvenc],
        "qsv" => vec![VideoEncoderBackend::Qsv],
        "amf" => vec![VideoEncoderBackend::Amf],
        "openh264" => vec![VideoEncoderBackend::OpenH264],
        _ => vec![
            VideoEncoderBackend::Nvenc,
            VideoEncoderBackend::Qsv,
            VideoEncoderBackend::Amf,
            VideoEncoderBackend::OpenH264,
        ],
    };

    if !fallback_allowed {
        order.truncate(1);
    }
    let forced = order.first().copied().unwrap_or(VideoEncoderBackend::Nvenc);

    for enc in order {
        if encoder_available(enc, &caps) && backend_supports_codec(enc, codec, &mut logs) {
            logs.push(format!("video encoder selected: {}", enc.as_str()));
            return (enc, logs);
        }
        logs.push(format!("video encoder {} unavailable", enc.as_str()));
    }

    if !fallback_allowed {
        logs.push(format!(
            "strict encoder selection active, forcing {}",
            forced.as_str()
        ));
        return (forced, logs);
    }

    logs.push("all requested encoders unavailable, fallback to openh264".to_string());
    (VideoEncoderBackend::OpenH264, logs)
}

fn backend_supports_codec(
    enc: VideoEncoderBackend,
    codec: EffectiveCodec,
    logs: &mut Vec<String>,
) -> bool {
    let Some(name) = ffmpeg_encoder_name(enc, codec) else {
        return true;
    };
    match probe_ffmpeg_encoder(name) {
        Some(true) => true,
        Some(false) => {
            logs.push(format!(
                "ffmpeg encoder {name} not available for codec {:?}",
                codec
            ));
            false
        }
        None => {
            logs.push(format!(
                "ffmpeg probe unavailable, keep optimistic path for {name}"
            ));
            true
        }
    }
}

fn ffmpeg_encoder_name(enc: VideoEncoderBackend, codec: EffectiveCodec) -> Option<&'static str> {
    match (enc, codec) {
        (VideoEncoderBackend::Nvenc, EffectiveCodec::H264) => Some("h264_nvenc"),
        (VideoEncoderBackend::Nvenc, EffectiveCodec::Hevc) => Some("hevc_nvenc"),
        (VideoEncoderBackend::Nvenc, EffectiveCodec::Av1) => Some("av1_nvenc"),
        (VideoEncoderBackend::Qsv, EffectiveCodec::H264) => Some("h264_qsv"),
        (VideoEncoderBackend::Qsv, EffectiveCodec::Hevc) => Some("hevc_qsv"),
        (VideoEncoderBackend::Qsv, EffectiveCodec::Av1) => Some("av1_qsv"),
        (VideoEncoderBackend::Amf, EffectiveCodec::H264) => Some("h264_amf"),
        (VideoEncoderBackend::Amf, EffectiveCodec::Hevc) => Some("hevc_amf"),
        (VideoEncoderBackend::Amf, EffectiveCodec::Av1) => Some("av1_amf"),
        (VideoEncoderBackend::OpenH264, _) => None,
    }
}

fn probe_ffmpeg_encoder(name: &str) -> Option<bool> {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(v) = guard.get(name) {
            return Some(*v);
        }
    }

    let ffmpeg_bin = std::env::var("AGENT_FFMPEG_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".to_string());
    let out = Command::new(ffmpeg_bin)
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let txt = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    let ok = txt.contains(&name.to_ascii_lowercase());
    if let Ok(mut guard) = cache.lock() {
        guard.insert(name.to_string(), ok);
    }
    Some(ok)
}

fn encoder_available(enc: VideoEncoderBackend, caps: &GpuCaps) -> bool {
    if caps.detect_unknown {
        return true;
    }
    match enc {
        VideoEncoderBackend::Nvenc => caps.has_nvidia,
        VideoEncoderBackend::Qsv => caps.has_intel,
        VideoEncoderBackend::Amf => caps.has_amd,
        VideoEncoderBackend::OpenH264 => true,
    }
}

fn detect_gpu_caps(logs: &mut Vec<String>) -> GpuCaps {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ])
        .output();

    let mut caps = GpuCaps::default();
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
            caps.has_nvidia = text.contains("nvidia");
            caps.has_intel = text.contains("intel");
            caps.has_amd = text.contains("amd") || text.contains("radeon");
            logs.push(format!(
                "gpu detect: nvidia={} intel={} amd={}",
                caps.has_nvidia, caps.has_intel, caps.has_amd
            ));
        }
        Ok(out) => {
            logs.push(format!(
                "gpu detect failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            caps.detect_unknown = true;
            logs.push("gpu detect unknown, optimistic encoder probe enabled".to_string());
        }
        Err(e) => {
            logs.push(format!("gpu detect spawn failed: {e}"));
            caps.detect_unknown = true;
            logs.push("gpu detect unknown, optimistic encoder probe enabled".to_string());
        }
    }
    caps
}
