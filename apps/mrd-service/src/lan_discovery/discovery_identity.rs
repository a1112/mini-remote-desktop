use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DISCOVERY_MAGIC: &str = "mrd-lan-discovery-v1";
pub const DISCOVERY_APP_ID: &str = "rdesk";

static LAN_DISCOVERY_INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(super) fn new_instance_id() -> String {
    let sequence = LAN_DISCOVERY_INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format_instance_id(std::process::id(), now_ms(), sequence)
}

pub(super) fn default_app_id() -> String {
    DISCOVERY_APP_ID.to_string()
}

pub(super) fn is_valid_discovery_packet(magic: &str, app_id: &str) -> bool {
    magic == DISCOVERY_MAGIC && app_id.eq_ignore_ascii_case(DISCOVERY_APP_ID)
}

fn format_instance_id(process_id: u32, timestamp_ms: u64, sequence: u64) -> String {
    format!("mrd-{process_id}-{timestamp_ms}-{sequence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_rdesk_discovery_namespace() {
        assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, DISCOVERY_APP_ID));
        assert!(is_valid_discovery_packet(DISCOVERY_MAGIC, "RDESK"));
        assert!(!is_valid_discovery_packet("other-magic", DISCOVERY_APP_ID));
        assert!(!is_valid_discovery_packet(DISCOVERY_MAGIC, "rsharemouse"));
    }

    #[test]
    fn instance_ids_include_process_time_and_sequence() {
        let id = format_instance_id(42, 1_234, 7);

        assert_eq!(id, "mrd-42-1234-7");
    }

    #[test]
    fn generated_instance_ids_are_unique_in_one_process() {
        let ids = (0..8).map(|_| new_instance_id()).collect::<Vec<_>>();
        let unique_ids = ids.iter().collect::<std::collections::HashSet<_>>();

        assert_eq!(unique_ids.len(), ids.len());
    }
}
