use std::collections::HashMap;

use webrtc::{
    data_channel::data_channel_state::RTCDataChannelState,
    stats::{ICECandidatePairStats, StatsReport, StatsReportType},
};

use crate::control::CTRL_REL_LABEL;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateKind {
    Unknown,
    Host,
    ServerReflexive,
    PeerReflexive,
    Relay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectedCandidatePairStats {
    pub local_candidate_id: String,
    pub remote_candidate_id: String,
    pub local_candidate_kind: CandidateKind,
    pub remote_candidate_kind: CandidateKind,
    pub nominated: bool,
    pub packets_sent: u32,
    pub packets_received: u32,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub current_round_trip_time: f64,
}

pub(crate) fn selected_candidate_pair(report: StatsReport) -> Option<SelectedCandidatePairStats> {
    let mut candidates = HashMap::new();
    let mut selected: Option<ICECandidatePairStats> = None;
    for entry in report.reports.into_values() {
        match entry {
            StatsReportType::LocalCandidate(candidate)
            | StatsReportType::RemoteCandidate(candidate) => {
                candidates.insert(
                    candidate.id,
                    candidate_kind(&candidate.candidate_type.to_string()),
                );
            }
            StatsReportType::CandidatePair(pair) if pair.nominated => selected = Some(pair),
            _ => {}
        }
    }
    let pair = selected?;
    Some(SelectedCandidatePairStats {
        local_candidate_kind: candidates
            .get(&pair.local_candidate_id)
            .copied()
            .unwrap_or(CandidateKind::Unknown),
        remote_candidate_kind: candidates
            .get(&pair.remote_candidate_id)
            .copied()
            .unwrap_or(CandidateKind::Unknown),
        local_candidate_id: pair.local_candidate_id,
        remote_candidate_id: pair.remote_candidate_id,
        nominated: pair.nominated,
        packets_sent: pair.packets_sent,
        packets_received: pair.packets_received,
        bytes_sent: pair.bytes_sent,
        bytes_received: pair.bytes_received,
        current_round_trip_time: pair.current_round_trip_time,
    })
}

fn candidate_kind(value: &str) -> CandidateKind {
    match value {
        "host" => CandidateKind::Host,
        "srflx" => CandidateKind::ServerReflexive,
        "prflx" => CandidateKind::PeerReflexive,
        "relay" => CandidateKind::Relay,
        _ => CandidateKind::Unknown,
    }
}

/// A checkpoint from one physical peer's native report. ICE 0.12 leaves the
/// candidate-pair payload counters at zero. Its data-channel and ICE transport
/// counters measure real traffic, so a live probe measures their change while
/// the physical peer and nominated pair remain the same.
pub(crate) struct ProbeTrafficObservation {
    peer_id: String,
    channel_id: String,
    channel_identifier: u16,
    transport_id: String,
    pair: SelectedCandidatePairStats,
    messages_sent: usize,
    messages_received: usize,
    bytes_sent: usize,
    bytes_received: usize,
    transport_bytes_sent: usize,
    transport_bytes_received: usize,
}

pub(crate) fn probe_traffic_observation(report: StatsReport) -> Option<ProbeTrafficObservation> {
    let mut candidates = HashMap::new();
    let mut selected = None;
    let mut peer = None;
    let mut channel = None;
    let mut transport = None;
    for entry in report.reports.into_values() {
        match entry {
            StatsReportType::LocalCandidate(candidate)
            | StatsReportType::RemoteCandidate(candidate) => {
                candidates.insert(
                    candidate.id,
                    candidate_kind(&candidate.candidate_type.to_string()),
                );
            }
            StatsReportType::CandidatePair(pair) if pair.nominated => {
                if selected.replace(pair).is_some() {
                    return None;
                }
            }
            StatsReportType::PeerConnection(value) => {
                if peer.replace(value).is_some() {
                    return None;
                }
            }
            StatsReportType::DataChannel(value) if value.label == CTRL_REL_LABEL => {
                if value.state != RTCDataChannelState::Open || channel.replace(value).is_some() {
                    return None;
                }
            }
            StatsReportType::Transport(value) => {
                if transport.replace(value).is_some() {
                    return None;
                }
            }
            _ => {}
        }
    }
    let pair = selected?;
    let peer = peer?;
    let channel = channel?;
    let transport = transport?;
    if peer.id.is_empty() || channel.id.is_empty() || transport.id.is_empty() {
        return None;
    }
    Some(ProbeTrafficObservation {
        peer_id: peer.id,
        channel_id: channel.id,
        channel_identifier: channel.data_channel_identifier,
        transport_id: transport.id,
        pair: SelectedCandidatePairStats {
            local_candidate_kind: candidates.get(&pair.local_candidate_id).copied()?,
            remote_candidate_kind: candidates.get(&pair.remote_candidate_id).copied()?,
            local_candidate_id: pair.local_candidate_id,
            remote_candidate_id: pair.remote_candidate_id,
            nominated: pair.nominated,
            packets_sent: pair.packets_sent,
            packets_received: pair.packets_received,
            bytes_sent: pair.bytes_sent,
            bytes_received: pair.bytes_received,
            current_round_trip_time: pair.current_round_trip_time,
        },
        messages_sent: channel.messages_sent,
        messages_received: channel.messages_received,
        bytes_sent: channel.bytes_sent,
        bytes_received: channel.bytes_received,
        transport_bytes_sent: transport.bytes_sent,
        transport_bytes_received: transport.bytes_received,
    })
}

impl ProbeTrafficObservation {
    pub(crate) fn measured_since(
        &self,
        before: &Self,
        payload_bytes: usize,
    ) -> Option<SelectedCandidatePairStats> {
        if payload_bytes == 0
            || self.peer_id != before.peer_id
            || self.channel_id != before.channel_id
            || self.channel_identifier != before.channel_identifier
            || self.transport_id != before.transport_id
            || self.pair.local_candidate_id != before.pair.local_candidate_id
            || self.pair.remote_candidate_id != before.pair.remote_candidate_id
            || !self.pair.nominated
            || !before.pair.nominated
            || self.pair.local_candidate_kind != CandidateKind::Relay
            || self.pair.remote_candidate_kind != CandidateKind::Relay
            || self.pair.local_candidate_kind != before.pair.local_candidate_kind
            || self.pair.remote_candidate_kind != before.pair.remote_candidate_kind
        {
            return None;
        }
        let sent = self.messages_sent.checked_sub(before.messages_sent)?;
        let received = self
            .messages_received
            .checked_sub(before.messages_received)?;
        let bytes_sent = self.bytes_sent.checked_sub(before.bytes_sent)?;
        let bytes_received = self.bytes_received.checked_sub(before.bytes_received)?;
        let transport_sent = self
            .transport_bytes_sent
            .checked_sub(before.transport_bytes_sent)?;
        let transport_received = self
            .transport_bytes_received
            .checked_sub(before.transport_bytes_received)?;
        if sent == 0
            || received == 0
            || bytes_sent < payload_bytes
            || bytes_received < payload_bytes
            || transport_sent < bytes_sent
            || transport_received < bytes_received
        {
            return None;
        }
        let mut pair = self.pair.clone();
        // These are actual SCTP application packet/payload deltas from this
        // probe window, not the unimplemented ICE candidate-pair counters.
        pair.packets_sent = u32::try_from(sent).ok()?;
        pair.packets_received = u32::try_from(received).ok()?;
        pair.bytes_sent = u64::try_from(bytes_sent).ok()?;
        pair.bytes_received = u64::try_from(bytes_received).ok()?;
        Some(pair)
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;

    fn observation(sent: usize, received: usize, bytes: usize) -> ProbeTrafficObservation {
        ProbeTrafficObservation {
            peer_id: "physical-peer-a".into(),
            channel_id: "channel-a".into(),
            channel_identifier: 0,
            transport_id: "ice_transport".into(),
            pair: SelectedCandidatePairStats {
                local_candidate_id: "local-a".into(),
                remote_candidate_id: "remote-a".into(),
                local_candidate_kind: CandidateKind::Relay,
                remote_candidate_kind: CandidateKind::Relay,
                nominated: true,
                packets_sent: 0,
                packets_received: 0,
                bytes_sent: 0,
                bytes_received: 0,
                current_round_trip_time: 0.0,
            },
            messages_sent: sent,
            messages_received: received,
            bytes_sent: bytes,
            bytes_received: bytes,
            transport_bytes_sent: bytes * 2,
            transport_bytes_received: bytes * 2,
        }
    }

    #[test]
    fn probe_measures_only_real_counter_changes_in_its_window() {
        let before = observation(100, 200, 1000);
        let after = observation(101, 201, 1021);
        let pair = after.measured_since(&before, 21).unwrap();
        assert_eq!((pair.packets_sent, pair.packets_received), (1, 1));
        assert_eq!((pair.bytes_sent, pair.bytes_received), (21, 21));
        assert!(before.measured_since(&before, 21).is_none());
    }

    #[test]
    fn probe_rejects_foreign_peers_routes_channels_and_counter_reset() {
        let before = observation(100, 200, 1000);
        for change in 0..12 {
            let mut after = observation(101, 201, 1021);
            match change {
                0 => after.peer_id = "physical-peer-b".into(),
                1 => after.channel_id = "channel-b".into(),
                2 => after.channel_identifier = 2,
                3 => after.transport_id = "other-transport".into(),
                4 => after.pair.local_candidate_id = "new-generation-local".into(),
                5 => after.pair.remote_candidate_id = "new-generation-remote".into(),
                6 => after.pair.nominated = false,
                7 => after.pair.local_candidate_kind = CandidateKind::Host,
                8 => after.messages_sent = 99,
                9 => after.messages_received = 199,
                10 => after.bytes_received = 1000,
                11 => after.transport_bytes_sent = before.transport_bytes_sent,
                _ => unreachable!(),
            }
            assert!(
                after.measured_since(&before, 21).is_none(),
                "change {change}"
            );
        }
    }
}
