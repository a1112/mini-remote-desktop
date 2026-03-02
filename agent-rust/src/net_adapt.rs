use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub struct NetAdaptController {
    min_fps: u32,
    max_fps: u32,
    current_fps: AtomicU32,
    min_bitrate_kbps: u32,
    max_bitrate_kbps: u32,
    current_bitrate_kbps: AtomicU32,
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
        initial_bitrate_kbps: u32,
    ) -> Self {
        let min_fps = min_fps.max(1);
        let max_fps = max_fps.max(min_fps);
        let min_bitrate_kbps = min_bitrate_kbps.max(100);
        let max_bitrate_kbps = max_bitrate_kbps.max(min_bitrate_kbps);
        let now = Instant::now();
        Self {
            min_fps,
            max_fps,
            current_fps: AtomicU32::new(initial_fps.clamp(min_fps, max_fps)),
            min_bitrate_kbps,
            max_bitrate_kbps,
            current_bitrate_kbps: AtomicU32::new(
                initial_bitrate_kbps.clamp(min_bitrate_kbps, max_bitrate_kbps),
            ),
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

    pub fn on_nack_burst(&self) -> (u32, u32) {
        let now = Instant::now();
        if let Ok(mut t) = self.last_nack.lock() {
            *t = now;
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
        self.current_bitrate_kbps.store(target_br, Ordering::Relaxed);
        (next_fps, target_br)
    }

    pub fn tick_recover(&self) -> Option<(u32, u32)> {
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
}
