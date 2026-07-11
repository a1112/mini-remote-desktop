use mrd_session_agent::runtime::{PrivateAgentEndpoint, AGENT_PRIVATE_ENDPOINT_ENV};
use std::process::ExitCode;

fn main() -> ExitCode {
    let Ok(endpoint) = std::env::var(AGENT_PRIVATE_ENDPOINT_ENV) else {
        eprintln!("mrd-session-agent: authenticated launcher endpoint is required");
        return ExitCode::FAILURE;
    };
    if PrivateAgentEndpoint::parse(endpoint).is_err() {
        eprintln!("mrd-session-agent: configured endpoint is not platform-local");
        return ExitCode::FAILURE;
    }

    // Task 23 supplies the authenticated signer and OS-verified descriptor.
    // Standalone launch must not synthesize either trust input.
    eprintln!("mrd-session-agent: authenticated machine-service bootstrap is unavailable");
    ExitCode::FAILURE
}
