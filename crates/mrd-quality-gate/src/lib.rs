use serde::{Deserialize, Serialize};

mod artifact;
mod evaluator;
mod policy;

pub use artifact::{validate_artifact, ArtifactError, RemoteExperienceRun};
pub use evaluator::{evaluate, evaluate_allowed_skip, Evaluation};
pub use policy::{AllowedSkip, GatePolicy, ThresholdRule};

/// Stable product-gate outcomes shared by scripts, CI, and release artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    #[serde(rename = "PASS")]
    Pass,
    #[serde(rename = "PRODUCT_FAIL")]
    ProductFail,
    #[serde(rename = "INFRA_FAIL")]
    InfraFail,
    #[serde(rename = "INVALID_ARTIFACT")]
    InvalidArtifact,
    #[serde(rename = "ALLOWED_SKIP")]
    AllowedSkip,
}

impl Verdict {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Pass | Self::AllowedSkip => 0,
            Self::ProductFail => 2,
            Self::InfraFail => 3,
            Self::InvalidArtifact => 4,
        }
    }
}
