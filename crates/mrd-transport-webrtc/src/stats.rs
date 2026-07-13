use std::collections::HashMap;

use webrtc::stats::{ICECandidatePairStats, StatsReport, StatsReportType};

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
