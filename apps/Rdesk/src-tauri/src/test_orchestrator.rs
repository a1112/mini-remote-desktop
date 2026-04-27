//! Test Orchestrator - Unified test execution and management
//!
//! This module provides the test orchestrator that manages test scenarios,
//! runs, metrics collection, and artifact storage.

use anyhow::Result;
use base64::Engine;
use mrd_pipeline_core::{
    CapturedFrame, DecodedFrame, DecodedFrameData, EncodedAccessUnit, FramePixelFormat,
    VideoEncoder,
};
use mrd_render::{RenderFrame, RenderTarget, RendererFactory};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::test_harness::{
    CaptureType, DecoderType, EncoderType, HarnessMetrics, RendererType, TestChain,
    TestConfig as HarnessConfig, TestHarness,
};
use std::thread;
use std::time::Duration;

/// Unique identifier for a test run
pub type RunId = String;

/// Test scenario kinds
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioKind {
    Capture,
    Encode,
    Decode,
    Render,
    Transport,
    #[serde(rename = "e2e_local")]
    E2eLocal,
    #[serde(rename = "e2e_remote")]
    E2eRemote,
    Custom,
}

/// Test run status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Preparing,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Test run mode
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    Manual,
    Batch,
    Matrix,
    Replay,
}

/// Test scenario definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestScenario {
    pub scenario_id: String,
    pub scenario_kind: ScenarioKind,
    pub component_scope: Vec<String>,
    pub display_name: String,
    pub description: String,
    pub supports_matrix: bool,
    #[serde(default)]
    pub default_config: TestConfigData,
}

/// Test config data (serializable version)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TestConfigData {
    pub capture_type: Option<String>,
    pub encoder_type: Option<String>,
    pub decoder_type: Option<String>,
    pub renderer_type: Option<String>,
    pub render_display: Option<bool>,
    pub transport_kind: Option<String>,
    pub resolution: Option<[usize; 2]>,
    pub fps: Option<u32>,
    pub bitrate: Option<u32>,
    pub duration_ms: Option<u64>,
    pub warmup_ms: Option<u64>,
    pub repeat_count: Option<u32>,
    pub input_source: Option<String>,
    pub window_hwnd: Option<String>,
    pub window_title: Option<String>,
    pub output_validation: Option<bool>,
}

/// Test run record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRun {
    pub run_id: RunId,
    pub scenario_id: String,
    pub run_mode: RunMode,
    pub status: RunStatus,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub config_snapshot: TestConfigData,
    pub environment_snapshot: EnvironmentSnapshot,
    pub summary: Option<TestRunSummary>,
}

/// Environment snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub cpu_brand: String,
    pub cpu_cores: u32,
    pub memory_gb: u32,
    pub gpu_info: String,
    pub available_encoders: Vec<String>,
    pub available_decoders: Vec<String>,
}

/// Test run summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunSummary {
    pub total_duration_ms: u64,
    pub first_frame_latency_ms: Option<f64>,
    pub capture_fps: Option<f64>,
    pub encode_latency_p50: Option<f64>,
    pub encode_latency_p95: Option<f64>,
    pub decode_latency_p50: Option<f64>,
    pub decode_latency_p95: Option<f64>,
    pub total_latency_p95: Option<f64>,
    pub dropped_frames: usize,
    pub frame_count: usize,
    pub error_message: Option<String>,
    pub failure_reason: Option<String>,
}

/// Test stage event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStageEvent {
    pub stage: String,
    pub status: String,
    pub timestamp: u64,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
}

/// Metric series data point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDataPoint {
    pub timestamp: u64,
    pub value: f64,
}

/// Metric series
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSeries {
    pub metric_name: String,
    pub unit: String,
    pub samples: Vec<MetricDataPoint>,
    pub aggregation: Option<MetricAggregation>,
}

/// Metric aggregation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricAggregation {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
}

/// Artifact record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: String,
    pub kind: String,
    pub run_id: String,
    pub created_at: u64,
    pub data: String,
    pub metadata: Option<ArtifactMetadata>,
}

/// Artifact metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub width: Option<usize>,
    pub height: Option<usize>,
    pub format: Option<String>,
    pub size_bytes: Option<usize>,
}

/// A visible top-level window that can be used as a WinRT capture target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowCaptureTarget {
    pub hwnd: String,
    pub title: String,
    pub class_name: String,
    pub width: u32,
    pub height: u32,
    pub process_id: u32,
}

struct WindowCaptureItemProbe {
    hwnd: isize,
    title: String,
    class_name: String,
    width: u32,
    height: u32,
}

struct WindowCaptureFrameProbe {
    hwnd: isize,
    title: String,
    class_name: String,
    width: u32,
    height: u32,
    byte_len: usize,
    pixel_format: String,
    frame: mrd_pipeline_core::CapturedFrame,
}

struct SingleWindowMediaProbe {
    transport: String,
    encoded_width: usize,
    encoded_height: usize,
    access_unit_count: usize,
    encoded_bytes: usize,
    keyframe_count: usize,
    transport_rtp_packet_count: usize,
    transport_payload_bytes: usize,
    encode_latency_ms: f64,
    decode_latency_ms: f64,
    decoded_frame_count: usize,
    decoded_width: Option<usize>,
    decoded_height: Option<usize>,
    decoded_pixel_format: Option<String>,
    render_backend: Option<String>,
    render_latency_ms: Option<f64>,
    rendered_frame_count: usize,
    first_access_unit: Option<Vec<u8>>,
}

struct SingleWindowTransportProbe {
    transport: String,
    access_units: Vec<EncodedAccessUnit>,
    rtp_packet_count: usize,
    payload_bytes: usize,
}

/// Test preset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestPreset {
    pub preset_id: String,
    pub name: String,
    pub description: String,
    pub scenario_id: String,
    pub config: TestConfigData,
    pub tags: Option<Vec<String>>,
    pub created_at: u64,
}

/// Test Orchestrator - manages test execution
pub struct TestOrchestrator {
    harness: Arc<Mutex<TestHarness>>,
    runs: Arc<Mutex<HashMap<RunId, TestRun>>>,
    run_metrics: Arc<Mutex<HashMap<RunId, HashMap<String, MetricSeries>>>>,
    run_events: Arc<Mutex<HashMap<RunId, Vec<TestStageEvent>>>>,
    run_artifacts: Arc<Mutex<HashMap<RunId, Vec<Artifact>>>>,
    presets: Arc<Mutex<HashMap<String, TestPreset>>>,
    current_harness_chain: Arc<Mutex<Option<TestChain>>>,
}

impl TestOrchestrator {
    pub fn new(harness: Arc<Mutex<TestHarness>>) -> Self {
        Self {
            harness,
            runs: Arc::new(Mutex::new(HashMap::new())),
            run_metrics: Arc::new(Mutex::new(HashMap::new())),
            run_events: Arc::new(Mutex::new(HashMap::new())),
            run_artifacts: Arc::new(Mutex::new(HashMap::new())),
            presets: Arc::new(Mutex::new(HashMap::new())),
            current_harness_chain: Arc::new(Mutex::new(None)),
        }
    }

