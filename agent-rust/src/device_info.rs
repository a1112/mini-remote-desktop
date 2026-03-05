//! 设备信息获取模块
//!
//! 获取硬件ID、机器名、操作系统版本等设备唯一标识信息

use serde::{Deserialize, Serialize};
use std::fmt;
use anyhow::Result;

/// 设备唯一标识信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    /// 机器唯一ID（基于硬件特征生成）
    pub machine_id: String,
    /// 机器名/主机名
    pub hostname: String,
    /// 操作系统类型
    pub os_type: String,
    /// 操作系统版本
    pub os_version: String,
    /// CPU 信息
    pub cpu_info: String,
    /// 内存总量 (MB)
    pub total_memory_mb: u64,
    /// GPU 信息（如果有）
    pub gpu_info: Vec<GpuInfo>,
    /// MAC 地址列表（用于设备指纹）
    pub mac_addresses: Vec<String>,
}

/// GPU 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub memory_mb: Option<u64>,
}

/// 设备注册信息（发送给信令服务器）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRegistrationInfo {
    /// 设备类型（agent-rust, agent-python, controller-rust 等）
    pub device_type: String,
    /// 设备名称（用户可配置的显示名称）
    pub name: String,
    /// 设备身份信息
    pub identity: DeviceIdentity,
    /// 协议版本
    pub protocol_version: u32,
    /// 支持的传输协议
    pub transports: Vec<String>,
    /// 能力信息
    pub capabilities: DeviceCapabilities,
}

/// 设备能力
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    /// 支持的协议
    pub protocols: Vec<String>,
    /// 支持的平台
    pub platforms: Vec<String>,
    /// 支持的编解码器
    pub codecs: Vec<String>,
    /// 支持的特性
    pub features: Vec<String>,
    /// 最大支持分辨率
    pub max_resolution: Option<(u32, u32)>,
    /// 最大支持帧率
    pub max_fps: Option<u32>,
}

impl Default for DeviceIdentity {
    fn default() -> Self {
        Self {
            machine_id: generate_machine_id(),
            hostname: get_hostname(),
            os_type: std::env::consts::OS.to_string(),
            os_version: get_os_version(),
            cpu_info: get_cpu_info(),
            total_memory_mb: get_total_memory_mb(),
            gpu_info: get_gpu_info(),
            mac_addresses: get_mac_addresses(),
        }
    }
}

impl fmt::Display for DeviceIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) - {} - {}",
            self.hostname, self.machine_id, self.os_type, self.os_version
        )
    }
}

/// 生成机器唯一ID
///
/// 基于多种硬件特征生成稳定的唯一标识符：
/// - Windows: 使用 MachineGuid (注册表)
/// - 回退方案: 使用组合的硬件特征
fn generate_machine_id() -> String {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        // 尝试从注册表获取 MachineGuid
        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Cryptography")
        {
            if let Ok(guid) = key.get_value::<String, _>("MachineGuid") {
                // MachineGuid 格式: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
                // 取其哈希作为短ID
                return hash_machine_guid(&guid);
            }
        }

        // 回退方案：使用计算机名 + CPU ID
        let hostname = get_hostname();
        let cpu_id = get_cpu_id();
        format!("{:08x}", hash_string(&format!("{}:{}", hostname, cpu_id)))
    }

    #[cfg(not(target_os = "windows"))]
    {
        // 非Windows系统：使用机器ID文件或组合特征
        let hostname = get_hostname();
        let cpu_id = get_cpu_id();
        format!("{:08x}", hash_string(&format!("{}:{}", hostname, cpu_id)))
    }
}

#[cfg(target_os = "windows")]
fn hash_machine_guid(guid: &str) -> String {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    guid.hash(&mut hasher);
    format!("{:08x}", hasher.finish())
}

fn hash_string(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// 获取主机名
fn get_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| Ok("Unknown".to_string()))
        .unwrap_or_else(|_| "Unknown".to_string())
}

/// 获取操作系统版本
fn get_os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::*;
        use winreg::RegKey;

        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        {
            let product_name = key
                .get_value::<String, _>("ProductName")
                .unwrap_or_else(|_| "Windows".to_string());
            let display_version = key
                .get_value::<String, _>("DisplayVersion")
                .unwrap_or_else(|_| "".to_string());

            if display_version.is_empty() {
                product_name
            } else {
                format!("{} {}", product_name, display_version)
            }
        } else {
            "Windows".to_string()
        }
    }

    #[cfg(target_os = "linux")]
    {
        // 尝试读取 /etc/os-release
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            content
                .lines()
                .find(|line| line.starts_with("PRETTY_NAME="))
                .and_then(|line| line.split('=').nth(1))
                .map(|s| s.trim_matches('"').to_string())
                .unwrap_or_else(|| "Linux".to_string())
        } else {
            "Linux".to_string()
        }
    }

    #[cfg(target_os = "macos")]
    {
        "macOS".to_string()
    }
}

/// 获取 CPU 信息
fn get_cpu_info() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        Command::new("wmic")
            .args(&["cpu", "get", "name"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.lines().nth(1).unwrap_or("").trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        "Unknown CPU".to_string()
    }
}

/// 获取 CPU ID（用于生成机器ID）
fn get_cpu_id() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        Command::new("wmic")
            .args(&["cpu", "get", "ProcessorId"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.lines().nth(1).unwrap_or("").trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "0000000000000000".to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        "0000000000000000".to_string()
    }
}

