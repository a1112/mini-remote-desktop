use mrd_quality_gate::{evaluate, evaluate_allowed_skip, Evaluation, GatePolicy, Verdict};

fn policy(name: &str) -> GatePolicy {
    let raw = match name {
        "strict-required-metrics.v1.json" => include_str!("../../../tests/quality-gates/policies/strict-required-metrics.v1.json"),
        "diagnostic-allowed-skip.v1.json" => include_str!("../../../tests/quality-gates/policies/diagnostic-allowed-skip.v1.json"),
        _ => panic!("unknown fixture"),
    };
    serde_json::from_str(raw).unwrap()
}

fn fixture(name: &str) -> mrd_quality_gate::RemoteExperienceRun {
    let raw = match name {
        "missing-present.json" => include_str!("../../../tests/quality-gates/fixtures/missing-present.json"),
        "valid-direct.json" => include_str!("../../../tests/quality-gates/fixtures/valid-direct.json"),
        _ => panic!("unknown fixture"),
    };
    serde_json::from_str(raw).unwrap()
}

#[test]
fn missing_required_metric_is_invalid_not_skipped() {
    let result = evaluate(&fixture("missing-present.json"), &policy("strict-required-metrics.v1.json"));
    assert_eq!(result.verdict, Verdict::InvalidArtifact);
}

#[test]
fn release_profile_downgrade_is_product_failure() {
    let mut run = fixture("valid-direct.json");
    run.media.profile_downgraded = true;
    let result = evaluate(&run, &policy("strict-required-metrics.v1.json"));
    assert_eq!(result.verdict, Verdict::ProductFail);
}

#[test]
fn explicitly_allowlisted_capability_skip_is_allowed() {
    let result: Evaluation = evaluate_allowed_skip(
        &policy("diagnostic-allowed-skip.v1.json"),
        "diagnostic.local",
        "gpu_probe",
        "hardware_unavailable",
    );
    assert_eq!(result.verdict, Verdict::AllowedSkip);
}
