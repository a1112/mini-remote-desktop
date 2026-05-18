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
    let current_profile = current_media_profile(app_state, &session_id, &config).await;
    let source = app_state
        .capture_sources
        .lock()
        .await
        .get(&session_id)
        .map(|selection| selection.source);
    let ladder = effective_ladder(&config, source.as_ref(), &current_profile);
    let ladder_index = ladder_index_for_profile(&ladder, &current_profile);
    let target_profile = ladder
        .get(ladder_index)
        .cloned()
        .unwrap_or_else(|| current_profile.clone());
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
            "configured".to_string()
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
) {
    if ladder.is_empty() {
        return;
    }

    let mut last_decision = Instant::now();
    let mut last_change = Instant::now();
    let mut stable_since: Option<Instant> = None;
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
        let stable_for_ms = stable_since
            .map(|started| now.duration_since(started).as_millis() as u64)
            .unwrap_or(0);
        let since_change_ms = now.duration_since(last_change).as_millis() as u64;
        let decision = choose_adaptation_decision(
            ladder_index,
            ladder.len(),
            observation,
            stable_for_ms,
            since_change_ms,
            &config,
        );

        match decision {
            MediaAdaptationDecision::Downshift(reason) => {
                ladder_index = (ladder_index + 1).min(ladder.len().saturating_sub(1));
                stable_since = None;
                last_change = now;
                apply_adaptation_profile(
                    &app_state,
                    &session_id,
                    &ladder,
                    ladder_index,
                    observation,
                    "downshift",
                    Some(reason),
                )
                .await;
            }
            MediaAdaptationDecision::Upshift(reason) => {
                ladder_index = ladder_index.saturating_sub(1);
                stable_since = None;
                last_change = now;
                apply_adaptation_profile(
                    &app_state,
                    &session_id,
                    &ladder,
                    ladder_index,
                    observation,
                    "upshift",
                    Some(reason),
                )
                .await;
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
        no_valid_frames,
    }
}

pub(crate) fn choose_adaptation_decision(
    ladder_index: usize,
    ladder_len: usize,
    observation: MediaAdaptationObservation,
    stable_for_ms: u64,
    since_last_change_ms: u64,
    config: &AdaptiveMediaConfig,
) -> MediaAdaptationDecision {
    if ladder_len == 0 || !config.enabled {
        return MediaAdaptationDecision::Hold;
    }

    if let Some(reason) = downshift_reason(observation) {
        if ladder_index + 1 < ladder_len && since_last_change_ms >= config.downshift_cooldown_ms {
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

fn downshift_reason(observation: MediaAdaptationObservation) -> Option<String> {
    if observation.no_valid_frames {
        return Some("no valid frames for 2s".to_string());
    }
    if observation.observed_fps < observation.target_fps as f32 * 0.85 {
        return Some(format!(
            "fps {:.1} below 85% of target {}",
            observation.observed_fps, observation.target_fps
        ));
    }
    if observation.drop_ratio > 0.03 {
        return Some(format!(
            "drop ratio {:.2}% above 3%",
            observation.drop_ratio * 100.0
        ));
    }
    if observation.queue_depth > 1 {
        return Some(format!("queue depth {} above 1", observation.queue_depth));
    }
    let frame_budget_ms = 1000.0 / observation.target_fps.max(1) as f64;
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
    if observation.queue_depth != 0 {
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
) {
    let Some(target_profile) = ladder.get(ladder_index).cloned() else {
        return;
    };

    match crate::lan_discovery::request_lan_media_profile_update(
        app_state,
        session_id,
        target_profile.clone(),
    )
    .await
    {
        Ok(negotiation) => {
            update_adaptation_snapshot_with_profiles(
                app_state,
                session_id,
                ladder_index,
                observation,
                state,
                reason,
                negotiation.selected,
                target_profile,
            )
            .await;
        }
        Err(error) => {
            update_adaptation_snapshot(
                app_state,
                session_id,
                ladder,
                ladder_index,
                observation,
                "error",
                Some(error.to_string()),
            )
            .await;
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
    let snapshot = MediaAdaptationSnapshot {
        enabled: true,
        state: state.to_string(),
        ladder_index: ladder_index as u32,
        current_profile,
        target_profile,
        last_reason: reason,
        last_change_ms: epoch_ms(),
        observed_fps: observation.observed_fps,
        drop_ratio: observation.drop_ratio,
        queue_depth: observation.queue_depth,
    };
    app_state
        .media_pipelines
        .lock()
        .await
        .set_adaptation(session_id.clone(), Some(snapshot));
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
    if !config.ladder.is_empty() {
        return sanitize_ladder(config.ladder.clone());
    }

    let ceiling = config
        .ceiling_profile
        .clone()
        .unwrap_or_else(|| current_profile.clone());
    let floor = config
        .floor_profile
        .clone()
        .unwrap_or_else(default_floor_profile);
    default_ladder_for_source(source, &ceiling, &floor)
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

    sanitize_ladder(vec![
        profile(high, high_fps, high_bitrate),
        profile(high, high_fps, second_bitrate.max(1)),
        profile(high, high_fps.min(120), 50.min(high_bitrate).max(1)),
        profile(mid, high_fps.min(120), 40.min(high_bitrate).max(1)),
        profile(mid, high_fps.min(90), 28.min(high_bitrate).max(1)),
        profile(mid, high_fps.min(60), 20.min(high_bitrate).max(1)),
        profile(
            low,
            floor.fps.min(high_fps).max(1),
            floor.bitrate_mbps.max(1),
        ),
    ])
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

fn profile(size: (u32, u32), fps: u32, bitrate_mbps: u32) -> MediaProfile {
    MediaProfile {
        width: size.0,
        height: size.1,
        fps,
        bitrate_mbps,
        codec: "h264".to_string(),
    }
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

fn default_ceiling_profile() -> MediaProfile {
    MediaProfile {
        width: DEFAULT_CEILING_WIDTH,
        height: DEFAULT_CEILING_HEIGHT,
        fps: DEFAULT_CEILING_FPS,
        bitrate_mbps: DEFAULT_CEILING_BITRATE_MBPS,
        codec: "h264".to_string(),
    }
}

fn default_floor_profile() -> MediaProfile {
    MediaProfile {
        width: DEFAULT_FLOOR_WIDTH,
        height: DEFAULT_FLOOR_HEIGHT,
        fps: DEFAULT_FLOOR_FPS,
        bitrate_mbps: DEFAULT_FLOOR_BITRATE_MBPS,
        codec: "h264".to_string(),
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
            }),
            floor_profile: Some(MediaProfile {
                width: 1280,
                height: 720,
                fps: 60,
                bitrate_mbps: 10,
                codec: "h264".to_string(),
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
        assert_eq!((ladder[3].width, ladder[3].height), (1920, 1200));
        assert_eq!(
            (ladder.last().unwrap().width, ladder.last().unwrap().height),
            (1280, 800)
        );
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
            no_valid_frames: false,
        };

        assert_eq!(
            choose_adaptation_decision(0, 7, observation, 0, 500, &config()),
            MediaAdaptationDecision::Hold
        );
    }
}
