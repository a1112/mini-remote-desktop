/**
 * Tauri adapter types
 *
 * Defines the contract between frontend and Tauri shell.
 * This is the single source of truth for IPC command shapes.
 */

// ============================================================================
// Test Workbench Unified Domain Model
// ============================================================================

/**
 * Test scenario types - describes "what to test"
 */
export type ScenarioKind =
  | "capture"      // 采集测试
  | "encode"       // 编码测试
  | "decode"       // 解码测试
  | "render"       // 渲染测试
  | "transport"    // 传输测试
  | "e2e_local"    // 端到端本地测试
  | "e2e_remote"   // 端到端远程测试
  | "custom";      // 自由组合

export type ComponentScope =
  | "dxgi"
  | "winrt"
  | "macos_capture"
  | "linux_capture"
  | "nvenc"
  | "openh264"
  | "videotoolbox"
  | "nvdec"
  | "software_decode"
  | "d3d11_render"
  | "metal_render"
  | "linux_render"
  | "quic"
  | "webrtc";

/**
 * Test scenario definition
 */
export interface TestScenario {
  scenario_id: string;
  scenario_kind: ScenarioKind;
  component_scope: ComponentScope[];
  display_name: string;
  description: string;
  supports_matrix: boolean;
  default_config: TestConfig;
}

/**
 * Test execution modes
 */
export type RunMode = "manual" | "batch" | "matrix" | "replay";

/**
 * Test run status
 */
export type RunStatus =
  | "queued"
  | "preparing"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

/**
 * Test stage types - unified stage naming for comparison
 */
export type TestStage =
  | "prepare"
  | "capability_check"
  | "initialize"
  | "warmup"
  | "capture"
  | "encode"
  | "decode"
  | "render"
  | "transport"
  | "validate"
  | "running"
  | "summarize";

/**
 * Test configuration - describes "how to test"
 */
export interface TestConfig {
  // Component selection
  capture_type?: "dxgi" | "winrt" | "macos" | "linux" | "synthetic";
  encoder_type?:
    | "none"
    | "nvenc_h264"
    | "nvenc_hevc"
    | "nvenc_hevc_main10"
    | "nvenc_av1"
    | "openh264"
    | "videotoolbox_h264";
  decoder_type?: "none" | "nvdec" | "software" | "linux_h264" | "linux_hevc" | "linux_hevc_main10" | "videotoolbox";
  renderer_type?: "d3d11" | "d3d12" | "opengl" | "macos" | "linux" | "webview";
  render_display?: boolean;
  renderer_target_hwnd?: string;
  zero_copy?: boolean;
  transport_kind?: "loopback" | "quic" | "webrtc";
  adaptive_media?: boolean;

  // Parameters
  resolution?: [number, number];
  fps?: number;
  bitrate?: number;

  // Execution control
  duration_ms?: number;
  warmup_ms?: number;
  repeat_count?: number;

  // I/O
  input_source?: "screen" | "window" | "synthetic";
  source_id?: string;
  source_kind?: "screen" | "window" | "portal" | string;
  display_id?: string;
  window_hwnd?: string;
  window_title?: string;
  visual_preview?: boolean;
  output_validation?: boolean;
}

/**
 * Test run record - describes "one specific execution"
 */
export interface TestRun {
  run_id: string;
  scenario_id: string;
  run_mode: RunMode;
  status: RunStatus;
  started_at: number;  // timestamp
  finished_at?: number;
  config_snapshot: TestConfig;
  environment_snapshot: EnvironmentSnapshot;
  summary?: TestRunSummary;
}

/**
 * Environment snapshot
 */
export interface EnvironmentSnapshot {
  os_type?: string;
  cpu_brand: string;
  cpu_cores: number;
  memory_gb: number;
  gpu_info: string;
  available_captures?: string[];
  available_encoders: string[];
  available_decoders: string[];
  available_renderers?: string[];
  available_memory_modes?: string[];
}

