use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fs, path::Path};

#[derive(Debug, Clone, Deserialize)]
pub struct CaptureConfig {
    pub fps: u32,
    pub jpeg_quality: u8,
    pub backend: String,
    pub allow_fallback: bool,
    pub encoder: String,
    pub allow_encoder_fallback: bool,
    pub strict_gpu_direct: bool,
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
    pub fps_mode: String,
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
    pub max_fps_mode: bool,
    pub idle_repeat_fps: u32,
    pub tier_limit_enable: bool,
    pub tier_fps_l1: u32,
    pub tier_fps_l2: u32,
    pub tier_fps_l3: u32,
    pub tier_fps_l4: u32,
    pub tier_fps_l5: u32,
    pub tier_bitrate_kbps_l1: u32,
    pub tier_bitrate_kbps_l2: u32,
    pub tier_bitrate_kbps_l3: u32,
    pub tier_bitrate_kbps_l4: u32,
    pub tier_bitrate_kbps_l5: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub ws_url: String,
    pub device_name: String,
    pub capture: CaptureConfig,
}

#[derive(Debug)]
pub struct SessionSwitch {
    generation: u64,
    current_running: Option<Arc<AtomicBool>>,
}

impl Default for SessionSwitch {
    fn default() -> Self {
        Self {
            generation: 0,
            current_running: None,
        }
    }
}

impl SessionSwitch {
    pub fn begin(&mut self) -> (u64, Arc<AtomicBool>) {
        if let Some(prev) = self.current_running.take() {
            prev.store(false, Ordering::SeqCst);
        }
        self.generation = self.generation.saturating_add(1);
        let running = Arc::new(AtomicBool::new(true));
        self.current_running = Some(running.clone());
        (self.generation, running)
    }

    pub fn stop_current(&mut self) {
        if let Some(flag) = self.current_running.take() {
            flag.store(false, Ordering::SeqCst);
        }
    }
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
                strict_gpu_direct: false,
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
                fps_mode: "balanced".to_string(),
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
                max_fps_mode: false,
                idle_repeat_fps: 12,
                tier_limit_enable: true,
                tier_fps_l1: 30,
                tier_fps_l2: 60,
                tier_fps_l3: 120,
                tier_fps_l4: 144,
                tier_fps_l5: 240,
                tier_bitrate_kbps_l1: 4000,
                tier_bitrate_kbps_l2: 8000,
                tier_bitrate_kbps_l3: 12000,
                tier_bitrate_kbps_l4: 18000,
                tier_bitrate_kbps_l5: 28000,
            },
        }
    }
}

