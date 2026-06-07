use mrd_proto::DeviceId;

pub fn default_lan_device_identity() -> (DeviceId, String) {
    lan_device_identity_from(
        std::env::var("MRD_LAN_DEVICE_ID").ok(),
        std::env::var("MRD_LAN_DEVICE_NAME").ok(),
        default_hostname(),
    )
}

fn default_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
}

fn lan_device_identity_from(
    configured_id: Option<String>,
    configured_name: Option<String>,
    hostname: Option<String>,
) -> (DeviceId, String) {
    let device_name = configured_name
        .and_then(non_empty_trimmed)
        .or_else(|| hostname.clone().and_then(non_empty_trimmed))
        .unwrap_or_else(|| "Rdesk LAN Device".to_string());
    let device_id = configured_id
        .and_then(non_empty_trimmed)
        .unwrap_or_else(|| build_lan_device_id(hostname.as_deref().unwrap_or(&device_name)));
    (DeviceId(device_id), device_name)
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_lan_device_id(seed: &str) -> String {
    let mut sanitized: String = seed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    if sanitized.len() > 16 {
        sanitized = sanitized[sanitized.len() - 16..].to_string();
    }
    if sanitized.is_empty() {
        sanitized = "local".to_string();
    }
    format!("lan-{sanitized}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lan_identity_uses_configured_id_and_name() {
        let (device_id, device_name) = lan_device_identity_from(
            Some(" lan-MOCK7EBPZ3RC ".to_string()),
            Some(" Target PC ".to_string()),
            Some("ignored-host".to_string()),
        );

        assert_eq!(device_id, DeviceId("lan-MOCK7EBPZ3RC".to_string()));
        assert_eq!(device_name, "Target PC");
    }

    #[test]
    fn default_lan_identity_falls_back_to_hostname() {
        let (device_id, device_name) =
            lan_device_identity_from(None, None, Some("DESKTOP-ABC/123".to_string()));

        assert_eq!(device_id, DeviceId("lan-DESKTOPABC123".to_string()));
        assert_eq!(device_name, "DESKTOP-ABC/123");
    }
}