export interface WindowCaptureTarget {
  id?: string;
  platform?: "windows" | "macos" | string;
  source_kind?: "window" | string;
  hwnd: string;
  title: string;
  class_name: string;
  width: number;
  height: number;
  process_id: number;
  app_name?: string | null;
  bundle_identifier?: string | null;
  window_layer?: number | null;
  preview_data_url?: string | null;
  preview_width?: number | null;
  preview_height?: number | null;
}

export interface CaptureShareSourceTarget {
  id: string;
  platform: "windows" | "macos" | "linux" | string;
  source_kind: "screen" | "window" | "portal" | string;
  native_id: string;
  title: string;
  subtitle: string;
  width: number;
  height: number;
  is_primary: boolean;
  requires_system_picker: boolean;
  hwnd?: string | null;
  class_name?: string | null;
  process_id?: number | null;
  app_name?: string | null;
  bundle_identifier?: string | null;
  window_layer?: number | null;
  preview_data_url?: string | null;
  preview_width?: number | null;
  preview_height?: number | null;
}

export interface RemoteDisplayWindowContext {
  label: string;
  session_id: string;
  surface_id: string;
  role: string;
  renderer_attached: boolean;
  render_mode: "web" | "d3d11_native" | "macos_native" | "linux_native" | string;
  native_surface_attached: boolean;
  session_window_count: number;
}

export interface NativeSurfaceRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface NativeRenderSurfaceSnapshot {
  label: string;
  backend: "web" | "d3d11" | string;
  attached: boolean;
  visible: boolean;
  parent_hwnd?: string | null;
  hwnd?: string | null;
  rect: NativeSurfaceRect;
}

export interface BrowserWebrtcPreviewAnswer {
  session_id: string;
  answer_sdp: string;
}

/**
 * Test run summary
 */
export interface TestRunSummary {
  total_duration_ms: number;
  first_frame_latency_ms?: number;
  capture_fps?: number;
  encode_latency_p50?: number;
  encode_latency_p95?: number;
  transport_latency_p50?: number;
  transport_latency_p95?: number;
  decode_latency_p50?: number;
  decode_latency_p95?: number;
  total_latency_p95?: number;
  dropped_frames: number;
  frame_count: number;
  adaptation_state?: string;
  adaptation_ladder_index?: number;
  adaptation_current_profile?: string;
  adaptation_target_profile?: string;
  adaptation_reason?: string;
  error_message?: string;
  failure_reason?: FailureReason;
}

/**
 * Failure reason types
 */
export type FailureReason =
  | "capability_mismatch"
  | "initialization_failure"
  | "runtime_failure"
  | "runtime_stopped"
  | "warmup_timeout"
  | "runtime_instability"
  | "threshold_breach"
  | "validation_failure"
  | "unknown";

/**
 * Test stage event
 */
export interface TestStageEvent {
  stage: TestStage;
  status: string;
  timestamp: number;
  duration_ms?: number;
  error?: string;
}

/**
 * Metric series - time series data
 */
export interface MetricSeries {
  metric_name: string;
  unit: string;
  samples: Array<{ timestamp: number; value: number }>;
  aggregation?: {
    min?: number;
    max?: number;
    mean?: number;
    p50?: number;
    p95?: number;
    p99?: number;
  };
  category?: string;
  display_name?: string;
  source?: string;
}

/**
 * Artifact types
 */
export type ArtifactKind =
  | "captured_frame"
  | "decoded_frame"
  | "rendered_frame"
  | "encoded_sample"
  | "structured_log"
  | "raw_log"
  | "summary_json";

/**
 * Artifact record
 */
export interface Artifact {
  artifact_id: string;
  kind: ArtifactKind;
  run_id: string;
  created_at: number;
  data: string;  // base64 or text
  metadata?: {
    width?: number;
    height?: number;
    format?: string;
    size_bytes?: number;
  };
}

/**
 * Matrix dimension definition
 */
export interface MatrixDimension {
  name: string;
  values: string[];
  selected: string[];
}

/**
 * Matrix run configuration
 */
