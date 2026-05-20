use crate::app_state::AppState;
use anyhow::{Context, Result};
use mrd_ipc::{
    AdaptiveMediaConfig, CaptureSource, MediaAdaptationSnapshot, MediaPipelineSnapshot,
    MediaProfile, ProbeSnapshot,
};
use mrd_proto::SessionId;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ADAPTATION_SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
const ADAPTATION_DECISION_INTERVAL: Duration = Duration::from_secs(2);
const ADAPTATION_INITIAL_PROFILE_GRACE_MS: u64 = 5_000;
const ADAPTATION_SUBSEQUENT_DOWNSHIFT_COOLDOWN_MS: u64 = 5_000;
const ADAPTATION_SAFE_START_MIN_BITRATE_MBPS: u32 = 80;
const ADAPTATION_SAFE_START_MIN_FPS: u32 = 120;
const ADAPTATION_HIGH_REFRESH_STABILITY_BITRATE_MBPS: u32 = 64;
const ADAPTATION_DOWNSHIFT_CONFIRMATION_WINDOWS: u32 = 2;
const ADAPTATION_HIGH_REFRESH_FPS_ONLY_CONFIRMATION_WINDOWS: u32 = 4;
const DEFAULT_CEILING_WIDTH: u32 = 2560;
const DEFAULT_CEILING_HEIGHT: u32 = 1440;
const DEFAULT_CEILING_FPS: u32 = 144;
const DEFAULT_CEILING_BITRATE_MBPS: u32 = 80;
const DEFAULT_FLOOR_WIDTH: u32 = 1280;
const DEFAULT_FLOOR_HEIGHT: u32 = 720;
const DEFAULT_FLOOR_FPS: u32 = 60;
const DEFAULT_FLOOR_BITRATE_MBPS: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MediaAdaptationObservation {
    pub observed_fps: f32,
    pub target_fps: u32,
    pub drop_ratio: f32,
    pub queue_depth: u32,
    pub decode_p95_ms: Option<f64>,
    pub render_p95_ms: Option<f64>,
    pub receive_p95_ms: Option<f64>,
    pub present_gap_p95_ms: Option<f64>,
    pub no_valid_frames: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaAdaptationDecision {
    Hold,
    Downshift(String),
    Upshift(String),
}

#[derive(Debug, Clone, Copy)]
struct AdaptationCounters {
    frames_decoded: u64,
    frames_dropped: u64,
    timestamp: Instant,
}

/// Configure and start the LAN media keyframe ladder controller.
pub async fn configure_media_adaptation(
    app_state: &Arc<AppState>,
    session_id: SessionId,
    config: AdaptiveMediaConfig,
) -> Result<MediaAdaptationSnapshot> {
    ensure_session_exists(app_state, &session_id).await?;
    let mut current_profile = current_media_profile(app_state, &session_id, &config).await;
    let source = app_state
        .capture_sources
        .lock()
        .await
        .get(&session_id)
        .map(|selection| selection.source);
    let ladder = effective_ladder(&config, source.as_ref(), &current_profile);
    let mut ladder_index = initial_ladder_index_for_profile(
        &ladder,
        ladder_index_for_profile(&ladder, &current_profile),
    );
    let mut target_profile = ladder
        .get(ladder_index)
        .cloned()
        .unwrap_or_else(|| current_profile.clone());
    let mut initial_profile_applied = false;
    if config.enabled && current_profile != target_profile {
        let negotiation = crate::lan_discovery::request_lan_media_profile_update(
            app_state,
            &session_id,
            target_profile.clone(),
        )
        .await
        .context("failed to apply initial adaptive media profile")?;
        current_profile = negotiation.selected;
        ladder_index = ladder_index_for_profile(&ladder, &current_profile);
        target_profile = ladder
            .get(ladder_index)
            .cloned()
            .unwrap_or_else(|| current_profile.clone());
        initial_profile_applied = true;
    }
    let snapshot = MediaAdaptationSnapshot {
        enabled: config.enabled,
        state: if config.enabled {
            "configured".to_string()
        } else {
            "disabled".to_string()
        },
        ladder_index: ladder_index as u32,
        current_profile: current_profile.clone(),
        target_profile: target_profile.clone(),
        last_reason: Some(if config.enabled {
            if initial_profile_applied {
                if ladder_index > 0 {
                    "initial adaptive safe-start profile applied".to_string()
                } else {
                    "initial adaptive profile applied".to_string()
                }
            } else {
                "configured".to_string()
            }
        } else {
            "disabled".to_string()
        }),
        last_change_ms: epoch_ms(),
        observed_fps: 0.0,
        drop_ratio: 0.0,
        queue_depth: 0,
    };
    app_state
        .media_pipelines
        .lock()
        .await
        .set_adaptation(session_id.clone(), Some(snapshot.clone()));

    if config.enabled {
        let task_app_state = app_state.clone();
        let task_config = AdaptiveMediaConfig {
            ladder: ladder.clone(),
            ..config
        };
        let task_session_id = session_id.clone();
        let handle = tokio::spawn(async move {
            run_media_adaptation_task(
                task_app_state,
                task_session_id,
                task_config,
                ladder,
                ladder_index,
                initial_profile_applied,
            )
            .await;
        });
        let abort_handle = handle.abort_handle();
        drop(handle);
        app_state
            .media_tasks
            .lock()
            .await
            .register(session_id, abort_handle);
    }

    Ok(snapshot)
}

async fn ensure_session_exists(app_state: &Arc<AppState>, session_id: &SessionId) -> Result<()> {
    let sessions = app_state.sessions.lock().await;
    sessions
        .get(session_id)
        .with_context(|| format!("session not found: {}", session_id.0))?;
    Ok(())
}

async fn current_media_profile(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    config: &AdaptiveMediaConfig,
) -> MediaProfile {
    app_state
        .media_profiles
        .lock()
        .await
        .get(session_id)
        .map(|negotiation| negotiation.selected)
        .or_else(|| config.ceiling_profile.clone())
        .unwrap_or_else(default_ceiling_profile)
}

async fn run_media_adaptation_task(
    app_state: Arc<AppState>,
    session_id: SessionId,
    config: AdaptiveMediaConfig,
    ladder: Vec<MediaProfile>,
    mut ladder_index: usize,
    initial_profile_was_applied: bool,
) {
    if ladder.is_empty() {
        return;
    }

    let mut last_decision = Instant::now();
    let mut last_change = Instant::now();
    let mut stable_since: Option<Instant> = None;
    let mut pending_downshift_reason: Option<String> = None;
    let mut pending_downshift_windows = 0_u32;
    let mut initial_profile_checked = false;
    let mut initial_profile_grace_until: Option<Instant> = None;
    let mut counters = sample_counters(&app_state, &session_id).await;
    let mut last_valid_frame = counters.timestamp;

    loop {
        tokio::time::sleep(ADAPTATION_SAMPLE_INTERVAL).await;
        if !session_can_adapt(&app_state, &session_id).await {
            return;
        }
        if !session_receiver_active(&app_state, &session_id).await {
            counters = sample_counters(&app_state, &session_id).await;
            last_decision = Instant::now();
            last_valid_frame = last_decision;
            continue;
        }

        let now = Instant::now();
        if !initial_profile_checked {
            initial_profile_checked = true;
            if initial_profile_was_applied {
                counters = sample_counters(&app_state, &session_id).await;
                last_decision = now;
                last_valid_frame = now;
                initial_profile_grace_until =
                    now.checked_add(Duration::from_millis(ADAPTATION_INITIAL_PROFILE_GRACE_MS));
                continue;
            }
            let current_profile = current_media_profile(&app_state, &session_id, &config).await;
            let target_profile = ladder
                .get(ladder_index)
                .cloned()
                .unwrap_or_else(default_ceiling_profile);
            if current_profile != target_profile {
                let observation = MediaAdaptationObservation {
                    observed_fps: 0.0,
                    target_fps: target_profile.fps,
                    drop_ratio: 0.0,
                    queue_depth: 0,
                    decode_p95_ms: None,
                    render_p95_ms: None,
                    receive_p95_ms: None,
                    present_gap_p95_ms: None,
                    no_valid_frames: false,
                };
                if apply_adaptation_profile(
                    &app_state,
                    &session_id,
                    &ladder,
                    ladder_index,
                    observation,
                    "configured",
                    Some("initial adaptive profile applied".to_string()),
                )
                .await
                .is_ok()
                {
                    counters = sample_counters(&app_state, &session_id).await;
                    last_decision = now;
                    last_change = now;
                    last_valid_frame = now;
                    initial_profile_grace_until =
                        now.checked_add(Duration::from_millis(ADAPTATION_INITIAL_PROFILE_GRACE_MS));
                } else {
                    initial_profile_checked = false;
                    last_decision = now;
                }
                continue;
            }
        }

        if now.duration_since(last_decision) < ADAPTATION_DECISION_INTERVAL {
            continue;
        }

        let probe = app_state.probes.lock().await.snapshot(&session_id);
        let pipeline = app_state.media_pipelines.lock().await.snapshot(&session_id);
        let next_counters = counters_from_snapshots(&probe, &pipeline, now);
        if next_counters.frames_decoded > counters.frames_decoded {
            last_valid_frame = now;
        }

        let current_profile = ladder
            .get(ladder_index)
            .cloned()
            .unwrap_or_else(default_ceiling_profile);
        let observation = observation_from_snapshots(
            &probe,
            &pipeline,
            counters,
            next_counters,
            &current_profile,
            now.duration_since(last_valid_frame) >= Duration::from_secs(2),
        );
        if initial_profile_grace_until.is_some_and(|deadline| now < deadline) {
            stable_since = None;
            pending_downshift_reason = None;
            pending_downshift_windows = 0;
            let settling_reason = if is_initial_safe_start_ladder_index(&ladder, ladder_index) {
                "initial adaptive safe-start settling"
            } else {
                "initial adaptive profile settling"
            };
            update_adaptation_snapshot(
                &app_state,
                &session_id,
                &ladder,
                ladder_index,
                observation,
                "settling",
                Some(settling_reason.to_string()),
            )
            .await;
            counters = next_counters;
            last_decision = now;
            continue;
        }
        let stable_for_ms = stable_since
            .map(|started| now.duration_since(started).as_millis() as u64)
            .unwrap_or(0);
        let since_change_ms = now.duration_since(last_change).as_millis() as u64;
        let downshift_confirmed = update_downshift_confirmation(
            observation,
            &mut pending_downshift_reason,
            &mut pending_downshift_windows,
        );
        let decision = choose_adaptation_decision_with_confirmation(
            ladder_index,
            ladder.len(),
            observation,
            stable_for_ms,
            since_change_ms,
            &config,
            downshift_confirmed,
        );

        match decision {
            MediaAdaptationDecision::Downshift(reason) => {
                pending_downshift_reason = None;
                pending_downshift_windows = 0;
                let next_ladder_index = (ladder_index + 1).min(ladder.len().saturating_sub(1));
                stable_since = None;
                if apply_adaptation_profile(
                    &app_state,
                    &session_id,
                    &ladder,
                    next_ladder_index,
                    observation,
                    "downshift",
                    Some(reason),
                )
                .await
                .is_ok()
                {
                    ladder_index = next_ladder_index;
                    last_change = now;
                }
            }
            MediaAdaptationDecision::Upshift(reason) => {
                pending_downshift_reason = None;
                pending_downshift_windows = 0;
                let next_ladder_index = ladder_index.saturating_sub(1);
                stable_since = None;
                if apply_adaptation_profile(
                    &app_state,
                    &session_id,
                    &ladder,
                    next_ladder_index,
                    observation,
                    "upshift",
                    Some(reason),
                )
                .await
                .is_ok()
                {
                    ladder_index = next_ladder_index;
                    last_change = now;
                }
            }
            MediaAdaptationDecision::Hold => {
                if observation_is_healthy(observation) {
                    stable_since.get_or_insert(now);
                } else {
                    stable_since = None;
                }
                update_adaptation_snapshot(
                    &app_state,
                    &session_id,
                    &ladder,
                    ladder_index,
                    observation,
                    "stable",
                    None,
                )
                .await;
            }
        }

        counters = next_counters;
        last_decision = now;
    }
}

async fn session_can_adapt(app_state: &Arc<AppState>, session_id: &SessionId) -> bool {
    let sessions = app_state.sessions.lock().await;
    sessions
        .get(session_id)
        .map(|snapshot| !matches!(snapshot.lifecycle_state.as_str(), "closed" | "failed"))
        .unwrap_or(false)
}

async fn session_receiver_active(app_state: &Arc<AppState>, session_id: &SessionId) -> bool {
    let sessions = app_state.sessions.lock().await;
    sessions
        .get(session_id)
        .map(|snapshot| snapshot.receiver_active)
        .unwrap_or(false)
}

async fn sample_counters(app_state: &Arc<AppState>, session_id: &SessionId) -> AdaptationCounters {
    let probe = app_state.probes.lock().await.snapshot(session_id);
    let pipeline = app_state.media_pipelines.lock().await.snapshot(session_id);
    counters_from_snapshots(&probe, &pipeline, Instant::now())
}

fn counters_from_snapshots(
    probe: &ProbeSnapshot,
    pipeline: &MediaPipelineSnapshot,
    timestamp: Instant,
) -> AdaptationCounters {
    AdaptationCounters {
        frames_decoded: probe.frames_decoded,
        frames_dropped: probe.frames_dropped.max(pipeline.dropped_frames),
        timestamp,
    }
}

fn observation_from_snapshots(
    probe: &ProbeSnapshot,
    pipeline: &MediaPipelineSnapshot,
    previous: AdaptationCounters,
    current: AdaptationCounters,
    profile: &MediaProfile,
    no_valid_frames: bool,
) -> MediaAdaptationObservation {
    let elapsed_ms = current
        .timestamp
        .duration_since(previous.timestamp)
        .as_secs_f32()
        .max(0.001);
    let decoded_delta = current
        .frames_decoded
        .saturating_sub(previous.frames_decoded);
    let dropped_delta = current
        .frames_dropped
        .saturating_sub(previous.frames_dropped);
    let total_delta = decoded_delta.saturating_add(dropped_delta).max(1);
    MediaAdaptationObservation {
        observed_fps: decoded_delta as f32 / elapsed_ms,
        target_fps: probe.media_probe_target_fps.unwrap_or(profile.fps).max(1),
        drop_ratio: dropped_delta as f32 / total_delta as f32,
        queue_depth: pipeline.queue_depth,
        decode_p95_ms: stage_p95(pipeline, &["receiver.decode", "decode"]),
        render_p95_ms: stage_p95(pipeline, &["render_present", "present", "render_upload"]),
        receive_p95_ms: stage_p95(pipeline, &["receiver.read", "receiver.message_wait"]),
        present_gap_p95_ms: stage_p95(pipeline, &["render_present_gap", "render_enqueue_gap"]),
        no_valid_frames,
    }
}

#[cfg(test)]
pub(crate) fn choose_adaptation_decision(
    ladder_index: usize,
    ladder_len: usize,
    observation: MediaAdaptationObservation,
    stable_for_ms: u64,
    since_last_change_ms: u64,
    config: &AdaptiveMediaConfig,
) -> MediaAdaptationDecision {
    choose_adaptation_decision_with_confirmation(
        ladder_index,
        ladder_len,
        observation,
        stable_for_ms,
        since_last_change_ms,
        config,
        true,
    )
}

fn choose_adaptation_decision_with_confirmation(
    ladder_index: usize,
    ladder_len: usize,
    observation: MediaAdaptationObservation,
    stable_for_ms: u64,
    since_last_change_ms: u64,
    config: &AdaptiveMediaConfig,
    downshift_confirmed: bool,
) -> MediaAdaptationDecision {
    if ladder_len == 0 || !config.enabled {
        return MediaAdaptationDecision::Hold;
    }

    if let Some(reason) = downshift_reason(observation) {
        if !downshift_confirmed {
            return MediaAdaptationDecision::Hold;
        }
        let required_cooldown_ms = if ladder_index > 0 {
            config
                .downshift_cooldown_ms
                .max(ADAPTATION_SUBSEQUENT_DOWNSHIFT_COOLDOWN_MS)
        } else {
            config.downshift_cooldown_ms
        };
        if ladder_index + 1 < ladder_len && since_last_change_ms >= required_cooldown_ms {
            return MediaAdaptationDecision::Downshift(reason);
        }
        return MediaAdaptationDecision::Hold;
    }

    if ladder_index > 0
        && observation_is_healthy(observation)
        && stable_for_ms >= config.upshift_hold_ms
    {
        return MediaAdaptationDecision::Upshift("stable window reached".to_string());
    }

    MediaAdaptationDecision::Hold
}

fn update_downshift_confirmation(
    observation: MediaAdaptationObservation,
    pending_reason: &mut Option<String>,
    pending_windows: &mut u32,
) -> bool {
    let Some(reason) = downshift_reason(observation) else {
        *pending_reason = None;
        *pending_windows = 0;
        return true;
    };

    if !downshift_requires_confirmation(observation) {
        *pending_reason = None;
        *pending_windows = 0;
        return true;
    }

    let required_windows = downshift_confirmation_windows_required(observation);
    if pending_reason.as_deref() == Some(reason.as_str()) {
        *pending_windows = pending_windows.saturating_add(1);
    } else {
        *pending_reason = Some(reason);
        *pending_windows = 1;
    }

    *pending_windows >= required_windows
}

fn downshift_requires_confirmation(observation: MediaAdaptationObservation) -> bool {
    if observation.no_valid_frames {
        return false;
    }
    if observation.drop_ratio > 0.08 {
        return false;
    }
    let frame_budget_ms = 1000.0 / observation.target_fps.max(1) as f64;
    let severe_perceptual_budget_ms = frame_budget_ms * 2.0;
    if observation
        .present_gap_p95_ms
        .is_some_and(|p95| p95 > severe_perceptual_budget_ms)
    {
        return false;
    }
    if observation
        .receive_p95_ms
        .is_some_and(|p95| p95 > severe_perceptual_budget_ms)
    {
        return false;
    }
    true
}

fn downshift_confirmation_windows_required(observation: MediaAdaptationObservation) -> u32 {
    let fps_low = observation.observed_fps < observation.target_fps as f32 * 0.85;
    if !fps_low || observation.target_fps <= ADAPTATION_SAFE_START_MIN_FPS {
        return ADAPTATION_DOWNSHIFT_CONFIRMATION_WINDOWS;
    }

    let frame_budget_ms = 1000.0 / observation.target_fps.max(1) as f64;
    let perceptual_budget_ms = frame_budget_ms * 1.5;
    let has_supporting_stress = observation.drop_ratio > 0.005
        || observation.queue_depth > 1
        || observation
            .decode_p95_ms
            .is_some_and(|p95| p95 > frame_budget_ms)
        || observation
            .render_p95_ms
            .is_some_and(|p95| p95 > frame_budget_ms)
        || observation
            .present_gap_p95_ms
            .is_some_and(|p95| p95 > perceptual_budget_ms)
        || observation
            .receive_p95_ms
            .is_some_and(|p95| p95 > perceptual_budget_ms);

    if has_supporting_stress {
        ADAPTATION_DOWNSHIFT_CONFIRMATION_WINDOWS
    } else {
        ADAPTATION_HIGH_REFRESH_FPS_ONLY_CONFIRMATION_WINDOWS
    }
}

fn downshift_reason(observation: MediaAdaptationObservation) -> Option<String> {
    if observation.no_valid_frames {
        return Some("no valid frames for 2s".to_string());
    }
    let frame_budget_ms = 1000.0 / observation.target_fps.max(1) as f64;
    if observation.observed_fps < observation.target_fps as f32 * 0.85 {
        return Some(format!(
            "fps {:.1} below 85% of target {}",
            observation.observed_fps, observation.target_fps
        ));
    }
    if observation.drop_ratio > 0.03 {
        let severe_drop = observation.drop_ratio > 0.10;
        let throughput_stressed = observation.observed_fps < observation.target_fps as f32 * 0.95
            || observation.queue_depth > 0;
        let perceptual_budget_ms = frame_budget_ms * 1.5;
        let perceptual_stressed = observation.target_fps <= 120
            || observation
                .present_gap_p95_ms
                .is_some_and(|p95| p95 > perceptual_budget_ms)
            || observation
                .receive_p95_ms
                .is_some_and(|p95| p95 > perceptual_budget_ms);
        if severe_drop || throughput_stressed || perceptual_stressed {
            return Some(format!(
                "drop ratio {:.2}% above 3%",
                observation.drop_ratio * 100.0
            ));
        }
    }
    if observation.queue_depth > 1 {
        return Some(format!("queue depth {} above 1", observation.queue_depth));
    }
    if observation
        .decode_p95_ms
        .is_some_and(|p95| p95 > frame_budget_ms)
    {
        return Some(format!(
            "decode p95 exceeds {:.2}ms budget",
            frame_budget_ms
        ));
    }
    if observation
        .render_p95_ms
        .is_some_and(|p95| p95 > frame_budget_ms)
    {
        return Some(format!(
            "render p95 exceeds {:.2}ms budget",
            frame_budget_ms
        ));
    }
    if observation.target_fps > 120 {
        let perceptual_budget_ms = frame_budget_ms * 1.5;
        if observation
            .present_gap_p95_ms
            .is_some_and(|p95| p95 > perceptual_budget_ms)
        {
            return Some(format!(
                "present gap p95 exceeds {:.2}ms perceptual budget",
                perceptual_budget_ms
            ));
        }
        if observation
            .receive_p95_ms
            .is_some_and(|p95| p95 > perceptual_budget_ms)
        {
            return Some(format!(
                "receiver read p95 exceeds {:.2}ms perceptual budget",
                perceptual_budget_ms
            ));
        }
    }
    None
}

fn observation_is_healthy(observation: MediaAdaptationObservation) -> bool {
    if observation.no_valid_frames {
        return false;
    }
    if observation.observed_fps < observation.target_fps as f32 * 0.95 {
        return false;
    }
    if observation.drop_ratio > 0.005 {
        return false;
    }
    if observation.queue_depth > 1 {
        return false;
    }
    let frame_budget_ms = 1000.0 / observation.target_fps.max(1) as f64;
    if observation
        .decode_p95_ms
        .is_some_and(|p95| p95 > frame_budget_ms)
    {
        return false;
    }
    if observation
        .render_p95_ms
        .is_some_and(|p95| p95 > frame_budget_ms)
    {
        return false;
    }
    let perceptual_budget_ms = frame_budget_ms * 1.25;
    if observation
        .present_gap_p95_ms
        .is_some_and(|p95| p95 > perceptual_budget_ms)
    {
        return false;
    }
    if observation
        .receive_p95_ms
        .is_some_and(|p95| p95 > perceptual_budget_ms)
    {
        return false;
    }
    true
}

async fn apply_adaptation_profile(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    ladder: &[MediaProfile],
    ladder_index: usize,
    observation: MediaAdaptationObservation,
    state: &str,
    reason: Option<String>,
) -> Result<MediaProfile> {
    let Some(target_profile) = ladder.get(ladder_index).cloned() else {
        anyhow::bail!("adaptive media ladder index {ladder_index} is out of range");
    };

    match crate::lan_discovery::request_lan_media_profile_update(
        app_state,
        session_id,
        target_profile.clone(),
    )
    .await
    {
        Ok(negotiation) => {
            let selected_profile = negotiation.selected.clone();
            update_adaptation_snapshot_with_profiles(
                app_state,
                session_id,
                ladder_index,
                observation,
                state,
                reason,
                selected_profile.clone(),
                target_profile,
            )
            .await;
            Ok(selected_profile)
        }
        Err(error) => {
            let message = error.to_string();
            update_adaptation_snapshot(
                app_state,
                session_id,
                ladder,
                ladder_index,
                observation,
                "error",
                Some(message),
            )
            .await;
            Err(error)
        }
    }
}

async fn update_adaptation_snapshot(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    ladder: &[MediaProfile],
    ladder_index: usize,
    observation: MediaAdaptationObservation,
    state: &str,
    reason: Option<String>,
) {
    let profile = ladder
        .get(ladder_index)
        .cloned()
        .unwrap_or_else(default_ceiling_profile);
    update_adaptation_snapshot_with_profiles(
        app_state,
        session_id,
        ladder_index,
        observation,
        state,
        reason,
        profile.clone(),
        profile,
    )
    .await;
}

async fn update_adaptation_snapshot_with_profiles(
    app_state: &Arc<AppState>,
    session_id: &SessionId,
    ladder_index: usize,
    observation: MediaAdaptationObservation,
    state: &str,
    reason: Option<String>,
    current_profile: MediaProfile,
    target_profile: MediaProfile,
) {
    let mut pipelines = app_state.media_pipelines.lock().await;
    let previous = pipelines.adaptation(session_id);
    let profile_changed = previous
        .as_ref()
        .is_none_or(|snapshot| snapshot.current_profile != current_profile);
    let last_reason = reason.or_else(|| {
        previous
            .as_ref()
            .and_then(|snapshot| snapshot.last_reason.clone())
    });
    let last_change_ms = if profile_changed || state != "stable" {
        epoch_ms()
    } else {
        previous
            .as_ref()
            .map(|snapshot| snapshot.last_change_ms)
            .unwrap_or_else(epoch_ms)
    };
    let snapshot = MediaAdaptationSnapshot {
        enabled: true,
        state: state.to_string(),
        ladder_index: ladder_index as u32,
        current_profile,
        target_profile,
        last_reason,
        last_change_ms,
        observed_fps: observation.observed_fps,
        drop_ratio: observation.drop_ratio,
        queue_depth: observation.queue_depth,
    };
    pipelines.set_adaptation(session_id.clone(), Some(snapshot));
}

fn stage_p95(pipeline: &MediaPipelineSnapshot, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|name| {
        pipeline
            .stage_metrics
            .iter()
            .find(|metric| metric.stage == *name)
            .and_then(|metric| metric.p95_ms)
    })
}

