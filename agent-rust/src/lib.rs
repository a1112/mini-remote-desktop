use std::{fs, path::Path};

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub fps: u32,
    pub jpeg_quality: u8,
    pub backend: String,
    pub allow_fallback: bool,
    pub encoder: String,
    pub allow_encoder_fallback: bool,
    pub target_width: u32,
    pub target_height: u32,
    pub queue_depth: u32,
    pub gop: u32,
    pub bframes: u32,
    pub encoder_preset: String,
    pub encoder_tune: String,
    pub rc_mode: String,
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub adapt_mode: String,
    pub adapt_enable: bool,
    pub min_fps: u32,
    pub max_fps: u32,
    pub performance_profile: String,
    pub queue_strategy: String,
    pub profile_template: String,
    pub enable_template_overlay: bool,
    pub frame_pacing_enable: bool,
    pub frame_pacing_batch_packets: u32,
    pub force_idr_on_pli: bool,
    pub idr_interval_sec: u32,
    pub capture_thread_priority: String,
    pub encode_thread_priority: String,
    pub max_frame_latency: u32,
    pub rtp_use_manual_packetizer: bool,
    pub rtp_mtu: u16,
    pub rtp_au_align: bool,
    pub network_adapt_enable: bool,
    pub network_adapt_floor_bitrate_kbps: u32,
    pub network_adapt_ceiling_bitrate_kbps: u32,
    pub stats_interval_ms: u32,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub ws_url: String,
    pub device_name: String,
    pub capture: CaptureConfig,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://127.0.0.1:9527".to_string(),
            device_name: "Rust Agent".to_string(),
            capture: CaptureConfig {
                fps: 2,
                jpeg_quality: 70,
                backend: "auto".to_string(),
                allow_fallback: true,
                encoder: "auto".to_string(),
                allow_encoder_fallback: true,
                target_width: 0,
                target_height: 0,
                queue_depth: 8,
                gop: 60,
                bframes: 0,
                encoder_preset: "p4".to_string(),
                encoder_tune: "balanced".to_string(),
                rc_mode: "vbr".to_string(),
                bitrate_kbps: 12000,
                max_bitrate_kbps: 20000,
                adapt_mode: "balanced".to_string(),
                adapt_enable: true,
                min_fps: 24,
                max_fps: 120,
                performance_profile: "balanced".to_string(),
                queue_strategy: "drop".to_string(),
                profile_template: "balanced".to_string(),
                enable_template_overlay: true,
                frame_pacing_enable: true,
                frame_pacing_batch_packets: 6,
                force_idr_on_pli: true,
                idr_interval_sec: 2,
                capture_thread_priority: "high".to_string(),
                encode_thread_priority: "time_critical".to_string(),
                max_frame_latency: 1,
                rtp_use_manual_packetizer: true,
                rtp_mtu: 1200,
                rtp_au_align: true,
                network_adapt_enable: true,
                network_adapt_floor_bitrate_kbps: 6000,
                network_adapt_ceiling_bitrate_kbps: 80000,
                stats_interval_ms: 1000,
            },
        }
    }
}

