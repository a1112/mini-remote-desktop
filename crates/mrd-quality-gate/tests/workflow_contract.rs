use std::fs;

fn workflow() -> String {
    fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../.github/workflows/mainline-e2e.yml"
    ))
    .unwrap()
}

#[test]
fn mainline_workflow_enforces_gate_after_upload_on_pull_requests() {
    let yaml = workflow();
    assert!(
        yaml.contains("quality-gate:"),
        "workflow must define a quality-gate job"
    );
    assert!(
        yaml.contains("cargo test -p mrd-quality-gate"),
        "quality-gate tests must run"
    );
    assert!(
        yaml.contains("if: always()"),
        "gate artifacts must upload on failure"
    );
    assert!(
        yaml.contains("name: Enforce quality gate"),
        "workflow must have an explicit enforcement step"
    );
    assert!(
        !yaml.contains("continue-on-error: true"),
        "enforcement must not be optional"
    );
}

#[test]
fn windows_required_row_invokes_release_policy() {
    let yaml = workflow();
    assert!(yaml.contains("windows-1080p60-direct.v1.json"));
    assert!(yaml.contains("cargo run -p mrd-quality-gate"));
    assert!(yaml.contains("--artifact tests/quality-gates/fixtures/valid-direct.json"));
}

#[test]
fn gate_zero_runs_and_archives_security_negative_evidence() {
    let yaml = workflow();
    assert!(yaml.contains("tests/benchmarks/scripts/run_secure_lan_negative.ps1"));
    assert!(yaml.contains("secure-lan-negative.log"));
    assert!(yaml.contains("artifacts/e2e/security-negative/"));
    assert!(
        !yaml.contains("run_secure_lan_negative.ps1 2>&1 | tee secure-lan-negative.log || true"),
        "security-negative failures must propagate through the gate"
    );
}

#[test]
fn secure_lan_positive_gate_is_explicit_and_device_lab_only() {
    let yaml = workflow();
    assert!(yaml.contains("secure-lan-device-lab:"));
    assert!(yaml.contains("needs: [l0-l1-generic, quality-gate]"));
    assert!(yaml.contains("vars.MRD_DEVICE_LAB_SECURE_LAN_ENABLED == 'true'"));
    assert!(yaml.contains("runs-on: [self-hosted, Windows, X64, device-lab]"));
    assert!(yaml.contains("-ScenarioId\", \"cross.e2e.secure_remote_display\""));
    assert!(yaml.contains("-ProfileId\", \"1080p60\""));
    assert!(yaml.contains("artifacts/e2e/device-lab/secure-lan/"));
}