pub(crate) fn effective_ladder(
    config: &AdaptiveMediaConfig,
    source: Option<&CaptureSource>,
    current_profile: &MediaProfile,
) -> Vec<MediaProfile> {
    effective_ladder_with_render_fps_cap(
        config,
        source,
        current_profile,
        crate::lan_discovery::lan_local_render_fps_cap(),
    )
}

fn effective_ladder_with_render_fps_cap(
    config: &AdaptiveMediaConfig,
    source: Option<&CaptureSource>,
    current_profile: &MediaProfile,
    render_fps_cap: Option<u32>,
) -> Vec<MediaProfile> {
    if !config.ladder.is_empty() {
        return sanitize_ladder(
            config
                .ladder
                .iter()
                .map(|profile| cap_profile_to_render_fps(profile, render_fps_cap))
                .collect(),
        );
    }

    let ceiling = config
        .ceiling_profile
        .clone()
        .unwrap_or_else(|| current_profile.clone());
    let ceiling = cap_profile_to_render_fps(&ceiling, render_fps_cap);
    let floor = config
        .floor_profile
        .clone()
        .unwrap_or_else(default_floor_profile);
    default_ladder_for_source(source, &ceiling, &floor)
}

fn cap_profile_to_render_fps(profile: &MediaProfile, render_fps_cap: Option<u32>) -> MediaProfile {
    let Some(render_fps_cap) = render_fps_cap.filter(|cap| *cap > 0) else {
        return profile.clone();
    };
    if profile.fps <= render_fps_cap {
        return profile.clone();
    }

    MediaProfile {
        fps: render_fps_cap,
        bitrate_mbps: cap_bitrate_for_render_fps(profile.bitrate_mbps, profile.fps, render_fps_cap),
        ..profile.clone()
    }
}