/// 获取总内存量（MB）
fn get_total_memory_mb() -> u64 {
    #[cfg(target_os = "windows")]
    {
        use winapi::um::sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

        unsafe {
            let mut stat: MEMORYSTATUSEX = std::mem::zeroed();
            stat.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            GlobalMemoryStatusEx(&mut stat);
            stat.ullTotalPhys / (1024 * 1024)
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        8192 // 默认 8GB
    }
}

/// 获取 GPU 信息
fn get_gpu_info() -> Vec<GpuInfo> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        let output = Command::new("wmic")
            .args(&["path", "win32_VideoController", "get", "name,AdapterRAM,DriverVersion"])
            .output();

        if let Ok(out) = output {
            if let Ok(text) = String::from_utf8(out.stdout) {
                return parse_gpu_info(&text);
            }
        }

        Vec::new()
    }

    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "windows")]
fn parse_gpu_info(text: &str) -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    // 跳过表头，从第2行开始
    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() || parts[0] == "No" {
            continue;
        }

        let name = parts.join(" ");
        let memory_mb = None; // 需要更复杂的解析

        // 简单的厂商检测
        let vendor = if name.contains("NVIDIA") || name.contains("GeForce") || name.contains("Quadro") {
            "NVIDIA".to_string()
        } else if name.contains("AMD") || name.contains("Radeon") {
            "AMD".to_string()
        } else if name.contains("Intel") {
            "Intel".to_string()
        } else {
            "Unknown".to_string()
        };

        gpus.push(GpuInfo {
            name,
            vendor,
            memory_mb,
        });
    }

    gpus
}

/// 获取 MAC 地址列表
fn get_mac_addresses() -> Vec<String> {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        let output = Command::new("getmac")
            .args(&["/fo", "csv", "/nh"])
            .output();

        if let Ok(out) = output {
            if let Ok(text) = String::from_utf8(out.stdout) {
                return text
                    .lines()
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.split(',').collect();
                        if parts.len() >= 2 {
                            Some(parts[0].trim_matches('"').replace('-', ":"))
                        } else {
                            None
                        }
                    })
                    .filter(|mac| !mac.is_empty() && mac != "00:00:00:00:00:00")
                    .collect();
            }
        }

        Vec::new()
    }

    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

impl DeviceRegistrationInfo {
    /// 创建默认的设备注册信息
    pub fn new(device_type: String, name: String) -> Self {
        Self {
            device_type,
            name,
            identity: DeviceIdentity::default(),
            protocol_version: 2,
            transports: vec!["webrtc".to_string(), "quic".to_string(), "webtransport".to_string()],
            capabilities: DeviceCapabilities::default(),
        }
    }

    /// 序列化为 JSON（用于发送到信令服务器）
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// 创建注册消息（符合现有信令协议）
    pub fn to_register_message(&self) -> String {
        serde_json::json!({
            "type": "device",
            "action": "register",
            "payload": {
                "type": self.device_type,
                "name": self.name,
                "protocolVersion": self.protocol_version,
                "transports": self.transports,
                "capabilities": {
                    "protocols": self.capabilities.protocols,
                    "platforms": self.capabilities.platforms,
                    "codecs": self.capabilities.codecs,
                    "features": self.capabilities.features,
                },
                "identity": {
                    "machineId": self.identity.machine_id,
                    "hostname": self.identity.hostname,
                    "osType": self.identity.os_type,
                    "osVersion": self.identity.os_version,
                    "cpuInfo": self.identity.cpu_info,
                    "totalMemoryMb": self.identity.total_memory_mb,
                    "gpuInfo": self.identity.gpu_info,
                }
            }
        }).to_string()
    }
}

impl Default for DeviceCapabilities {
    fn default() -> Self {
        Self {
            protocols: vec!["webrtc".to_string(), "quic".to_string(), "webtransport".to_string()],
            platforms: vec!["windows".to_string(), "linux".to_string(), "macos".to_string()],
            codecs: vec!["h264".to_string()],
            features: vec![
                "multi-end-compat".to_string(),
                "capability-negotiation".to_string(),
                "transport-failover".to_string(),
            ],
            max_resolution: Some((3840, 2160)),
            max_fps: Some(120),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_machine_id_generation() {
        let id1 = generate_machine_id();
        let id2 = generate_machine_id();
        assert_eq!(id1, id2, "Machine ID should be stable");
        assert!(!id1.is_empty(), "Machine ID should not be empty");
        assert!(id1.len() == 8, "Machine ID should be 8 characters");
    }

    #[test]
    fn test_device_identity_default() {
        let identity = DeviceIdentity::default();
        assert!(!identity.machine_id.is_empty());
        assert!(!identity.hostname.is_empty());
        assert!(!identity.os_type.is_empty());
    }

    #[test]
    fn test_device_registration_info() {
        let info = DeviceRegistrationInfo::new(
            "agent-rust".to_string(),
            "Test Device".to_string(),
        );

        assert_eq!(info.device_type, "agent-rust");
        assert_eq!(info.name, "Test Device");
        assert!(!info.identity.machine_id.is_empty());

        let json = info.to_register_message();
        assert!(json.contains("\"type\":\"device\""));
        assert!(json.contains("\"action\":\"register\""));
        assert!(json.contains("\"machineId\""));
    }
}
