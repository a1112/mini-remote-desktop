use crate::{validate_artifact, ArtifactError, GatePolicy, RemoteExperienceRun, Verdict};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evaluation {
    pub verdict: Verdict,
    pub failures: Vec<String>,
}

impl Evaluation {
    fn invalid(error: ArtifactError) -> Self {
        Self {
            verdict: Verdict::InvalidArtifact,
            failures: vec![error.to_string()],
        }
    }
}

pub fn evaluate(run: &RemoteExperienceRun, policy: &GatePolicy) -> Evaluation {
    if let Err(error) = validate_artifact(run) {
        return Evaluation::invalid(error);
    }

    let required = policy
        .required_scenarios
        .iter()
        .any(|scenario| scenario == &run.scenario.id);
    let mut failures = Vec::new();

    if run.scenario.required && !required {
        failures.push(format!("scenario is not declared by policy: {}", run.scenario.id));
    }
    if required && run.media.profile_downgraded {
        failures.push("required media profile was downgraded".to_owned());
    }
    if run.producer_status != "completed" {
        failures.push(format!("producer status is {}", run.producer_status));
    }

    for rule in &policy.thresholds {
        if rule.route != run.route.selected {
            continue;
        }
        let value = match rule.metric.as_str() {
            "visible_first_frame_ms" => run.present.visible_first_frame_ms,
            _ => None,
        };
        let Some(value) = value else {
            failures.push(format!("threshold metric is unavailable: {}", rule.metric));
            continue;
        };
        if rule.min.is_some_and(|min| value < min) {
            failures.push(format!("{}={} is below minimum", rule.metric, value));
        }
        if rule.max.is_some_and(|max| value > max) {
            failures.push(format!("{}={} exceeds maximum", rule.metric, value));
        }
    }

    if failures.is_empty() {
        Evaluation {
            verdict: Verdict::Pass,
            failures,
        }
    } else {
        Evaluation {
            verdict: Verdict::ProductFail,
            failures,
        }
    }
}

pub fn evaluate_allowed_skip(
    policy: &GatePolicy,
    scenario: &str,
    capability: &str,
    reason: &str,
) -> Evaluation {
    let allowed = policy.allow_skips.iter().any(|rule| {
        rule.scenario == scenario && rule.capability == capability && rule.reason == reason
    });
    if allowed {
        Evaluation {
            verdict: Verdict::AllowedSkip,
            failures: Vec::new(),
        }
    } else {
        Evaluation {
            verdict: Verdict::ProductFail,
            failures: vec![format!(
                "skip is not allowlisted: scenario={scenario}, capability={capability}, reason={reason}"
            )],
        }
    }
}
