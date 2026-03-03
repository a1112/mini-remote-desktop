use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const TIER_REASON_INIT: u32 = 1;
pub const TIER_REASON_NACK: u32 = 2;
pub const TIER_REASON_REMB_LOW: u32 = 3;
pub const TIER_REASON_QUALITY_LOW: u32 = 4;
pub const TIER_REASON_RECOVER: u32 = 5;
pub const TIER_REASON_REMB_HIGH: u32 = 6;

pub fn tier_reason_label(code: u32) -> &'static str {
    match code {
        TIER_REASON_INIT => "init",
        TIER_REASON_NACK => "nack",
        TIER_REASON_REMB_LOW => "remb_low",
        TIER_REASON_QUALITY_LOW => "quality_low",
        TIER_REASON_RECOVER => "recover",
        TIER_REASON_REMB_HIGH => "remb_high",
        _ => "none",
    }
}

pub struct NetAdaptController {
    min_fps: u32,
    max_fps: u32,
    current_fps: AtomicU32,
    min_bitrate_kbps: u32,
    max_bitrate_kbps: u32,
    current_bitrate_kbps: AtomicU32,
    tier_enable: bool,
    tier_fps: [u32; 5],
    tier_bitrate_kbps: [u32; 5],
    current_tier_idx: AtomicU32, // 0..4 => L1..L5
    tier_reason_code: AtomicU32,
    tier_switch_count: AtomicU64,
    quality_low_streak: std::sync::Mutex<u32>,
    quality_high_streak: std::sync::Mutex<u32>,
    last_tier_change: std::sync::Mutex<Instant>,
    last_nack: std::sync::Mutex<Instant>,
    last_recover: std::sync::Mutex<Instant>,
}

impl NetAdaptController {
    pub fn new(
        min_fps: u32,
        max_fps: u32,
        initial_fps: u32,
        min_bitrate_kbps: u32,
        max_bitrate_kbps: u32,
        _initial_bitrate_kbps: u32,
        tier_enable: bool,
        tier_fps: [u32; 5],
        tier_bitrate_kbps: [u32; 5],
    ) -> Self {
        let min_fps = min_fps.max(1);
        let max_fps = max_fps.max(min_fps);
        let min_bitrate_kbps = min_bitrate_kbps.max(100);
        let max_bitrate_kbps = max_bitrate_kbps.max(min_bitrate_kbps);
        let mut tier_fps_norm = tier_fps;
        let mut tier_br_norm = tier_bitrate_kbps;
        for v in &mut tier_fps_norm {
            *v = (*v).clamp(min_fps, max_fps);
        }
        for v in &mut tier_br_norm {
            *v = (*v).clamp(min_bitrate_kbps, max_bitrate_kbps);
        }
        for i in 1..5 {
            tier_fps_norm[i] = tier_fps_norm[i].max(tier_fps_norm[i - 1]);
            tier_br_norm[i] = tier_br_norm[i].max(tier_br_norm[i - 1]);
        }
        let mut initial_tier_idx = 0usize;
        let init_fps = initial_fps.clamp(min_fps, max_fps);
        for (i, fps) in tier_fps_norm.iter().enumerate() {
            if *fps <= init_fps {
                initial_tier_idx = i;
            }
        }
        let initial_fps = tier_fps_norm[initial_tier_idx];
        let initial_bitrate_kbps = tier_br_norm[initial_tier_idx];
        let now = Instant::now();
        Self {
            min_fps,
            max_fps,
            current_fps: AtomicU32::new(initial_fps),
            min_bitrate_kbps,
            max_bitrate_kbps,
            current_bitrate_kbps: AtomicU32::new(initial_bitrate_kbps),
            tier_enable,
            tier_fps: tier_fps_norm,
            tier_bitrate_kbps: tier_br_norm,
            current_tier_idx: AtomicU32::new(initial_tier_idx as u32),
            tier_reason_code: AtomicU32::new(TIER_REASON_INIT),
            tier_switch_count: AtomicU64::new(0),
            quality_low_streak: std::sync::Mutex::new(0),
            quality_high_streak: std::sync::Mutex::new(0),
            last_tier_change: std::sync::Mutex::new(now),
            last_nack: std::sync::Mutex::new(now),
            last_recover: std::sync::Mutex::new(now),
        }
    }

    pub fn current_fps(&self) -> u32 {
        self.current_fps
            .load(Ordering::Relaxed)
            .clamp(self.min_fps, self.max_fps)
    }

    pub fn current_bitrate_kbps(&self) -> u32 {
        self.current_bitrate_kbps
            .load(Ordering::Relaxed)
            .clamp(self.min_bitrate_kbps, self.max_bitrate_kbps)
    }

    pub fn current_tier_level(&self) -> u32 {
        self.current_tier_idx.load(Ordering::Relaxed).clamp(0, 4) + 1
    }