export interface MatrixConfig {
  base_scenario_id: string;
  base_config: TestConfig;
  dimensions: MatrixDimension[];
  max_concurrent?: number;
  stop_on_failure?: boolean;
  repeat_count?: number;
}

/**
 * Matrix result
 */
export interface MatrixResult {
  matrix_id: string;
  config: MatrixConfig;
  total_combinations: number;
  completed: number;
  failed: number;
  runs: TestRun[];
  started_at: number;
  finished_at?: number;
}

/**
 * Test preset
 */
export interface TestPreset {
  preset_id: string;
  name: string;
  description: string;
  scenario_id: string;
  config: TestConfig;
  tags?: string[];
  created_at: number;
}

// ============================================================================
// Legacy Test Harness Types (for backward compatibility)
// ============================================================================
export interface ServiceStatusResponse {
  is_running: boolean;
}

export interface ServiceHealthResponse {
  healthy: boolean;
}

export interface ServicePidResponse {
  pid: number | null;
}

export type ShutdownMode = "graceful" | "force" | "after_sessions";

export interface ShellStatusSnapshot {
  service_pid: number;
  ui_pid: number | null;
  tray_available: boolean;
  autostart_enabled: boolean | null;
  active_session_count: number;
  last_error: string | null;
}

export interface NativeBackdropStatus {
  platform: string;
  effect: string;
  applied: boolean;
  detail: string;
}

export interface ClientDiagnostics {
  app_pid: number;
  app_exe_path: string | null;
  current_dir: string | null;
  log_dir: string;
  service_exe_path: string;
  service_stdout_log: string;
  service_stderr_log: string;
}

/**
 * IPC Device types
 */
export interface DeviceInfo {
  device_id: string;
  device_name: string;
  is_online: boolean;
}

export interface LanPeerInfo {
  device_id: string;
  device_name: string;
  device_type: string;
  ip: string;
  discovery_port: number;
  p2p_control_addr: string;
  transports: string[];
  protocol_version: number;
  service_build_id?: string | null;
  media_protocol_version?: number | null;
  media_capabilities?: string[];
  age_ms: number;
  p2p_available: boolean;
}

export interface LanDiscoverySnapshot {
  enabled: boolean;
  running: boolean;
  discovery_port: number;
  instance_id: string;
  last_probe_ms?: number | null;
  peers: LanPeerInfo[];
}

export type CapabilityPlatform =
  | "windows"
  | "macos"
  | "linux"
  | "android"
  | "ios"
  | "web"
  | "unknown";

export type CapabilityDomain =
  | "capture"
  | "capture_source"
  | "encode"
  | "decode"
  | "render"
  | "memory"
  | "transport"
  | "control"
  | "audio"
  | "service"
  | "security";

export type CapabilityStatus =
  | "supported"
  | "available"
  | "usable"
  | "degraded"
  | "permission_missing"
  | "driver_missing"
  | "hardware_missing"
  | "unimplemented"
  | "unsupported"
  | "unknown";

export interface CapabilityItem {
  id: string;
  domain: CapabilityDomain;
  label: string;
  status: CapabilityStatus;
  platform: CapabilityPlatform;
  reason?: string | null;
  detail?: string | null;
  requires?: string[];
  conflicts_with?: string[];
  depends_on?: string[];
  fallback_ids?: string[];
  last_probe_time_ms?: number | null;
}

export type CapabilityConstraintStatus =
  | "allow"
  | "block"
  | "degrade"
  | "requires_copy"
  | "requires_probe";

export interface CapabilityConstraint {
  id: string;
  applies_to: string[];
  status: CapabilityConstraintStatus;
  reason: string;
  fallback_ids?: string[];
}

export interface CapabilityProfile {
  id: string;
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
  codec: "h264" | "hevc" | "av1" | string;
  latency_budget_ms?: number | null;
  min_stable_fps_ratio?: number | null;
  max_drop_ratio?: number | null;
  required_capabilities: string[];
}

export interface CapabilitySnapshot {
  schema_version: number;
  platform: CapabilityPlatform;
  service_version: string;
  capabilities: CapabilityItem[];
  constraints: CapabilityConstraint[];
  profiles: CapabilityProfile[];
  updated_at_ms: number;
}