    /// Convert scenario_id to TestChain
    fn scenario_to_chain(&self, scenario_id: &str, config: &TestConfigData) -> Result<TestChain> {
        match scenario_id {
            "e2e.local" => Ok(TestChain::NvencNvdec),
            "encode.nvenc_h264" => Ok(TestChain::NvencOnly),
            "encode.openh264" => Ok(TestChain::OpenH264),
            "custom" | "matrix" => Ok(TestChain::Custom {
                capture: match config.capture_type.as_deref().unwrap_or("dxgi") {
                    "dxgi" => CaptureType::Dxgi,
                    "winrt" => CaptureType::Winrt,
                    "synthetic" => CaptureType::Synthetic,
                    other => anyhow::bail!("Unsupported capture for {}: {}", scenario_id, other),
                },
                encoder: match config.encoder_type.as_deref() {
                    Some("nvenc_h264") => EncoderType::NvencH264,
                    Some("openh264") => EncoderType::OpenH264,
                    Some("nvenc_av1") => EncoderType::NvencAv1,
                    Some(other) => {
                        anyhow::bail!("Unsupported encoder for {}: {}", scenario_id, other)
                    }
                    None => anyhow::bail!("Missing encoder_type for {}", scenario_id),
                },
                decoder: match config.decoder_type.as_deref().unwrap_or("software") {
                    "nvdec" => DecoderType::Nvdec,
                    "software" => DecoderType::Software,
                    other => anyhow::bail!("Unsupported decoder for {}: {}", scenario_id, other),
                },
            }),
            other => anyhow::bail!("Unsupported test scenario: {}", other),
        }
    }

    /// List all available test scenarios
    pub fn list_scenarios(&self) -> Vec<TestScenario> {
        vec![
            TestScenario {
                scenario_id: "capture.dxgi".to_string(),
                scenario_kind: ScenarioKind::Capture,
                component_scope: vec!["dxgi".to_string()],
                display_name: "DXGI 屏幕捕获测试".to_string(),
                description: "测试 DXGI 捕获性能和稳定性".to_string(),
                supports_matrix: true,
                default_config: TestConfigData {
                    capture_type: Some("dxgi".to_string()),
                    ..Default::default()
                },
            },
            TestScenario {
                scenario_id: "encode.nvenc_h264".to_string(),
                scenario_kind: ScenarioKind::Encode,
                component_scope: vec!["nvenc".to_string()],
                display_name: "NVENC H.264 编码测试".to_string(),
                description: "测试 NVIDIA H.264 硬件编码器性能".to_string(),
                supports_matrix: true,
                default_config: TestConfigData {
                    encoder_type: Some("nvenc_h264".to_string()),
                    ..Default::default()
                },
            },
            TestScenario {
                scenario_id: "encode.openh264".to_string(),
                scenario_kind: ScenarioKind::Encode,
                component_scope: vec!["openh264".to_string()],
                display_name: "OpenH264 软件编码测试".to_string(),
                description: "测试 OpenH264 软件编码器性能".to_string(),
                supports_matrix: true,
                default_config: TestConfigData {
                    encoder_type: Some("openh264".to_string()),
                    ..Default::default()
                },
            },
            TestScenario {
                scenario_id: "decode.nvdec_h264".to_string(),
                scenario_kind: ScenarioKind::Decode,
                component_scope: vec!["nvdec".to_string()],
                display_name: "NVDEC H.264 解码测试".to_string(),
                description: "测试 NVIDIA H.264 硬件解码器性能".to_string(),
                supports_matrix: true,
                default_config: TestConfigData {
                    decoder_type: Some("nvdec".to_string()),
                    ..Default::default()
                },
            },
            TestScenario {
                scenario_id: "e2e.local".to_string(),
                scenario_kind: ScenarioKind::E2eLocal,
                component_scope: vec!["dxgi".to_string(), "nvenc".to_string(), "nvdec".to_string()],
                display_name: "端到端本地测试".to_string(),
                description: "测试完整的采集→编码→解码流程".to_string(),
                supports_matrix: true,
                default_config: TestConfigData {
                    capture_type: Some("dxgi".to_string()),
                    encoder_type: Some("nvenc_h264".to_string()),
                    decoder_type: Some("nvdec".to_string()),
                    resolution: Some([1920, 1080]),
                    fps: Some(60),
                    bitrate: Some(5000000),
                    ..Default::default()
                },
            },
            TestScenario {
                scenario_id: "single_window.local".to_string(),
                scenario_kind: ScenarioKind::E2eLocal,
                component_scope: vec![
                    "winrt".to_string(),
                    "openh264".to_string(),
                    "webrtc".to_string(),
                    "software_decode".to_string(),
                    "d3d11_render".to_string(),
                ],
                display_name: "Single window local probe".to_string(),
                description:
                    "Captures one WinRT window frame and runs it through encode, WebRTC RTP, decode, and render."
                        .to_string(),
                supports_matrix: false,
                default_config: TestConfigData {
                    capture_type: Some("winrt".to_string()),
                    encoder_type: Some("openh264".to_string()),
                    decoder_type: Some("software".to_string()),
                    transport_kind: Some("webrtc".to_string()),
                    input_source: Some("window".to_string()),
                    duration_ms: Some(1_000),
                    ..Default::default()
                },
            },
        ]
    }

