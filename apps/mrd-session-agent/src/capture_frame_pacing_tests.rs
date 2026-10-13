use super::next_capture_frame_start;
use std::time::Duration;

#[test]
fn frame_pacing_small_wake_lateness_does_not_accumulate_phase_drift() {
    for fps in [30_u64, 60, 120] {
        let period = Duration::from_nanos(1_000_000_000 / fps);
        for work in [Duration::from_millis(1), Duration::from_millis(3)] {
            let mut scheduled = Duration::ZERO;
            let mut actual = Duration::ZERO;
            for frame in 1..=180 {
                let next = next_capture_frame_start(scheduled, actual, period).unwrap();
                assert_eq!(
                    next,
                    period * frame,
                    "phase drift at fps={fps}, frame={frame}"
                );
                let finished = actual + work;
                actual = next.max(finished) + Duration::from_millis(1);
                scheduled = next;
            }
        }
    }
}

#[test]
fn frame_pacing_preserves_half_period_spacing_at_the_boundary() {
    let period = Duration::from_millis(10);
    let actual = Duration::from_millis(5);
    let next = next_capture_frame_start(Duration::ZERO, actual, period).unwrap();
    assert_eq!(next, period);
    assert_eq!(next - actual, period / 2);

    let later = actual + Duration::from_nanos(1);
    assert_eq!(
        next_capture_frame_start(Duration::ZERO, later, period),
        Some(later + period),
        "compressed next interval must rebase rather than catch up"
    );
}

#[test]
fn frame_pacing_long_stall_schedules_one_future_frame_without_replaying_history() {
    let period = Duration::from_millis(10);
    let actual = Duration::from_millis(250);
    let next = next_capture_frame_start(Duration::from_millis(20), actual, period).unwrap();
    assert_eq!(next, actual + period);
    assert_eq!(
        next_capture_frame_start(next, next, period),
        Some(actual + period * 2)
    );
}

#[test]
fn frame_pacing_work_above_one_period_does_not_force_half_speed_or_extra_sleep() {
    let period = Duration::from_nanos(8_333_333);
    let work = Duration::from_micros(9_600);
    let mut scheduled = Duration::ZERO;
    let mut actual = Duration::ZERO;
    for _ in 0..64 {
        let next = next_capture_frame_start(scheduled, actual, period).unwrap();
        let finished = actual + work;
        assert_eq!(next.saturating_sub(finished), Duration::ZERO);
        scheduled = next;
        actual = finished;
    }
    assert_eq!(actual, work * 64);
    assert!(
        actual < period * 128,
        "overrun must not skip every other grid slot"
    );
}

#[test]
fn frame_pacing_rejects_zero_or_unrepresentable_half_period() {
    for period in [Duration::ZERO, Duration::from_nanos(1)] {
        assert_eq!(
            next_capture_frame_start(Duration::ZERO, Duration::ZERO, period),
            None
        );
    }
}

#[test]
fn frame_pacing_rejects_nominal_deadline_overflow() {
    assert_eq!(
        next_capture_frame_start(Duration::MAX, Duration::ZERO, Duration::from_millis(10)),
        None
    );
}

#[test]
fn frame_pacing_rejects_minimum_spacing_overflow() {
    assert_eq!(
        next_capture_frame_start(Duration::ZERO, Duration::MAX, Duration::from_millis(10)),
        None
    );
}

#[test]
fn frame_pacing_rejects_rebased_deadline_overflow() {
    let actual = Duration::MAX - Duration::from_millis(6);
    assert_eq!(
        next_capture_frame_start(Duration::ZERO, actual, Duration::from_millis(10)),
        None
    );
}