fn cap_bitrate_for_render_fps(bitrate_mbps: u32, source_fps: u32, render_fps_cap: u32) -> u32 {
    let bitrate_mbps = bitrate_mbps.max(1);
    let source_fps = source_fps.max(1);
    let render_fps_cap = render_fps_cap.max(1).min(source_fps);
    let fps_scaled =
        ((bitrate_mbps as f64) * (render_fps_cap as f64 / source_fps as f64)).round() as u32;
    let stability_scaled = ((bitrate_mbps as f64) * 0.8).round() as u32;
    fps_scaled.min(stability_scaled).max(1)
}

pub(crate) fn default_ladder_for_source(
    source: Option<&CaptureSource>,
    ceiling: &MediaProfile,
    floor: &MediaProfile,
) -> Vec<MediaProfile> {
    let (source_width, source_height) = source
        .map(|source| (source.width.max(2), source.height.max(2)))
        .unwrap_or((ceiling.width.max(2), ceiling.height.max(2)));
    let aspect = source_width as f64 / source_height.max(1) as f64;
    let high_width = ceiling.width.min(source_width).max(2);
    let high = even_size_for_width(high_width, aspect, source_width, source_height);
    let mid = even_size_for_width(1920.min(high.0).max(2), aspect, source_width, source_height);
    let low = even_size_for_width(
        floor.width.min(1280).min(high.0).max(2),
        aspect,
        source_width,
        source_height,
    );
    let high_fps = ceiling.fps.max(1);
    let high_bitrate = ceiling.bitrate_mbps.max(1);
    let second_bitrate = ((high_bitrate as f32) * 0.8).round() as u32;

    let mut ladder = vec![profile(high, high_fps, high_bitrate, ceiling)];
    if let Some(stability_bitrate) =
        high_refresh_stability_bitrate(high, high_fps, high_bitrate, second_bitrate)
    {
        ladder.push(profile(high, high_fps, stability_bitrate, ceiling));
    } else {
        ladder.push(profile(high, high_fps, second_bitrate.max(1), ceiling));
    }
    ladder.extend([
        profile(
            high,
            high_fps.min(120),
            50.min(high_bitrate).max(1),
            ceiling,
        ),
        profile(
            high,
            high_fps.min(120),
            40.min(high_bitrate).max(1),
            ceiling,
        ),
        profile(mid, high_fps.min(120), 40.min(high_bitrate).max(1), ceiling),
        profile(mid, high_fps.min(90), 28.min(high_bitrate).max(1), ceiling),
        profile(mid, high_fps.min(60), 20.min(high_bitrate).max(1), ceiling),
        profile(
            low,
            floor.fps.min(high_fps).max(1),
            floor.bitrate_mbps.max(1),
            ceiling,
        ),
    ]);
    sanitize_ladder(ladder)
}

