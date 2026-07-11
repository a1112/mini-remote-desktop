use mrd_session_agent::bootstrap::run_from_authenticated_launcher;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match run_from_authenticated_launcher().await {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mrd-session-agent: {error}");
            ExitCode::FAILURE
        }
    }
}
