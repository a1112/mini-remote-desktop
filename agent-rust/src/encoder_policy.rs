use agent_rust::CaptureConfig;
use std::process::Command;

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

pub fn choose_encoder_backend(cfg: &CaptureConfig) -> (VideoEncoderBackend, Vec<String>) {
    let mut logs = Vec::new();
    let requested = cfg.encoder.to_ascii_lowercase();
    let caps = detect_gpu_caps(&mut logs);

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

    if !cfg.allow_encoder_fallback {
        order.truncate(1);
    }

    for enc in order {
        if encoder_available(enc, &caps) {
            logs.push(format!("video encoder selected: {}", enc.as_str()));
            return (enc, logs);
        }
        logs.push(format!("video encoder {} unavailable", enc.as_str()));
    }

    logs.push("all requested encoders unavailable, fallback to openh264".to_string());
    (VideoEncoderBackend::OpenH264, logs)
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