export interface DeviceRegistrationResponse {
  device_id: string;
  device_name: string;
  access_token: string;
}

/**
 * IPC Session types
 */
export interface SessionRuntimeSnapshot {
  session_id: string;
  role: "controller" | "agent" | "unknown";
  state:
    | "created"
    | "listening"
    | "connecting"
    | "connected"
    | "streaming"
    | "failed"
    | "closed";
  transport_kind: "quic" | "webrtc";
  local_bootstrap?: {
    listen_addr?: string;
    server_name?: string;
    cert_der?: string;
  };
  remote_bootstrap?: {
    listen_addr?: string;
    server_name?: string;
    cert_der?: string;
  };
  last_error?: string;
  sender_active: boolean;
  receiver_active: boolean;
}

export interface SessionInfo {
  session_id: string;
  role: "controller" | "agent" | "unknown";
  state:
    | "created"
    | "listening"
    | "connecting"
    | "connected"
    | "streaming"
    | "failed"
    | "closed";
  transport_kind: "quic" | "webrtc" | string;
  last_error?: string | null;
  sender_active: boolean;
  receiver_active: boolean;
}

export interface RuntimeSnapshot {
  sessions: SessionRuntimeSnapshot[];
  device_id?: string | null;
  is_registered: boolean;
}

export interface AuditLogQuery {
  session_id?: string | null;
  action?: string | null;
  limit?: number | null;
}

export interface AuditEvent {
  id: number;
  timestamp_ms: number;
  action: string;
  outcome: string;
  session_id?: string | null;
  actor_device_id?: string | null;
  peer_device_id?: string | null;
  transport_kind?: string | null;
  reason?: string | null;
  details: Array<[string, string]>;
}

export interface MediaProfile {
  width: number;
  height: number;
  fps: number;
  bitrate_mbps: number;
  codec: string;
  codec_profile?: string | null;
  bit_depth?: number | null;
  chroma_subsampling?: string | null;
  pixel_format?: string | null;
  hdr_enabled?: boolean | null;
}

export interface MediaProfileNegotiation {
  requested: MediaProfile;
  selected: MediaProfile;
  status: "accepted" | "downgraded" | "rejected" | string;
  reason?: string | null;
  selected_source_id?: string | null;
  selected_width?: number | null;
  selected_height?: number | null;
  downgrade_reason?: string | null;
}

export interface AttachedRenderSurface {
  surface_id: string;
  backend: string;
  window_handle?: number | null;
}

export interface MediaStageMetrics {
  stage: string;
  p50_ms?: number | null;
  p95_ms?: number | null;
}

export interface MediaTestImpairmentSnapshot {
  loss_pct: number;
  base_delay_ms: number;
  jitter_ms: number;
  mtu_bytes?: number | null;
  seed: number;
  datagrams_sent: number;
  datagrams_dropped: number;
  datagrams_delayed: number;
  datagrams_fragmented_by_mtu: number;
}

export interface TelemetryRunMetadata {
  run_id: string;
  scenario_id: string;
  status: string;
  started_at: number;
  finished_at?: number | null;
  tags: string[];
}

export interface TelemetryLogEntry {
  run_id: string;
  timestamp: number;
  level: string;
  source: string;
  message: string;
  fields?: unknown;
}

export interface TelemetryQuery {
  start_ms?: number | null;
  end_ms?: number | null;
  metric_names?: string[];
  log_sources?: string[];
  max_points?: number | null;
}

export interface TelemetryDiagnostics {
  corrupt_rows: number;
  warnings: string[];
}

export interface TelemetryBundle {
  run?: TelemetryRunMetadata | null;
  metrics: Record<string, MetricSeries>;
  events: TestStageEvent[];
  logs: TelemetryLogEntry[];
  artifacts: Artifact[];
  diagnostics: TelemetryDiagnostics;
}

