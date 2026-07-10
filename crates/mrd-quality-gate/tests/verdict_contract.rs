use mrd_quality_gate::Verdict;

#[test]
fn verdict_exit_codes_are_stable() {
    assert_eq!(Verdict::Pass.exit_code(), 0);
    assert_eq!(Verdict::AllowedSkip.exit_code(), 0);
    assert_eq!(Verdict::ProductFail.exit_code(), 2);
    assert_eq!(Verdict::InfraFail.exit_code(), 3);
    assert_eq!(Verdict::InvalidArtifact.exit_code(), 4);
}

#[test]
fn verdict_serializes_to_stable_wire_names() {
    assert_eq!(serde_json::to_string(&Verdict::Pass).unwrap(), "\"PASS\"");
    assert_eq!(serde_json::to_string(&Verdict::AllowedSkip).unwrap(), "\"ALLOWED_SKIP\"");
    assert_eq!(serde_json::to_string(&Verdict::ProductFail).unwrap(), "\"PRODUCT_FAIL\"");
    assert_eq!(serde_json::to_string(&Verdict::InfraFail).unwrap(), "\"INFRA_FAIL\"");
    assert_eq!(serde_json::to_string(&Verdict::InvalidArtifact).unwrap(), "\"INVALID_ARTIFACT\"");
}
