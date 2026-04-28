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
  | "nvenc"
  | "openh264"
  | "nvdec"
  | "software_decode"
  | "d3d11_render"
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
  capture_type?: "dxgi" | "winrt" | "synthetic";
  encoder_type?: "nvenc_h264" | "nvenc_av1" | "openh264";
  decoder_type?: "none" | "nvdec" | "software";
  renderer_type?: "d3d11";
  render_display?: boolean;
  renderer_target_hwnd?: number;
  zero_copy?: boolean;
  transport_kind?: "loopback" | "quic" | "webrtc";

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
  window_hwnd?: string;
  window_title?: string;
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
  cpu_brand: string;
  cpu_cores: number;
  memory_gb: number;
  gpu_info: string;
  available_encoders: string[];
  available_decoders: string[];
}

export interface WindowCaptureTarget {
  hwnd: string;
  title: string;
  class_name: string;
  width: number;
  height: number;
  process_id: number;
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
  render_mode: "web" | "d3d11_native" | string;
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
  parent_hwnd?: number | null;
  hwnd?: number | null;
  rect: NativeSurfaceRect;
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

export interface ProbeSnapshot {
  session_id: string;
  frames_received: number;
  frames_decoded: number;
  frames_dropped: number;
  current_fps?: number | null;
  bitrate_mbps?: number | null;
  last_error?: string | null;
}

/**
 * Hardware info
 */
export interface HardwareInfo {
  cpu_brand: string;
  cpu_cores: number;
  memory_gb: number;
  gpu_info: string;
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
export type TestChain = "capture_only" | "nvenc_nvdec" | "nvenc_only" | "openh264" | "custom";

export interface TestChainOption {
  value: TestChain;
  label: string;
  description?: string;
}

/**
 * Test matrix configuration for custom pipeline setups
 */
export type CaptureType = 'dxgi' | 'winrt' | 'synthetic';
export type EncoderType = 'nvenc_h264' | 'nvenc_av1' | 'openh264';
export type DecoderType = 'none' | 'nvdec' | 'software';

export interface TestMatrixConfig {
  capture: CaptureType;
  encoder: EncoderType;
  decoder: DecoderType;
  transport?: "loopback" | "quic" | "webrtc";
  renderer?: "none" | "d3d11";
  zero_copy?: boolean;
  resolution?: [number, number];
  fps?: number;
  bitrate?: number;
}

export interface HarnessMetrics {
  is_running: boolean;
  capture_fps: number;
  capture_latency_p50_ms: number;
  capture_latency_p95_ms: number;
  encode_latency_p50_ms: number;
  encode_latency_p95_ms: number;
  decode_latency_p50_ms: number;
  decode_latency_p95_ms: number;
  total_latency_p50_ms: number;
  total_latency_p95_ms: number;
  frame_count: number;
  dropped_frames: number;
  resolution: [number, number];
  error_message?: string;
}

export type FrameData = [string, number, number]; // [base64_data, width, height]

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
