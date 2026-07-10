use mrd_quality_gate::{validate_artifact, ArtifactError, RemoteExperienceRun};

#[test]
fn required_present_metric_cannot_be_missing() {
    let raw = include_str!("../../../tests/quality-gates/fixtures/missing-present.json");
    let run: RemoteExperienceRun = serde_json::from_str(raw).unwrap();
    assert_eq!(
        validate_artifact(&run),
        Err(ArtifactError::MissingRequiredMetric("visible_first_frame_ms"))
    );
}

#[test]
fn finite_complete_direct_fixture_is_valid() {
    let raw = include_str!("../../../tests/quality-gates/fixtures/valid-direct.json");
    let run: RemoteExperienceRun = serde_json::from_str(raw).unwrap();
    assert!(validate_artifact(&run).is_ok());
}
