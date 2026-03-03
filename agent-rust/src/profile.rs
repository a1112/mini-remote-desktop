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
        cfg.frame_pacing_enable = false;
        cfg.queue_strategy = "drop".to_string();
        cfg.enable_template_overlay = false;
        if cfg.fps >= 100 {
            // High-fps mode benefits from freshest-frame delivery and fewer duplicate sends.
            cfg.rtp_use_manual_packetizer = true;
            cfg.queue_depth = cfg.queue_depth.min(2);
        }
    }
}
