pub fn apply_capture_profile(cfg: &mut agent_rust::CaptureConfig) {
    let mut template = cfg.profile_template.to_ascii_lowercase();
    if template.is_empty() {
        template = match cfg.performance_profile.as_str() {
            "smooth" | "latency_first" => "latency_first".to_string(),
            "quality" | "quality_first" => "quality_first".to_string(),
            _ => "balanced".to_string(),
        };
    }
    if template == "custom" && !cfg.enable_template_overlay {
        return;
    }
    match template.as_str() {
        "latency_first" => {
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p1".to_string();
                cfg.encoder_tune = "ull".to_string();
                cfg.rc_mode = "cbr".to_string();
                cfg.bframes = cfg.bframes.min(0);
                cfg.gop = cfg.gop.clamp(30, 60);
                cfg.queue_depth = cfg.queue_depth.clamp(2, 6);
                cfg.queue_strategy = "drop".to_string();
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(16000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps);
                cfg.min_fps = cfg.min_fps.max(30);
            }
            cfg.performance_profile = "smooth".to_string();
        }
        "quality_first" => {
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p5".to_string();
                cfg.encoder_tune = "hq".to_string();
                cfg.rc_mode = "vbr".to_string();
                cfg.bframes = cfg.bframes.max(2).min(3);
                cfg.gop = cfg.gop.max(120);
                cfg.queue_depth = cfg.queue_depth.clamp(12, 32);
                cfg.queue_strategy = "block".to_string();
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(28000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps + 12000);
            }
            cfg.performance_profile = "quality".to_string();
        }
        "custom" => {}
        _ => {
            cfg.profile_template = "balanced".to_string();
            if cfg.enable_template_overlay {
                cfg.encoder_preset = "p3".to_string();
                cfg.encoder_tune = "ll".to_string();
                cfg.rc_mode = "cbr".to_string();
                cfg.bframes = cfg.bframes.min(1);
                cfg.gop = cfg.gop.clamp(45, 90);
                cfg.queue_depth = cfg.queue_depth.clamp(4, 16);
                cfg.bitrate_kbps = cfg.bitrate_kbps.max(20000);
                cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.max(cfg.bitrate_kbps + 8000);
                cfg.queue_strategy = "drop".to_string();
            }
            cfg.performance_profile = "balanced".to_string();
        }
    }
    if cfg.max_bitrate_kbps < cfg.bitrate_kbps {
        cfg.max_bitrate_kbps = cfg.bitrate_kbps;
    }
    if cfg.network_adapt_ceiling_bitrate_kbps < cfg.network_adapt_floor_bitrate_kbps {
        cfg.network_adapt_ceiling_bitrate_kbps = cfg.network_adapt_floor_bitrate_kbps;
    }
    if !matches!(cfg.queue_strategy.as_str(), "drop" | "block") {
        cfg.queue_strategy = "drop".to_string();
    }
    if cfg.max_fps_mode {
        // In max-fps mode, prefer throughput/latency over quality knobs.
        cfg.encoder_preset = "p1".to_string();
        cfg.encoder_tune = "ull".to_string();
        cfg.rc_mode = "cbr".to_string();
        cfg.bframes = 0;
        cfg.gop = cfg.gop.clamp(30, 60);
        cfg.frame_pacing_enable = false;
        cfg.queue_strategy = "drop".to_string();
        cfg.enable_template_overlay = false;
        if cfg.fps >= 100 {
            // High-fps mode benefits from freshest-frame delivery and fewer duplicate sends.
            cfg.rtp_use_manual_packetizer = true;
            cfg.queue_depth = cfg.queue_depth.min(2);
        }
    }
    if cfg.tier_limit_enable {
        apply_multi_tier_limits(cfg);
    }
    // Apply GPU-synchronized profile at the end so template/tier logic cannot override it.
    apply_gpu_synchronized_profile(cfg);
}