fn high_refresh_stability_bitrate(
    high: (u32, u32),
    high_fps: u32,
    high_bitrate: u32,
    second_bitrate: u32,
) -> Option<u32> {
    let pixels = high.0 as u64 * high.1 as u64;
    let baseline_pixels = DEFAULT_CEILING_WIDTH as u64 * DEFAULT_CEILING_HEIGHT as u64;
    if high_fps >= ADAPTATION_SAFE_START_MIN_FPS
        && pixels > baseline_pixels
        && high_bitrate > ADAPTATION_HIGH_REFRESH_STABILITY_BITRATE_MBPS
        && second_bitrate > ADAPTATION_HIGH_REFRESH_STABILITY_BITRATE_MBPS
    {
        Some(ADAPTATION_HIGH_REFRESH_STABILITY_BITRATE_MBPS)
    } else {
        None
    }
}

fn sanitize_ladder(ladder: Vec<MediaProfile>) -> Vec<MediaProfile> {
    let mut sanitized = Vec::new();
    for mut profile in ladder {
        profile.width = profile.width.max(2) & !1;
        profile.height = profile.height.max(2) & !1;
        profile.fps = profile.fps.max(1);
        profile.bitrate_mbps = profile.bitrate_mbps.max(1);
        if profile.codec.trim().is_empty() {
            profile.codec = "h264".to_string();
        }
        if sanitized.last() != Some(&profile) {
            sanitized.push(profile);
        }
    }
    sanitized
}