pub fn load_config(path: &Path) -> AgentConfig {
    let raw = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return AgentConfig::default(),
    };

    let mut cfg = AgentConfig::default();
    if let Some(v) = extract_string(&raw, "ws_url") {
        cfg.ws_url = v;
    }
    if let Some(v) = extract_string(&raw, "device_name") {
        cfg.device_name = v;
    }
    if let Some(v) = extract_u32(&raw, "fps")
        && (1..=240).contains(&v)
    {
        cfg.capture.fps = v;
    }
    if let Some(v) = extract_u32(&raw, "jpeg_quality")
        && (1..=100).contains(&v)
    {
        cfg.capture.jpeg_quality = v as u8;
    }
    if let Some(v) = extract_string(&raw, "backend") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "auto" | "dxgi" | "powershell" | "dummy") {
            cfg.capture.backend = v;
        }
    }
    if let Some(v) = extract_bool(&raw, "allow_fallback") {
        cfg.capture.allow_fallback = v;
    }
    if let Some(v) = extract_string(&raw, "encoder") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "auto" | "nvenc" | "qsv" | "amf" | "openh264") {
            cfg.capture.encoder = v;
        }
    }
    if let Some(v) = extract_bool(&raw, "allow_encoder_fallback") {
        cfg.capture.allow_encoder_fallback = v;
    }
    if let Some(v) = extract_u32(&raw, "target_width")
        && (0..=7680).contains(&v)
    {
        cfg.capture.target_width = v;
    }
    if let Some(v) = extract_u32(&raw, "target_height")
        && (0..=4320).contains(&v)
    {
        cfg.capture.target_height = v;
    }
    if let Some(v) = extract_u32(&raw, "queue_depth")
        && (1..=64).contains(&v)
    {
        cfg.capture.queue_depth = v;
    }
    if let Some(v) = extract_u32(&raw, "gop")
        && (1..=600).contains(&v)
    {
        cfg.capture.gop = v;
    }
    if let Some(v) = extract_u32(&raw, "bframes")
        && (0..=8).contains(&v)
    {
        cfg.capture.bframes = v;
    }
    if let Some(v) = extract_string(&raw, "encoder_preset") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7") {
            cfg.capture.encoder_preset = v;
        }
    }
    if let Some(v) = extract_string(&raw, "encoder_tune") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "ll" | "ull" | "hq" | "balanced") {
            cfg.capture.encoder_tune = v;
        }
    }
    if let Some(v) = extract_string(&raw, "rc_mode") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "vbr" | "cbr") {
            cfg.capture.rc_mode = v;
        }
    }
    if let Some(v) = extract_u32(&raw, "bitrate_kbps")
        && (100..=200000).contains(&v)
    {
        cfg.capture.bitrate_kbps = v;
    }
    if let Some(v) = extract_u32(&raw, "max_bitrate_kbps")
        && (100..=300000).contains(&v)
    {
        cfg.capture.max_bitrate_kbps = v;
    }
    if let Some(v) = extract_string(&raw, "adapt_mode") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "smooth" | "balanced" | "quality") {
            cfg.capture.adapt_mode = v;
        }
    }
    if let Some(v) = extract_bool(&raw, "adapt_enable") {
        cfg.capture.adapt_enable = v;
    }
    if let Some(v) = extract_u32(&raw, "min_fps")
        && (1..=240).contains(&v)
    {
        cfg.capture.min_fps = v;
    }
    if let Some(v) = extract_u32(&raw, "max_fps")
        && (1..=240).contains(&v)
    {
        cfg.capture.max_fps = v;
    }
    if let Some(v) = extract_string(&raw, "performance_profile") {
        let v = v.to_ascii_lowercase();
        if matches!(
            v.as_str(),
            "smooth" | "balanced" | "quality" | "latency_first" | "quality_first"
        ) {
            cfg.capture.performance_profile = v;
        }
    }
    if let Some(v) = extract_string(&raw, "queue_strategy") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "drop" | "block") {
            cfg.capture.queue_strategy = v;
        }
    }
    if let Some(v) = extract_string(&raw, "profile_template") {
        let v = v.to_ascii_lowercase();
        if matches!(
            v.as_str(),
            "latency_first" | "balanced" | "quality_first" | "custom"
        ) {
            cfg.capture.profile_template = v;
        }
    } else {
        cfg.capture.profile_template = match cfg.capture.performance_profile.as_str() {
            "smooth" | "latency_first" => "latency_first".to_string(),
            "quality" | "quality_first" => "quality_first".to_string(),
            _ => "balanced".to_string(),
        };
    }
    if let Some(v) = extract_bool(&raw, "enable_template_overlay") {
        cfg.capture.enable_template_overlay = v;
    }
    if let Some(v) = extract_bool(&raw, "frame_pacing_enable") {
        cfg.capture.frame_pacing_enable = v;
    }
    if let Some(v) = extract_u32(&raw, "frame_pacing_batch_packets")
        && (1..=64).contains(&v)
    {
        cfg.capture.frame_pacing_batch_packets = v;
    }
    if let Some(v) = extract_bool(&raw, "force_idr_on_pli") {
        cfg.capture.force_idr_on_pli = v;
    }
    if let Some(v) = extract_u32(&raw, "idr_interval_sec")
        && (1..=30).contains(&v)
    {
        cfg.capture.idr_interval_sec = v;
    }
    if let Some(v) = extract_string(&raw, "capture_thread_priority") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "normal" | "high" | "time_critical") {
            cfg.capture.capture_thread_priority = v;
        }
    }
    if let Some(v) = extract_string(&raw, "encode_thread_priority") {
        let v = v.to_ascii_lowercase();
        if matches!(v.as_str(), "normal" | "high" | "time_critical") {
            cfg.capture.encode_thread_priority = v;
        }
    }
    if let Some(v) = extract_u32(&raw, "max_frame_latency")
        && (1..=4).contains(&v)
    {
        cfg.capture.max_frame_latency = v;
    }
    if let Some(v) = extract_bool(&raw, "rtp_use_manual_packetizer") {
        cfg.capture.rtp_use_manual_packetizer = v;
    }
    if let Some(v) = extract_u32(&raw, "rtp_mtu")
        && (576..=1460).contains(&v)
    {
        cfg.capture.rtp_mtu = v as u16;
    }
    if let Some(v) = extract_bool(&raw, "rtp_au_align") {
        cfg.capture.rtp_au_align = v;
    }
    if let Some(v) = extract_bool(&raw, "network_adapt_enable") {
        cfg.capture.network_adapt_enable = v;
    }
    if let Some(v) = extract_u32(&raw, "network_adapt_floor_bitrate_kbps")
        && (100..=200000).contains(&v)
    {
        cfg.capture.network_adapt_floor_bitrate_kbps = v;
    }
    if let Some(v) = extract_u32(&raw, "network_adapt_ceiling_bitrate_kbps")
        && (100..=300000).contains(&v)
    {
        cfg.capture.network_adapt_ceiling_bitrate_kbps = v;
    }
    if let Some(v) = extract_u32(&raw, "stats_interval_ms")
        && (200..=10_000).contains(&v)
    {
        cfg.capture.stats_interval_ms = v;
    }
    cfg
}

