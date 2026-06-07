pub(crate) const DISCOVERY_MAGIC: &str = "mrd-lan-discovery-v1";
pub(crate) const DISCOVERY_APP_ID: &str = "rdesk";

pub(crate) fn default_app_id() -> String {
    DISCOVERY_APP_ID.to_string()
}

pub(crate) fn is_valid_discovery_packet(magic: &str, app_id: &str) -> bool {
    magic == DISCOVERY_MAGIC && app_id.eq_ignore_ascii_case(DISCOVERY_APP_ID)
}
