use std::fs;

fn workflow() -> String {
    fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.github/workflows/mainline-e2e.yml")).unwrap()
}

#[test]
fn mainline_workflow_enforces_gate_after_upload_on_pull_requests() {
    let yaml = workflow();
    assert!(yaml.contains("quality-gate:"), "workflow must define a quality-gate job");
    assert!(yaml.contains("cargo test -p mrd-quality-gate"), "quality-gate tests must run");
    assert!(yaml.contains("if: always()"), "gate artifacts must upload on failure");
    assert!(yaml.contains("name: Enforce quality gate"), "workflow must have an explicit enforcement step");
    assert!(!yaml.contains("continue-on-error: true"), "enforcement must not be optional");
}

#[test]
fn windows_required_row_invokes_release_policy() {
    let yaml = workflow();
    assert!(yaml.contains("windows-1080p60-direct.v1.json"));
    assert!(yaml.contains("cargo run -p mrd-quality-gate"));
    assert!(yaml.contains("--artifact tests/quality-gates/fixtures/valid-direct.json"));
}
