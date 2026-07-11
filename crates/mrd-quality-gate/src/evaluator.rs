use crate::policy::route_selector_matches;
use crate::{
    validate_artifact_for_policy, validate_policy, ArtifactError, GatePolicy, RemoteExperienceRun,
    Verdict,
};
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

    fn infra(message: impl Into<String>) -> Self {
        Self {
            verdict: Verdict::InfraFail,
            failures: vec![message.into()],
        }
    }
}

pub fn evaluate(run: &RemoteExperienceRun, policy: &GatePolicy) -> Evaluation {
    if let Err(error) = validate_policy(policy) {
        return Evaluation::infra(format!("policy is invalid: {error}"));
    }
    if let Err(error) = validate_artifact_for_policy(run, policy) {
        return Evaluation::invalid(error);
    }

    let mut failures = Vec::new();
    let required = policy
        .required_scenarios
        .iter()
        .any(|scenario| scenario == &run.scenario.id);
    if !required && (run.scenario.required || !policy.required_scenarios.is_empty()) {
        failures.push(format!(
            "scenario is not declared by policy: {}",
            run.scenario.id
        ));
    }
    if required && !run.scenario.required {
        failures.push(format!(
            "policy-required scenario is marked optional by artifact: {}",
            run.scenario.id
        ));
    }
    evaluate_secure_lan_requirements(run, policy, &mut failures);
    evaluate_security_negative_requirements(run, policy, &mut failures);
    if !failures.is_empty() {
        return Evaluation {
            verdict: Verdict::ProductFail,
            failures,
        };
    }

    if required && run.media.profile_downgraded {
        failures.push("required media profile was downgraded".to_owned());
    }
    if run.producer_status != "completed" {
        failures.push(format!("producer status is {}", run.producer_status));
    }

    let mut applicable_thresholds = 0usize;
    for rule in &policy.thresholds {
        if !route_selector_matches(&rule.route, &run.route.selected) {
            continue;
        }
        applicable_thresholds += 1;
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
    if !policy.thresholds.is_empty() && applicable_thresholds == 0 {
        failures.push(format!(
            "no threshold applies to selected route: {}",
            run.route.selected
        ));
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

fn evaluate_secure_lan_requirements(
    run: &RemoteExperienceRun,
    policy: &GatePolicy,
    failures: &mut Vec<String>,
) {
    let Some(requirements) = &policy.secure_lan_requirements else {
        return;
    };
    let Some(security) = &run.security else {
        failures.push("secure LAN security evidence is missing".to_owned());
        return;
    };
    let Some(side_effects) = &run.side_effects else {
        failures.push("secure LAN side-effect evidence is missing".to_owned());
        return;
    };

    if security.identity_state != requirements.identity_state {
        failures.push(format!(
            "identity state is {}, expected {}",
            security.identity_state, requirements.identity_state
        ));
    }
    if security.authorization_outcome != requirements.authorization_outcome {
        failures.push(format!(
            "authorization outcome is {}, expected {}",
            security.authorization_outcome, requirements.authorization_outcome
        ));
    }
    if !requirements
        .allowed_authorization_bases
        .contains(&security.authorization_basis)
    {
        failures.push(format!(
            "authorization basis is not allowed: {}",
            security.authorization_basis
        ));
    }
    if security.scope_authorized != requirements.scope_authorized {
        failures.push(format!(
            "scope authorization is {}, expected {}",
            security.scope_authorized, requirements.scope_authorized
        ));
    }
    if run.route.selected != requirements.route_selected {
        failures.push(format!(
            "selected route is {}, expected {}",
            run.route.selected, requirements.route_selected
        ));
    }
    if security.quic_peer_authenticated != requirements.quic_peer_authenticated {
        failures.push(format!(
            "QUIC peer authentication is {}, expected {}",
            security.quic_peer_authenticated, requirements.quic_peer_authenticated
        ));
    }
    if security.rejected {
        failures.push("authorized secure LAN session was rejected".to_owned());
    }
    if side_effects.media_frames_presented < requirements.min_real_frames_presented {
        failures.push(format!(
            "media_frames_presented={} is below minimum {}",
            side_effects.media_frames_presented, requirements.min_real_frames_presented
        ));
    }
    if security.control_input_authenticated != requirements.control_input_authenticated {
        failures.push(format!(
            "control input authentication is {}, expected {}",
            security.control_input_authenticated, requirements.control_input_authenticated
        ));
    }
    if side_effects.control_events_injected < requirements.min_control_events_injected {
        failures.push(format!(
            "control_events_injected={} is below minimum {}",
            side_effects.control_events_injected, requirements.min_control_events_injected
        ));
    }
    if security.cleanup_completed != requirements.cleanup_completed {
        failures.push(format!(
            "cleanup completion is {}, expected {}",
            security.cleanup_completed, requirements.cleanup_completed
        ));
    }
    if run.audit_event_ids.len() < requirements.min_audit_events {
        failures.push(format!(
            "audit_event_ids count {} is below minimum {}",
            run.audit_event_ids.len(),
            requirements.min_audit_events
        ));
    }
}

fn evaluate_security_negative_requirements(
    run: &RemoteExperienceRun,
    policy: &GatePolicy,
    failures: &mut Vec<String>,
) {
    let Some(requirements) = &policy.security_negative_requirements else {
        return;
    };
    let Some(security) = &run.security else {
        failures.push("security-negative evidence is missing".to_owned());
        return;
    };
    let Some(side_effects) = &run.side_effects else {
        failures.push("security-negative side-effect evidence is missing".to_owned());
        return;
    };

    let Some(attempt) = requirements
        .attempts
        .iter()
        .find(|attempt| attempt.scenario == run.scenario.id)
    else {
        failures.push(format!(
            "security-negative scenario is not mapped by policy: {}",
            run.scenario.id
        ));
        return;
    };
    if security.attempt_kind != attempt.attempt_kind {
        failures.push(format!(
            "security attempt kind is {}, expected {} for {}",
            security.attempt_kind, attempt.attempt_kind, run.scenario.id
        ));
    }
    if security.rejection_reason != attempt.rejection_reason {
        failures.push(format!(
            "security rejection reason is {}, expected {} for {}",
            security.rejection_reason, attempt.rejection_reason, run.scenario.id
        ));
    }
    if requirements.require_rejected && !security.rejected {
        failures.push("security attempt was not rejected".to_owned());
    }
    for (name, actual, maximum) in [
        (
            "sender_tasks_started",
            side_effects.sender_tasks_started,
            requirements.max_sender_tasks_started,
        ),
        (
            "receiver_tasks_started",
            side_effects.receiver_tasks_started,
            requirements.max_receiver_tasks_started,
        ),
        (
            "media_packets_sent",
            side_effects.media_packets_sent,
            requirements.max_media_packets_sent,
        ),
        (
            "media_frames_presented",
            side_effects.media_frames_presented,
            requirements.max_media_frames_presented,
        ),
        (
            "control_events_injected",
            side_effects.control_events_injected,
            requirements.max_control_events_injected,
        ),
    ] {
        if actual > maximum {
            failures.push(format!("{name}={actual} exceeds maximum {maximum}"));
        }
    }
    if requirements.require_cleanup_completed && !security.cleanup_completed {
        failures.push("security-negative cleanup did not complete".to_owned());
    }
    if run.audit_event_ids.len() < requirements.min_audit_events {
        failures.push(format!(
            "audit_event_ids count {} is below minimum {}",
            run.audit_event_ids.len(),
            requirements.min_audit_events
        ));
    }
}

pub fn evaluate_allowed_skip(
    policy: &GatePolicy,
    scenario: &str,
    capability: &str,
    reason: &str,
) -> Evaluation {
    if let Err(error) = validate_policy(policy) {
        return Evaluation::infra(format!("policy is invalid: {error}"));
    }
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