    pub fn tier_reason_code(&self) -> u32 {
        self.tier_reason_code.load(Ordering::Relaxed)
    }

    pub fn tier_switch_count(&self) -> u64 {
        self.tier_switch_count.load(Ordering::Relaxed)
    }

    pub fn on_nack_burst(&self) -> (u32, u32) {
        let now = Instant::now();
        if let Ok(mut t) = self.last_nack.lock() {
            *t = now;
        }
        if self.tier_enable {
            if self.try_step_tier_down(TIER_REASON_NACK) {
                return (self.current_fps(), self.current_bitrate_kbps());
            }
            return (self.current_fps(), self.current_bitrate_kbps());
        }
        let cur_fps = self.current_fps();
        let next_fps = cur_fps.saturating_sub(5).max(self.min_fps);
        self.current_fps.store(next_fps, Ordering::Relaxed);

        let cur_br = self.current_bitrate_kbps();
        let next_br = (cur_br as f32 * 0.88) as u32;
        let next_br = next_br.clamp(self.min_bitrate_kbps, self.max_bitrate_kbps);
        self.current_bitrate_kbps.store(next_br, Ordering::Relaxed);

        (next_fps, next_br)
    }

    pub fn on_remb_bps(&self, bitrate_bps: f32) -> (u32, u32) {
        if self.tier_enable {
            let cur_br_bps = (self.current_bitrate_kbps() as f32) * 1000.0;
            if bitrate_bps < cur_br_bps * 0.78 {
                let _ = self.try_step_tier_down(TIER_REASON_REMB_LOW);
            } else if bitrate_bps > cur_br_bps * 1.35 {
                let _ = self.try_step_tier_up(TIER_REASON_REMB_HIGH);
            }
            return (self.current_fps(), self.current_bitrate_kbps());
        }
        let cur = self.current_fps();
        let next_fps = if bitrate_bps < 8_000_000.0 {
            cur.saturating_sub(8).max(self.min_fps)
        } else if bitrate_bps < 14_000_000.0 {
            cur.saturating_sub(3).max(self.min_fps)
        } else if bitrate_bps > 25_000_000.0 {
            (cur + 2).min(self.max_fps)
        } else {
            cur
        };
        self.current_fps.store(next_fps, Ordering::Relaxed);

        let remb_kbps = (bitrate_bps / 1000.0).max(self.min_bitrate_kbps as f32) as u32;
        let target_br = (remb_kbps as f32 * 0.92) as u32;
        let target_br = target_br.clamp(self.min_bitrate_kbps, self.max_bitrate_kbps);
        self.current_bitrate_kbps
            .store(target_br, Ordering::Relaxed);
        (next_fps, target_br)
    }

    pub fn tick_recover(&self) -> Option<(u32, u32)> {
        if self.tier_enable {
            let now = Instant::now();
            let last_nack = self.last_nack.lock().ok().map(|v| *v).unwrap_or(now);
            if now.duration_since(last_nack) >= Duration::from_secs(3)
                && self.try_step_tier_up(TIER_REASON_RECOVER)
            {
                return Some((self.current_fps(), self.current_bitrate_kbps()));
            }
            return None;
        }
        let now = Instant::now();
        let last_nack = self.last_nack.lock().ok().map(|v| *v).unwrap_or(now);
        if now.duration_since(last_nack) < Duration::from_secs(2) {
            return None;
        }
        let mut recovered = None;
        if let Ok(mut last_recover) = self.last_recover.lock()
            && now.duration_since(*last_recover) >= Duration::from_secs(1)
        {
            let cur = self.current_fps();
            if cur < self.max_fps {
                let next = (cur + 1).min(self.max_fps);
                self.current_fps.store(next, Ordering::Relaxed);
                recovered = Some(next);
            }

            let cur_br = self.current_bitrate_kbps();
            if cur_br < self.max_bitrate_kbps {
                let next_br = (cur_br + 400).min(self.max_bitrate_kbps);
                self.current_bitrate_kbps.store(next_br, Ordering::Relaxed);
            }
            *last_recover = now;
        }
        recovered.map(|fps| (fps, self.current_bitrate_kbps()))
    }

