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
}

fn apply_multi_tier_limits(cfg: &mut agent_rust::CaptureConfig) {
    // 5-tier ladder inspired by cpp_capture style quality ladders.
    let tiers = [
        (cfg.tier_fps_l1, cfg.tier_bitrate_kbps_l1),
        (cfg.tier_fps_l2, cfg.tier_bitrate_kbps_l2),
        (cfg.tier_fps_l3, cfg.tier_bitrate_kbps_l3),
        (cfg.tier_fps_l4, cfg.tier_bitrate_kbps_l4),
        (cfg.tier_fps_l5, cfg.tier_bitrate_kbps_l5),
    ];
    let target = cfg.fps.max(1);
    let (tier_fps, tier_br) = tiers
        .iter()
        .copied()
        .filter(|(fps, _)| *fps <= target)
        .next_back()
        .unwrap_or(tiers[0]);

    cfg.fps = cfg.fps.min(tier_fps);
    cfg.max_fps = cfg.max_fps.min(tier_fps).max(cfg.min_fps.min(tier_fps));
    cfg.min_fps = cfg.min_fps.min(cfg.max_fps).max(1);
    cfg.idle_repeat_fps = cfg.idle_repeat_fps.min(tier_fps).max(1);
    cfg.bitrate_kbps = cfg.bitrate_kbps.min(tier_br).max(100);
    cfg.max_bitrate_kbps = cfg.max_bitrate_kbps.min(tier_br).max(cfg.bitrate_kbps);

    // Lower tiers should keep queue shallow to avoid stale-frame bursts.
    cfg.queue_depth = if tier_fps >= 144 {
        cfg.queue_depth.min(4).max(2)
    } else if tier_fps >= 120 {
        cfg.queue_depth.min(6).max(2)
    } else if tier_fps >= 60 {
        cfg.queue_depth.min(8).max(3)
    } else {
        cfg.queue_depth.min(12).max(4)
    };
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
    fn tier_limits_clamp_to_144_for_target_180() {
        let mut cfg = base_cfg();
        cfg.fps = 180;
        cfg.min_fps = 180;
        cfg.max_fps = 180;
        cfg.idle_repeat_fps = 180;
        cfg.bitrate_kbps = 26000;
        cfg.max_bitrate_kbps = 30000;
        apply_capture_profile(&mut cfg);
        assert_eq!(cfg.fps, 144);
        assert_eq!(cfg.max_fps, 144);
        assert_eq!(cfg.idle_repeat_fps, 144);
        assert!(cfg.bitrate_kbps <= 18000);
        assert!(cfg.max_bitrate_kbps <= 18000);
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
