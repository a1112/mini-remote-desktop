//! Local IPC protocol between Rdesk and `mrd-service`.
//!
//! This crate owns the stable request/response DTOs for local communication
//! and deliberately stays independent of Tauri types so the UI remains a thin
//! shell over the service-owned runtime.

#![warn(missing_docs)]

/// Client-side helpers for connecting to the local service IPC endpoint.
pub mod client;
/// Transport abstractions used by IPC client/server implementations.
pub mod transport;

use mrd_proto::{DeviceId, SessionId};
use serde::{Deserialize, Serialize};

// The current wire DTOs predate the service-kernel split and intentionally keep
// their serde field names stable. New architecture docs track the module split;
// this scoped allow keeps the existing wire shape quiet while the DTOs are
// migrated into domain modules without a noisy mechanical field-doc commit.
#[allow(missing_docs)]
mod wire {
    use super::*;

    // === Shell / Lifecycle DTOs (Phase 2) ===
    // Defined first to avoid forward references

    /// Reason for opening the UI
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum OpenUiReason {
        /// User clicked tray menu
        TrayOpen,
        /// Incoming session request
        SessionIncoming,
        /// User action (e.g., from diagnostics)
        UserRequest,
        /// Opening diagnostics/debugging view
        Diagnostics,
    }

    /// Result of UI open operation
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum UiOpenStatus {
        /// Focused existing UI window
        FocusedExisting,
        /// Spawned new UI process
        SpawnedNew,
        /// UI unavailable (e.g., not configured)
        Unavailable,
    }

    /// Reason for UI detachment
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum UiDetachReason {
        /// User closed UI window normally
        UserClose,
        /// User explicitly quit UI
        UserQuit,
        /// UI crashed
        Crash,
        /// Connection lost
        ConnectionLost,
    }

    /// Service shutdown mode
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ShutdownMode {
        /// Graceful shutdown - finish active sessions if possible
        Graceful,
        /// Force shutdown - terminate immediately
        Force,
        /// Shutdown after sessions end
        AfterSessions,
    }

    /// Shell/service status snapshot
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ShellStatusSnapshot {
        pub service_pid: u32,
        pub ui_pid: Option<u32>,
        pub tray_available: bool,
        pub autostart_enabled: Option<bool>,
        pub active_session_count: usize,
        pub last_error: Option<String>,
    }

