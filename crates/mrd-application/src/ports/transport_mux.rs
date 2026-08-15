//! Transport-neutral lanes and the application-facing session transport port.

use std::collections::BTreeMap;

use anyhow::Result;
use mrd_proto::SessionId;

/// Logical traffic classes exposed to application use cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportLane {
    /// Encoded video access units.
    Video,
    /// Ordered and reliable control messages.
    ControlReliable,
    /// Latest-value realtime control messages.
    ControlRealtime,
    /// Reliable bulk data independent of interactive control.
    Bulk,
}

impl TransportLane {
    /// All lanes in stable presentation order.
    pub const ALL: [Self; 4] = [
        Self::Video,
        Self::ControlReliable,
        Self::ControlRealtime,
        Self::Bulk,
    ];
}

/// Transport-neutral metadata for one encoded video access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoEnvelopeMetadata {
    /// Lowercase codec identifier such as `h264`.
    pub codec: String,
    /// Capture or presentation timestamp in microseconds.
    pub timestamp_us: u64,
    /// Whether the access unit can start a decoder independently.
    pub keyframe: bool,
    /// Coded frame width, or zero when the media transport does not carry dimensions.
    pub width: u32,
    /// Coded frame height, or zero when the media transport does not carry dimensions.
    pub height: u32,
}

/// One session-bound payload submitted to or received from a transport mux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportEnvelope {
    /// Authenticated session that owns this traffic.
    pub session_id: SessionId,
    /// Logical delivery lane.
    pub lane: TransportLane,
    /// Monotonic lane-local sequence number at this endpoint.
    ///
    /// Datagram adapters preserve the submitted number. RTP adapters may reconstruct
    /// a monotonic receive-side number because RTP does not carry this application value.
    pub sequence: u64,
    /// Encoded application payload.
    pub payload: Vec<u8>,
    /// Video metadata, present only for the video lane.
    pub video: Option<VideoEnvelopeMetadata>,
}

/// Immediate result of submitting an envelope to a bounded mux queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportSendOutcome {
    /// The envelope entered the lane queue.
    Enqueued,
    /// A pending realtime value was replaced by this newer value.
    ReplacedStale,
    /// The bounded lane cannot currently accept another envelope.
    Backpressured,
    /// The mux has already closed.
    Closed,
}

/// Verified route classification exposed by a concrete adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportRouteKind {
    /// In-memory test route with no network claims.
    TestMemory,
    /// Direct LAN route carried by QUIC.
    QuicLan,
    /// WebRTC route whose selected ICE candidate pair is not yet observable.
    WebRtcPending,
    /// Direct WebRTC candidate pair.
    WebRtcDirect,
    /// WebRTC candidate pair relayed through TURN.
    WebRtcRelay,
}

/// Monotonic evidence counters for one logical lane.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportLaneStats {
    /// Envelopes accepted for transmission.
    pub sent: u64,
    /// Envelopes received from the peer.
    pub received: u64,
    /// Envelopes intentionally dropped to keep latency bounded.
    pub dropped: u64,
    /// Pending realtime envelopes replaced by newer values.
    pub stale_replaced: u64,
    /// Reordered realtime envelopes discarded because a newer sequence was already accepted.
    pub stale_discarded: u64,
    /// Send attempts rejected by a bounded queue.
    pub backpressured: u64,
}

/// Auditable snapshot of the selected route and its lane activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportRouteSnapshot {
    /// Session represented by this route.
    pub session_id: SessionId,
    /// Concrete route classification.
    pub kind: TransportRouteKind,
    /// Sanitized local endpoint evidence.
    pub local_endpoint: String,
    /// Sanitized peer endpoint evidence.
    pub peer_endpoint: String,
    /// Selected local candidate kind when WebRTC supplies one.
    pub local_candidate_kind: Option<String>,
    /// Selected remote candidate kind when WebRTC supplies one.
    pub remote_candidate_kind: Option<String>,
    /// Per-lane monotonic counters.
    lanes: BTreeMap<TransportLane, TransportLaneStats>,
    /// Whether the route has closed.
    pub closed: bool,
    /// Sanitized terminal transport error, absent after an intentional close.
    pub last_error: Option<String>,
}

impl TransportRouteSnapshot {
    /// Create an empty snapshot whose map contains all logical lanes.
    pub fn new(
        session_id: SessionId,
        kind: TransportRouteKind,
        local_endpoint: impl Into<String>,
        peer_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            session_id,
            kind,
            local_endpoint: local_endpoint.into(),
            peer_endpoint: peer_endpoint.into(),
            local_candidate_kind: None,
            remote_candidate_kind: None,
            lanes: TransportLane::ALL
                .into_iter()
                .map(|lane| (lane, TransportLaneStats::default()))
                .collect(),
            closed: false,
            last_error: None,
        }
    }

    /// Return evidence counters for one logical lane.
    pub fn lane(&self, lane: TransportLane) -> &TransportLaneStats {
        self.lanes
            .get(&lane)
            .expect("transport snapshots always contain every lane")
    }

    /// Return mutable evidence counters for one logical lane.
    pub fn lane_mut(&mut self, lane: TransportLane) -> &mut TransportLaneStats {
        self.lanes
            .get_mut(&lane)
            .expect("transport snapshots always contain every lane")
    }
}

/// Session-scoped transport multiplexer used by application and media code.
#[async_trait::async_trait]
pub trait TransportMuxPort: Send + Sync {
    /// Submit an envelope without waiting for remote delivery.
    async fn send(&self, envelope: TransportEnvelope) -> Result<TransportSendOutcome>;

    /// Receive the next envelope for one logical lane.
    async fn recv(&self, lane: TransportLane) -> Result<Option<TransportEnvelope>>;

    /// Read current route evidence and monotonic counters.
    async fn route_snapshot(&self) -> TransportRouteSnapshot;

    /// Close this mux and wake pending lane receivers.
    async fn close(&self) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_snapshot_initializes_every_lane() {
        let snapshot = TransportRouteSnapshot::new(
            SessionId("session".into()),
            TransportRouteKind::TestMemory,
            "memory:left",
            "memory:right",
        );

        assert_eq!(snapshot.lanes.len(), TransportLane::ALL.len());
        for lane in TransportLane::ALL {
            assert_eq!(snapshot.lane(lane), &TransportLaneStats::default());
        }
    }
}
