use webrtc::stats::{StatsReport, StatsReportType};

#[derive(Debug, Clone, PartialEq)]
pub struct SelectedCandidatePairStats {
    pub local_candidate_id: String,
    pub remote_candidate_id: String,
    pub nominated: bool,
    pub packets_sent: u32,
    pub packets_received: u32,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub current_round_trip_time: f64,
}

pub(crate) fn selected_candidate_pair(report: StatsReport) -> Option<SelectedCandidatePairStats> {
    report.reports.into_values().find_map(|entry| match entry {
        StatsReportType::CandidatePair(pair) if pair.nominated => {
            Some(SelectedCandidatePairStats {
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
        _ => None,
    })
}