pub fn register_message(name: &str) -> String {
    format!(
        "{{\"type\":\"device\",\"action\":\"register\",\"payload\":{{\"type\":\"agent-rust\",\"name\":\"{}\"}}}}",
        escape_json(name)
    )
}

pub fn frame_message(image_b64: &str, width: u32, height: u32) -> String {
    format!(
        "{{\"type\":\"stream\",\"action\":\"frame\",\"payload\":{{\"image\":\"{}\",\"width\":{},\"height\":{},\"format\":\"jpeg\",\"ts\":{}}}}}",
        image_b64,
        width,
        height,
        now_ms()
    )
}

fn now_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn extract_string(raw: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let idx = raw.find(&pat)?;
    let rest = &raw[idx + pat.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let q1 = rest.find('"')?;
    let rest = &rest[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(rest[..q2].to_string())
}

fn extract_u32(raw: &str, key: &str) -> Option<u32> {
    let pat = format!("\"{key}\"");
    let idx = raw.find(&pat)?;
    let rest = &raw[idx + pat.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let digits: String = rest
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn extract_bool(raw: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{key}\"");
    let idx = raw.find(&pat)?;
    let rest = &raw[idx + pat.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    if rest.starts_with("true") {
        return Some(true);
    }
    if rest.starts_with("false") {
        return Some(false);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, time::Duration};

    #[test]
    fn uses_default_if_missing_file() {
        let cfg = load_config(&PathBuf::from("missing-config.json"));
        assert_eq!(cfg.ws_url, "ws://127.0.0.1:9527");
        assert_eq!(cfg.capture.fps, 2);
        assert_eq!(cfg.capture.backend, "auto");
        assert!(cfg.capture.allow_fallback);
        assert_eq!(cfg.capture.encoder, "auto");
        assert!(cfg.capture.allow_encoder_fallback);
        assert_eq!(cfg.capture.profile_template, "balanced");
        assert!(cfg.capture.rtp_use_manual_packetizer);
    }

    #[test]
    fn frame_message_contains_required_fields() {
        let msg = frame_message("abc", 1920, 1080);
        assert!(msg.contains("\"type\":\"stream\""));
        assert!(msg.contains("\"action\":\"frame\""));
        assert!(msg.contains("\"width\":1920"));
        assert!(msg.contains("\"height\":1080"));
    }

    #[test]
    fn now_is_monotonic_enough() {
        let a = now_ms();
        std::thread::sleep(Duration::from_millis(1));
        let b = now_ms();
        assert!(b >= a);
    }

    #[test]
    fn parses_extended_capture_config_fields() {
        let p = PathBuf::from("test-config-extended.json");
        let raw = r#"{
            "ws_url":"ws://1.2.3.4:9527",
            "capture":{
                "fps":120,
                "target_width":2560,
                "target_height":1440,
                "queue_depth":16,
                "gop":120,
                "bframes":0,
                "encoder_preset":"p5",
                "encoder_tune":"balanced",
                "rc_mode":"vbr",
                "bitrate_kbps":20000,
                "max_bitrate_kbps":30000,
                "adapt_mode":"balanced",
                "adapt_enable":true,
                "min_fps":24,
                "max_fps":120,
                "performance_profile":"smooth",
                "queue_strategy":"block",
                "profile_template":"custom",
                "enable_template_overlay":false,
                "frame_pacing_enable":true,
                "frame_pacing_batch_packets":8,
                "force_idr_on_pli":true,
                "idr_interval_sec":2,
                "capture_thread_priority":"high",
                "encode_thread_priority":"time_critical",
                "max_frame_latency":1,
                "rtp_use_manual_packetizer":true,
                "rtp_mtu":1200,
                "rtp_au_align":true,
                "network_adapt_enable":true,
                "network_adapt_floor_bitrate_kbps":6000,
                "network_adapt_ceiling_bitrate_kbps":80000,
                "stats_interval_ms":1000
            }
        }"#;
        fs::write(&p, raw).expect("write test config");
        let cfg = load_config(&p);
        fs::remove_file(&p).ok();
        assert_eq!(cfg.ws_url, "ws://1.2.3.4:9527");
        assert_eq!(cfg.capture.fps, 120);
        assert_eq!(cfg.capture.target_width, 2560);
        assert_eq!(cfg.capture.target_height, 1440);
        assert_eq!(cfg.capture.queue_depth, 16);
        assert_eq!(cfg.capture.gop, 120);
        assert_eq!(cfg.capture.encoder_preset, "p5");
        assert_eq!(cfg.capture.bitrate_kbps, 20000);
        assert_eq!(cfg.capture.max_bitrate_kbps, 30000);
        assert!(cfg.capture.adapt_enable);
        assert_eq!(cfg.capture.performance_profile, "smooth");
        assert_eq!(cfg.capture.queue_strategy, "block");
        assert_eq!(cfg.capture.profile_template, "custom");
        assert!(!cfg.capture.enable_template_overlay);
        assert_eq!(cfg.capture.frame_pacing_batch_packets, 8);
        assert_eq!(cfg.capture.rtp_mtu, 1200);
        assert_eq!(cfg.capture.network_adapt_floor_bitrate_kbps, 6000);
    }
}