    /// Get environment capabilities
    pub fn get_capabilities(&self) -> Result<EnvironmentSnapshot> {
        let hw_info = crate::device_info::get_hardware_info();

        // Detect available encoders
        let mut available_encoders = vec!["openh264".to_string()];

        // Try to detect NVENC
        if mrd_encode_nvenc::NvencH264Encoder::new_max_speed(1920, 1080, 60).is_ok() {
            available_encoders.push("nvenc_h264".to_string());
        }

        // Detect available decoders
        let mut available_decoders = vec!["software".to_string()];
        if mrd_decode_nvdec::NvdecDecoder::new().is_ok() {
            available_decoders.push("nvdec".to_string());
        }

        // Get GPU info string
        let gpu_info = hw_info
            .gpu_info
            .iter()
            .map(|g| g.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        Ok(EnvironmentSnapshot {
            cpu_brand: hw_info.cpu_info.name,
            cpu_cores: hw_info.cpu_info.cores,
            memory_gb: (hw_info.total_memory_mb / 1024) as u32,
            gpu_info: if gpu_info.is_empty() {
                "Unknown".to_string()
            } else {
                gpu_info
            },
            available_encoders,
            available_decoders,
        })
    }

    /// Start a new test run
    pub fn start_run(&self, scenario_id: String, config: TestConfigData) -> Result<RunId> {
        if scenario_id == "single_window.local" {
            return self.start_single_window_probe(scenario_id, config);
        }

        // Resolve the scenario before recording a run. Unsupported scenarios must
        // fail fast instead of leaving a phantom running record behind.
        let chain = self.scenario_to_chain(&scenario_id, &config)?;
        let run_id = generate_run_id();
        let started_at = now_ms();
        let env_snapshot = self.get_capabilities()?;

        let run = TestRun {
            run_id: run_id.clone(),
            scenario_id: scenario_id.clone(),
            run_mode: RunMode::Manual,
            status: RunStatus::Running,
            started_at,
            finished_at: None,
            config_snapshot: config.clone(),
            environment_snapshot: env_snapshot,
            summary: None,
        };

        self.runs.lock().unwrap().insert(run_id.clone(), run);

        // Record stage event
        self.record_stage_event(run_id.clone(), "prepare", "started", None, None);

        // Convert scenario to chain and start the shared harness used by legacy
        // frame/metric commands, so run state and visualization stay aligned.
        self.record_stage_event(run_id.clone(), "prepare", "chain_resolved", None, None);

        self.record_stage_event(run_id.clone(), "initialize", "started", None, None);

        let mut harness = self.harness.lock().unwrap();
        harness.set_chain(chain.clone());
        harness.set_config(harness_config_from_data(&config));
        if let Err(error) = harness.start() {
            let message = format!("Failed to start test harness: {}", error);
            drop(harness);

            if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                run.status = RunStatus::Failed;
                run.finished_at = Some(now_ms());
                run.summary = Some(TestRunSummary {
                    total_duration_ms: now_ms().saturating_sub(run.started_at),
                    error_message: Some(message.clone()),
                    failure_reason: Some("initialization_failure".to_string()),
                    ..Default::default()
                });
            }

            self.record_stage_event(
                run_id.clone(),
                "initialize",
                "failed",
                None,
                Some(message.clone()),
            );
            anyhow::bail!(message);
        }
        drop(harness);

        self.record_stage_event(run_id.clone(), "initialize", "completed", None, None);
        self.record_stage_event(run_id.clone(), "running", "started", None, None);

        // Store harness reference for this run
        self.set_harness_chain(chain);

        // Spawn background thread to collect metrics
        let run_id_clone = run_id.clone();
        let orchestrator_runs = self.runs.clone();
        let orchestrator_events = self.run_events.clone();
        let orchestrator_metrics = self.run_metrics.clone();
        let harness = self.harness.clone();
        let duration_ms = config.duration_ms.unwrap_or(30_000);

        thread::spawn(move || {
            let started_at = now_ms();
            loop {
                thread::sleep(Duration::from_millis(500));

                // Check if run still exists and is running
                let is_running = {
                    let runs = orchestrator_runs.lock().unwrap();
                    runs.get(&run_id_clone)
                        .map(|r| r.status == RunStatus::Running)
                        .unwrap_or(false)
                };

                if !is_running {
                    orchestrator_events
                        .lock()
                        .unwrap()
                        .entry(run_id_clone.clone())
                        .or_insert_with(Vec::new)
                        .push(TestStageEvent {
                            stage: "running".to_string(),
                            status: "stopped".to_string(),
                            timestamp: now_ms(),
                            duration_ms: None,
                            error: None,
                        });
                    break;
                }

                let metrics = harness.lock().unwrap().get_metrics();

                if let Some(error) = metrics.error_message.clone() {
                    let _ = harness.lock().unwrap().stop();
                    mark_run_failed(
                        &orchestrator_runs,
                        &orchestrator_events,
                        &run_id_clone,
                        &metrics,
                        "runtime_failure",
                        error,
                    );
                    break;
                }

                if !metrics.is_running {
                    let message = "test harness stopped before duration elapsed".to_string();
                    mark_run_failed(
                        &orchestrator_runs,
                        &orchestrator_events,
                        &run_id_clone,
                        &metrics,
                        "runtime_stopped",
                        message,
                    );
                    break;
                }

                {
                    let mut series = orchestrator_metrics.lock().unwrap();
                    let run_series = series
                        .entry(run_id_clone.clone())
                        .or_insert_with(HashMap::new);
                    push_metric_sample(run_series, "capture_fps", "fps", metrics.capture_fps);
                    push_metric_sample(
                        run_series,
                        "encode_latency_p95_ms",
                        "ms",
                        metrics.encode_latency_p95_ms,
                    );
                    push_metric_sample(
                        run_series,
                        "decode_latency_p95_ms",
                        "ms",
                        metrics.decode_latency_p95_ms,
                    );
                    push_metric_sample(
                        run_series,
                        "total_latency_p95_ms",
                        "ms",
                        metrics.total_latency_p95_ms,
                    );
                }

                if now_ms().saturating_sub(started_at) >= duration_ms {
                    let metrics = {
                        let mut harness = harness.lock().unwrap();
                        let _ = harness.stop();
                        harness.get_metrics()
                    };
                    let mut runs = orchestrator_runs.lock().unwrap();
                    if let Some(run) = runs.get_mut(&run_id_clone) {
                        run.status = RunStatus::Completed;
                        run.finished_at = Some(now_ms());
                        run.summary = Some(summary_from_metrics(run.started_at, &metrics));
                    }
                    break;
                }
            }
        });

        Ok(run_id)
    }