fn apply_multi_tier_limits(cfg: &mut agent_rust::CaptureConfig) {
    // 5-tier ladder inspired by cpp_capture style quality ladders.
    // Keep FPS continuous (no hard 144/120 cliffs), and use tier bitrates as anchors.
    let tiers = [
        (cfg.tier_fps_l1.max(1), cfg.tier_bitrate_kbps_l1.max(100)),
        (cfg.tier_fps_l2.max(1), cfg.tier_bitrate_kbps_l2.max(100)),
        (cfg.tier_fps_l3.max(1), cfg.tier_bitrate_kbps_l3.max(100)),
        (cfg.tier_fps_l4.max(1), cfg.tier_bitrate_kbps_l4.max(100)),
        (cfg.tier_fps_l5.max(1), cfg.tier_bitrate_kbps_l5.max(100)),
    ];
    let target = cfg.fps.clamp(cfg.min_fps.max(1), cfg.max_fps.max(1)).max(1);

    let tier_br = if target <= tiers[0].0 {
        tiers[0].1
    } else if target >= tiers[4].0 {
        tiers[4].1
    } else {
        let mut out = tiers[4].1;
        for w in tiers.windows(2) {
            let (fps_lo, br_lo) = w[0];
            let (fps_hi, br_hi) = w[1];
            if target >= fps_lo && target <= fps_hi {
                let span = (fps_hi.saturating_sub(fps_lo)).max(1) as f64;
                let t = (target.saturating_sub(fps_lo)) as f64 / span;
                out = (br_lo as f64 + (br_hi as f64 - br_lo as f64) * t).round() as u32;
                break;
            }
        }
        out
    };

    cfg.fps = target;
    cfg.max_fps = cfg.max_fps.max(cfg.fps).max(1);
    cfg.min_fps = cfg.min_fps.min(cfg.max_fps).max(1);
    cfg.idle_repeat_fps = cfg.idle_repeat_fps.min(cfg.max_fps).max(1);
    cfg.bitrate_kbps = cfg.bitrate_kbps.min(tier_br).max(100);
    cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.min(tier_br).max(cfg.bitrate_kbps);

    // Lower tiers should keep queue shallow to avoid stale-frame bursts.
    cfg.queue_depth = if target >= 144 {
        cfg.queue_depth.min(4).max(2)
    } else if target >= 120 {
        cfg.queue_depth.min(6).max(2)
    } else if target >= 60 {
        cfg.queue_depth.min(8).max(3)
    } else {
        cfg.queue_depth.min(12).max(4)
    };
}

/// Apply GPU-synchronized profile to align with controller's shared texture slots.
/// This ensures the agent's queue depth doesn't exceed the controller's shared keyed mutex slots,
/// preventing deadlock and minimizing latency.
fn apply_gpu_synchronized_profile(cfg: &mut agent_rust::CaptureConfig) {
    // Read the controller's shared texture slot count
    let shared_slots = std::env::var("MRD_SHARED_KEYED_SLOTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(8u32); // Default to 8 slots

    // Limit queue depth to not exceed shared slots
    // This prevents the agent from producing frames faster than the controller can consume them
    cfg.queue_depth = cfg.queue_depth.min(shared_slots);

    // Set max frame latency to 1 for minimum latency
    // This ensures each frame is processed immediately without buffering
    cfg.max_frame_latency = cfg.max_frame_latency.min(1);

    tracing::debug!(
        shared_slots,
        queue_depth = cfg.queue_depth,
        max_frame_latency = cfg.max_frame_latency,
        "applied GPU-synchronized profile"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_cfg() -> agent_rust::CaptureConfig {
        let mut cfg = agent_rust::AgentConfig::default().capture;
        cfg.tier_limit_enable = true;
        cfg.tier_fps_l1 = 30;
        cfg.tier_fps_l2 = 60;
        cfg.tier_fps_l3 = 120;
        cfg.tier_fps_l4 = 144;
        cfg.tier_fps_l5 = 240;
        cfg.tier_bitrate_kbps_l1 = 4000;
        cfg.tier_bitrate_kbps_l2 = 8000;
        cfg.tier_bitrate_kbps_l3 = 12000;
        cfg.tier_bitrate_kbps_l4 = 18000;
        cfg.tier_bitrate_kbps_l5 = 28000;
        cfg
    }

    #[test]
    fn tier_limits_keep_continuous_fps_for_target_180() {
        let mut cfg = base_cfg();
        cfg.fps = 180;
        cfg.min_fps = 180;
        cfg.max_fps = 180;
        cfg.idle_repeat_fps = 180;
        cfg.bitrate_kbps = 26000;
        cfg.max_bitrate_kbps = 30000;
        apply_capture_profile(&mut cfg);
        assert_eq!(cfg.fps, 180);
        assert_eq!(cfg.max_fps, 180);
        assert_eq!(cfg.idle_repeat_fps, 180);
        assert!(cfg.bitrate_kbps <= 23000);
        assert!(cfg.max_bitrate_kbps <= 23000);
    }

    #[test]
    fn tier_limits_keep_240_for_target_240() {
        let mut cfg = base_cfg();
        cfg.fps = 240;
        cfg.min_fps = 240;
        cfg.max_fps = 240;
        cfg.idle_repeat_fps = 240;
        cfg.bitrate_kbps = 26000;
        cfg.max_bitrate_kbps = 30000;
        apply_capture_profile(&mut cfg);
        assert_eq!(cfg.fps, 240);
        assert!(cfg.bitrate_kbps <= 28000);
    }

    #[test]
    fn max_fps_mode_forces_latency_encoder_knobs() {
        let mut cfg = base_cfg();
        cfg.max_fps_mode = true;
        cfg.fps = 240;
        cfg.encoder_preset = "p5".to_string();
        cfg.encoder_tune = "balanced".to_string();
        cfg.rc_mode = "vbr".to_string();
        cfg.bframes = 3;
        cfg.gop = 240;
        apply_capture_profile(&mut cfg);
        assert_eq!(cfg.encoder_preset, "p1");
        assert_eq!(cfg.encoder_tune, "ull");
        assert_eq!(cfg.rc_mode, "cbr");
        assert_eq!(cfg.bframes, 0);
        assert!(cfg.gop <= 60);
    }
}
