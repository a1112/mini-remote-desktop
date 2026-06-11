use super::dynamic_window_fps::DynamicWindowFpsDecision;
use super::LAN_RENDER_PACING_POLL_INTERVAL;
use mrd_ipc::MediaProfile;
use std::time::Duration;
use tokio::time::{sleep_until, Instant};
#[cfg(windows)]
use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};

const LAN_MEDIA_HIGH_RESOLUTION_TIMER_MIN_FPS: u32 = 90;
#[cfg(windows)]
const LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS: u32 = 1;
const LAN_MEDIA_PRECISE_SLEEP_MIN_FPS: u32 = 90;
const LAN_MEDIA_PRECISE_SLEEP_GUARD: Duration = Duration::from_millis(2);

pub(super) fn media_frame_interval(profile: &MediaProfile) -> Duration {
    media_frame_interval_for_fps(profile.fps)
}

pub(super) fn media_frame_interval_for_fps(fps: u32) -> Duration {
    Duration::from_micros((1_000_000 / u64::from(fps.max(1))).max(1))
}

pub(super) fn media_frame_interval_for_dynamic_decision(
    profile: &MediaProfile,
    decision: Option<DynamicWindowFpsDecision>,
) -> Duration {
    let fps = decision
        .map(|decision| decision.target_fps)
        .unwrap_or(profile.fps)
        .max(1);
    Duration::from_micros((1_000_000 / u64::from(fps)).max(1))
}

pub(super) fn media_profile_requests_high_resolution_timer(profile: &MediaProfile) -> bool {
    profile.fps >= LAN_MEDIA_HIGH_RESOLUTION_TIMER_MIN_FPS
}

pub(super) fn media_frame_precise_sleep_guard(profile: &MediaProfile) -> Duration {
    if profile.fps < LAN_MEDIA_PRECISE_SLEEP_MIN_FPS {
        return Duration::ZERO;
    }

    LAN_MEDIA_PRECISE_SLEEP_GUARD.min(media_frame_interval(profile) / 2)
}

pub(super) async fn sleep_until_media_frame(delay_until: Instant, profile: &MediaProfile) {
    let guard = media_frame_precise_sleep_guard(profile);
    if guard.is_zero() {
        sleep_until(delay_until).await;
        return;
    }

    loop {
        let now = Instant::now();
        if now >= delay_until {
            break;
        }
        let remaining = delay_until - now;
        if let Some(sleep_for) = media_frame_precise_sleep_chunk(remaining, guard) {
            std::thread::sleep(sleep_for);
        } else {
            std::hint::spin_loop();
        }
    }
}

pub(super) fn media_frame_precise_sleep_chunk(
    remaining: Duration,
    guard: Duration,
) -> Option<Duration> {
    if remaining <= guard {
        return None;
    }
    Some((remaining - guard).min(LAN_RENDER_PACING_POLL_INTERVAL))
}

#[derive(Default)]
pub(super) struct MediaTimerResolution {
    requested: bool,
    #[cfg(windows)]
    period: Option<WindowsMediaTimerPeriod>,
}

impl MediaTimerResolution {
    pub(super) fn update_for_profile(&mut self, profile: &MediaProfile) {
        if media_profile_requests_high_resolution_timer(profile) {
            self.request();
        } else {
            self.release();
        }
    }

    pub(super) fn request(&mut self) {
        if self.requested {
            return;
        }
        #[cfg(windows)]
        {
            match WindowsMediaTimerPeriod::begin(LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS) {
                Some(period) => {
                    self.period = Some(period);
                    self.requested = true;
                }
                None => {
                    tracing::debug!(
                        period_ms = LAN_MEDIA_HIGH_RESOLUTION_TIMER_PERIOD_MS,
                        "failed to request high resolution media timer"
                    );
                }
            }
        }
        #[cfg(not(windows))]
        {
            self.requested = true;
        }
    }

    pub(super) fn release(&mut self) {
        #[cfg(windows)]
        {
            self.period = None;
        }
        self.requested = false;
    }
}

#[cfg(windows)]
struct WindowsMediaTimerPeriod {
    period_ms: u32,
}

#[cfg(windows)]
impl WindowsMediaTimerPeriod {
    fn begin(period_ms: u32) -> Option<Self> {
        let result = unsafe { timeBeginPeriod(period_ms) };
        if result == 0 {
            Some(Self { period_ms })
        } else {
            None
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsMediaTimerPeriod {
    fn drop(&mut self) {
        unsafe {
            timeEndPeriod(self.period_ms);
        }
    }
}

pub(super) fn schedule_next_media_frame(
    now: Instant,
    next_frame_at: &mut Instant,
    frame_interval: Duration,
) -> Option<Instant> {
    if now >= *next_frame_at && now.duration_since(*next_frame_at) > frame_interval {
        *next_frame_at = now;
    }

    let delay_until = (*next_frame_at > now).then_some(*next_frame_at);
    *next_frame_at += frame_interval;
    delay_until
}
