use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GatePolicy {
    pub id: String,
    #[serde(default)]
    pub required_scenarios: Vec<String>,
    #[serde(default)]
    pub allow_skips: Vec<AllowedSkip>,
    #[serde(default)]
    pub thresholds: Vec<ThresholdRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowedSkip {
    pub scenario: String,
    pub capability: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ThresholdRule {
    pub metric: String,
    pub route: String,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
}
