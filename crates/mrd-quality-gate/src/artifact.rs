use crate::Verdict;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteExperienceRun {
    pub schema_version: String,
    pub run_id: String,
    pub scenario: ScenarioIdentity,
    pub route: RouteEvidence,
    pub media: MediaEvidence,
    pub present: PresentMetrics,
    pub resources: ResourceEvidence,
    pub producer_status: String,
    pub gate_status: Verdict,
    pub audit_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScenarioIdentity {
    pub id: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteEvidence {
    pub requested: String,
    pub selected: String,
    pub candidate_pair: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MediaEvidence {
    pub requested_profile: String,
    pub selected_profile: String,
    pub profile_downgraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresentMetrics {
    pub visible_first_frame_ms: Option<f64>,
    pub input_to_photon_ms: Vec<f64>,
    pub fps_windows: Vec<f64>,
    pub freeze_count: u64,
    pub stall_ms: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceEvidence {
    pub cpu_percent: Vec<f64>,
    pub gpu_percent: Vec<f64>,
    pub rss_bytes: Vec<f64>,
    pub vram_bytes: Vec<f64>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ArtifactError {
    #[error("missing required metric: {0}")]
    MissingRequiredMetric(&'static str),
    #[error("required sample set is empty: {0}")]
    EmptyRequiredSamples(&'static str),
    #[error("numeric field is not finite: {0}")]
    NonFiniteMetric(&'static str),
}

pub fn validate_artifact(run: &RemoteExperienceRun) -> Result<(), ArtifactError> {
    if run.present.visible_first_frame_ms.is_none() {
        return Err(ArtifactError::MissingRequiredMetric("visible_first_frame_ms"));
    }

    let required_samples = [
        ("input_to_photon_ms", run.present.input_to_photon_ms.as_slice()),
        ("fps_windows", run.present.fps_windows.as_slice()),
        ("cpu_percent", run.resources.cpu_percent.as_slice()),
        ("gpu_percent", run.resources.gpu_percent.as_slice()),
        ("rss_bytes", run.resources.rss_bytes.as_slice()),
        ("vram_bytes", run.resources.vram_bytes.as_slice()),
    ];
    for (name, values) in required_samples {
        if values.is_empty() {
            return Err(ArtifactError::EmptyRequiredSamples(name));
        }
        if values.iter().any(|value| !value.is_finite()) {
            return Err(ArtifactError::NonFiniteMetric(name));
        }
    }

    if run
        .present
        .visible_first_frame_ms
        .is_some_and(|value| !value.is_finite())
    {
        return Err(ArtifactError::NonFiniteMetric("visible_first_frame_ms"));
    }
    for (name, values) in [
        ("stall_ms", run.present.stall_ms.as_slice()),
    ] {
        if values.iter().any(|value| !value.is_finite()) {
            return Err(ArtifactError::NonFiniteMetric(name));
        }
    }
    if run.audit_event_ids.is_empty() {
        return Err(ArtifactError::EmptyRequiredSamples("audit_event_ids"));
    }
    Ok(())
}
