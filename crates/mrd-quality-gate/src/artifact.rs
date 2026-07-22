use crate::{GatePolicy, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

const REMOTE_EXPERIENCE_SCHEMA_VERSION: &str = "remote-experience-run.v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub security: Option<SecurityEvidence>,
    #[serde(default)]
    pub side_effects: Option<SideEffectEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScenarioIdentity {
    pub id: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouteEvidence {
    pub requested: String,
    pub selected: String,
    pub candidate_pair: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MediaEvidence {
    pub requested_profile: String,
    pub selected_profile: String,
    pub profile_downgraded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PresentMetrics {
    pub visible_first_frame_ms: Option<f64>,
    pub input_to_photon_ms: Vec<f64>,
    pub fps_windows: Vec<f64>,
    pub freeze_count: u64,
    pub stall_ms: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResourceEvidence {
    pub cpu_percent: Vec<f64>,
    pub gpu_percent: Vec<f64>,
    pub rss_bytes: Vec<f64>,
    pub vram_bytes: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SecurityEvidence {
    pub attempt_kind: String,
    pub identity_state: String,
    pub authorization_outcome: String,
    pub authorization_basis: String,
    pub scope_authorized: bool,
    pub quic_peer_authenticated: bool,
    pub control_input_authenticated: bool,
    pub rejected: bool,
    pub rejection_reason: String,
    pub cleanup_completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SideEffectEvidence {
    pub sender_tasks_started: u64,
    pub receiver_tasks_started: u64,
    pub media_packets_sent: u64,
    pub media_frames_presented: u64,
    pub control_events_injected: u64,
}

#[derive(Debug, Error, PartialEq)]
pub enum ArtifactError {
    #[error("unsupported schema version: {0}")]
    UnsupportedSchemaVersion(String),
    #[error("required string is empty: {0}")]
    EmptyRequiredString(&'static str),
    #[error("missing required metric: {0}")]
    MissingRequiredMetric(&'static str),
    #[error("required sample set is empty: {0}")]
    EmptyRequiredSamples(&'static str),
    #[error("numeric field is not finite: {0}")]
    NonFiniteMetric(&'static str),
    #[error("duplicate audit event id: {0}")]
    DuplicateAuditEventId(String),
}

pub fn validate_artifact(run: &RemoteExperienceRun) -> Result<(), ArtifactError> {
    let profile = match run.security.as_ref() {
        Some(security) if security.rejected => ArtifactValidationProfile::SecurityNegative,
        Some(_) => ArtifactValidationProfile::SecureLanPositive,
        None => ArtifactValidationProfile::Standard,
    };
    validate_artifact_with_profile(run, profile)
}

pub fn validate_artifact_for_policy(
    run: &RemoteExperienceRun,
    policy: &GatePolicy,
) -> Result<(), ArtifactError> {
    let profile = if policy.security_negative_requirements.is_some() {
        ArtifactValidationProfile::SecurityNegative
    } else if policy.secure_lan_requirements.is_some() {
        ArtifactValidationProfile::SecureLanPositive
    } else {
        ArtifactValidationProfile::Standard
    };
    validate_artifact_with_profile(run, profile)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactValidationProfile {
    Standard,
    SecureLanPositive,
    SecurityNegative,
}

fn validate_artifact_with_profile(
    run: &RemoteExperienceRun,
    profile: ArtifactValidationProfile,
) -> Result<(), ArtifactError> {
    validate_artifact_identity(run)?;
    validate_security_evidence_pair(run, profile)?;

    if profile != ArtifactValidationProfile::SecurityNegative
        && run.present.visible_first_frame_ms.is_none()
    {
        return Err(ArtifactError::MissingRequiredMetric(
            "visible_first_frame_ms",
        ));
    }

    let all_samples = [
        (
            "input_to_photon_ms",
            run.present.input_to_photon_ms.as_slice(),
        ),
        ("fps_windows", run.present.fps_windows.as_slice()),
        ("stall_ms", run.present.stall_ms.as_slice()),
        ("cpu_percent", run.resources.cpu_percent.as_slice()),
        ("gpu_percent", run.resources.gpu_percent.as_slice()),
        ("rss_bytes", run.resources.rss_bytes.as_slice()),
        ("vram_bytes", run.resources.vram_bytes.as_slice()),
    ];
    for (name, values) in all_samples {
        if values.iter().any(|value| !value.is_finite()) {
            return Err(ArtifactError::NonFiniteMetric(name));
        }
    }

    let required_samples: &[(&'static str, &[f64])] = match profile {
        ArtifactValidationProfile::Standard => &[
            (
                "input_to_photon_ms",
                run.present.input_to_photon_ms.as_slice(),
            ),
            ("fps_windows", run.present.fps_windows.as_slice()),
            ("cpu_percent", run.resources.cpu_percent.as_slice()),
            ("gpu_percent", run.resources.gpu_percent.as_slice()),
            ("rss_bytes", run.resources.rss_bytes.as_slice()),
            ("vram_bytes", run.resources.vram_bytes.as_slice()),
        ],
        ArtifactValidationProfile::SecureLanPositive => {
            &[("fps_windows", run.present.fps_windows.as_slice())]
        }
        ArtifactValidationProfile::SecurityNegative => &[],
    };
    for (name, values) in required_samples {
        if values.is_empty() {
            return Err(ArtifactError::EmptyRequiredSamples(name));
        }
    }

    if run
        .present
        .visible_first_frame_ms
        .is_some_and(|value| !value.is_finite())
    {
        return Err(ArtifactError::NonFiniteMetric("visible_first_frame_ms"));
    }
    Ok(())
}

fn validate_artifact_identity(run: &RemoteExperienceRun) -> Result<(), ArtifactError> {
    if run.schema_version != REMOTE_EXPERIENCE_SCHEMA_VERSION {
        return Err(ArtifactError::UnsupportedSchemaVersion(
            run.schema_version.clone(),
        ));
    }
    if run.run_id.trim().is_empty() {
        return Err(ArtifactError::EmptyRequiredString("run_id"));
    }
    if run.scenario.id.trim().is_empty() {
        return Err(ArtifactError::EmptyRequiredString("scenario.id"));
    }
    if run.audit_event_ids.is_empty() {
        return Err(ArtifactError::EmptyRequiredSamples("audit_event_ids"));
    }

    let mut audit_ids = HashSet::new();
    for audit_id in &run.audit_event_ids {
        let audit_id = audit_id.trim();
        if audit_id.is_empty() {
            return Err(ArtifactError::EmptyRequiredString("audit_event_ids[]"));
        }
        if !audit_ids.insert(audit_id) {
            return Err(ArtifactError::DuplicateAuditEventId(audit_id.to_owned()));
        }
    }
    Ok(())
}

fn validate_security_evidence_pair(
    run: &RemoteExperienceRun,
    profile: ArtifactValidationProfile,
) -> Result<(), ArtifactError> {
    match (run.security.is_some(), run.side_effects.is_some()) {
        (true, false) => return Err(ArtifactError::MissingRequiredMetric("side_effects")),
        (false, true) => return Err(ArtifactError::MissingRequiredMetric("security")),
        _ => {}
    }

    if profile != ArtifactValidationProfile::Standard && run.security.is_none() {
        return Err(ArtifactError::MissingRequiredMetric("security"));
    }
    if let Some(security) = &run.security {
        for (name, value) in [
            ("security.attempt_kind", security.attempt_kind.as_str()),
            ("security.identity_state", security.identity_state.as_str()),
            (
                "security.authorization_outcome",
                security.authorization_outcome.as_str(),
            ),
            (
                "security.authorization_basis",
                security.authorization_basis.as_str(),
            ),
            (
                "security.rejection_reason",
                security.rejection_reason.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(ArtifactError::EmptyRequiredString(name));
            }
        }
    }
    Ok(())
}