fn even_size_for_width(
    width: u32,
    aspect: f64,
    source_width: u32,
    source_height: u32,
) -> (u32, u32) {
    let even_width = width.min(source_width).max(2) & !1;
    let mut height = ((even_width as f64 / aspect).round() as u32).min(source_height);
    height = height.max(2) & !1;
    (even_width, height)
}

fn profile(size: (u32, u32), fps: u32, bitrate_mbps: u32, template: &MediaProfile) -> MediaProfile {
    let mut profile = template.clone();
    profile.width = size.0;
    profile.height = size.1;
    profile.fps = fps;
    profile.bitrate_mbps = bitrate_mbps;
    profile
}

fn ladder_index_for_profile(ladder: &[MediaProfile], profile: &MediaProfile) -> usize {
    ladder
        .iter()
        .position(|entry| {
            entry.width == profile.width
                && entry.height == profile.height
                && entry.fps == profile.fps
                && entry.bitrate_mbps == profile.bitrate_mbps
        })
        .unwrap_or(0)
}

fn initial_ladder_index_for_profile(ladder: &[MediaProfile], current_index: usize) -> usize {
    if current_index != 0 || ladder.len() < 2 {
        return current_index;
    }

    let top = &ladder[0];
    let safe_index = initial_safe_start_candidate_index(ladder);
    let safe = &ladder[safe_index];
    let safe_keeps_shape = safe.width == top.width
        && safe.height == top.height
        && safe.fps == top.fps
        && safe.codec == top.codec
        && safe.codec_profile == top.codec_profile
        && safe.bit_depth == top.bit_depth
        && safe.chroma_subsampling == top.chroma_subsampling
        && safe.pixel_format == top.pixel_format
        && safe.hdr_enabled == top.hdr_enabled
        && safe.bitrate_mbps < top.bitrate_mbps;
    if safe_keeps_shape
        && top.fps >= ADAPTATION_SAFE_START_MIN_FPS
        && top.bitrate_mbps >= ADAPTATION_SAFE_START_MIN_BITRATE_MBPS
    {
        safe_index
    } else {
        current_index
    }
}