export interface AdaptiveMediaConfig {
  enabled: boolean;
  mode?: "keyframe_ladder" | string;
  ceiling_profile?: MediaProfile | null;
  floor_profile?: MediaProfile | null;
  ladder?: MediaProfile[];
  downshift_cooldown_ms?: number;
  upshift_hold_ms?: number;
}

export interface MediaAdaptationSnapshot {
  enabled: boolean;
  state: string;
  ladder_index: number;
  current_profile: MediaProfile;
  target_profile: MediaProfile;
  last_reason?: string | null;
  last_change_ms: number;
  observed_fps: number;
  drop_ratio: number;
  queue_depth: number;
}

export interface MediaPipelineSnapshot {
  session_id: string;
  attached_surfaces: AttachedRenderSurface[];
  active_decoder?: string | null;
  active_renderer?: string | null;
  active_codec?: string | null;
  active_codec_profile?: string | null;
  active_bit_depth?: number | null;
  active_chroma_subsampling?: string | null;
  active_pixel_format?: string | null;
  active_hdr_enabled?: boolean | null;
  active_width?: number | null;
  active_height?: number | null;
  active_fps?: number | null;
  active_bitrate_mbps?: number | null;
  codec_fallback_reason?: string | null;
  queue_depth: number;
  dropped_frames: number;
  render_presented_frames?: number;
  render_queue_replacements?: number;
  render_lock_drops?: number;
  render_pacing_target_fps?: number | null;
  stage_metrics: MediaStageMetrics[];
  test_impairment?: MediaTestImpairmentSnapshot | null;
  adaptation?: MediaAdaptationSnapshot | null;
}

export interface CaptureSource {
  id: string;
  platform: string;
  source_kind: string;
  title: string;
  class_name: string;
  width: number;
  height: number;
  process_id: number;
  app_name?: string | null;
  bundle_identifier?: string | null;
  preview_data_url?: string | null;
  preview_width?: number | null;
  preview_height?: number | null;
}

export interface CaptureSourceSelection {
  session_id: string;
  source: CaptureSource;
  status: "selected" | "rejected" | string;
  reason?: string | null;
}

export interface DisplayMode {
  id: string;
  source_id?: string | null;
  width: number;
  height: number;
  refresh_hz: number;
  bit_depth?: number | null;
  is_current: boolean;
}

export interface DisplayModeChange {
  session_id: string;
  requested?: DisplayMode | null;
  previous?: DisplayMode | null;
  active?: DisplayMode | null;
  status: "changed" | "restored" | "rejected" | string;
  reason?: string | null;
  restore_required: boolean;
}

export interface ProbeSnapshot {
  session_id: string;
  frames_received: number;
  frames_decoded: number;
  frames_dropped: number;
  current_fps?: number | null;
  bitrate_mbps?: number | null;
  media_probe_valid?: boolean;
  media_probe_format?: string | null;
  media_probe_width?: number | null;
  media_probe_height?: number | null;
  media_probe_target_fps?: number | null;
  media_probe_target_bitrate_mbps?: number | null;
  media_probe_payload_bytes?: number | null;
  last_media_sequence?: number | null;
  last_media_timestamp_us?: number | null;
  last_media_payload_hash?: string | null;
  latest_frame_data_url?: string | null;
  latest_frame_width?: number | null;
  latest_frame_height?: number | null;
  latest_frame_pixel_format?: string | null;
  last_error?: string | null;
}

/**
 * Hardware info
 */
export interface HardwareInfo {
  motherboard_serial: string;
  hostname: string;
  os_type: string;
  os_version: string;
  cpu_info: {
    name: string;
    vendor_id: string;
    cores: number;
    max_frequency_mhz?: number | null;
  };
  total_memory_mb: number;
  gpu_info: Array<{
    name: string;
    vendor: string;
    memory_mb?: number | null;
  }>;
}

