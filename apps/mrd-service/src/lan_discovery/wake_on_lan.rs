use anyhow::{Context, Result};

pub(crate) const WAKE_MAC_ADDRESS_ENV: &str = "MRD_WAKE_MAC_ADDRESS";

pub(crate) fn mac_address_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let value = lookup(WAKE_MAC_ADDRESS_ENV)?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    normalize_mac_address(value).ok()
}

fn normalize_mac_address(value: &str) -> Result<String> {
    let hex = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    if hex.len() != 12 {
        anyhow::bail!("Wake-on-LAN MAC address must contain 12 hex digits");
    }

    let mut bytes = Vec::with_capacity(6);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16)
            .context("invalid Wake-on-LAN MAC address")?;
        bytes.push(format!("{byte:02X}"));
    }
    Ok(bytes.join(":"))
}
