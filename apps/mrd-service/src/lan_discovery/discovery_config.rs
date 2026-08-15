use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::time::Duration;

const DEFAULT_DISCOVERY_PORT: u16 = 21116;
const LAN_DISCOVERY_PORT_ENV: &str = "MRD_LAN_DISCOVERY_PORT";
const LAN_DISCOVERY_PROBE_ENDPOINTS_ENV: &str = "MRD_LAN_DISCOVERY_PROBE_ENDPOINTS";
const LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV: &str = "MRD_LAN_ALLOW_UNSIGNED_DIAGNOSTICS";
const LAN_DISCOVERY_BROADCAST_ENABLED_ENV: &str = "MRD_LAN_DISCOVERY_BROADCAST_ENABLED";
const ANNOUNCE_INTERVAL_SECS: u64 = 3;
const PEER_TTL_SECS: u64 = 12;

#[derive(Debug, Clone)]
pub struct LanDiscoveryConfig {
    pub enabled: bool,
    pub broadcast_enabled: bool,
    pub discovery_port: u16,
    pub probe_endpoints: Vec<SocketAddr>,
    pub announce_interval: Duration,
    pub peer_ttl: Duration,
    /// Allows unsigned v1 announcements to appear as non-controllable diagnostics only.
    pub allow_unsigned_diagnostics: bool,
}

impl Default for LanDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            broadcast_enabled: true,
            discovery_port: DEFAULT_DISCOVERY_PORT,
            probe_endpoints: Vec::new(),
            announce_interval: Duration::from_secs(ANNOUNCE_INTERVAL_SECS),
            peer_ttl: Duration::from_secs(PEER_TTL_SECS),
            allow_unsigned_diagnostics: false,
        }
    }
}

impl LanDiscoveryConfig {
    pub fn from_env() -> Result<Self> {
        Self::from_env_lookup(|key| std::env::var(key).ok())
    }

    pub(super) fn from_env_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut config = Self::default();
        if let Some(port) = lookup(LAN_DISCOVERY_PORT_ENV) {
            let port = port.trim();
            if !port.is_empty() {
                config.discovery_port = port
                    .parse::<u16>()
                    .with_context(|| format!("invalid {LAN_DISCOVERY_PORT_ENV}: {port}"))?;
            }
        }
        if let Some(endpoints) = lookup(LAN_DISCOVERY_PROBE_ENDPOINTS_ENV) {
            config.probe_endpoints = parse_probe_endpoints(&endpoints)?;
        }
        if let Some(value) = lookup(LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV) {
            let value = value.trim();
            if !value.is_empty() {
                config.allow_unsigned_diagnostics = match value.to_ascii_lowercase().as_str() {
                    "1" | "true" => true,
                    "0" | "false" => false,
                    _ => anyhow::bail!(
                        "invalid {LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV}: expected true/false or 1/0"
                    ),
                };
            }
        }
        if let Some(enabled) = lookup(LAN_DISCOVERY_BROADCAST_ENABLED_ENV) {
            let enabled = enabled.trim();
            if !enabled.is_empty() {
                config.broadcast_enabled = enabled.parse::<bool>().with_context(|| {
                    format!("invalid {LAN_DISCOVERY_BROADCAST_ENABLED_ENV}: {enabled}")
                })?;
            }
        }
        Ok(config)
    }
}

fn parse_probe_endpoints(value: &str) -> Result<Vec<SocketAddr>> {
    let mut endpoints = Vec::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        endpoints.push(
            entry
                .parse::<SocketAddr>()
                .with_context(|| format!("invalid LAN discovery probe endpoint: {entry}"))?,
        );
    }
    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::time::Duration;

    #[test]
    fn default_config_uses_stable_discovery_timing() {
        let config = LanDiscoveryConfig::default();

        assert!(config.enabled);
        assert!(config.broadcast_enabled);
        assert_eq!(config.discovery_port, DEFAULT_DISCOVERY_PORT);
        assert!(config.probe_endpoints.is_empty());
        assert_eq!(config.announce_interval, Duration::from_secs(3));
        assert_eq!(config.peer_ttl, Duration::from_secs(12));
        assert!(!config.allow_unsigned_diagnostics);
    }

    #[test]
    fn env_config_reads_port_and_probe_endpoints() {
        let config = LanDiscoveryConfig::from_env_lookup(|key| match key {
            LAN_DISCOVERY_PORT_ENV => Some("21216".to_string()),
            LAN_DISCOVERY_PROBE_ENDPOINTS_ENV => {
                Some("127.0.0.1:21217, 127.0.0.1:21218".to_string())
            }
            LAN_DISCOVERY_BROADCAST_ENABLED_ENV => Some("false".to_string()),
            _ => None,
        })
        .expect("env config");

        assert_eq!(config.discovery_port, 21216);
        assert!(!config.broadcast_enabled);
        assert_eq!(
            config.probe_endpoints,
            vec![
                "127.0.0.1:21217".parse::<SocketAddr>().unwrap(),
                "127.0.0.1:21218".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn env_config_ignores_blank_port() {
        let config = LanDiscoveryConfig::from_env_lookup(|key| match key {
            LAN_DISCOVERY_PORT_ENV => Some(" ".to_string()),
            _ => None,
        })
        .expect("env config");

        assert_eq!(config.discovery_port, DEFAULT_DISCOVERY_PORT);
    }

    #[test]
    fn env_config_reports_invalid_probe_endpoint() {
        let error = LanDiscoveryConfig::from_env_lookup(|key| match key {
            LAN_DISCOVERY_PROBE_ENDPOINTS_ENV => Some("not-an-endpoint".to_string()),
            _ => None,
        })
        .expect_err("invalid endpoint should fail");

        assert!(format!("{error:#}").contains("invalid LAN discovery probe endpoint"));
    }

    #[test]
    fn unsigned_legacy_diagnostics_require_an_explicit_valid_opt_in() {
        let enabled = LanDiscoveryConfig::from_env_lookup(|key| {
            (key == LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV).then(|| "true".to_string())
        })
        .expect("explicit diagnostics setting");
        assert!(enabled.allow_unsigned_diagnostics);

        let error = LanDiscoveryConfig::from_env_lookup(|key| {
            (key == LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV).then(|| "maybe".to_string())
        })
        .expect_err("ambiguous compatibility setting must fail closed");
        assert!(error
            .to_string()
            .contains(LAN_ALLOW_UNSIGNED_DIAGNOSTICS_ENV));
    }

    #[test]
    fn env_config_reports_invalid_broadcast_flag() {
        let error = LanDiscoveryConfig::from_env_lookup(|key| match key {
            LAN_DISCOVERY_BROADCAST_ENABLED_ENV => Some("sometimes".to_string()),
            _ => None,
        })
        .expect_err("invalid broadcast flag should fail");

        assert!(format!("{error:#}").contains(LAN_DISCOVERY_BROADCAST_ENABLED_ENV));
    }
}