export interface SystemResourceSnapshot {
  target_name: string;
  target_pid?: number | null;
  target_found: boolean;
  cpu_usage_percent: number;
  memory_used_mb: number;
  memory_total_mb: number;
  memory_usage_percent: number;
  gpu_usage_percent?: number | null;
  gpu_memory_used_mb?: number | null;
  gpu_memory_total_mb?: number | null;
  gpu_metrics_available: boolean;
  gpu_metrics_scope?: "process" | "system" | "unavailable" | string;
  network_rx_bps: number;
  network_tx_bps: number;
  network_metrics_available: boolean;
  network_metrics_scope?: "process" | "system" | "unavailable" | string;
  sampled_at_ms: number;
}

/**
 * Decode policy
 */
export type DecodePolicy = 'auto' | 'software' | 'd3d11va' | 'nvdec';

export interface DecodePolicyResponse {
  decode_policy: DecodePolicy;
}

/**
 * Test harness types - end-to-end pipeline visualization
 */
export type TestChain = "capture_only" | "nvenc_nvdec" | "nvenc_only" | "openh264" | "linux_openh264" | "custom";

export interface TestChainOption {
  value: TestChain;
  label: string;
  description?: string;
}

/**
 * Test matrix configuration for custom pipeline setups
 */
export type CaptureType = 'dxgi' | 'winrt' | 'macos' | 'linux' | 'synthetic';
export type EncoderType =
  | 'none'
  | 'nvenc_h264'
  | 'nvenc_hevc'
  | 'nvenc_hevc_main10'
  | 'nvenc_av1'
  | 'openh264'
  | 'videotoolbox_h264';
export type DecoderType = 'none' | 'nvdec' | 'software' | 'linux_h264' | 'linux_hevc' | 'linux_hevc_main10' | 'videotoolbox';

export interface TestMatrixConfig {
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport?: "loopback" | "quic" | "webrtc";
  renderer?: "none" | "d3d11" | "macos" | "linux";
  zero_copy?: boolean;
  resolution?: [number, number];
  fps?: number;
  bitrate?: number;
}

export interface HarnessMetrics {
  is_running: boolean;
  capture_fps: number;
  encoded_fps?: number;
  decoded_fps?: number;
  capture_latency_avg_ms: number;
  capture_latency_p50_ms: number;
  capture_latency_p95_ms: number;
  source_wait_latency_avg_ms?: number;
  source_wait_latency_p50_ms?: number;
  source_wait_latency_p95_ms?: number;
  interactive_latency_avg_ms?: number;
  interactive_latency_p50_ms?: number;
  interactive_latency_p95_ms?: number;
  encode_latency_avg_ms: number;
  encode_latency_p50_ms: number;
  encode_latency_p95_ms: number;
  transport_latency_avg_ms: number;
  transport_latency_p50_ms: number;
  transport_latency_p95_ms: number;
  decode_latency_avg_ms: number;
  decode_latency_p50_ms: number;
  decode_latency_p95_ms: number;
  render_latency_avg_ms: number;
  present_latency_avg_ms: number;
  total_latency_avg_ms: number;
  total_latency_p50_ms: number;
  total_latency_p95_ms: number;
  frame_count: number;
  encoded_units: number;
  decoded_frames: number;
  encode_failures: number;
  decode_failures: number;
  total_bitstream_bytes: number;
  dropped_frames: number;
  resolution: [number, number];
  error_message?: string;
}

export interface PipelineComparisonResult {
  pipeline: string;
  codec: string;
  transport?: string | null;
  memory_path: string;
  frames: number;
  encoded_units: number;
  decoded_frames: number;
  encode_failures: number;
  decode_failures: number;
  avg_capture_time_ms?: number | null;
  avg_encode_time_ms?: number | null;
  avg_transport_time_ms?: number | null;
  avg_decode_time_ms?: number | null;
  avg_render_time_ms?: number | null;
  avg_present_time_ms?: number | null;
  avg_total_time_ms?: number | null;
  avg_fps?: number | null;
  total_bitstream_bytes: number;
}

export type FrameData = [string, number, number, number?]; // [base64_data, width, height, generation]

/**
 * Error response shape from Tauri commands
 */
export interface TauriError {
  code?: string;
  message: string;
}

/**
 * Result type for adapter responses
 */
export type AdapterResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: TauriError };
