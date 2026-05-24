use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Networks, Pid, System};

#[derive(Debug, Clone, Default)]
struct GpuSample {
    usage_percent: Option<f32>,
    memory_used_mb: Option<u64>,
    memory_total_mb: Option<u64>,
    scope: MetricScope,
}

#[derive(Debug, Clone, Default)]
struct NetworkSample {
    rx_bps: u64,
    tx_bps: u64,
    available: bool,
    scope: MetricScope,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum MetricScope {
    Process,
    System,
    #[default]
    Unavailable,
}

impl MetricScope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::System => "system",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTarget {
    #[default]
    MrdService,
    System,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceSnapshotRequest {
    #[serde(default)]
    pub target: ResourceTarget,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemResourceSnapshot {
    pub target_name: String,
    pub target_pid: Option<u32>,
    pub target_found: bool,
    pub cpu_metrics_available: bool,
    pub cpu_usage_percent: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub memory_usage_percent: f32,
    pub gpu_usage_percent: Option<f32>,
    pub gpu_memory_used_mb: Option<u64>,
    pub gpu_memory_total_mb: Option<u64>,
    pub gpu_metrics_available: bool,
    pub gpu_metrics_scope: String,
    pub network_rx_bps: u64,
    pub network_tx_bps: u64,
    pub network_metrics_available: bool,
    pub network_metrics_scope: String,
    pub sampled_at_ms: u64,
}

pub struct ResourceMonitor {
    system: System,
    networks: Networks,
    nvidia_smi_available: Option<bool>,
    last_gpu_pids: Vec<u32>,
    last_gpu_sample: GpuSample,
    last_gpu_refresh: Option<Instant>,
    last_network_refresh: Option<Instant>,
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_cpu();
        system.refresh_memory();
        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh();

        Self {
            system,
            networks,
            nvidia_smi_available: None,
            last_gpu_pids: Vec::new(),
            last_gpu_sample: GpuSample::default(),
            last_gpu_refresh: None,
            last_network_refresh: Some(Instant::now()),
        }
    }

    pub fn snapshot(&mut self, target: ResourceTarget) -> SystemResourceSnapshot {
        match target {
            ResourceTarget::MrdService => {
                self.snapshot_for_process(Some(std::process::id()), "mrd-service")
            }
            ResourceTarget::System => self.snapshot_for_process(None, "system"),
        }
    }

    fn snapshot_for_process(
        &mut self,
        target_pid: Option<u32>,
        target_name: impl Into<String>,
    ) -> SystemResourceSnapshot {
        self.system.refresh_cpu();
        self.system.refresh_memory();
        self.system.refresh_processes();

        let target_name = target_name.into();
        let memory_total = self.system.total_memory();
        let target_pids = target_pid
            .map(|pid| collect_process_tree_pids(&self.system, pid))
            .unwrap_or_default();
        let target_found = target_pid.is_none() || !target_pids.is_empty();
        let mut raw_cpu_usage = 0.0;
        let mut memory_used = 0u64;

        if target_pid.is_none() {
            raw_cpu_usage = self.system.global_cpu_info().cpu_usage();
            memory_used = self.system.used_memory();
        } else {
            for pid in &target_pids {
                if let Some(process) = self.system.process(Pid::from_u32(*pid)) {
                    raw_cpu_usage += process.cpu_usage();
                    memory_used = memory_used.saturating_add(process.memory());
                }
            }
        }

        let cpu_usage_percent = if target_pid.is_none() {
            percent(raw_cpu_usage)
        } else {
            normalize_process_cpu(raw_cpu_usage, self.system.cpus().len().max(1))
        };
        let gpu = self.gpu_sample_for_pids(&target_pids);
        let network = self.network_sample();

        SystemResourceSnapshot {
            target_name,
            target_pid,
            target_found,
            cpu_metrics_available: target_found,
            cpu_usage_percent,
            memory_used_mb: bytes_to_mb(memory_used),
            memory_total_mb: bytes_to_mb(memory_total),
            memory_usage_percent: if memory_total == 0 {
                0.0
            } else {
                percent((memory_used as f32 / memory_total as f32) * 100.0)
            },
            gpu_usage_percent: gpu.usage_percent,
            gpu_memory_used_mb: gpu.memory_used_mb,
            gpu_memory_total_mb: gpu.memory_total_mb,
            gpu_metrics_available: gpu.usage_percent.is_some() || gpu.memory_used_mb.is_some(),
            gpu_metrics_scope: gpu.scope.as_str().to_string(),
            network_rx_bps: network.rx_bps,
            network_tx_bps: network.tx_bps,
            network_metrics_available: network.available,
            network_metrics_scope: network.scope.as_str().to_string(),
            sampled_at_ms: unix_epoch_ms(),
        }
    }

    fn gpu_sample_for_pids(&mut self, target_pids: &[u32]) -> GpuSample {
        let mut sorted_pids = target_pids.to_vec();
        sorted_pids.sort_unstable();

        if self
            .last_gpu_refresh
            .map(|updated_at| {
                updated_at.elapsed() < Duration::from_secs(2) && self.last_gpu_pids == sorted_pids
            })
            .unwrap_or(false)
        {
            return self.last_gpu_sample.clone();
        }

        let sample = sample_nvidia_gpu_for_pids(&mut self.nvidia_smi_available, &sorted_pids);
        self.last_gpu_pids = sorted_pids;
        self.last_gpu_sample = sample.clone();
        self.last_gpu_refresh = Some(Instant::now());
        sample
    }

    fn network_sample(&mut self) -> NetworkSample {
        let now = Instant::now();
        let elapsed_secs = self
            .last_network_refresh
            .map(|updated_at| updated_at.elapsed().as_secs_f64())
            .unwrap_or_default();

        self.networks.refresh();
        self.last_network_refresh = Some(now);

        let mut rx_bytes = 0u64;
        let mut tx_bytes = 0u64;
        let mut interface_count = 0usize;

        for (_name, network) in &self.networks {
            interface_count += 1;
            rx_bytes = rx_bytes.saturating_add(network.received());
            tx_bytes = tx_bytes.saturating_add(network.transmitted());
        }

        if interface_count == 0 {
            return NetworkSample::default();
        }

        NetworkSample {
            rx_bps: bytes_per_second(rx_bytes, elapsed_secs),
            tx_bps: bytes_per_second(tx_bytes, elapsed_secs),
            available: true,
            scope: MetricScope::System,
        }
    }
}

fn collect_process_tree_pids(system: &System, root_pid: u32) -> Vec<u32> {
    let root = Pid::from_u32(root_pid);
    if system.process(root).is_none() {
        return Vec::new();
    }

    let mut seen = HashSet::from([root]);
    let mut ordered = vec![root_pid];

    loop {
        let mut changed = false;
        for (pid, process) in system.processes() {
            if seen.contains(pid) {
                continue;
            }
            if process
                .parent()
                .is_some_and(|parent| seen.contains(&parent))
            {
                seen.insert(*pid);
                ordered.push(pid.as_u32());
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    ordered
}

fn bytes_to_mb(bytes: u64) -> u64 {
    bytes / 1024 / 1024
}

fn percent(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

fn normalize_process_cpu(cpu_usage: f32, cpu_count: usize) -> f32 {
    percent(cpu_usage / cpu_count as f32)
}

fn bytes_per_second(bytes: u64, elapsed_secs: f64) -> u64 {
    if !elapsed_secs.is_finite() || elapsed_secs <= 0.0 {
        0
    } else {
        (bytes as f64 / elapsed_secs).round().max(0.0) as u64
    }
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn sample_nvidia_gpu_for_pids(
    nvidia_smi_available: &mut Option<bool>,
    target_pids: &[u32],
) -> GpuSample {
    if matches!(nvidia_smi_available, Some(false)) {
        return GpuSample::default();
    }

    let system_sample = sample_nvidia_system_gpu(nvidia_smi_available);
    if let Some(process_sample) = sample_nvidia_process_gpu(nvidia_smi_available, target_pids) {
        return GpuSample {
            usage_percent: system_sample.usage_percent,
            memory_used_mb: process_sample.memory_used_mb,
            memory_total_mb: system_sample.memory_total_mb,
            scope: MetricScope::Process,
        };
    }

    system_sample
}

fn sample_nvidia_process_gpu(
    nvidia_smi_available: &mut Option<bool>,
    target_pids: &[u32],
) -> Option<GpuSample> {
    if target_pids.is_empty() {
        return None;
    }

    run_nvidia_smi(
        nvidia_smi_available,
        &[
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ],
    )
    .and_then(|text| parse_nvidia_smi_process_sample(&text, target_pids))
}

fn sample_nvidia_system_gpu(nvidia_smi_available: &mut Option<bool>) -> GpuSample {
    run_nvidia_smi(
        nvidia_smi_available,
        &[
            "--query-gpu=utilization.gpu,memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ],
    )
    .and_then(|text| parse_nvidia_smi_system_sample(&text))
    .unwrap_or_default()
}

fn run_nvidia_smi(nvidia_smi_available: &mut Option<bool>, args: &[&str]) -> Option<String> {
    if matches!(nvidia_smi_available, Some(false)) {
        return None;
    }

    let mut saw_executable = false;

    for executable in nvidia_smi_candidates() {
        match Command::new(executable).args(args).output() {
            Ok(output) if output.status.success() => {
                *nvidia_smi_available = Some(true);
                return String::from_utf8(output.stdout).ok();
            }
            Ok(_) => {
                saw_executable = true;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                saw_executable = true;
            }
        }
    }

    if !saw_executable {
        *nvidia_smi_available = Some(false);
    }

    None
}

#[cfg(target_os = "windows")]
fn nvidia_smi_candidates() -> &'static [&'static str] {
    &["nvidia-smi", "C:\\Windows\\System32\\nvidia-smi.exe"]
}

#[cfg(not(target_os = "windows"))]
fn nvidia_smi_candidates() -> &'static [&'static str] {
    &["nvidia-smi"]
}

fn parse_nvidia_smi_process_sample(text: &str, target_pids: &[u32]) -> Option<GpuSample> {
    let targets = target_pids.iter().copied().collect::<HashSet<_>>();
    let mut memory_used_mb = 0u64;
    let mut matched = false;

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split(',').map(str::trim);
        let pid = parts.next().and_then(|value| value.parse::<u32>().ok());
        if !pid.is_some_and(|pid| targets.contains(&pid)) {
            continue;
        }

        matched = true;
        if let Some(value) = parts.next().and_then(|value| value.parse::<u64>().ok()) {
            memory_used_mb = memory_used_mb.saturating_add(value);
        }
    }

    matched.then_some(GpuSample {
        usage_percent: None,
        memory_used_mb: Some(memory_used_mb),
        memory_total_mb: None,
        scope: MetricScope::Process,
    })
}

fn parse_nvidia_smi_system_sample(text: &str) -> Option<GpuSample> {
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    let mut parts = line.split(',').map(str::trim);

    Some(GpuSample {
        usage_percent: parts.next().and_then(|value| value.parse::<f32>().ok()),
        memory_used_mb: parts.next().and_then(|value| value.parse::<u64>().ok()),
        memory_total_mb: parts.next().and_then(|value| value.parse::<u64>().ok()),
        scope: MetricScope::System,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_process_output_is_parsed_for_target_pid() {
        let sample = parse_nvidia_smi_process_sample("100, 256\n200, 1024\n", &[200]).unwrap();

        assert_eq!(sample.usage_percent, None);
        assert_eq!(sample.memory_used_mb, Some(1024));
        assert_eq!(sample.memory_total_mb, None);
        assert_eq!(sample.scope, MetricScope::Process);
    }

    #[test]
    fn process_cpu_usage_is_normalized_to_total_capacity() {
        assert_eq!(normalize_process_cpu(100.0, 8), 12.5);
        assert_eq!(normalize_process_cpu(900.0, 8), 100.0);
    }

    #[test]
    fn nvidia_smi_system_output_is_parsed() {
        let sample = parse_nvidia_smi_system_sample("39, 2048, 8192\n").unwrap();

        assert_eq!(sample.usage_percent, Some(39.0));
        assert_eq!(sample.memory_used_mb, Some(2048));
        assert_eq!(sample.memory_total_mb, Some(8192));
        assert_eq!(sample.scope, MetricScope::System);
    }

    #[test]
    fn byte_delta_is_normalized_to_rate() {
        assert_eq!(bytes_per_second(2048, 2.0), 1024);
        assert_eq!(bytes_per_second(2048, 0.0), 0);
    }
}