    pub fn on_quality_sample(&self, unique_send_fps: f32) -> Option<(u32, u32)> {
        if !self.tier_enable {
            return None;
        }
        let target = self.current_fps().max(1) as f32;
        let low = target * 0.72;
        let high = target * 0.92;

        if unique_send_fps < low {
            if let Ok(mut s) = self.quality_low_streak.lock() {
                *s = s.saturating_add(1);
            }
            if let Ok(mut s) = self.quality_high_streak.lock() {
                *s = 0;
            }
            let streak = self.quality_low_streak.lock().ok().map(|v| *v).unwrap_or(0);
            if streak >= 2 && self.try_step_tier_down(TIER_REASON_QUALITY_LOW) {
                if let Ok(mut s) = self.quality_low_streak.lock() {
                    *s = 0;
                }
                return Some((self.current_fps(), self.current_bitrate_kbps()));
            }
            return None;
        }

        if unique_send_fps > high {
            if let Ok(mut s) = self.quality_high_streak.lock() {
                *s = s.saturating_add(1);
            }
            if let Ok(mut s) = self.quality_low_streak.lock() {
                *s = 0;
            }
            let streak = self.quality_high_streak.lock().ok().map(|v| *v).unwrap_or(0);
            let now = Instant::now();
            let last_nack = self.last_nack.lock().ok().map(|v| *v).unwrap_or(now);
            if streak >= 4
                && now.duration_since(last_nack) >= Duration::from_secs(2)
                && self.try_step_tier_up(TIER_REASON_RECOVER)
            {
                if let Ok(mut s) = self.quality_high_streak.lock() {
                    *s = 0;
                }
                return Some((self.current_fps(), self.current_bitrate_kbps()));
            }
            return None;
        }

        if let Ok(mut s) = self.quality_low_streak.lock() {
            *s = 0;
        }
        if let Ok(mut s) = self.quality_high_streak.lock() {
            *s = 0;
        }
        None
    }

    fn try_step_tier_down(&self, reason_code: u32) -> bool {
        let cur = self.current_tier_idx.load(Ordering::Relaxed).clamp(0, 4);
        if cur == 0 || !self.can_switch_tier() {
            return false;
        }
        self.apply_tier(cur - 1, reason_code);
        true
    }

    fn try_step_tier_up(&self, reason_code: u32) -> bool {
        let cur = self.current_tier_idx.load(Ordering::Relaxed).clamp(0, 4);
        if cur >= 4 || !self.can_switch_tier() {
            return false;
        }
        self.apply_tier(cur + 1, reason_code);
        true
    }

    fn can_switch_tier(&self) -> bool {
        let now = Instant::now();
        let Some(last) = self.last_tier_change.lock().ok().map(|v| *v) else {
            return false;
        };
        now.duration_since(last) >= Duration::from_secs(2)
    }

    fn apply_tier(&self, idx: u32, reason_code: u32) {
        let idx = idx.clamp(0, 4);
        let fps = self.tier_fps[idx as usize].clamp(self.min_fps, self.max_fps);
        let br = self.tier_bitrate_kbps[idx as usize].clamp(self.min_bitrate_kbps, self.max_bitrate_kbps);
        self.current_tier_idx.store(idx, Ordering::Relaxed);
        self.current_fps.store(fps, Ordering::Relaxed);
        self.current_bitrate_kbps.store(br, Ordering::Relaxed);
        self.tier_reason_code.store(reason_code, Ordering::Relaxed);
        self.tier_switch_count.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut t) = self.last_tier_change.lock() {
            *t = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctrl() -> NetAdaptController {
        NetAdaptController::new(
            1,
            240,
            144,
            1000,
            50000,
            18000,
            true,
            [30, 60, 120, 144, 240],
            [4000, 8000, 12000, 18000, 28000],
        )
    }

    #[test]
    fn quality_low_steps_down_tier() {
        let ctrl = make_ctrl();
        {
            let mut t = ctrl.last_tier_change.lock().expect("tier lock");
            *t = Instant::now() - Duration::from_secs(10);
        }
        assert_eq!(ctrl.current_tier_level(), 4);
        let _ = ctrl.on_quality_sample(10.0);
        {
            let mut t = ctrl.last_tier_change.lock().expect("tier lock");
            *t = Instant::now() - Duration::from_secs(10);
        }
        let changed = ctrl.on_quality_sample(10.0);
        assert!(changed.is_some());
        assert_eq!(ctrl.current_tier_level(), 3);
        assert_eq!(ctrl.tier_reason_code(), TIER_REASON_QUALITY_LOW);
    }

    #[test]
    fn recover_can_step_up_tier() {
        let ctrl = make_ctrl();
        {
            let mut t = ctrl.last_tier_change.lock().expect("tier lock");
            *t = Instant::now() - Duration::from_secs(10);
        }
        let _ = ctrl.on_nack_burst();
        assert_eq!(ctrl.current_tier_level(), 3);
        {
            let mut n = ctrl.last_nack.lock().expect("nack lock");
            *n = Instant::now() - Duration::from_secs(10);
        }
        {
            let mut t = ctrl.last_tier_change.lock().expect("tier lock");
            *t = Instant::now() - Duration::from_secs(10);
        }
        let changed = ctrl.tick_recover();
        assert!(changed.is_some());
        assert_eq!(ctrl.current_tier_level(), 4);
    }
}