fn initial_safe_start_candidate_index(ladder: &[MediaProfile]) -> usize {
    let top = &ladder[0];
    let pixels = top.width as u64 * top.height as u64;
    let baseline_pixels = DEFAULT_CEILING_WIDTH as u64 * DEFAULT_CEILING_HEIGHT as u64;
    if pixels <= baseline_pixels {
        return 1;
    }

    ladder
        .iter()
        .position(|profile| {
            profile.width == top.width
                && profile.height == top.height
                && profile.fps == top.fps
                && profile.bitrate_mbps == ADAPTATION_HIGH_REFRESH_STABILITY_BITRATE_MBPS
                && profile.bitrate_mbps < top.bitrate_mbps
        })
        .unwrap_or(1)
}

fn is_initial_safe_start_ladder_index(ladder: &[MediaProfile], ladder_index: usize) -> bool {
    ladder_index > 0 && initial_ladder_index_for_profile(ladder, 0) == ladder_index
}

fn default_ceiling_profile() -> MediaProfile {
    MediaProfile {
        width: DEFAULT_CEILING_WIDTH,
        height: DEFAULT_CEILING_HEIGHT,
        fps: DEFAULT_CEILING_FPS,
        bitrate_mbps: DEFAULT_CEILING_BITRATE_MBPS,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    }
}