/// 使用 serde 解析配置文件
/// 如果文件不存在或解析失败，返回默认配置
pub fn load_config(path: &Path) -> AgentConfig {
    let raw = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return AgentConfig::default(),
    };

    // 使用 serde_json 解析，使用默认值填充缺失字段
    // serde_json 会自动使用 Default trait 来填充缺失字段
    let json_val: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_e) => {
            // JSON 解析失败，返回默认配置
            return AgentConfig::default();
        }
    };

    // 先获取默认配置，然后合并 JSON 中的值
    let mut cfg = AgentConfig::default();

    // 手动合并顶层字段
    if let Some(ws_url) = json_val.get("ws_url").and_then(|v| v.as_str()) {
        cfg.ws_url = ws_url.to_string();
    }
    if let Some(device_name) = json_val.get("device_name").and_then(|v| v.as_str()) {
        cfg.device_name = device_name.to_string();
    }

    // 合并 capture 对象的字段
    if let Some(capture) = json_val.get("capture").and_then(|v| v.as_object()) {
        // 使用宏解析所有字段
        if let Some(v) = capture.get("fps").and_then(|v| v.as_u64()) {
            cfg.capture.fps = v as u32;
        }
        if let Some(v) = capture.get("jpeg_quality").and_then(|v| v.as_u64()) {
            cfg.capture.jpeg_quality = v as u8;
        }
        if let Some(v) = capture.get("backend").and_then(|v| v.as_str()) {
            cfg.capture.backend = v.to_string();
        }
        if let Some(v) = capture.get("allow_fallback").and_then(|v| v.as_bool()) {
            cfg.capture.allow_fallback = v;
        }
        if let Some(v) = capture.get("encoder").and_then(|v| v.as_str()) {
            cfg.capture.encoder = v.to_string();
        }
        if let Some(v) = capture
            .get("allow_encoder_fallback")
            .and_then(|v| v.as_bool())
        {
            cfg.capture.allow_encoder_fallback = v;
        }
        if let Some(v) = capture.get("strict_gpu_direct").and_then(|v| v.as_bool()) {
            cfg.capture.strict_gpu_direct = v;
        }
        if let Some(v) = capture.get("target_width").and_then(|v| v.as_u64()) {
            cfg.capture.target_width = v as u32;
        }
        if let Some(v) = capture.get("target_height").and_then(|v| v.as_u64()) {
            cfg.capture.target_height = v as u32;
        }
        if let Some(v) = capture.get("queue_depth").and_then(|v| v.as_u64()) {
            cfg.capture.queue_depth = v as u32;
        }
        if let Some(v) = capture.get("gop").and_then(|v| v.as_u64()) {
            cfg.capture.gop = v as u32;
        }
        if let Some(v) = capture.get("bframes").and_then(|v| v.as_u64()) {
            cfg.capture.bframes = v as u32;
        }
        if let Some(v) = capture.get("encoder_preset").and_then(|v| v.as_str()) {
            cfg.capture.encoder_preset = v.to_string();
        }
        if let Some(v) = capture.get("encoder_tune").and_then(|v| v.as_str()) {
            cfg.capture.encoder_tune = v.to_string();
        }
        if let Some(v) = capture.get("rc_mode").and_then(|v| v.as_str()) {
            cfg.capture.rc_mode = v.to_string();
        }
        if let Some(v) = capture.get("bitrate_kbps").and_then(|v| v.as_u64()) {
            cfg.capture.bitrate_kbps = v as u32;
        }
        if let Some(v) = capture.get("max_bitrate_kbps").and_then(|v| v.as_u64()) {
            cfg.capture.max_bitrate_kbps = v as u32;
        }
        if let Some(v) = capture.get("adapt_mode").and_then(|v| v.as_str()) {
            cfg.capture.adapt_mode = v.to_string();
        }
        if let Some(v) = capture.get("adapt_enable").and_then(|v| v.as_bool()) {
            cfg.capture.adapt_enable = v;
        }
        if let Some(v) = capture.get("min_fps").and_then(|v| v.as_u64()) {
            cfg.capture.min_fps = v as u32;
        }
        if let Some(v) = capture.get("max_fps").and_then(|v| v.as_u64()) {
            cfg.capture.max_fps = v as u32;
        }
        if let Some(v) = capture.get("performance_profile").and_then(|v| v.as_str()) {
            cfg.capture.performance_profile = v.to_string();
        }
        if let Some(v) = capture.get("fps_mode").and_then(|v| v.as_str()) {
            cfg.capture.fps_mode = v.to_string();
        }
        if let Some(v) = capture.get("queue_strategy").and_then(|v| v.as_str()) {
            cfg.capture.queue_strategy = v.to_string();
        }
        if let Some(v) = capture.get("profile_template").and_then(|v| v.as_str()) {
            cfg.capture.profile_template = v.to_string();
        }
        if let Some(v) = capture
            .get("enable_template_overlay")
            .and_then(|v| v.as_bool())
        {
            cfg.capture.enable_template_overlay = v;
        }
        if let Some(v) = capture.get("frame_pacing_enable").and_then(|v| v.as_bool()) {
            cfg.capture.frame_pacing_enable = v;
        }
        if let Some(v) = capture
            .get("frame_pacing_batch_packets")
            .and_then(|v| v.as_u64())
        {
            cfg.capture.frame_pacing_batch_packets = v as u32;
        }
        if let Some(v) = capture.get("force_idr_on_pli").and_then(|v| v.as_bool()) {
            cfg.capture.force_idr_on_pli = v;
        }
        if let Some(v) = capture.get("idr_interval_sec").and_then(|v| v.as_u64()) {
            cfg.capture.idr_interval_sec = v as u32;
        }
        if let Some(v) = capture
            .get("capture_thread_priority")
            .and_then(|v| v.as_str())
        {
            cfg.capture.capture_thread_priority = v.to_string();
        }
        if let Some(v) = capture
            .get("encode_thread_priority")
            .and_then(|v| v.as_str())
        {
            cfg.capture.encode_thread_priority = v.to_string();
        }
        if let Some(v) = capture.get("max_frame_latency").and_then(|v| v.as_u64()) {
            cfg.capture.max_frame_latency = v as u32;
        }
        if let Some(v) = capture
            .get("rtp_use_manual_packetizer")
            .and_then(|v| v.as_bool())
        {
            cfg.capture.rtp_use_manual_packetizer = v;
        }
        if let Some(v) = capture.get("rtp_mtu").and_then(|v| v.as_u64()) {
            cfg.capture.rtp_mtu = v as u16;
        }
        if let Some(v) = capture.get("rtp_au_align").and_then(|v| v.as_bool()) {
            cfg.capture.rtp_au_align = v;
        }
        if let Some(v) = capture
            .get("network_adapt_enable")
            .and_then(|v| v.as_bool())
        {
            cfg.capture.network_adapt_enable = v;
        }
        if let Some(v) = capture
            .get("network_adapt_floor_bitrate_kbps")
            .and_then(|v| v.as_u64())
        {
            cfg.capture.network_adapt_floor_bitrate_kbps = v as u32;
        }
        if let Some(v) = capture
            .get("network_adapt_ceiling_bitrate_kbps")
            .and_then(|v| v.as_u64())
        {
            cfg.capture.network_adapt_ceiling_bitrate_kbps = v as u32;
        }
        if let Some(v) = capture.get("stats_interval_ms").and_then(|v| v.as_u64()) {
            cfg.capture.stats_interval_ms = v as u32;
        }
        if let Some(v) = capture.get("max_fps_mode").and_then(|v| v.as_bool()) {
            cfg.capture.max_fps_mode = v;
        }
        if let Some(v) = capture.get("idle_repeat_fps").and_then(|v| v.as_u64()) {
            cfg.capture.idle_repeat_fps = v as u32;
        }
        if let Some(v) = capture.get("tier_limit_enable").and_then(|v| v.as_bool()) {
            cfg.capture.tier_limit_enable = v;
        }
        if let Some(v) = capture.get("tier_fps_l1").and_then(|v| v.as_u64()) {
            cfg.capture.tier_fps_l1 = v as u32;
        }
        if let Some(v) = capture.get("tier_fps_l2").and_then(|v| v.as_u64()) {
            cfg.capture.tier_fps_l2 = v as u32;
        }
        if let Some(v) = capture.get("tier_fps_l3").and_then(|v| v.as_u64()) {
            cfg.capture.tier_fps_l3 = v as u32;
        }
        if let Some(v) = capture.get("tier_fps_l4").and_then(|v| v.as_u64()) {
            cfg.capture.tier_fps_l4 = v as u32;
        }
        if let Some(v) = capture.get("tier_fps_l5").and_then(|v| v.as_u64()) {
            cfg.capture.tier_fps_l5 = v as u32;
        }
        if let Some(v) = capture.get("tier_bitrate_kbps_l1").and_then(|v| v.as_u64()) {
            cfg.capture.tier_bitrate_kbps_l1 = v as u32;
        }
        if let Some(v) = capture.get("tier_bitrate_kbps_l2").and_then(|v| v.as_u64()) {
            cfg.capture.tier_bitrate_kbps_l2 = v as u32;
        }
        if let Some(v) = capture.get("tier_bitrate_kbps_l3").and_then(|v| v.as_u64()) {
            cfg.capture.tier_bitrate_kbps_l3 = v as u32;
        }
        if let Some(v) = capture.get("tier_bitrate_kbps_l4").and_then(|v| v.as_u64()) {
            cfg.capture.tier_bitrate_kbps_l4 = v as u32;
        }
        if let Some(v) = capture.get("tier_bitrate_kbps_l5").and_then(|v| v.as_u64()) {
            cfg.capture.tier_bitrate_kbps_l5 = v as u32;
        }
    }

    // 应用值范围验证和标准化
    normalize_config(&mut cfg);
    cfg
}

