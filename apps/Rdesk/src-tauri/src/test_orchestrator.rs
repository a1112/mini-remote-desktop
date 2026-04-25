//! Test Orchestrator - Unified test execution and management
//!
//! This module provides the test orchestrator that manages test scenarios,
//! runs, metrics collection, and artifact storage.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::test_harness::{TestChain, HarnessMetrics, TestHarness};
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
    pub transport_kind: Option<String>,
    pub resolution: Option<[usize; 2]>,
    pub fps: Option<u32>,
    pub bitrate: Option<u32>,
    pub duration_ms: Option<u64>,
    pub warmup_ms: Option<u64>,
    pub repeat_count: Option<u32>,
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
            "custom" | "matrix" => match config.encoder_type.as_deref() {
                Some("openh264") => Ok(TestChain::OpenH264),
                Some("nvenc_h264") if config.decoder_type.as_deref() == Some("nvdec") => {
                    Ok(TestChain::NvencNvdec)
                }
                Some("nvenc_h264") => Ok(TestChain::NvencOnly),
                Some(other) => anyhow::bail!("Unsupported encoder for {}: {}", scenario_id, other),
                None => anyhow::bail!("Missing encoder_type for {}", scenario_id),
            },
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
        let gpu_info = hw_info.gpu_info
            .iter()
            .map(|g| g.name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        Ok(EnvironmentSnapshot {
            cpu_brand: hw_info.cpu_info.name,
            cpu_cores: hw_info.cpu_info.cores,
            memory_gb: (hw_info.total_memory_mb / 1024) as u32,
            gpu_info: if gpu_info.is_empty() { "Unknown".to_string() } else { gpu_info },
            available_encoders,
            available_decoders,
        })
    }

    /// Start a new test run
    pub fn start_run(&self, scenario_id: String, config: TestConfigData) -> Result<RunId> {
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
                    orchestrator_events.lock().unwrap()
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
                    let run_series = series.entry(run_id_clone.clone()).or_insert_with(HashMap::new);
                    push_metric_sample(run_series, "capture_fps", "fps", metrics.capture_fps);
                    push_metric_sample(run_series, "encode_latency_p95_ms", "ms", metrics.encode_latency_p95_ms);
                    push_metric_sample(run_series, "decode_latency_p95_ms", "ms", metrics.decode_latency_p95_ms);
                    push_metric_sample(run_series, "total_latency_p95_ms", "ms", metrics.total_latency_p95_ms);
                }

                if now_ms().saturating_sub(started_at) >= duration_ms {
                    let _ = harness.lock().unwrap().stop();
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

    /// Stop a running test
    pub fn stop_run(&self, run_id: &str) -> Result<()> {
        let mut runs = self.runs.lock().unwrap();
        if let Some(run) = runs.get_mut(run_id) {
            if run.status == RunStatus::Running {
                let metrics = self.harness.lock().unwrap().get_metrics();
                let _ = self.harness.lock().unwrap().stop();
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
    pub fn list_runs(&self, scenario_id: Option<String>, status: Option<String>, limit: Option<usize>) -> Vec<TestRun> {
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
    pub fn record_stage_event(&self, run_id: String, stage: &str, status: &str, duration_ms: Option<u64>, error: Option<String>) {
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
    pub fn save_preset(&self, name: String, description: String, scenario_id: String, config: TestConfigData) -> String {
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

        self.presets.lock().unwrap().insert(preset_id.clone(), preset);
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
            TestHarness::new().expect("failed to create default TestHarness")
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
    fn matrix_dispatch_maps_explicit_encoder_decoder_pairs() {
        let orchestrator = TestOrchestrator::default();
        let openh264_config = TestConfigData {
            encoder_type: Some("openh264".to_string()),
            ..Default::default()
        };
        let nvenc_decode_config = TestConfigData {
            encoder_type: Some("nvenc_h264".to_string()),
            decoder_type: Some("nvdec".to_string()),
            ..Default::default()
        };
        let nvenc_encode_config = TestConfigData {
            encoder_type: Some("nvenc_h264".to_string()),
            decoder_type: Some("software".to_string()),
            ..Default::default()
        };

        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &openh264_config)
                .unwrap(),
            TestChain::OpenH264
        );
        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &nvenc_decode_config)
                .unwrap(),
            TestChain::NvencNvdec
        );
        assert_eq!(
            orchestrator
                .scenario_to_chain("matrix", &nvenc_encode_config)
                .unwrap(),
            TestChain::NvencOnly
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
        assert!(events.iter().any(|event| {
            event.stage == "running" && event.status == "failed"
        }));
    }
}