    fn start_single_window_probe(
        &self,
        scenario_id: String,
        config: TestConfigData,
    ) -> Result<RunId> {
        let run_id = generate_run_id();
        let started_at = now_ms();
        let env_snapshot = self.get_capabilities()?;
        let requested_hwnd = config.window_hwnd.clone();

        let run = TestRun {
            run_id: run_id.clone(),
            scenario_id,
            run_mode: RunMode::Manual,
            status: RunStatus::Running,
            started_at,
            finished_at: None,
            config_snapshot: config.clone(),
            environment_snapshot: env_snapshot,
            summary: None,
        };

        self.runs.lock().unwrap().insert(run_id.clone(), run);
        self.record_stage_event(run_id.clone(), "prepare", "started", None, None);
        self.record_stage_event(run_id.clone(), "capability_check", "started", None, None);

        match list_window_capture_targets() {
            Ok(targets) => {
                self.record_stage_event(
                    run_id.clone(),
                    "capability_check",
                    "completed",
                    None,
                    None,
                );
                let mut selected_window = serde_json::Value::Null;
                let mut first_frame = serde_json::Value::Null;
                let mut media_probe = serde_json::Value::Null;
                let mut encoded_sample = None::<Vec<u8>>;

                if let Some(hwnd_text) = requested_hwnd.as_deref() {
                    self.record_stage_event(
                        run_id.clone(),
                        "capture",
                        "item_probe_started",
                        None,
                        None,
                    );
                    let hwnd = match parse_hwnd(hwnd_text) {
                        Ok(hwnd) => hwnd,
                        Err(error) => {
                            let message = error.to_string();
                            self.record_stage_event(
                                run_id.clone(),
                                "capture",
                                "item_probe_failed",
                                None,
                                Some(message.clone()),
                            );
                            if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                                run.status = RunStatus::Failed;
                                run.finished_at = Some(now_ms());
                                run.summary = Some(TestRunSummary {
                                    total_duration_ms: now_ms().saturating_sub(started_at),
                                    error_message: Some(message),
                                    failure_reason: Some("initialization_failure".to_string()),
                                    ..Default::default()
                                });
                            }
                            return Ok(run_id);
                        }
                    };

                    match probe_window_capture_item(hwnd) {
                        Ok(probe) => {
                            selected_window = serde_json::json!({
                                "requested_hwnd": hwnd_text,
                                "hwnd": format!("0x{:X}", probe.hwnd as usize),
                                "title": probe.title,
                                "class_name": probe.class_name,
                                "width": probe.width,
                                "height": probe.height,
                                "capture_item_created": true,
                            });
                            self.record_stage_event(
                                run_id.clone(),
                                "capture",
                                "item_probe_completed",
                                None,
                                None,
                            );
                        }
                        Err(error) => {
                            let message = error.to_string();
                            self.record_stage_event(
                                run_id.clone(),
                                "capture",
                                "item_probe_failed",
                                None,
                                Some(message.clone()),
                            );
                            if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                                run.status = RunStatus::Failed;
                                run.finished_at = Some(now_ms());
                                run.summary = Some(TestRunSummary {
                                    total_duration_ms: now_ms().saturating_sub(started_at),
                                    error_message: Some(message),
                                    failure_reason: Some("initialization_failure".to_string()),
                                    ..Default::default()
                                });
                            }
                            return Ok(run_id);
                        }
                    }

                    self.record_stage_event(
                        run_id.clone(),
                        "capture",
                        "frame_probe_started",
                        None,
                        None,
                    );
                    match probe_window_first_frame(hwnd, Duration::from_millis(1_000)) {
                        Ok(probe) => {
                            let media_result =
                                self.run_single_window_media_probe(&run_id, &probe.frame, &config);

                            first_frame = serde_json::json!({
                                "hwnd": format!("0x{:X}", probe.hwnd as usize),
                                "title": probe.title,
                                "class_name": probe.class_name,
                                "width": probe.width,
                                "height": probe.height,
                                "byte_len": probe.byte_len,
                                "pixel_format": probe.pixel_format,
                                "captured": true,
                            });
                            self.record_stage_event(
                                run_id.clone(),
                                "capture",
                                "frame_probe_completed",
                                None,
                                None,
                            );

                            match media_result {
                                Ok(probe) => {
                                    encoded_sample = probe.first_access_unit.clone();
                                    media_probe = serde_json::json!({
                                        "encoder": "openh264",
                                        "decoder": "h264_software",
                                        "transport": probe.transport,
                                        "encoded_width": probe.encoded_width,
                                        "encoded_height": probe.encoded_height,
                                        "access_unit_count": probe.access_unit_count,
                                        "encoded_bytes": probe.encoded_bytes,
                                        "keyframe_count": probe.keyframe_count,
                                        "transport_rtp_packet_count": probe.transport_rtp_packet_count,
                                        "transport_payload_bytes": probe.transport_payload_bytes,
                                        "encode_latency_ms": probe.encode_latency_ms,
                                        "decode_latency_ms": probe.decode_latency_ms,
                                        "decoded_frame_count": probe.decoded_frame_count,
                                        "decoded_width": probe.decoded_width,
                                        "decoded_height": probe.decoded_height,
                                        "decoded_pixel_format": probe.decoded_pixel_format,
                                        "render_backend": probe.render_backend,
                                        "render_latency_ms": probe.render_latency_ms,
                                        "rendered_frame_count": probe.rendered_frame_count,
                                    });
                                }
                                Err(error) => {
                                    let message = error.to_string();
                                    self.record_stage_event(
                                        run_id.clone(),
                                        "encode",
                                        "failed",
                                        None,
                                        Some(message.clone()),
                                    );
                                    if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                                        run.status = RunStatus::Failed;
                                        run.finished_at = Some(now_ms());
                                        run.summary = Some(TestRunSummary {
                                            total_duration_ms: now_ms().saturating_sub(started_at),
                                            error_message: Some(message),
                                            failure_reason: Some("runtime_failure".to_string()),
                                            frame_count: 1,
                                            ..Default::default()
                                        });
                                    }
                                    return Ok(run_id);
                                }
                            }
                        }
                        Err(error) => {
                            let message = error.to_string();
                            self.record_stage_event(
                                run_id.clone(),
                                "capture",
                                "frame_probe_failed",
                                None,
                                Some(message.clone()),
                            );
                            if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                                run.status = RunStatus::Failed;
                                run.finished_at = Some(now_ms());
                                run.summary = Some(TestRunSummary {
                                    total_duration_ms: now_ms().saturating_sub(started_at),
                                    error_message: Some(message),
                                    failure_reason: Some("runtime_failure".to_string()),
                                    ..Default::default()
                                });
                            }
                            return Ok(run_id);
                        }
                    }
                }

                let artifact = serde_json::json!({
                    "targets": targets,
                    "target_count": targets.len(),
                    "selected_window": selected_window,
                    "first_frame": first_frame,
                    "media_probe": media_probe,
                });
                let data =
                    serde_json::to_string_pretty(&artifact).unwrap_or_else(|_| "[]".to_string());
                let size_bytes = data.len();

                self.run_artifacts
                    .lock()
                    .unwrap()
                    .entry(run_id.clone())
                    .or_insert_with(Vec::new)
                    .push(Artifact {
                        artifact_id: format!("artifact_{}", now_ms()),
                        kind: "structured_log".to_string(),
                        run_id: run_id.clone(),
                        created_at: now_ms(),
                        data,
                        metadata: Some(ArtifactMetadata {
                            width: None,
                            height: None,
                            format: Some("json".to_string()),
                            size_bytes: Some(size_bytes),
                        }),
                    });

                if let Some(sample) = encoded_sample {
                    let sample_size = sample.len();
                    self.run_artifacts
                        .lock()
                        .unwrap()
                        .entry(run_id.clone())
                        .or_insert_with(Vec::new)
                        .push(Artifact {
                            artifact_id: format!("encoded_{}", now_ms()),
                            kind: "encoded_sample".to_string(),
                            run_id: run_id.clone(),
                            created_at: now_ms(),
                            data: base64::engine::general_purpose::STANDARD.encode(sample),
                            metadata: Some(ArtifactMetadata {
                                width: None,
                                height: None,
                                format: Some("h264_annex_b".to_string()),
                                size_bytes: Some(sample_size),
                            }),
                        });
                }

                if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                    run.status = RunStatus::Completed;
                    run.finished_at = Some(now_ms());
                    run.summary = Some(TestRunSummary {
                        total_duration_ms: now_ms().saturating_sub(started_at),
                        frame_count: if first_frame.is_null() { 0 } else { 1 },
                        encode_latency_p50: media_probe
                            .get("encode_latency_ms")
                            .and_then(|value| value.as_f64()),
                        encode_latency_p95: media_probe
                            .get("encode_latency_ms")
                            .and_then(|value| value.as_f64()),
                        decode_latency_p50: media_probe
                            .get("decode_latency_ms")
                            .and_then(|value| value.as_f64()),
                        decode_latency_p95: media_probe
                            .get("decode_latency_ms")
                            .and_then(|value| value.as_f64()),
                        ..Default::default()
                    });
                }
                self.record_stage_event(run_id.clone(), "summarize", "completed", None, None);
            }
            Err(error) => {
                let message = error.to_string();
                self.record_stage_event(
                    run_id.clone(),
                    "capability_check",
                    "failed",
                    None,
                    Some(message.clone()),
                );
                if let Some(run) = self.runs.lock().unwrap().get_mut(&run_id) {
                    run.status = RunStatus::Failed;
                    run.finished_at = Some(now_ms());
                    run.summary = Some(TestRunSummary {
                        total_duration_ms: now_ms().saturating_sub(started_at),
                        error_message: Some(message),
                        failure_reason: Some("capability_mismatch".to_string()),
                        ..Default::default()
                    });
                }
            }
        }

        Ok(run_id)
    }

    fn run_single_window_media_probe(
        &self,
        run_id: &str,
        frame: &CapturedFrame,
        config: &TestConfigData,
    ) -> Result<SingleWindowMediaProbe> {
        let fps = config.fps.unwrap_or(30).max(1);
        let encode_frame = Self::openh264_compatible_frame(frame)?;

        self.record_stage_event(run_id.to_string(), "encode", "started", None, None);
        let encode_started = std::time::Instant::now();
        let mut encoder =
            mrd_encode_openh264::OpenH264Encoder::new(encode_frame.width, encode_frame.height, fps)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let access_units = encoder
            .encode(&encode_frame)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let encode_latency_ms = encode_started.elapsed().as_secs_f64() * 1000.0;

        if access_units.is_empty() {
            anyhow::bail!("OpenH264 produced no access units");
        }

        self.record_stage_event(run_id.to_string(), "encode", "completed", None, None);

        let encoded_bytes = access_units
            .iter()
            .map(|unit| unit.bytes.len())
            .sum::<usize>();
        let keyframe_count = access_units.iter().filter(|unit| unit.is_keyframe).count();
        let first_access_unit = access_units.first().map(|unit| unit.bytes.clone());

        let transport_started_status = if config.transport_kind.as_deref() == Some("webrtc") {
            "webrtc_rtp_started"
        } else {
            "loopback_started"
        };
        let transport_completed_status = if config.transport_kind.as_deref() == Some("webrtc") {
            "webrtc_rtp_completed"
        } else {
            "loopback_completed"
        };
        self.record_stage_event(
            run_id.to_string(),
            "transport",
            transport_started_status,
            None,
            None,
        );
        let transport_probe =
            Self::transport_single_window_access_units(&access_units, fps, config)?;
        self.record_stage_event(
            run_id.to_string(),
            "transport",
            transport_completed_status,
            None,
            None,
        );

        self.record_stage_event(run_id.to_string(), "decode", "started", None, None);
        let decode_started = std::time::Instant::now();
        let mut decoder = mrd_decode::create_decoder("h264_software")
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        for unit in &transport_probe.access_units {
            decoder
                .push_access_unit(&unit.bytes)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        let decoded_frames = decoder.drain_decoded_frames();
        let decoded_frame_count = decoded_frames.len();
        let decode_latency_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
        let decode_status = if decoded_frame_count > 0 {
            "completed"
        } else {
            "accepted_no_frame_drain"
        };
        self.record_stage_event(run_id.to_string(), "decode", decode_status, None, None);
        let decoded_width = decoded_frames.first().map(|frame| frame.width);
        let decoded_height = decoded_frames.first().map(|frame| frame.height);
        let decoded_pixel_format = decoded_frames.first().map(Self::decoded_frame_format);

        let (render_backend, render_latency_ms, rendered_frame_count) = if decoded_frames.is_empty()
        {
            self.record_stage_event(
                run_id.to_string(),
                "render",
                "skipped_no_decoded_frame",
                None,
                None,
            );
            (None, None, 0)
        } else {
            self.record_stage_event(run_id.to_string(), "render", "started", None, None);
            let render_started = std::time::Instant::now();
            let factory = mrd_render_d3d11::D3d11RendererFactory;
            let mut renderer = factory
                .create()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            renderer
                .attach_target(RenderTarget::WindowHandle(0))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            for frame in &decoded_frames {
                renderer
                    .upload_frame(Self::decoded_frame_to_render_frame(frame))
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            }
            let render_latency = render_started.elapsed().as_secs_f64() * 1000.0;
            let uploaded = renderer.snapshot().uploaded_frame_count as usize;
            self.record_stage_event(run_id.to_string(), "render", "completed", None, None);
            (Some("d3d11".to_string()), Some(render_latency), uploaded)
        };

        Ok(SingleWindowMediaProbe {
            transport: transport_probe.transport,
            encoded_width: encode_frame.width,
            encoded_height: encode_frame.height,
            access_unit_count: transport_probe.access_units.len(),
            encoded_bytes,
            keyframe_count,
            transport_rtp_packet_count: transport_probe.rtp_packet_count,
            transport_payload_bytes: transport_probe.payload_bytes,
            encode_latency_ms,
            decode_latency_ms,
            decoded_frame_count,
            decoded_width,
            decoded_height,
            decoded_pixel_format,
            render_backend,
            render_latency_ms,
            rendered_frame_count,
            first_access_unit,
        })
    }

    fn transport_single_window_access_units(
        access_units: &[EncodedAccessUnit],
        fps: u32,
        config: &TestConfigData,
    ) -> Result<SingleWindowTransportProbe> {
        if config.transport_kind.as_deref() != Some("webrtc") {
            let payload_bytes = access_units
                .iter()
                .map(|access_unit| access_unit.bytes.len())
                .sum::<usize>();
            return Ok(SingleWindowTransportProbe {
                transport: "loopback".to_string(),
                access_units: access_units.to_vec(),
                rtp_packet_count: 0,
                payload_bytes,
            });
        }

        let mut sender = mrd_transport_webrtc::H264RtpSender::new(
            "single-window-video",
            "single-window-stream",
            fps,
            1200,
        );
        let mut ingress = mrd_transport_webrtc::H264RtpIngress::default();
        let mut reassembled = Vec::new();
        let mut rtp_packet_count = 0usize;
        let mut payload_bytes = 0usize;

        for access_unit in access_units {
            let packets = sender
                .packetize_access_unit(access_unit)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            for packet in packets {
                rtp_packet_count += 1;
                payload_bytes += packet.payload.len();
                if let Some(received) = ingress.push_packet(
                    &packet.payload,
                    packet.header.marker,
                    packet.header.sequence_number,
                    access_unit.timestamp_us,
                ) {
                    reassembled.push(received);
                }
            }
        }

        if reassembled.is_empty() {
            anyhow::bail!("WebRTC RTP loopback produced no H264 access units");
        }

        Ok(SingleWindowTransportProbe {
            transport: "webrtc_rtp_loopback".to_string(),
            access_units: reassembled,
            rtp_packet_count,
            payload_bytes,
        })
    }

    fn openh264_compatible_frame(frame: &CapturedFrame) -> Result<CapturedFrame> {
        let width = frame.width - (frame.width % 2);
        let height = frame.height - (frame.height % 2);

        if width == 0 || height == 0 {
            anyhow::bail!(
                "captured frame is too small for OpenH264: {}x{}",
                frame.width,
                frame.height
            );
        }

        let bytes_per_pixel = match frame.pixel_format {
            FramePixelFormat::Bgra32 | FramePixelFormat::Rgba32 => 4,
            FramePixelFormat::Rgb24 => 3,
        };
        let source_stride = frame
            .width
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| anyhow::anyhow!("captured frame stride overflow"))?;
        let target_stride = width
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| anyhow::anyhow!("encoded frame stride overflow"))?;
        let expected_len = source_stride
            .checked_mul(frame.height)
            .ok_or_else(|| anyhow::anyhow!("captured frame buffer size overflow"))?;

        if frame.data.len() != expected_len {
            anyhow::bail!(
                "captured frame bytes mismatch: expected {}, got {}",
                expected_len,
                frame.data.len()
            );
        }

        if width == frame.width && height == frame.height {
            return Ok(frame.clone());
        }

        let mut data = Vec::with_capacity(target_stride * height);
        for row in 0..height {
            let start = row * source_stride;
            data.extend_from_slice(&frame.data[start..start + target_stride]);
        }

        Ok(CapturedFrame {
            width,
            height,
            pixel_format: frame.pixel_format,
            timestamp_us: frame.timestamp_us,
            data,
        })
    }

    fn decoded_frame_format(frame: &DecodedFrame) -> String {
        match &frame.data {
            DecodedFrameData::CpuRgb24(_) => "Rgb24".to_string(),
            DecodedFrameData::CpuBgra32(_) => "Bgra32".to_string(),
            DecodedFrameData::CpuNv12 { .. } => "Nv12".to_string(),
            #[cfg(windows)]
            DecodedFrameData::D3D11SharedNv12 { .. } => "D3D11SharedNv12".to_string(),
        }
    }

    fn decoded_frame_to_render_frame(frame: &DecodedFrame) -> RenderFrame {
        match &frame.data {
            DecodedFrameData::CpuRgb24(data) => {
                RenderFrame::from_rgb24(frame.width, frame.height, data.clone())
            }
            DecodedFrameData::CpuBgra32(data) => {
                RenderFrame::from_bgra32(frame.width, frame.height, data.clone())
            }
            DecodedFrameData::CpuNv12 { data, pitch } => RenderFrame::from_rgb24(
                frame.width,
                frame.height,
                Self::cpu_nv12_to_rgb24(data, frame.width, frame.height, *pitch),
            ),
            #[cfg(windows)]
            DecodedFrameData::D3D11SharedNv12 { shared_handle, .. } => {
                RenderFrame::from_d3d11_shared_nv12(frame.width, frame.height, *shared_handle)
            }
        }
    }

    fn cpu_nv12_to_rgb24(nv12: &[u8], width: usize, height: usize, pitch: usize) -> Vec<u8> {
        let mut rgb = vec![0_u8; width * height * 3];
        let uv_base = pitch * height;
        let mut out_idx = 0;

        for y in 0..height {
            let uv_row_start = uv_base + (y / 2) * pitch;
            for x in 0..width {
                let y_sample = nv12[y * pitch + x] as i32 - 16;
                let uv_offset = uv_row_start + (x / 2) * 2;
                let u = nv12[uv_offset] as i32 - 128;
                let v = nv12[uv_offset + 1] as i32 - 128;

                let r = (298 * y_sample + 409 * v + 128) >> 8;
                let g = (298 * y_sample - 100 * u - 208 * v + 128) >> 8;
                let b = (298 * y_sample + 516 * u + 128) >> 8;

                rgb[out_idx] = r.clamp(0, 255) as u8;
                rgb[out_idx + 1] = g.clamp(0, 255) as u8;
                rgb[out_idx + 2] = b.clamp(0, 255) as u8;
                out_idx += 3;
            }
        }

        rgb
    }

    /// Stop a running test
    pub fn stop_run(&self, run_id: &str) -> Result<()> {
        let mut runs = self.runs.lock().unwrap();
        if let Some(run) = runs.get_mut(run_id) {
            if run.status == RunStatus::Running {
                let metrics = {
                    let mut harness = self.harness.lock().unwrap();
                    let _ = harness.stop();
                    harness.get_metrics()
                };
                run.status = RunStatus::Cancelled;
                run.finished_at = Some(now_ms());
                run.summary = Some(summary_from_metrics(run.started_at, &metrics));
            }
        }
        Ok(())
    }

    /// Get a test run
    pub fn get_run(&self, run_id: &str) -> Option<TestRun> {
        self.runs.lock().unwrap().get(run_id).cloned()
    }

    /// List test runs
    pub fn list_runs(
        &self,
        scenario_id: Option<String>,
        status: Option<String>,
        limit: Option<usize>,
    ) -> Vec<TestRun> {
        let runs = self.runs.lock().unwrap();
        let mut result: Vec<TestRun> = runs.values().cloned().collect();

        // Apply filters
        if let Some(sid) = scenario_id {
            result.retain(|r| r.scenario_id == sid);
        }
        if let Some(s) = status {
            if let Ok(run_status) = serde_json::from_str::<RunStatus>(&format!("\"{}\"", s)) {
                result.retain(|r| r.status == run_status);
            }
        }

        // Sort by started_at descending
        result.sort_by(|a, b| b.started_at.cmp(&a.started_at));

        // Apply limit
        if let Some(limit) = limit {
            result.truncate(limit);
        }

        result
    }

    /// Update run metrics from harness
    pub fn update_run_metrics(&self, run_id: &str, metrics: &HarnessMetrics) {
        let mut runs = self.runs.lock().unwrap();
        if let Some(run) = runs.get_mut(run_id) {
            if run.summary.is_none() {
                run.summary = Some(TestRunSummary {
                    total_duration_ms: now_ms() - run.started_at,
                    capture_fps: Some(metrics.capture_fps),
                    encode_latency_p50: Some(metrics.encode_latency_p50_ms),
                    encode_latency_p95: Some(metrics.encode_latency_p95_ms),
                    decode_latency_p50: Some(metrics.decode_latency_p50_ms),
                    decode_latency_p95: Some(metrics.decode_latency_p95_ms),
                    total_latency_p95: Some(metrics.total_latency_p95_ms),
                    dropped_frames: metrics.dropped_frames,
                    frame_count: metrics.frame_count,
                    ..Default::default()
                });
            }
        }
    }

    /// Record a stage event
    pub fn record_stage_event(
        &self,
        run_id: String,
        stage: &str,
        status: &str,
        duration_ms: Option<u64>,
        error: Option<String>,
    ) {
        let event = TestStageEvent {
            stage: stage.to_string(),
            status: status.to_string(),
            timestamp: now_ms(),
            duration_ms,
            error,
        };

        self.run_events
            .lock()
            .unwrap()
            .entry(run_id)
            .or_insert_with(Vec::new)
            .push(event);
    }

    /// Get run events
    pub fn get_run_events(&self, run_id: &str) -> Vec<TestStageEvent> {
        self.run_events
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get run metrics.
    pub fn get_run_metrics(&self, run_id: &str) -> HashMap<String, MetricSeries> {
        self.run_metrics
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get run artifacts.
    pub fn get_run_artifacts(&self, run_id: &str) -> Vec<Artifact> {
        self.run_artifacts
            .lock()
            .unwrap()
            .get(run_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Save a preset
    pub fn save_preset(
        &self,
        name: String,
        description: String,
        scenario_id: String,
        config: TestConfigData,
    ) -> String {
        let preset_id = generate_preset_id();
        let preset = TestPreset {
            preset_id: preset_id.clone(),
            name,
            description,
            scenario_id,
            config,
            tags: None,
            created_at: now_ms() / 1000,
        };

        self.presets
            .lock()
            .unwrap()
            .insert(preset_id.clone(), preset);
        preset_id
    }

    /// List presets
    pub fn list_presets(&self) -> Vec<TestPreset> {
        self.presets.lock().unwrap().values().cloned().collect()
    }

    /// Delete a preset
    pub fn delete_preset(&self, preset_id: &str) -> Result<()> {
        self.presets
            .lock()
            .unwrap()
            .remove(preset_id)
            .ok_or_else(|| anyhow::anyhow!("Preset not found"))?;
        Ok(())
    }

    /// Get current harness chain
    pub fn get_harness_chain(&self) -> Option<TestChain> {
        self.current_harness_chain.lock().unwrap().clone()
    }

    /// Set harness chain
    pub fn set_harness_chain(&self, chain: TestChain) {
        *self.current_harness_chain.lock().unwrap() = Some(chain);
    }
}

impl Default for TestOrchestrator {
    fn default() -> Self {
        Self::new(Arc::new(Mutex::new(
            TestHarness::new().expect("failed to create default TestHarness"),
        )))
    }
}

fn generate_run_id() -> String {
    format!("run_{}", now_ms())
}

fn generate_preset_id() -> String {
    format!("preset_{}", now_ms())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

pub fn list_window_capture_targets() -> Result<Vec<WindowCaptureTarget>> {
    list_window_capture_targets_impl()
}

fn parse_hwnd(input: &str) -> Result<isize> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("window hwnd is empty");
    }

    let value = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        usize::from_str_radix(hex, 16)
            .map_err(|error| anyhow::anyhow!("invalid window hwnd '{trimmed}': {error}"))?
    } else {
        trimmed
            .parse::<usize>()
            .map_err(|error| anyhow::anyhow!("invalid window hwnd '{trimmed}': {error}"))?
    };

    Ok(value as isize)
}

#[cfg(windows)]
fn list_window_capture_targets_impl() -> Result<Vec<WindowCaptureTarget>> {
    let targets = mrd_capture_winrt::enumerate_window_capture_targets()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(targets
        .into_iter()
        .map(|target| WindowCaptureTarget {
            hwnd: format!("0x{:X}", target.hwnd as usize),
            title: target.title,
            class_name: target.class_name,
            width: target.width,
            height: target.height,
            process_id: target.process_id,
        })
        .collect())
}

#[cfg(not(windows))]
fn list_window_capture_targets_impl() -> Result<Vec<WindowCaptureTarget>> {
    anyhow::bail!("WinRT window capture is only available on Windows")
}

#[cfg(windows)]
fn probe_window_capture_item(hwnd: isize) -> Result<WindowCaptureItemProbe> {
    let probe = mrd_capture_winrt::probe_window_capture_item(hwnd)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(WindowCaptureItemProbe {
        hwnd: probe.hwnd,
        title: probe.title,
        class_name: probe.class_name,
        width: probe.width,
        height: probe.height,
    })
}

#[cfg(not(windows))]
fn probe_window_capture_item(_hwnd: isize) -> Result<WindowCaptureItemProbe> {
    anyhow::bail!("WinRT window capture is only available on Windows")
}

#[cfg(windows)]
fn probe_window_first_frame(hwnd: isize, timeout: Duration) -> Result<WindowCaptureFrameProbe> {
    let probe = mrd_capture_winrt::probe_window_first_frame(hwnd, timeout)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    Ok(WindowCaptureFrameProbe {
        hwnd: probe.hwnd,
        title: probe.title,
        class_name: probe.class_name,
        width: probe.width,
        height: probe.height,
        byte_len: probe.byte_len,
        pixel_format: format!("{:?}", probe.pixel_format),
        frame: probe.frame,
    })
}

#[cfg(not(windows))]
fn probe_window_first_frame(_hwnd: isize, _timeout: Duration) -> Result<WindowCaptureFrameProbe> {
    anyhow::bail!("WinRT window capture is only available on Windows")
}

impl Default for TestRunSummary {
    fn default() -> Self {
        Self {
            total_duration_ms: 0,
            first_frame_latency_ms: None,
            capture_fps: None,
            encode_latency_p50: None,
            encode_latency_p95: None,
            decode_latency_p50: None,
            decode_latency_p95: None,
            total_latency_p95: None,
            dropped_frames: 0,
            frame_count: 0,
            error_message: None,
            failure_reason: None,
        }
    }
}

fn summary_from_metrics(started_at: u64, metrics: &HarnessMetrics) -> TestRunSummary {
    TestRunSummary {
        total_duration_ms: now_ms().saturating_sub(started_at),
        capture_fps: Some(metrics.capture_fps),
        encode_latency_p50: Some(metrics.encode_latency_p50_ms),
        encode_latency_p95: Some(metrics.encode_latency_p95_ms),
        decode_latency_p50: Some(metrics.decode_latency_p50_ms),
        decode_latency_p95: Some(metrics.decode_latency_p95_ms),
        total_latency_p95: Some(metrics.total_latency_p95_ms),
        dropped_frames: metrics.dropped_frames,
        frame_count: metrics.frame_count,
        error_message: metrics.error_message.clone(),
        ..Default::default()
    }
}

fn harness_config_from_data(config: &TestConfigData) -> HarnessConfig {
    HarnessConfig {
        resolution: config.resolution.map(|[width, height]| (width, height)),
        fps: config.fps,
        bitrate: config.bitrate,
        renderer: match (config.render_display, config.renderer_type.as_deref()) {
            (Some(true), Some("d3d11")) => Some(RendererType::D3d11),
            _ => None,
        },
    }
}

fn mark_run_failed(
    runs: &Arc<Mutex<HashMap<RunId, TestRun>>>,
    events: &Arc<Mutex<HashMap<RunId, Vec<TestStageEvent>>>>,
    run_id: &str,
    metrics: &HarnessMetrics,
    failure_reason: &str,
    error_message: String,
) {
    let mut should_record_event = false;

    {
        let mut runs = runs.lock().unwrap();
        if let Some(run) = runs.get_mut(run_id) {
            if run.status == RunStatus::Running {
                let mut summary = summary_from_metrics(run.started_at, metrics);
                summary.error_message = Some(error_message.clone());
                summary.failure_reason = Some(failure_reason.to_string());
                run.status = RunStatus::Failed;
                run.finished_at = Some(now_ms());
                run.summary = Some(summary);
                should_record_event = true;
            }
        }
    }

    if should_record_event {
        events
            .lock()
            .unwrap()
            .entry(run_id.to_string())
            .or_insert_with(Vec::new)
            .push(TestStageEvent {
                stage: "running".to_string(),
                status: "failed".to_string(),
                timestamp: now_ms(),
                duration_ms: None,
                error: Some(error_message),
            });
    }
}

fn push_metric_sample(
    run_series: &mut HashMap<String, MetricSeries>,
    metric_name: &str,
    unit: &str,
    value: f64,
) {
    let series = run_series
        .entry(metric_name.to_string())
        .or_insert_with(|| MetricSeries {
            metric_name: metric_name.to_string(),
            unit: unit.to_string(),
            samples: Vec::new(),
            aggregation: None,
        });

    series.samples.push(MetricDataPoint {
        timestamp: now_ms(),
        value,
    });
    series.aggregation = Some(compute_aggregation(&series.samples));
}

fn compute_aggregation(samples: &[MetricDataPoint]) -> MetricAggregation {
    if samples.is_empty() {
        return MetricAggregation {
            min: None,
            max: None,
            mean: None,
            p50: None,
            p95: None,
            p99: None,
        };
    }

    let mut values: Vec<f64> = samples.iter().map(|sample| sample.value).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let sum: f64 = values.iter().sum();
    let last = values.len().saturating_sub(1);

    MetricAggregation {
        min: values.first().copied(),
        max: values.last().copied(),
        mean: Some(sum / values.len() as f64),
        p50: Some(values[values.len() / 2]),
        p95: Some(values[((values.len() * 95) / 100).min(last)]),
        p99: Some(values[((values.len() * 99) / 100).min(last)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env() -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            cpu_brand: "test-cpu".to_string(),
            cpu_cores: 8,
            memory_gb: 16,
            gpu_info: "test-gpu".to_string(),
            available_encoders: vec!["openh264".to_string()],
            available_decoders: vec!["software".to_string()],
        }
    }

    #[test]
    fn scenario_dispatch_rejects_unsupported_scenarios() {
        let orchestrator = TestOrchestrator::default();
        let error = orchestrator
            .scenario_to_chain("capture.dxgi", &TestConfigData::default())
            .unwrap_err();

        assert!(error.to_string().contains("Unsupported test scenario"));
    }

    #[test]
    fn list_scenarios_includes_single_window_local_probe() {
        let orchestrator = TestOrchestrator::default();
        let scenario = orchestrator
            .list_scenarios()
            .into_iter()
            .find(|scenario| scenario.scenario_id == "single_window.local")
            .expect("single window probe scenario should be registered");

        assert_eq!(scenario.scenario_kind, ScenarioKind::E2eLocal);
        assert!(!scenario.supports_matrix);
        assert_eq!(
            scenario.default_config.capture_type.as_deref(),
            Some("winrt")
        );
        assert_eq!(
            scenario.default_config.input_source.as_deref(),
            Some("window")
        );
        assert_eq!(
            scenario.default_config.transport_kind.as_deref(),
            Some("webrtc")
        );
        assert!(scenario
            .component_scope
            .iter()
            .any(|scope| scope == "winrt"));
        assert!(scenario
            .component_scope
            .iter()
            .any(|scope| scope == "webrtc"));
    }

    #[test]
    fn parse_hwnd_accepts_hex_and_decimal() {
        assert_eq!(parse_hwnd("0x2A").unwrap(), 42);
        assert_eq!(parse_hwnd("42").unwrap(), 42);
    }

    #[test]
    fn matrix_dispatch_maps_explicit_encoder_decoder_pairs() {
        let orchestrator = TestOrchestrator::default();
        let openh264_config = TestConfigData {
            capture_type: Some("dxgi".to_string()),
            encoder_type: Some("openh264".to_string()),
            decoder_type: Some("software".to_string()),
            ..Default::default()
        };
        let nvenc_decode_config = TestConfigData {
            capture_type: Some("dxgi".to_string()),
            encoder_type: Some("nvenc_h264".to_string()),
            decoder_type: Some("nvdec".to_string()),
            ..Default::default()
        };
        let nvenc_encode_config = TestConfigData {
            capture_type: Some("dxgi".to_string()),
            encoder_type: Some("nvenc_h264".to_string()),
            decoder_type: Some("software".to_string()),
            ..Default::default()
        };

        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &openh264_config)
                .unwrap(),
            TestChain::Custom {
                capture: CaptureType::Dxgi,
                encoder: EncoderType::OpenH264,
                decoder: DecoderType::Software,
            }
        );
        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &nvenc_decode_config)
                .unwrap(),
            TestChain::Custom {
                capture: CaptureType::Dxgi,
                encoder: EncoderType::NvencH264,
                decoder: DecoderType::Nvdec,
            }
        );
        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &nvenc_encode_config)
                .unwrap(),
            TestChain::Custom {
                capture: CaptureType::Dxgi,
                encoder: EncoderType::NvencH264,
                decoder: DecoderType::Software,
            }
        );
    }

    #[test]
    fn harness_config_requires_explicit_render_display_for_d3d11() {
        let legacy_config = TestConfigData {
            renderer_type: Some("d3d11".to_string()),
            ..Default::default()
        };
        let disabled_config = TestConfigData {
            renderer_type: Some("d3d11".to_string()),
            render_display: Some(false),
            ..Default::default()
        };
        let enabled_config = TestConfigData {
            renderer_type: Some("d3d11".to_string()),
            render_display: Some(true),
            ..Default::default()
        };

        assert_eq!(harness_config_from_data(&legacy_config).renderer, None);
        assert_eq!(harness_config_from_data(&disabled_config).renderer, None);
        assert_eq!(
            harness_config_from_data(&enabled_config).renderer,
            Some(RendererType::D3d11)
        );
    }

    #[test]
    fn runtime_harness_error_marks_run_failed() {
        let orchestrator = TestOrchestrator::default();
        let run_id = "run_runtime_error".to_string();
        let started_at = now_ms();

        orchestrator.runs.lock().unwrap().insert(
            run_id.clone(),
            TestRun {
                run_id: run_id.clone(),
                scenario_id: "encode.openh264".to_string(),
                run_mode: RunMode::Manual,
                status: RunStatus::Running,
                started_at,
                finished_at: None,
                config_snapshot: TestConfigData::default(),
                environment_snapshot: test_env(),
                summary: None,
            },
        );

        let metrics = HarnessMetrics {
            is_running: false,
            frame_count: 12,
            error_message: Some("gpu unavailable".to_string()),
            ..Default::default()
        };

        mark_run_failed(
            &orchestrator.runs,
            &orchestrator.run_events,
            &run_id,
            &metrics,
            "runtime_failure",
            "gpu unavailable".to_string(),
        );

        let run = orchestrator.get_run(&run_id).unwrap();
        let summary = run.summary.unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(summary.frame_count, 12);
        assert_eq!(summary.error_message.as_deref(), Some("gpu unavailable"));
        assert_eq!(summary.failure_reason.as_deref(), Some("runtime_failure"));

        let events = orchestrator.get_run_events(&run_id);
        assert!(events
            .iter()
            .any(|event| { event.stage == "running" && event.status == "failed" }));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "manual smoke test: requires a visible capturable window and WinRT capture access"]
    fn single_window_local_probe_smoke() {
        let targets = list_window_capture_targets().expect("failed to list capture targets");
        let target = targets
            .into_iter()
            .find(|target| {
                target.width >= 32 && target.height >= 32 && !target.title.trim().is_empty()
            })
            .expect("no visible capture target found");

        println!(
            "capturing hwnd={} title={:?} size={}x{} pid={}",
            target.hwnd, target.title, target.width, target.height, target.process_id
        );

        let orchestrator = TestOrchestrator::default();
        let run_id = orchestrator
            .start_run(
                "single_window.local".to_string(),
                TestConfigData {
                    capture_type: Some("winrt".to_string()),
                    input_source: Some("window".to_string()),
                    window_hwnd: Some(target.hwnd.clone()),
                    window_title: Some(target.title.clone()),
                    encoder_type: Some("openh264".to_string()),
                    decoder_type: Some("software".to_string()),
                    renderer_type: Some("d3d11".to_string()),
                    transport_kind: Some("webrtc".to_string()),
                    duration_ms: Some(1_000),
                    fps: Some(30),
                    ..Default::default()
                },
            )
            .expect("failed to start single-window probe");

        let run = orchestrator
            .get_run(&run_id)
            .expect("probe run should be recorded");
        println!(
            "run status={:?} summary={}",
            run.status,
            serde_json::to_string_pretty(&run.summary).unwrap()
        );

        let events = orchestrator.get_run_events(&run_id);
        println!("events={}", serde_json::to_string_pretty(&events).unwrap());

        let artifacts = orchestrator.get_run_artifacts(&run_id);
        for artifact in &artifacts {
            println!(
                "artifact kind={} metadata={}",
                artifact.kind,
                serde_json::to_string_pretty(&artifact.metadata).unwrap()
            );
            if artifact.kind == "structured_log" {
                println!("{}", artifact.data);
            }
        }

        assert_eq!(run.status, RunStatus::Completed);
        assert!(artifacts
            .iter()
            .any(|artifact| artifact.kind == "structured_log"
                && artifact
                    .data
                    .contains("\"transport\": \"webrtc_rtp_loopback\"")
                && artifact.data.contains("\"rendered_frame_count\"")));
    }
}