fn default_floor_profile() -> MediaProfile {
    MediaProfile {
        width: DEFAULT_FLOOR_WIDTH,
        height: DEFAULT_FLOOR_HEIGHT,
        fps: DEFAULT_FLOOR_FPS,
        bitrate_mbps: DEFAULT_FLOOR_BITRATE_MBPS,
        codec: "h264".to_string(),
        ..MediaProfile::default()
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AdaptiveMediaConfig {
        AdaptiveMediaConfig {
            enabled: true,
            mode: "keyframe_ladder".to_string(),
            ceiling_profile: Some(MediaProfile {
                width: 2560,
                height: 1440,
                fps: 144,
                bitrate_mbps: 80,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }),
            floor_profile: Some(MediaProfile {
                width: 1280,
                height: 720,
                fps: 60,
                bitrate_mbps: 10,
                codec: "h264".to_string(),
                ..MediaProfile::default()
            }),
            ladder: Vec::new(),
            downshift_cooldown_ms: 2_000,
            upshift_hold_ms: 5_000,
        }
    }

    fn source(width: u32, height: u32) -> CaptureSource {
        CaptureSource {
            id: "display:0".to_string(),
            platform: "windows".to_string(),
            source_kind: "display_shared".to_string(),
            title: "Display".to_string(),
            class_name: String::new(),
            width,
            height,
            process_id: 0,
            app_name: None,
            bundle_identifier: None,
            preview_data_url: None,
            preview_width: None,
            preview_height: None,
        }
    }

    #[test]
    fn default_ladder_uses_2k144_80mbps_baseline() {
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1440)),
            &config().ceiling_profile.unwrap(),
            &config().floor_profile.unwrap(),
        );

        assert_eq!(ladder[0].width, 2560);
        assert_eq!(ladder[0].height, 1440);
        assert_eq!(ladder[0].fps, 144);
        assert_eq!(ladder[0].bitrate_mbps, 80);
        assert_eq!(ladder[1].bitrate_mbps, 64);
        assert_eq!(ladder.last().unwrap().width, 1280);
        assert_eq!(ladder.last().unwrap().height, 720);
    }

    #[test]
    fn default_ladder_preserves_16_to_10_source_aspect() {
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1600)),
            &config().ceiling_profile.unwrap(),
            &config().floor_profile.unwrap(),
        );

        assert_eq!((ladder[0].width, ladder[0].height), (2560, 1600));
        assert_eq!((ladder[3].width, ladder[3].height), (2560, 1600));
        assert_eq!(ladder[3].bitrate_mbps, 40);
        assert_eq!((ladder[4].width, ladder[4].height), (1920, 1200));
        assert_eq!(
            (ladder.last().unwrap().width, ladder.last().unwrap().height),
            (1280, 800)
        );
    }

    #[test]
    fn default_ladder_preserves_ceiling_codec_and_sampling() {
        let ceiling = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
        };
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1600)),
            &ceiling,
            &config().floor_profile.unwrap(),
        );

        for profile in ladder {
            assert_eq!(profile.codec, "hevc");
            assert_eq!(profile.codec_profile.as_deref(), Some("main"));
            assert_eq!(profile.bit_depth, Some(8));
            assert_eq!(profile.chroma_subsampling.as_deref(), Some("4:2:0"));
            assert_eq!(profile.pixel_format.as_deref(), Some("nv12"));
            assert_eq!(profile.hdr_enabled, Some(false));
        }
    }

    #[test]
    fn default_ladder_can_cap_ceiling_to_local_render_fps() {
        let ceiling = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
        };
        let capped = cap_profile_to_render_fps(&ceiling, Some(144));
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1600)),
            &capped,
            &config().floor_profile.unwrap(),
        );

        assert_eq!(ladder[0].fps, 144);
        assert_eq!(ladder[0].bitrate_mbps, 96);
        assert_eq!(ladder[0].codec, "hevc");
        assert_eq!(ladder[0].codec_profile.as_deref(), Some("main"));
        assert_eq!(ladder[0].bit_depth, Some(8));
        assert_eq!(ladder[0].chroma_subsampling.as_deref(), Some("4:2:0"));
        assert_eq!(ladder[0].pixel_format.as_deref(), Some("nv12"));
        assert_eq!(ladder[0].hdr_enabled, Some(false));
    }

    #[test]
    fn high_bitrate_adaptive_profile_safe_starts_on_stability_rung() {
        let ceiling = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
        };
        let capped = cap_profile_to_render_fps(&ceiling, Some(144));
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1600)),
            &capped,
            &config().floor_profile.unwrap(),
        );

        assert_eq!(ladder[0].bitrate_mbps, 96);
        assert_eq!(ladder[1].fps, 144);
        assert_eq!(ladder[1].bitrate_mbps, 64);
        assert_eq!(ladder[2].fps, 120);
        assert_eq!(ladder[2].bitrate_mbps, 50);
        assert_eq!(
            initial_ladder_index_for_profile(
                &ladder,
                ladder_index_for_profile(&ladder, &ladder[0])
            ),
            1
        );
        assert!(is_initial_safe_start_ladder_index(&ladder, 1));
    }

    #[test]
    fn standard_2k144_adaptive_profile_safe_starts_on_second_rung() {
        let ladder = default_ladder_for_source(
            Some(&source(2560, 1440)),
            &config().ceiling_profile.unwrap(),
            &config().floor_profile.unwrap(),
        );

        assert_eq!(ladder[0].bitrate_mbps, 80);
        assert_eq!(ladder[1].bitrate_mbps, 64);
        assert_eq!(
            initial_ladder_index_for_profile(
                &ladder,
                ladder_index_for_profile(&ladder, &ladder[0])
            ),
            1
        );
        assert!(is_initial_safe_start_ladder_index(&ladder, 1));
    }

    #[test]
    fn low_bitrate_adaptive_profile_starts_at_ceiling() {
        let ceiling = MediaProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_mbps: 20,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
        };
        let ladder = default_ladder_for_source(
            Some(&source(1920, 1080)),
            &ceiling,
            &config().floor_profile.unwrap(),
        );

        assert_eq!(ladder[0].bitrate_mbps, 20);
        assert_eq!(ladder[1].bitrate_mbps, 16);
        assert_eq!(
            initial_ladder_index_for_profile(
                &ladder,
                ladder_index_for_profile(&ladder, &ladder[0])
            ),
            0
        );
        assert!(!is_initial_safe_start_ladder_index(&ladder, 0));
        assert!(!is_initial_safe_start_ladder_index(&ladder, 1));
    }

    #[test]
    fn effective_ladder_caps_explicit_ladder_to_local_render_fps() {
        let ceiling = MediaProfile {
            width: 2560,
            height: 1600,
            fps: 165,
            bitrate_mbps: 120,
            codec: "hevc".to_string(),
            codec_profile: Some("main".to_string()),
            bit_depth: Some(8),
            chroma_subsampling: Some("4:2:0".to_string()),
            pixel_format: Some("nv12".to_string()),
            hdr_enabled: Some(false),
        };
        let config = AdaptiveMediaConfig {
            ladder: vec![
                ceiling.clone(),
                MediaProfile {
                    fps: 120,
                    bitrate_mbps: 50,
                    ..ceiling.clone()
                },
            ],
            ..config()
        };

        let ladder = effective_ladder_with_render_fps_cap(
            &config,
            Some(&source(2560, 1600)),
            &ceiling,
            Some(144),
        );

        assert_eq!(ladder[0].fps, 144);
        assert_eq!(ladder[0].bitrate_mbps, 96);
        assert_eq!(ladder[0].codec, "hevc");
        assert_eq!(ladder[1].fps, 120);
    }

    #[test]
    fn poor_health_downshifts_after_cooldown() {
        let observation = MediaAdaptationObservation {
            observed_fps: 90.0,
            target_fps: 144,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(2.0),
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(0, 7, observation, 0, 2_000, &config()),
            MediaAdaptationDecision::Downshift("fps 90.0 below 85% of target 144".to_string())
        );
    }

    #[test]
    fn stable_health_upshifts_after_hold_window() {
        let observation = MediaAdaptationObservation {
            observed_fps: 140.0,
            target_fps: 144,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(2.0),
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(2, 7, observation, 5_000, 5_000, &config()),
            MediaAdaptationDecision::Upshift("stable window reached".to_string())
        );
    }

    #[test]
    fn cooldown_prevents_rapid_downshift() {
        let observation = MediaAdaptationObservation {
            observed_fps: 90.0,
            target_fps: 144,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(2.0),
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(0, 7, observation, 0, 500, &config()),
            MediaAdaptationDecision::Hold
        );
    }

    #[test]
    fn subsequent_downshift_uses_longer_reconfigure_grace() {
        let observation = MediaAdaptationObservation {
            observed_fps: 90.0,
            target_fps: 120,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(2.0),
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(2, 7, observation, 0, 2_000, &config()),
            MediaAdaptationDecision::Hold
        );
        assert_eq!(
            choose_adaptation_decision(2, 7, observation, 0, 5_000, &config()),
            MediaAdaptationDecision::Downshift("fps 90.0 below 85% of target 120".to_string())
        );
    }

    #[test]
    fn perceptual_jitter_downshifts_after_cooldown() {
        let observation = MediaAdaptationObservation {
            observed_fps: 144.0,
            target_fps: 144,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(12.0),
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(0, 7, observation, 0, 2_000, &config()),
            MediaAdaptationDecision::Downshift(
                "present gap p95 exceeds 10.42ms perceptual budget".to_string()
            )
        );
    }

    #[test]
    fn high_fps_drop_burst_without_qoe_stress_holds() {
        let observation = MediaAdaptationObservation {
            observed_fps: 164.0,
            target_fps: 165,
            drop_ratio: 0.0695,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(8.5),
            no_valid_frames: false,
        };

        assert_eq!(downshift_reason(observation), None);
    }

    #[test]
    fn high_fps_drop_burst_with_perceptual_stress_downshifts() {
        let observation = MediaAdaptationObservation {
            observed_fps: 164.0,
            target_fps: 165,
            drop_ratio: 0.0695,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(10.0),
            no_valid_frames: false,
        };

        assert_eq!(
            downshift_reason(observation),
            Some("drop ratio 6.95% above 3%".to_string())
        );
    }

    #[test]
    fn transient_drop_reason_requires_two_windows_in_task_path() {
        let observation = MediaAdaptationObservation {
            observed_fps: 150.0,
            target_fps: 165,
            drop_ratio: 0.06,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(1.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        let first_confirmed =
            update_downshift_confirmation(observation, &mut pending_reason, &mut pending_windows);
        assert!(!first_confirmed);
        assert_eq!(pending_windows, 1);
        assert_eq!(
            choose_adaptation_decision_with_confirmation(
                0,
                7,
                observation,
                0,
                2_000,
                &config(),
                first_confirmed,
            ),
            MediaAdaptationDecision::Hold
        );

        let second_confirmed =
            update_downshift_confirmation(observation, &mut pending_reason, &mut pending_windows);
        assert!(second_confirmed);
        assert_eq!(pending_windows, 2);
        assert_eq!(
            choose_adaptation_decision_with_confirmation(
                0,
                7,
                observation,
                0,
                2_000,
                &config(),
                second_confirmed,
            ),
            MediaAdaptationDecision::Downshift("drop ratio 6.00% above 3%".to_string())
        );
    }

    #[test]
    fn severe_drop_burst_downshifts_without_confirmation() {
        let observation = MediaAdaptationObservation {
            observed_fps: 130.0,
            target_fps: 144,
            drop_ratio: 0.09,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(27.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        let confirmed =
            update_downshift_confirmation(observation, &mut pending_reason, &mut pending_windows);

        assert!(confirmed);
        assert_eq!(
            choose_adaptation_decision_with_confirmation(
                0,
                7,
                observation,
                0,
                2_000,
                &config(),
                confirmed,
            ),
            MediaAdaptationDecision::Downshift("drop ratio 9.00% above 3%".to_string())
        );
    }

    #[test]
    fn transient_low_fps_downshift_waits_for_confirmation() {
        let observation = MediaAdaptationObservation {
            observed_fps: 120.0,
            target_fps: 165,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(1.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 1);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 2);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 3);
        assert!(update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 4);
    }

    #[test]
    fn severe_fps_without_drop_or_perceptual_stress_waits_for_confirmation() {
        let observation = MediaAdaptationObservation {
            observed_fps: 70.0,
            target_fps: 165,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(1.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 1);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 2);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 3);
        assert!(update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 4);
    }

    #[test]
    fn low_fps_with_single_paced_queue_frame_waits_for_fps_only_confirmation() {
        let observation = MediaAdaptationObservation {
            observed_fps: 70.0,
            target_fps: 165,
            drop_ratio: 0.0,
            queue_depth: 1,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(1.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 1);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 2);
        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 3);
        assert!(update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 4);
    }

    #[test]
    fn low_fps_with_real_queue_backlog_confirms_after_two_windows() {
        let observation = MediaAdaptationObservation {
            observed_fps: 70.0,
            target_fps: 165,
            drop_ratio: 0.0,
            queue_depth: 2,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(1.0),
            present_gap_p95_ms: Some(1.0),
            no_valid_frames: false,
        };
        let mut pending_reason = None;
        let mut pending_windows = 0;

        assert!(!update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 1);
        assert!(update_downshift_confirmation(
            observation,
            &mut pending_reason,
            &mut pending_windows
        ));
        assert_eq!(pending_windows, 2);
    }

    #[test]
    fn stable_health_requires_perceptual_jitter_budget() {
        let observation = MediaAdaptationObservation {
            observed_fps: 144.0,
            target_fps: 144,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(9.0),
            present_gap_p95_ms: Some(2.0),
            no_valid_frames: false,
        };

        assert!(!observation_is_healthy(observation));
    }

    #[test]
    fn stable_health_allows_single_paced_render_queue_frame() {
        let observation = MediaAdaptationObservation {
            observed_fps: 120.0,
            target_fps: 120,
            drop_ratio: 0.004,
            queue_depth: 1,
            decode_p95_ms: Some(1.9),
            render_p95_ms: Some(0.4),
            receive_p95_ms: Some(0.1),
            present_gap_p95_ms: Some(8.7),
            no_valid_frames: false,
        };

        assert!(observation_is_healthy(observation));
    }

    #[test]
    fn perceptual_jitter_downshift_stops_at_120fps_guard() {
        let observation = MediaAdaptationObservation {
            observed_fps: 120.0,
            target_fps: 120,
            drop_ratio: 0.0,
            queue_depth: 0,
            decode_p95_ms: Some(2.0),
            render_p95_ms: Some(2.0),
            receive_p95_ms: Some(2.0),
            present_gap_p95_ms: Some(20.0),
            no_valid_frames: false,
        };

        assert_eq!(downshift_reason(observation), None);
    }
}
