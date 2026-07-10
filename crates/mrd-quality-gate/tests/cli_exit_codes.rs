use std::{path::PathBuf, process::Command};

fn run_gate(artifact: &str, policy: &str) -> std::process::Output {
    let output = std::env::temp_dir().join(format!("mrd-quality-gate-{}.json", std::process::id()));
    let artifact_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/quality-gates/fixtures")
        .join(artifact);
    let policy_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/quality-gates/policies")
        .join(policy);
    let binary = std::env::var("CARGO_BIN_EXE_mrd-quality-gate")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_mrd_quality_gate"))
        .unwrap();
    Command::new(binary)
        .args([
            "--artifact",
            artifact_path.to_str().unwrap(),
            "--policy",
            policy_path.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

#[test]
fn invalid_artifact_exits_four() {
    let output = run_gate("missing-present.json", "windows-1080p60-direct.v1.json");
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn valid_direct_fixture_exits_zero() {
    let output = run_gate("valid-direct.json", "windows-1080p60-direct.v1.json");
    assert_eq!(output.status.code(), Some(0));
}