    /// Requested or selected media stream profile.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct MediaProfile {
        pub width: u32,
        pub height: u32,
        pub fps: u32,
        pub bitrate_mbps: u32,
        pub codec: String,
        /// Codec profile name, for example `main` or `main10`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub codec_profile: Option<String>,
        /// Video bit depth. HEVC Main uses 8, HEVC Main10 uses 10.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub bit_depth: Option<u8>,
        /// Chroma subsampling label such as `4:2:0`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chroma_subsampling: Option<String>,
        /// Runtime pixel format associated with this profile, for example `nv12`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pixel_format: Option<String>,
        /// Whether HDR is expected for this media profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub hdr_enabled: Option<bool>,
    }

    impl Default for MediaProfile {
        fn default() -> Self {
            Self {
                width: 0,
                height: 0,
                fps: 0,
                bitrate_mbps: 0,
                codec: "h264".to_string(),
                codec_profile: None,
                bit_depth: None,
                chroma_subsampling: None,
                pixel_format: None,
                hdr_enabled: None,
            }
        }
    }

    /// Result of media profile negotiation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct MediaProfileNegotiation {
        pub requested: MediaProfile,
        pub selected: MediaProfile,
        pub status: String,
        pub reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub selected_source_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub selected_width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub selected_height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub downgrade_reason: Option<String>,
    }

    /// A native render surface attached to a media pipeline.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AttachedRenderSurface {
        pub surface_id: String,
        pub backend: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub window_handle: Option<i64>,
    }

    /// Aggregated latency metrics for one media pipeline stage.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct MediaStageMetrics {
        pub stage: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub p50_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub p95_ms: Option<f64>,
    }

    /// Synthetic transport impairment settings and counters for test runs.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct MediaTestImpairmentSnapshot {
        pub loss_pct: f64,
        pub base_delay_ms: u64,
        pub jitter_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub mtu_bytes: Option<u32>,
        pub seed: u64,
        pub datagrams_sent: u64,
        pub datagrams_dropped: u64,
        pub datagrams_delayed: u64,
        pub datagrams_fragmented_by_mtu: u64,
    }

    /// Sender-side LAN media transport counters.
    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
    pub struct MediaSenderTransportSnapshot {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub capture_source_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub capture_source_kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub capture_memory_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub dynamic_fps_tier: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub target_fps: Option<u32>,
        pub datagram_fragments_attempted: u64,
        pub datagram_fragments_sent: u64,
        pub datagram_fragments_delayed: u64,
        pub datagram_fragments_dropped_by_impairment: u64,
        pub datagram_fragments_dropped_for_capacity: u64,
        pub datagram_fragments_dropped_for_budget: u64,
        pub datagram_frames_cut_short_for_capacity: u64,
        pub datagram_frames_cut_short_for_budget: u64,
        pub reliable_fragments_sent: u64,
        pub reliable_frames_sent: u64,
    }

    fn default_adaptation_mode() -> String {
        "keyframe_ladder".to_string()
    }

    fn default_downshift_cooldown_ms() -> u64 {
        2_000
    }

    fn default_upshift_hold_ms() -> u64 {
        5_000
    }

    /// Runtime configuration for LAN media bitrate/FPS/resolution adaptation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AdaptiveMediaConfig {
        pub enabled: bool,
        #[serde(default = "default_adaptation_mode")]
        pub mode: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub ceiling_profile: Option<MediaProfile>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub floor_profile: Option<MediaProfile>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub ladder: Vec<MediaProfile>,
        #[serde(default)]
        pub dynamic_resolution_enabled: bool,
        #[serde(default = "default_downshift_cooldown_ms")]
        pub downshift_cooldown_ms: u64,
        #[serde(default = "default_upshift_hold_ms")]
        pub upshift_hold_ms: u64,
    }

    /// Current adaptive LAN media controller state.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct MediaAdaptationSnapshot {
        pub enabled: bool,
        pub state: String,
        pub ladder_index: u32,
        pub current_profile: MediaProfile,
        pub target_profile: MediaProfile,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_reason: Option<String>,
        pub last_change_ms: u64,
        pub observed_fps: f32,
        pub drop_ratio: f32,
        pub queue_depth: u32,
    }

    /// Runtime state for a session media pipeline.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct MediaPipelineSnapshot {
        pub session_id: SessionId,
        pub attached_surfaces: Vec<AttachedRenderSurface>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_decoder: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_renderer: Option<String>,
        /// Codec currently flowing through the receiver pipeline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_codec: Option<String>,
        /// Active codec profile, for example `main`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_codec_profile: Option<String>,
        /// Active profile bit depth, for example `8` for NV12 or `10` for P010.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_bit_depth: Option<u8>,
        /// Active chroma subsampling label, for example `4:2:0`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_chroma_subsampling: Option<String>,
        /// Active decoded pixel format, for example `d3d11_shared_nv12`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_pixel_format: Option<String>,
        /// Whether HDR metadata is enabled for the active profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_hdr_enabled: Option<bool>,
        /// Active negotiated width in pixels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_width: Option<u32>,
        /// Active negotiated height in pixels.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_height: Option<u32>,
        /// Active negotiated frame rate.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_fps: Option<u32>,
        /// Active negotiated bitrate in Mbps.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active_bitrate_mbps: Option<u32>,
        /// Last reason the runtime fell back from a requested codec.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub codec_fallback_reason: Option<String>,
        pub queue_depth: u32,
        /// Legacy aggregate of receiver-side render drops. Prefer the explicit
        /// render counters below for diagnostics.
        pub dropped_frames: u64,
        /// Frames actually presented by the native render pipeline.
        #[serde(default)]
        pub render_presented_frames: u64,
        #[serde(default)]
        pub render_queue_replacements: u64,
        /// Stale queued render frames dropped when the render worker catches up to latest.
        #[serde(default)]
        pub render_stale_frame_drops: u64,
        #[serde(default)]
        pub render_lock_drops: u64,
        /// Frames accepted by the renderer but skipped by non-blocking present.
        #[serde(default)]
        pub render_present_skips: u64,
        /// Receiver-side render pacing target after local display refresh caps.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub render_pacing_target_fps: Option<u32>,
        /// Receiver-side render queue policy, for example `latest` or `paced_fifo`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub render_queue_policy: Option<String>,
        pub stage_metrics: Vec<MediaStageMetrics>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub test_impairment: Option<MediaTestImpairmentSnapshot>,
        #[serde(default)]
        pub sender_transport: MediaSenderTransportSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub adaptation: Option<MediaAdaptationSnapshot>,
    }

    /// A capture source that can be selected for a remote session.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CaptureSource {
        pub id: String,
        pub platform: String,
        pub source_kind: String,
        pub title: String,
        pub class_name: String,
        pub width: u32,
        pub height: u32,
        pub process_id: u32,
        pub app_name: Option<String>,
        pub bundle_identifier: Option<String>,
        pub preview_data_url: Option<String>,
        pub preview_width: Option<u32>,
        pub preview_height: Option<u32>,
    }

    /// Result of selecting a capture source on the remote peer.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CaptureSourceSelection {
        pub session_id: SessionId,
        pub source: CaptureSource,
        pub status: String,
        pub reason: Option<String>,
    }

    /// A display output mode that can be applied to a remote capture display.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct DisplayMode {
        /// Stable mode identifier, usually platform/source/resolution/refresh.
        pub id: String,
        /// Optional capture source id this mode belongs to.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub source_id: Option<String>,
        /// Pixel width.
        pub width: u32,
        /// Pixel height.
        pub height: u32,
        /// Refresh rate rounded to Hz.
        pub refresh_hz: u32,
        /// Color depth when the platform exposes it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub bit_depth: Option<u32>,
        /// Whether this mode is currently active.
        pub is_current: bool,
    }

    /// Result of a display mode change or restore operation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct DisplayModeChange {
        /// Session associated with the temporary display mode request.
        pub session_id: SessionId,
        /// Requested mode, absent for restore-only responses.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub requested: Option<DisplayMode>,
        /// Mode observed before the change, used for restore.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub previous: Option<DisplayMode>,
        /// Active mode after the operation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active: Option<DisplayMode>,
        /// Machine-readable status such as changed, restored, unsupported, or failed.
        pub status: String,
        /// Human-readable reason when status is not a clean change.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub reason: Option<String>,
        /// Whether the original mode should be restored when the session ends.
        pub restore_required: bool,
    }

    /// Platform identifier used by structured capability snapshots.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum CapabilityPlatform {
        /// Microsoft Windows desktop.
        Windows,
        /// Apple macOS desktop.
        Macos,
        /// Linux desktop.
        Linux,
        /// Android client/host.
        Android,
        /// iOS client.
        Ios,
        /// Browser/web runtime.
        Web,
        /// Unknown or unsupported platform.
        Unknown,
    }

    /// Product capability domain.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum CapabilityDomain {
        /// Screen/window capture.
        Capture,
        /// Selectable capture sources.
        CaptureSource,
        /// Video encoding.
        Encode,
        /// Video decoding.
        Decode,
        /// Frame rendering.
        Render,
        /// Frame memory/interoperability path.
        Memory,
        /// Media or control transport.
        Transport,
        /// Keyboard/mouse/control-plane input.
        Control,
        /// Audio capture/playback/media path.
        Audio,
        /// Local service lifecycle features.
        Service,
        /// Pairing, consent, and encryption features.
        Security,
    }

    /// Runtime support state for one capability.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum CapabilityStatus {
        /// Product code exists but runtime validation has not proven usability.
        Supported,
        /// Runtime probe found required APIs, drivers, or permissions.
        Available,
        /// Lightweight validation succeeded.
        Usable,
        /// Usable fallback path below preferred parity.
        Degraded,
        /// Blocked by an OS permission.
        PermissionMissing,
        /// Driver/runtime library is missing.
        DriverMissing,
        /// Required hardware is absent.
        HardwareMissing,
        /// Matrix concept exists but no runner is wired.
        Unimplemented,
        /// Unsupported on this platform or product mode.
        Unsupported,
        /// Not yet probed or not recognized.
        Unknown,
    }

    /// Structured capability item shared by service, UI, and LAN discovery.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CapabilityItem {
        /// Stable capability id, for example `capture.dxgi`.
        pub id: String,
        /// Product domain for grouping and evaluation.
        pub domain: CapabilityDomain,
        /// Human-readable short label.
        pub label: String,
        /// Current support state.
        pub status: CapabilityStatus,
        /// Platform that produced the capability.
        pub platform: CapabilityPlatform,
        /// Short reason when the status is not plainly available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub reason: Option<String>,
        /// Optional diagnostic detail.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub detail: Option<String>,
        /// Capability ids required by this item.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub requires: Vec<String>,
        /// Capability ids that conflict with this item.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub conflicts_with: Vec<String>,
        /// Capability ids this item depends on.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub depends_on: Vec<String>,
        /// Lower-parity fallback capability ids.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub fallback_ids: Vec<String>,
        /// Last probe timestamp in milliseconds since Unix epoch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_probe_time_ms: Option<u64>,
    }

    /// Compatibility status for a cross-capability constraint.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum CapabilityConstraintStatus {
        /// Combination is allowed.
        Allow,
        /// Combination must not run.
        Block,
        /// Combination runs below preferred parity.
        Degrade,
        /// Combination needs a copy/conversion step.
        RequiresCopy,
        /// Combination requires runtime probe validation.
        RequiresProbe,
    }

    /// Rule describing whether multiple capabilities can be combined.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct CapabilityConstraint {
        /// Stable constraint id.
        pub id: String,
        /// Capability ids or prefixes this rule applies to.
        pub applies_to: Vec<String>,
        /// Constraint result.
        pub status: CapabilityConstraintStatus,
        /// Deterministic explanation for UI and automation.
        pub reason: String,
        /// Fallback capability ids when applicable.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub fallback_ids: Vec<String>,
    }

    /// Named performance profile used by static and runtime validation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct CapabilityProfile {
        /// Stable profile id.
        pub id: String,
        /// Target frame width.
        pub width: u32,
        /// Target frame height.
        pub height: u32,
        /// Target frame rate.
        pub fps: u32,
        /// Target bitrate in Mbps.
        pub bitrate_mbps: u32,
        /// Requested codec, for example `h264`.
        pub codec: String,
        /// Optional latency budget in milliseconds.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latency_budget_ms: Option<u32>,
        /// Optional minimum stable FPS ratio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub min_stable_fps_ratio: Option<f32>,
        /// Optional maximum frame drop ratio.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_drop_ratio: Option<f32>,
        /// Capabilities required for static support.
        pub required_capabilities: Vec<String>,
    }

    /// Structured local or peer capability snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct CapabilitySnapshot {
        /// Schema version for forward-compatible readers.
        pub schema_version: u32,
        /// Platform that produced the snapshot.
        pub platform: CapabilityPlatform,
        /// Service or application version that produced the snapshot.
        pub service_version: String,
        /// Capability items.
        pub capabilities: Vec<CapabilityItem>,
        /// Cross-capability constraints.
        pub constraints: Vec<CapabilityConstraint>,
        /// Built-in performance profiles known by the producer.
        pub profiles: Vec<CapabilityProfile>,
        /// Snapshot timestamp in milliseconds since Unix epoch.
        pub updated_at_ms: u64,
    }

    /// Readiness state for a product scenario/profile evaluation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ScenarioEvaluationStatus {
        /// All required capabilities are available for the requested scenario.
        Ready,
        /// Scenario can run, but not at preferred parity.
        Degraded,
        /// Scenario must not run until the blocker is addressed.
        Blocked,
        /// Scenario is intentionally excluded, usually for peer/version mismatch.
        Skipped,
    }

    /// One deterministic reason emitted by scenario/profile evaluation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ScenarioEvaluationReason {
        /// Stable reason code, for example `capability.missing`.
        pub code: String,
        /// Severity label such as `info`, `warning`, or `error`.
        pub severity: String,
        /// Human-readable explanation for UI and reports.
        pub message: String,
        /// Related capability id when the reason is tied to a capability.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub capability_id: Option<String>,
    }

    /// Result of evaluating whether a scenario/profile should run.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct ScenarioEvaluation {
        /// Stable scenario/profile id such as `lan.2k144`.
        pub scenario_id: String,
        /// Overall readiness result.
        pub status: ScenarioEvaluationStatus,
        /// Profile selected after capability/profile evaluation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub selected_profile: Option<MediaProfile>,
        /// Transport selected by the policy layer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub transport_kind: Option<String>,
        /// Ordered explanation list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub reasons: Vec<ScenarioEvaluationReason>,
        /// Required capability ids for the evaluated scenario.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub required_capabilities: Vec<String>,
        /// Required capability ids not available on the evaluated side.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub missing_capabilities: Vec<String>,
        /// Optional fallback profile when the preferred profile is degraded/blocked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub fallback_profile: Option<MediaProfile>,
    }

    /// Runtime transport policy requested by the UI shell or automation.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TransportPolicyConfig {
        /// Policy mode, usually `auto`, `lan`, or `wan`.
        pub mode: String,
        /// Optional preferred transport, for example `quic` or `webrtc`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub preferred_transport: Option<String>,
        /// Whether LAN QUIC can be selected.
        pub allow_lan_quic: bool,
        /// Whether WebRTC can be selected.
        pub allow_webrtc: bool,
        /// Whether relay/TURN paths can be selected.
        pub allow_relay: bool,
    }

    /// Service-owned transport policy decision snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TransportPolicySnapshot {
        /// Optional session id associated with the decision.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub session_id: Option<SessionId>,
        /// Policy mode applied for this decision.
        pub mode: String,
        /// Transport selected by policy.
        pub selected_transport: String,
        /// Candidate transports considered by policy.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub candidate_transports: Vec<String>,
        /// Whether relay/TURN is required by the selected route.
        pub relay_required: bool,
        /// Human-readable reason for the selected route.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub reason: Option<String>,
        /// Fallback reason when preferred transport was not selected.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub fallback_reason: Option<String>,
    }

    /// Control channel reliability class.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ControlChannelReliability {
        /// Reliable ordered lane for non-droppable control events.
        ReliableOrdered,
        /// Low-latency lane for coalescible/droppable realtime events.
        UnreliableRealtime,
    }

    /// Runtime counters for one control channel lane.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ControlChannelLaneSnapshot {
        /// Stable lane name, for example `ctrl_rel` or `ctrl_rt`.
        pub name: String,
        /// Reliability semantics for this lane.
        pub reliability: ControlChannelReliability,
        /// Whether messages are ordered.
        pub ordered: bool,
        /// Maximum retransmits, absent for fully reliable lanes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_retransmits: Option<u16>,
        /// Messages currently queued.
        pub queued_messages: u64,
        /// Messages dropped by lane policy.
        pub dropped_messages: u64,
        /// Messages coalesced by lane policy.
        pub coalesced_messages: u64,
        /// Messages accepted by the service for this lane.
        #[serde(default)]
        pub accepted_messages: u64,
        /// Messages injected successfully on the controlled side.
        #[serde(default)]
        pub injected_messages: u64,
        /// Messages that failed injection or validation.
        #[serde(default)]
        pub failed_messages: u64,
        /// Last lane-specific error.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_error: Option<String>,
    }

    /// Runtime control channel state for a session.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ControlChannelSnapshot {
        /// Session id associated with these lanes.
        pub session_id: SessionId,
        /// Reliable control lane.
        pub reliable: ControlChannelLaneSnapshot,
        /// Realtime control lane.
        pub realtime: ControlChannelLaneSnapshot,
    }

    /// Mouse button carried by a service-owned control input request.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ControlInputButton {
        /// Primary mouse button.
        Left,
        /// Secondary mouse button.
        Right,
        /// Middle mouse button.
        Middle,
        /// First extended mouse button.
        X1,
        /// Second extended mouse button.
        X2,
    }

    /// Keyboard key carried by a service-owned control input request.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum ControlInputKey {
        /// Windows virtual-key code.
        VirtualKey {
            /// Numeric virtual-key code.
            code: u16,
        },
    }

    /// Normalized control input event sent to mrd-service.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum ControlInputEvent {
        /// Absolute mouse position in remote frame coordinates.
        MouseMove {
            /// X coordinate.
            x: i32,
            /// Y coordinate.
            y: i32,
        },
        /// Mouse button transition.
        MouseButton {
            /// Button id.
            button: ControlInputButton,
            /// Whether the button is pressed.
            pressed: bool,
        },
        /// Mouse wheel delta.
        MouseWheel {
            /// Wheel delta.
            delta: i32,
        },
        /// Keyboard key transition.
        Key {
            /// Key id.
            key: ControlInputKey,
            /// Whether the key is pressed.
            pressed: bool,
        },
        /// Release all pressed keys and mouse buttons tracked by the service.
        ReleaseAll,
    }

    /// Control lane selected for an accepted input event.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    pub enum ControlInputLane {
        /// Reliable ordered control lane.
        Reliable,
        /// Realtime low-latency control lane.
        Realtime,
        /// Local cleanup path, for example release-all.
        Cleanup,
    }

    /// One paired device identity known by the local service.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct PairedDeviceIdentity {
        /// Paired peer device id.
        pub device_id: DeviceId,
        /// Peer display name at pairing time.
        pub display_name: String,
        /// Pinned peer certificate or key fingerprint.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub certificate_fingerprint: Option<String>,
        /// Trust status such as `pending`, `paired`, or `revoked`.
        pub trust_status: String,
        /// Last observation timestamp in milliseconds since Unix epoch.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_seen_ms: Option<u64>,
    }

    /// Local service device identity and pairing state.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct DeviceIdentitySnapshot {
        /// Registered local device id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub local_device_id: Option<DeviceId>,
        /// Registered local display name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub display_name: Option<String>,
        /// Local certificate or key fingerprint when provisioned.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub certificate_fingerprint: Option<String>,
        /// Whether user consent is required before starting remote control.
        pub consent_required: bool,
        /// Known paired devices.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub paired_devices: Vec<PairedDeviceIdentity>,
    }

    /// Summary statistics for a telemetry metric series.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct TelemetryMetricSummary {
        /// Metric name.
        pub name: String,
        /// Display unit.
        pub unit: String,
        /// Number of samples available.
        pub sample_count: u64,
        /// Median value when available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub p50: Option<f64>,
        /// 95th percentile value when available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub p95: Option<f64>,
    }

    /// File artifact associated with a telemetry run.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct TelemetryArtifactRef {
        /// Artifact display name.
        pub name: String,
        /// Local or exported artifact path.
        pub path: String,
        /// Artifact kind such as `json`, `markdown`, or `log`.
        pub kind: String,
    }

    /// Compact service-facing telemetry bundle for run/session diagnostics.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct TelemetryBundle {
        /// Test or session run id.
        pub run_id: String,
        /// Optional session id linked to the run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub session_id: Option<SessionId>,
        /// Metric summaries available for the run.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub metrics: Vec<TelemetryMetricSummary>,
        /// Number of structured events available.
        pub event_count: u64,
        /// Number of log entries available.
        pub log_count: u64,
        /// Linked report/log artifacts.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub artifacts: Vec<TelemetryArtifactRef>,
    }

    /// Query used to retrieve service-owned audit events.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
    pub struct AuditLogQuery {
        /// Optional session id filter.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub session_id: Option<SessionId>,
        /// Optional action filter, for example `session.start`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub action: Option<String>,
        /// Optional maximum number of newest matching events to return.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub limit: Option<u32>,
    }

    /// Service-owned audit event for security, control, and operations review.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct AuditEvent {
        /// Monotonic event id within one service process.
        pub id: u64,
        /// Event time in milliseconds since Unix epoch.
        pub timestamp_ms: u64,
        /// Stable action id, for example `session.start`.
        pub action: String,
        /// Machine-readable outcome, usually `success` or `error`.
        pub outcome: String,
        /// Optional related session id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub session_id: Option<SessionId>,
        /// Optional local actor device id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub actor_device_id: Option<DeviceId>,
        /// Optional peer device id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub peer_device_id: Option<DeviceId>,
        /// Optional transport kind, for example `quic`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub transport_kind: Option<String>,
        /// Optional reason or error detail.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub reason: Option<String>,
        /// Deterministic key/value details for UI and export.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub details: Vec<(String, String)>,
    }

    // === Core IPC Types ===

    /// IPC request from Rdesk to mrd-service
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    #[serde(tag = "type")]
    pub enum IpcRequest {
        /// Register local device with the service
        RegisterDevice {
            device_id: DeviceId,
            device_name: String,
        },
        /// List available devices
        ListDevices,
        /// Get the current LAN peer discovery snapshot.
        LanDiscoverySnapshot,
        /// Send an immediate LAN discovery probe and return the current snapshot.
        RefreshLanDiscovery,
        /// List all active sessions
        ListSessions,
        /// Start a new session as controller
        StartSession {
            session_id: SessionId,
            target_device_id: DeviceId,
            transport_kind: String, // "quic" or "webrtc"
        },
        /// Start a LAN P2P session as controller and ask the discovered peer to accept it.
        StartLanRemoteSession {
            session_id: SessionId,
            target_device_id: DeviceId,
            transport_kind: String, // "quic" or "webrtc"
            #[serde(default, skip_serializing_if = "Option::is_none")]
            requested_profile: Option<MediaProfile>,
        },
        /// Request a runtime media profile switch for an existing session.
        UpdateMediaProfile {
            session_id: SessionId,
            requested_profile: MediaProfile,
        },
        /// Configure LAN media bitrate/FPS/resolution adaptation for an existing session.
        ConfigureMediaAdaptation {
            session_id: SessionId,
            config: AdaptiveMediaConfig,
        },
        /// List selectable capture sources on the local service host.
        ListLocalCaptureSources {
            include_previews: bool,
            limit: Option<u32>,
        },
        /// List selectable capture sources from the remote peer for a session.
        ListRemoteCaptureSources {
            session_id: SessionId,
            include_previews: bool,
            limit: Option<u32>,
        },
        /// Select one remote capture source for a session.
        SelectRemoteCaptureSource {
            session_id: SessionId,
            source_id: String,
        },
        /// List display modes from the remote peer for a session.
        ListRemoteDisplayModes { session_id: SessionId },
        /// Temporarily set a remote display mode.
        SetRemoteDisplayMode {
            session_id: SessionId,
            mode: DisplayMode,
            restore_after_session: bool,
        },
        /// Restore the display mode saved for a session.
        RestoreRemoteDisplayMode { session_id: SessionId },
        /// Attach a native render surface to a session media pipeline.
        AttachRenderSurface {
            session_id: SessionId,
            surface_id: String,
            backend: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            window_handle: Option<i64>,
        },
        /// Detach a native render surface from a session media pipeline.
        DetachRenderSurface {
            session_id: SessionId,
            surface_id: String,
        },
        /// Accept an incoming session as agent
        AcceptSession {
            session_id: SessionId,
            source_device_id: DeviceId,
        },
        /// Start sending media (controller role)
        StartSender { session_id: SessionId },
        /// Start receiving media (agent role)
        StartReceiver { session_id: SessionId },
        /// Stop a session
        StopSession { session_id: SessionId },
        /// Mark a session as failed and retain its failure reason.
        FailSession {
            session_id: SessionId,
            reason: String,
        },
        /// Recover a failed or closed session back to its role-appropriate startup state.
        RecoverSession { session_id: SessionId },
        /// Get current session runtime snapshot
        SessionRuntimeSnapshot { session_id: SessionId },
        /// Get aggregated runtime snapshot
        RuntimeSnapshot,
        /// Query service-owned audit events.
        AuditLog { query: AuditLogQuery },
        /// Get structured local capability snapshot.
        CapabilitySnapshot,
        /// Evaluate a scenario/profile before starting a session.
        EvaluateScenarioProfile {
            /// Stable scenario/profile id.
            scenario_id: String,
            /// Optional peer device id to include in evaluation.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            peer_device_id: Option<DeviceId>,
            /// Optional concrete requested profile override.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            requested_profile: Option<MediaProfile>,
        },
        /// Get structured capability snapshot for a discovered peer.
        GetPeerCapabilitySnapshot {
            /// Peer device id.
            peer_device_id: DeviceId,
        },
        /// Set transport policy for a session.
        SetTransportPolicy {
            /// Target session id.
            session_id: SessionId,
            /// Requested policy.
            policy: TransportPolicyConfig,
        },
        /// Get control channel lane state for a session.
        GetControlChannelSnapshot {
            /// Target session id.
            session_id: SessionId,
        },
        /// Send a keyboard or mouse event to the service-owned control path.
        SendControlInput {
            /// Target session id.
            session_id: SessionId,
            /// Normalized input event.
            event: ControlInputEvent,
        },
        /// Start or refresh local pairing intent for a device.
        PairDevice {
            /// Peer device id.
            device_id: DeviceId,
            /// Optional presented peer fingerprint.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            certificate_fingerprint: Option<String>,
        },
        /// Approve a pending pairing.
        ApprovePairing {
            /// Peer device id.
            device_id: DeviceId,
        },
        /// Revoke a paired device.
        RevokeDevice {
            /// Peer device id.
            device_id: DeviceId,
        },
        /// Get local device identity and pairing snapshot.
        GetDeviceIdentitySnapshot,
        /// Get compact telemetry bundle for a run/session.
        GetTelemetryBundle {
            /// Test or session run id.
            run_id: String,
            /// Optional linked session id.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            session_id: Option<SessionId>,
        },
        /// Get probe snapshot data
        ProbeSnapshot { session_id: SessionId },
        /// Get media pipeline snapshot data.
        MediaPipelineSnapshot { session_id: SessionId },
        /// Stream probe events
        StreamProbeEvents,
        /// Health check for service
        ServiceHealth,

        // === Shell / Lifecycle Commands (Phase 2) ===
        /// Request to open/focus the UI
        OpenUi { reason: OpenUiReason },
        /// Request to focus an existing UI window
        FocusUi,
        /// Notify service that UI has attached
        UiAttached {
            pid: u32,
            executable_path: Option<String>,
        },
        /// Notify service that UI is detaching
        UiDetached { pid: u32, reason: UiDetachReason },
        /// Get current shell/service status
        GetShellStatus,
        /// Set autostart enabled state
        SetAutostart { enabled: bool },
        /// Get autostart status
        GetAutostartStatus,
        /// Request service shutdown
        ShutdownService { mode: ShutdownMode },
    }

    /// IPC response from mrd-service to Rdesk
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[allow(clippy::large_enum_variant)]
    #[serde(tag = "type")]
    pub enum IpcResponse {
        /// Device registration successful
        DeviceRegistered { device_id: DeviceId },
        /// List of available devices
        DeviceList { devices: Vec<DeviceInfo> },
        /// LAN peer discovery snapshot.
        LanDiscoverySnapshot {
            /// Current discovery state.
            snapshot: LanDiscoverySnapshot,
        },
        /// List of active sessions
        SessionList { sessions: Vec<SessionInfo> },
        /// Session started successfully
        SessionStarted { session_id: SessionId },
        /// Session accepted successfully
        SessionAccepted { session_id: SessionId },
        /// Sender started
        SenderStarted { session_id: SessionId },
        /// Receiver started
        ReceiverStarted { session_id: SessionId },
        /// Session stopped
        SessionStopped { session_id: SessionId },
        /// Session failed
        SessionFailed { session_id: SessionId },
        /// Session recovered
        SessionRecovered { session_id: SessionId },
        /// Media profile switch completed.
        MediaProfileUpdated {
            session_id: SessionId,
            negotiation: MediaProfileNegotiation,
        },
        /// LAN media adaptation controller configured.
        MediaAdaptationConfigured {
            session_id: SessionId,
            snapshot: MediaAdaptationSnapshot,
        },
        /// Local selectable capture sources returned by mrd-service.
        LocalCaptureSourceList { sources: Vec<CaptureSource> },
        /// Selectable capture sources returned by the remote peer.
        CaptureSourceList {
            session_id: SessionId,
            sources: Vec<CaptureSource>,
        },
        /// Capture source selection result returned by the remote peer.
        CaptureSourceSelected {
            session_id: SessionId,
            selection: CaptureSourceSelection,
        },
        /// Display modes returned by the remote peer.
        DisplayModeList {
            session_id: SessionId,
            modes: Vec<DisplayMode>,
        },
        /// Display mode change or restore result.
        DisplayModeChanged {
            session_id: SessionId,
            change: DisplayModeChange,
        },
        /// Native render surface attached.
        RenderSurfaceAttached {
            session_id: SessionId,
            surface_id: String,
        },
        /// Native render surface detached.
        RenderSurfaceDetached {
            session_id: SessionId,
            surface_id: String,
        },
        /// Session runtime snapshot
        SessionSnapshot { snapshot: SessionRuntimeSnapshot },
        /// Aggregated runtime snapshot
        RuntimeSnapshot { snapshot: RuntimeSnapshot },
        /// Service-owned audit events.
        AuditLog { events: Vec<AuditEvent> },
        /// Structured local capability snapshot.
        CapabilitySnapshot {
            /// Current local capability snapshot.
            snapshot: CapabilitySnapshot,
        },
        /// Scenario/profile preflight evaluation result.
        ScenarioProfileEvaluated {
            /// Evaluation result.
            evaluation: ScenarioEvaluation,
        },
        /// Structured peer capability snapshot.
        PeerCapabilitySnapshot {
            /// Peer device id.
            peer_device_id: DeviceId,
            /// Snapshot when the peer is known and can be mapped.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            snapshot: Option<CapabilitySnapshot>,
        },
        /// Transport policy update result.
        TransportPolicyUpdated {
            /// Service-owned policy decision snapshot.
            snapshot: TransportPolicySnapshot,
        },
        /// Control channel lane snapshot.
        ControlChannelSnapshot {
            /// Reliable/realtime lane snapshot.
            snapshot: ControlChannelSnapshot,
        },
        /// Control input event accepted by the service.
        ControlInputAccepted {
            /// Target session id.
            session_id: SessionId,
            /// Lane selected by the service.
            lane: ControlInputLane,
            /// Number of input events applied or queued.
            event_count: u32,
        },
        /// Pairing operation result.
        PairingUpdated {
            /// Device identity snapshot after the operation.
            snapshot: DeviceIdentitySnapshot,
        },
        /// Local device identity snapshot.
        DeviceIdentitySnapshot {
            /// Current identity state.
            snapshot: DeviceIdentitySnapshot,
        },
        /// Compact telemetry bundle.
        TelemetryBundle {
            /// Requested telemetry bundle.
            bundle: TelemetryBundle,
        },
        /// Probe snapshot data
        ProbeSnapshot { snapshot: ProbeSnapshot },
        /// Media pipeline snapshot data.
        MediaPipelineSnapshot { snapshot: MediaPipelineSnapshot },
        /// Probe event data
        ProbeEvent {
            event: Vec<u8>, // Serialized probe event
        },
        /// Service health status
        ServiceHealth { status: ServiceStatus },

        // === Shell / Lifecycle Responses (Phase 2) ===
        /// Result of UI open request
        UiOpenResult {
            status: UiOpenStatus,
            pid: Option<u32>,
        },
        /// Shell/service status snapshot
        ShellStatus { status: ShellStatusSnapshot },
        /// Autostart status
        AutostartStatus { enabled: bool, supported: bool },
        /// Generic acknowledgment
        Ack,
        /// Error response
        Error { code: String, message: String },
    }

    /// Device information DTO
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct DeviceInfo {
        pub device_id: DeviceId,
        pub device_name: String,
        pub is_online: bool,
    }

    /// Discovered LAN peer information.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct LanPeerInfo {
        /// Remote device id.
        pub device_id: DeviceId,
        /// Remote display name.
        pub device_name: String,
        /// Device role/type advertised by the peer.
        pub device_type: String,
        /// Peer IP address string.
        pub ip: String,
        /// UDP port used by the LAN discovery control plane.
        pub discovery_port: u16,
        /// Direct LAN control endpoint as `ip:port`.
        pub p2p_control_addr: String,
        /// Supported media/session transports, for example `webrtc` or `quic`.
        pub transports: Vec<String>,
        /// Protocol version advertised by the peer.
        pub protocol_version: u32,
        /// Service build identifier advertised by the peer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub service_build_id: Option<String>,
        /// LAN media protocol version advertised by the peer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub media_protocol_version: Option<u32>,
        /// Structured media capabilities advertised by the peer.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub media_capabilities: Vec<String>,
        /// Milliseconds since this peer was last observed.
        pub age_ms: u64,
        /// Whether this peer was discovered through the local P2P LAN path.
        pub p2p_available: bool,
    }

    /// LAN discovery state exposed over IPC.
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct LanDiscoverySnapshot {
        /// Whether LAN discovery is enabled in this service process.
        pub enabled: bool,
        /// Whether the UDP discovery task is currently running.
        pub running: bool,
        /// Local UDP discovery port.
        pub discovery_port: u16,
        /// Local discovery instance id.
        pub instance_id: String,
        /// Last successful announce/probe timestamp in milliseconds since Unix epoch.
        pub last_probe_ms: Option<u64>,
        /// Currently known LAN peers.
        pub peers: Vec<LanPeerInfo>,
    }

    /// Session information DTO (for list responses)
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SessionInfo {
        pub session_id: SessionId,
        pub role: String,  // "controller" or "agent"
        pub state: String, // "created", "listening", "connecting", "connected", "streaming", "failed", "closed"
        pub transport_kind: String,
        pub last_error: Option<String>,
        /// Whether the media sender is currently marked active.
        pub sender_active: bool,
        /// Whether the media receiver is currently marked active.
        pub receiver_active: bool,
    }

    /// Session runtime snapshot DTO (stable IPC contract)
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SessionRuntimeSnapshot {
        pub session_id: SessionId,
        pub role: String,           // "controller" or "agent"
        pub state: String, // "created", "listening", "connecting", "connected", "streaming", "failed", "closed"
        pub transport_kind: String, // "quic" or "webrtc"
        pub local_bootstrap: Option<SessionBootstrap>,
        pub remote_bootstrap: Option<SessionBootstrap>,
        pub last_error: Option<String>,
        /// Media pipeline state
        pub sender_active: bool,
        pub receiver_active: bool,
    }

    /// Session bootstrap metadata
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct SessionBootstrap {
        pub listen_addr: Option<String>,
        pub server_name: Option<String>,
        pub cert_der: Option<String>, // Base64-encoded DER certificate
    }

    /// Aggregated runtime snapshot DTO
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct RuntimeSnapshot {
        pub sessions: Vec<SessionRuntimeSnapshot>,
        pub device_id: Option<DeviceId>,
        pub is_registered: bool,
    }

    /// Probe snapshot DTO
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct ProbeSnapshot {
        pub session_id: SessionId,
        pub frames_received: u64,
        pub frames_decoded: u64,
        pub frames_dropped: u64,
        #[serde(default)]
        pub sequence_gap_drops: u64,
        #[serde(default)]
        pub decode_error_drops: u64,
        #[serde(default)]
        pub transient_drops: u64,
        pub current_fps: Option<f32>,
        pub bitrate_mbps: Option<f32>,
        pub media_probe_valid: bool,
        pub media_probe_format: Option<String>,
        pub media_probe_width: Option<u32>,
        pub media_probe_height: Option<u32>,
        pub media_probe_target_fps: Option<u32>,
        pub media_probe_target_bitrate_mbps: Option<u32>,
        pub media_probe_payload_bytes: Option<u32>,
        pub last_media_sequence: Option<u64>,
        pub last_media_timestamp_us: Option<u64>,
        pub last_media_payload_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latest_frame_data_url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latest_frame_width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latest_frame_height: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latest_frame_pixel_format: Option<String>,
        pub last_error: Option<String>,
    }

    /// Service status DTO
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ServiceStatus {
        pub running: bool,
        pub healthy: bool,
        pub pid: Option<u32>,
    }
}