/// 标准化和验证配置值
fn normalize_config(cfg: &mut AgentConfig) {
    // 字符串值标准化为小写
    cfg.capture.backend = cfg.capture.backend.to_ascii_lowercase();
    cfg.capture.encoder = cfg.capture.encoder.to_ascii_lowercase();
    cfg.capture.encoder_preset = cfg.capture.encoder_preset.to_ascii_lowercase();
    cfg.capture.encoder_tune = cfg.capture.encoder_tune.to_ascii_lowercase();
    cfg.capture.rc_mode = cfg.capture.rc_mode.to_ascii_lowercase();
    cfg.capture.adapt_mode = cfg.capture.adapt_mode.to_ascii_lowercase();
    cfg.capture.performance_profile = cfg.capture.performance_profile.to_ascii_lowercase();
    cfg.capture.fps_mode = cfg.capture.fps_mode.to_ascii_lowercase();
    cfg.capture.queue_strategy = cfg.capture.queue_strategy.to_ascii_lowercase();
    cfg.capture.profile_template = cfg.capture.profile_template.to_ascii_lowercase();
    cfg.capture.capture_thread_priority = cfg.capture.capture_thread_priority.to_ascii_lowercase();
    cfg.capture.encode_thread_priority = cfg.capture.encode_thread_priority.to_ascii_lowercase();

    if cfg.capture.strict_gpu_direct {
        cfg.capture.backend = "dxgi".to_string();
        cfg.capture.encoder = "nvenc".to_string();
        cfg.capture.allow_fallback = false;
        cfg.capture.allow_encoder_fallback = false;
    }

    // 数值范围验证和限制
    cfg.capture.fps = cfg.capture.fps.clamp(1, 240);
    cfg.capture.jpeg_quality = cfg.capture.jpeg_quality.clamp(1, 100);
    cfg.capture.target_width = cfg.capture.target_width.clamp(0, 7680);
    cfg.capture.target_height = cfg.capture.target_height.clamp(0, 4320);
    cfg.capture.queue_depth = cfg.capture.queue_depth.clamp(1, 64);
    cfg.capture.gop = cfg.capture.gop.clamp(1, 600);
    cfg.capture.bframes = cfg.capture.bframes.clamp(0, 8);
    cfg.capture.bitrate_kbps = cfg.capture.bitrate_kbps.clamp(100, 200_000);
    cfg.capture.max_bitrate_kbps = cfg.capture.max_bitrate_kbps.clamp(100, 300_000);
    cfg.capture.min_fps = cfg.capture.min_fps.clamp(1, 240);
    cfg.capture.max_fps = cfg.capture.max_fps.clamp(1, 240);
    cfg.capture.frame_pacing_batch_packets = cfg.capture.frame_pacing_batch_packets.clamp(1, 64);
    cfg.capture.idr_interval_sec = cfg.capture.idr_interval_sec.clamp(1, 30);
    cfg.capture.max_frame_latency = cfg.capture.max_frame_latency.clamp(1, 4);
    cfg.capture.rtp_mtu = cfg.capture.rtp_mtu.clamp(576, 1460);
    cfg.capture.network_adapt_floor_bitrate_kbps = cfg
        .capture
        .network_adapt_floor_bitrate_kbps
        .clamp(100, 200_000);
    cfg.capture.network_adapt_ceiling_bitrate_kbps = cfg
        .capture
        .network_adapt_ceiling_bitrate_kbps
        .clamp(100, 300_000);
    cfg.capture.stats_interval_ms = cfg.capture.stats_interval_ms.clamp(200, 10_000);
    cfg.capture.idle_repeat_fps = cfg.capture.idle_repeat_fps.clamp(1, 240);
    cfg.capture.tier_fps_l1 = cfg.capture.tier_fps_l1.clamp(1, 240);
    cfg.capture.tier_fps_l2 = cfg.capture.tier_fps_l2.clamp(1, 240);
    cfg.capture.tier_fps_l3 = cfg.capture.tier_fps_l3.clamp(1, 240);
    cfg.capture.tier_fps_l4 = cfg.capture.tier_fps_l4.clamp(1, 240);
    cfg.capture.tier_fps_l5 = cfg.capture.tier_fps_l5.clamp(1, 240);
    cfg.capture.tier_bitrate_kbps_l1 = cfg.capture.tier_bitrate_kbps_l1.clamp(100, 300_000);
    cfg.capture.tier_bitrate_kbps_l2 = cfg.capture.tier_bitrate_kbps_l2.clamp(100, 300_000);
    cfg.capture.tier_bitrate_kbps_l3 = cfg.capture.tier_bitrate_kbps_l3.clamp(100, 300_000);
    cfg.capture.tier_bitrate_kbps_l4 = cfg.capture.tier_bitrate_kbps_l4.clamp(100, 300_000);
    cfg.capture.tier_bitrate_kbps_l5 = cfg.capture.tier_bitrate_kbps_l5.clamp(100, 300_000);

    // Keep 5-tier profile monotonic to avoid oscillation and invalid ladders.
    cfg.capture.tier_fps_l2 = cfg.capture.tier_fps_l2.max(cfg.capture.tier_fps_l1);
    cfg.capture.tier_fps_l3 = cfg.capture.tier_fps_l3.max(cfg.capture.tier_fps_l2);
    cfg.capture.tier_fps_l4 = cfg.capture.tier_fps_l4.max(cfg.capture.tier_fps_l3);
    cfg.capture.tier_fps_l5 = cfg.capture.tier_fps_l5.max(cfg.capture.tier_fps_l4);
    cfg.capture.tier_bitrate_kbps_l2 = cfg
        .capture
        .tier_bitrate_kbps_l2
        .max(cfg.capture.tier_bitrate_kbps_l1);
    cfg.capture.tier_bitrate_kbps_l3 = cfg
        .capture
        .tier_bitrate_kbps_l3
        .max(cfg.capture.tier_bitrate_kbps_l2);
    cfg.capture.tier_bitrate_kbps_l4 = cfg
        .capture
        .tier_bitrate_kbps_l4
        .max(cfg.capture.tier_bitrate_kbps_l3);
    cfg.capture.tier_bitrate_kbps_l5 = cfg
        .capture
        .tier_bitrate_kbps_l5
        .max(cfg.capture.tier_bitrate_kbps_l4);

    // 字符串枚举验证（使用默认值替代无效值）
    if !matches!(
        cfg.capture.backend.as_str(),
        "auto" | "dxgi" | "wgc" | "powershell" | "dummy"
    ) {
        cfg.capture.backend = "auto".to_string();
    }
    if !matches!(
        cfg.capture.encoder.as_str(),
        "auto" | "nvenc" | "qsv" | "amf" | "openh264"
    ) {
        cfg.capture.encoder = "auto".to_string();
    }
    if !matches!(
        cfg.capture.encoder_preset.as_str(),
        "p1" | "p2" | "p3" | "p4" | "p5" | "p6" | "p7"
    ) {
        cfg.capture.encoder_preset = "p4".to_string();
    }
    if !matches!(
        cfg.capture.encoder_tune.as_str(),
        "ll" | "ull" | "hq" | "balanced"
    ) {
        cfg.capture.encoder_tune = "balanced".to_string();
    }
    if !matches!(cfg.capture.rc_mode.as_str(), "vbr" | "cbr") {
        cfg.capture.rc_mode = "vbr".to_string();
    }
    if !matches!(
        cfg.capture.adapt_mode.as_str(),
        "smooth" | "balanced" | "quality"
    ) {
        cfg.capture.adapt_mode = "balanced".to_string();
    }
    if !matches!(
        cfg.capture.performance_profile.as_str(),
        "smooth" | "balanced" | "quality" | "latency_first" | "quality_first"
    ) {
        cfg.capture.performance_profile = "balanced".to_string();
    }
    if !matches!(
        cfg.capture.fps_mode.as_str(),
        "latency"
            | "balanced"
            | "throughput"
            | "latency_first"
            | "balanced_first"
            | "throughput_first"
            | "max"
    ) {
        cfg.capture.fps_mode = "balanced".to_string();
    }
    if !matches!(cfg.capture.queue_strategy.as_str(), "drop" | "block") {
        cfg.capture.queue_strategy = "drop".to_string();
    }
    if !matches!(
        cfg.capture.profile_template.as_str(),
        "latency_first" | "balanced" | "quality_first" | "custom"
    ) {
        // 如果未明确设置，根据 performance_profile 推导
        cfg.capture.profile_template = match cfg.capture.performance_profile.as_str() {
            "smooth" | "latency_first" => "latency_first".to_string(),
            "quality" | "quality_first" => "quality_first".to_string(),
            _ => "balanced".to_string(),
        };
    }
    if !matches!(
        cfg.capture.capture_thread_priority.as_str(),
        "normal" | "high" | "time_critical"
    ) {
        cfg.capture.capture_thread_priority = "high".to_string();
    }
    if !matches!(
        cfg.capture.encode_thread_priority.as_str(),
        "normal" | "high" | "time_critical"
    ) {
        cfg.capture.encode_thread_priority = "time_critical".to_string();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
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
        assert!(!cfg.capture.strict_gpu_direct);
        assert_eq!(cfg.capture.profile_template, "balanced");
        assert_eq!(cfg.capture.fps_mode, "balanced");
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
                "strict_gpu_direct":true,
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
                "fps_mode":"throughput",
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
                ,"max_fps_mode":true,
                "idle_repeat_fps":12,
                "tier_limit_enable":true,
                "tier_fps_l1":30,
                "tier_fps_l2":60,
                "tier_fps_l3":120,
                "tier_fps_l4":144,
                "tier_fps_l5":240,
                "tier_bitrate_kbps_l1":4000,
                "tier_bitrate_kbps_l2":8000,
                "tier_bitrate_kbps_l3":12000,
                "tier_bitrate_kbps_l4":18000,
                "tier_bitrate_kbps_l5":28000
            }
        }"#;
        fs::write(&p, raw).expect("write test config");
        let cfg = load_config(&p);
        fs::remove_file(&p).ok();
        assert_eq!(cfg.ws_url, "ws://1.2.3.4:9527");
        assert_eq!(cfg.capture.fps, 120);
        assert!(cfg.capture.strict_gpu_direct);
        assert_eq!(cfg.capture.backend, "dxgi");
        assert_eq!(cfg.capture.encoder, "nvenc");
        assert!(!cfg.capture.allow_fallback);
        assert!(!cfg.capture.allow_encoder_fallback);
        assert_eq!(cfg.capture.target_width, 2560);
        assert_eq!(cfg.capture.target_height, 1440);
        assert_eq!(cfg.capture.queue_depth, 16);
        assert_eq!(cfg.capture.gop, 120);
        assert_eq!(cfg.capture.encoder_preset, "p5");
        assert_eq!(cfg.capture.bitrate_kbps, 20000);
        assert_eq!(cfg.capture.max_bitrate_kbps, 30000);
        assert!(cfg.capture.adapt_enable);
        assert_eq!(cfg.capture.performance_profile, "smooth");
        assert_eq!(cfg.capture.fps_mode, "throughput");
        assert_eq!(cfg.capture.queue_strategy, "block");
        assert_eq!(cfg.capture.profile_template, "custom");
        assert!(!cfg.capture.enable_template_overlay);
        assert_eq!(cfg.capture.frame_pacing_batch_packets, 8);
        assert_eq!(cfg.capture.rtp_mtu, 1200);
        assert_eq!(cfg.capture.network_adapt_floor_bitrate_kbps, 6000);
        assert!(cfg.capture.max_fps_mode);
        assert_eq!(cfg.capture.idle_repeat_fps, 12);
        assert!(cfg.capture.tier_limit_enable);
        assert_eq!(cfg.capture.tier_fps_l4, 144);
        assert_eq!(cfg.capture.tier_bitrate_kbps_l5, 28000);
    }

    #[test]
    fn session_switch_stops_previous_session_on_begin() {
        let mut sw = SessionSwitch::default();
        let (_gen1, flag1) = sw.begin();
        assert!(flag1.load(Ordering::Relaxed));

        let (_gen2, flag2) = sw.begin();
        assert!(!flag1.load(Ordering::Relaxed));
        assert!(flag2.load(Ordering::Relaxed));
    }
}
