use super::discovery_identity::{now_ms, DISCOVERY_APP_ID, DISCOVERY_MAGIC};
use super::protocol::{LanDiscoveryPacket, DISCOVERY_SAFE_UDP_PAYLOAD_BYTES};
use mrd_ipc::CaptureSource;

pub(super) fn fit_capture_sources_ack_packet(
    instance_id: String,
    session_id: String,
    accepted: bool,
    message: Option<String>,
    sources: Vec<CaptureSource>,
) -> LanDiscoveryPacket {
    let sources = sources
        .into_iter()
        .map(strip_capture_source_preview)
        .collect();
    let mut packet = LanDiscoveryPacket::CaptureSourcesAck {
        magic: DISCOVERY_MAGIC.to_string(),
        app_id: DISCOVERY_APP_ID.to_string(),
        instance_id,
        session_id,
        accepted,
        message,
        sources,
        timestamp_ms: now_ms(),
    };

    while serialized_packet_len(&packet) > DISCOVERY_SAFE_UDP_PAYLOAD_BYTES {
        let LanDiscoveryPacket::CaptureSourcesAck { sources, .. } = &mut packet else {
            break;
        };

        if sources.len() > 1 {
            sources.pop();
            continue;
        }

        break;
    }

    packet
}

fn strip_capture_source_preview(mut source: CaptureSource) -> CaptureSource {
    source.preview_data_url = None;
    source.preview_width = None;
    source.preview_height = None;
    source
}

pub(super) fn serialized_packet_len(packet: &LanDiscoveryPacket) -> usize {
    serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}