pub use wire::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_ipc_round_trips_with_restore_metadata() {
        let session_id = SessionId("display-mode-session".to_string());
        let mode = DisplayMode {
            id: "windows:display:0:1920x1080@60".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            bit_depth: Some(32),
            is_current: false,
        };
        let change = DisplayModeChange {
            session_id: session_id.clone(),
            requested: Some(mode.clone()),
            previous: Some(DisplayMode {
                id: "windows:display:0:2560x1600@60".to_string(),
                source_id: Some("windows:display:0".to_string()),
                width: 2560,
                height: 1600,
                refresh_hz: 60,
                bit_depth: Some(32),
                is_current: true,
            }),
            active: Some(mode.clone()),
            status: "changed".to_string(),
            reason: None,
            restore_required: true,
        };

        let response = IpcResponse::DisplayModeChanged {
            session_id: session_id.clone(),
            change,
        };
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(encoded.contains("DisplayModeChanged"));
        assert!(encoded.contains("restore_required"));

        let decoded: IpcResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn display_mode_control_requests_are_tagged_ipc_messages() {
        let session_id = SessionId("display-mode-session".to_string());
        let mode = DisplayMode {
            id: "windows:display:0:1920x1080@144".to_string(),
            source_id: Some("windows:display:0".to_string()),
            width: 1920,
            height: 1080,
            refresh_hz: 144,
            bit_depth: None,
            is_current: false,
        };

        let request = IpcRequest::SetRemoteDisplayMode {
            session_id,
            mode,
            restore_after_session: true,
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(encoded.contains("SetRemoteDisplayMode"));

        let decoded: IpcRequest = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn media_profile_round_trips_hevc_chroma_metadata() {
        let profile = MediaProfile {
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

        let encoded = serde_json::to_string(&profile).unwrap();
        assert!(encoded.contains("\"codec\":\"hevc\""));
        assert!(encoded.contains("\"chroma_subsampling\":\"4:2:0\""));
        assert!(encoded.contains("\"hdr_enabled\":false"));

        let decoded: MediaProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }
}
