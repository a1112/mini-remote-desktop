use serde::Serialize;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, System};

#[derive(Debug, Clone, Default)]
struct GpuSample {
    usage_percent: Option<f32>,
    memory_used_mb: Option<u64>,
    memory_total_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemResourceSnapshot {
    pub target_name: String,
    pub target_pid: Option<u32>,
    pub target_found: bool,
    pub cpu_usage_percent: f32,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub memory_usage_percent: f32,
    pub gpu_usage_percent: Option<f32>,
    pub gpu_memory_used_mb: Option<u64>,
    pub gpu_memory_total_mb: Option<u64>,
    pub gpu_metrics_available: bool,
    pub network_rx_bps: u64,
    pub network_tx_bps: u64,
    pub network_metrics_available: bool,
    pub sampled_at_ms: u64,
}

pub struct ResourceMonitor {
    system: System,
    nvidia_smi_available: Option<bool>,
    last_gpu_pid: Option<u32>,
    last_gpu_sample: GpuSample,
    last_gpu_refresh: Option<Instant>,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_cpu();
        system.refresh_memory();

        Self {
            system,
            nvidia_smi_available: None,
            last_gpu_pid: None,
            last_gpu_sample: GpuSample::default(),
            last_gpu_refresh: None,
        }
    }

    pub fn snapshot_for_process(
        &mut self,
        target_pid: Option<u32>,
        target_name: impl Into<String>,
    ) -> SystemResourceSnapshot {
        self.system.refresh_memory();

        let target_name = target_name.into();
        let memory_total = self.system.total_memory();
        let mut target_found = false;
        let mut cpu_usage_percent = 0.0;
        let mut memory_used = 0;

        if let Some(pid) = target_pid {
            let sysinfo_pid = Pid::from_u32(pid);
            target_found = self.system.refresh_process(sysinfo_pid);

            if let Some(process) = self.system.process(sysinfo_pid) {
                target_found = true;
                cpu_usage_percent =
                    normalize_process_cpu(process.cpu_usage(), self.system.cpus().len().max(1));
                memory_used = process.memory();
            }
        }

        let gpu = self.gpu_sample_for_pid(target_pid);

        SystemResourceSnapshot {
            target_name,
            target_pid,
            target_found,
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
            network_rx_bps: 0,
            network_tx_bps: 0,
            network_metrics_available: false,
            sampled_at_ms: unix_epoch_ms(),
        }
    }

    fn gpu_sample_for_pid(&mut self, target_pid: Option<u32>) -> GpuSample {
        if self
            .last_gpu_refresh
            .map(|updated_at| {
                updated_at.elapsed() < Duration::from_secs(2) && self.last_gpu_pid == target_pid
            })
            .unwrap_or(false)
        {
            return self.last_gpu_sample.clone();
        }

        let sample = sample_nvidia_gpu_for_pid(&mut self.nvidia_smi_available, target_pid);
        self.last_gpu_pid = target_pid;
        self.last_gpu_sample = sample.clone();
        self.last_gpu_refresh = Some(Instant::now());
        sample
    }
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

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn sample_nvidia_gpu_for_pid(
    nvidia_smi_available: &mut Option<bool>,
    target_pid: Option<u32>,
) -> GpuSample {
    if matches!(nvidia_smi_available, Some(false)) {
        return GpuSample::default();
    }

    let Some(target_pid) = target_pid else {
        return GpuSample::default();
    };

    let output = Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            *nvidia_smi_available = Some(true);
            String::from_utf8(output.stdout)
                .ok()
                .and_then(|text| parse_nvidia_smi_process_sample(&text, target_pid))
                .unwrap_or_default()
        }
        Ok(_) => {
            *nvidia_smi_available = Some(false);
            GpuSample::default()
        }
        Err(error) => {
            if error.kind() == std::io::ErrorKind::NotFound {
                *nvidia_smi_available = Some(false);
            }
            GpuSample::default()
        }
    }
}

fn parse_nvidia_smi_process_sample(text: &str, target_pid: u32) -> Option<GpuSample> {
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split(',').map(str::trim);
        let pid = parts.next().and_then(|value| value.parse::<u32>().ok());
        if pid != Some(target_pid) {
            continue;
        }

        return Some(GpuSample {
            usage_percent: None,
            memory_used_mb: parts.next().and_then(|value| value.parse::<u64>().ok()),
            memory_total_mb: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_smi_process_output_is_parsed_for_target_pid() {
        let sample = parse_nvidia_smi_process_sample("100, 256\n200, 1024\n", 200).unwrap();

        assert_eq!(sample.usage_percent, None);
        assert_eq!(sample.memory_used_mb, Some(1024));
        assert_eq!(sample.memory_total_mb, None);
    }

    #[test]
    fn nvidia_smi_process_output_ignores_other_pids() {
        let sample = parse_nvidia_smi_process_sample("100, 256\n", 200);

        assert!(sample.is_none());
    }

    #[test]
    fn process_cpu_usage_is_normalized_to_total_capacity() {
        assert_eq!(normalize_process_cpu(100.0, 8), 12.5);
        assert_eq!(normalize_process_cpu(900.0, 8), 100.0);
    }
}
