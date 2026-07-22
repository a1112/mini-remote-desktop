use mrd_quality_gate::{evaluate, Evaluation, GatePolicy, RemoteExperienceRun, Verdict};
use std::{env, fs, path::PathBuf, process};

fn main() {
    process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = env::args().collect();
    let Some(artifact_path) = value_for(&args, "--artifact") else {
        eprintln!("missing --artifact");
        return Verdict::InfraFail.exit_code();
    };
    let Some(policy_path) = value_for(&args, "--policy") else {
        eprintln!("missing --policy");
        return Verdict::InfraFail.exit_code();
    };
    let output_path = value_for(&args, "--output");

    let result = match (
        fs::read_to_string(&artifact_path),
        fs::read_to_string(&policy_path),
    ) {
        (Ok(artifact_raw), Ok(policy_raw)) => {
            match serde_json::from_str::<RemoteExperienceRun>(&artifact_raw) {
                Ok(artifact) => match serde_json::from_str::<GatePolicy>(&policy_raw) {
                    Ok(policy) => evaluate(&artifact, &policy),
                    Err(error) => infra(format!("policy is unreadable: {error}")),
                },
                Err(error) => invalid(format!("artifact is invalid: {error}")),
            }
        }
        (Err(error), _) | (_, Err(error)) => infra(format!("input is unreadable: {error}")),
    };

    let serialized = match serde_json::to_string_pretty(&result) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("cannot serialize evaluation: {error}");
            return Verdict::InfraFail.exit_code();
        }
    };
    if let Some(output_path) = output_path {
        if let Err(error) = fs::write(PathBuf::from(output_path), &serialized) {
            eprintln!("cannot write evaluation: {error}");
            return Verdict::InfraFail.exit_code();
        }
        println!("{:?}: {} failure(s)", result.verdict, result.failures.len());
    } else {
        println!("{serialized}");
    }
    result.verdict.exit_code()
}

fn value_for(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

fn invalid(message: String) -> Evaluation {
    Evaluation {
        verdict: Verdict::InvalidArtifact,
        failures: vec![message],
    }
}

fn infra(message: String) -> Evaluation {
    Evaluation {
        verdict: Verdict::InfraFail,
        failures: vec![message],
    }
}
