use anyhow::{Context, Result};

const REMOTE_POWER_ACTIONS_ENABLED_ENV: &str = "MRD_ENABLE_REMOTE_POWER_ACTIONS";

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemotePowerCommand {
    program: String,
    args: Vec<String>,
}

pub(super) fn accept_lan_remote_device_power_action(
    action: &mrd_ipc::RemoteDevicePowerAction,
) -> Result<()> {
    accept_lan_remote_device_power_action_with_runner(
        action,
        |key| std::env::var(key).ok(),
        run_remote_power_command,
    )
}

fn accept_lan_remote_device_power_action_with_runner<E, R>(
    action: &mrd_ipc::RemoteDevicePowerAction,
    env_lookup: E,
    mut runner: R,
) -> Result<()>
where
    E: Fn(&str) -> Option<String>,
    R: FnMut(&RemotePowerCommand) -> Result<()>,
{
    if !remote_power_actions_enabled(env_lookup) {
        let action_label = remote_power_action_label(action);
        anyhow::bail!("remote power executor is not enabled on this peer for {action_label}");
    }
    let command = platform_remote_power_command(action);
    runner(&command)
}

fn remote_power_actions_enabled<E>(env_lookup: E) -> bool
where
    E: Fn(&str) -> Option<String>,
{
    matches!(
        env_lookup(REMOTE_POWER_ACTIONS_ENABLED_ENV)
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn remote_power_action_label(action: &mrd_ipc::RemoteDevicePowerAction) -> &'static str {
    match action {
        mrd_ipc::RemoteDevicePowerAction::Restart => "restart",
        mrd_ipc::RemoteDevicePowerAction::Shutdown => "shutdown",
    }
}

#[cfg(windows)]
fn platform_remote_power_command(action: &mrd_ipc::RemoteDevicePowerAction) -> RemotePowerCommand {
    let mode = match action {
        mrd_ipc::RemoteDevicePowerAction::Restart => "/r",
        mrd_ipc::RemoteDevicePowerAction::Shutdown => "/s",
    };
    RemotePowerCommand {
        program: "shutdown.exe".to_string(),
        args: vec![mode.to_string(), "/t".to_string(), "0".to_string()],
    }
}

#[cfg(not(windows))]
fn platform_remote_power_command(action: &mrd_ipc::RemoteDevicePowerAction) -> RemotePowerCommand {
    let mode = match action {
        mrd_ipc::RemoteDevicePowerAction::Restart => "-r",
        mrd_ipc::RemoteDevicePowerAction::Shutdown => "-h",
    };
    RemotePowerCommand {
        program: "shutdown".to_string(),
        args: vec![mode.to_string(), "now".to_string()],
    }
}

fn run_remote_power_command(command: &RemotePowerCommand) -> Result<()> {
    std::process::Command::new(&command.program)
        .args(&command.args)
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn remote power command {} {}",
                command.program,
                command.args.join(" ")
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_power_executor_rejects_by_default() {
        let mut invoked = false;
        let error = accept_lan_remote_device_power_action_with_runner(
            &mrd_ipc::RemoteDevicePowerAction::Restart,
            |_| None,
            |_| {
                invoked = true;
                Ok(())
            },
        )
        .expect_err("remote power must be opt-in");

        assert!(!invoked);
        assert!(error.to_string().contains("not enabled"));
    }

    #[test]
    fn remote_power_executor_invokes_platform_command_when_enabled() {
        let mut commands = Vec::new();
        accept_lan_remote_device_power_action_with_runner(
            &mrd_ipc::RemoteDevicePowerAction::Shutdown,
            |key| {
                if key == REMOTE_POWER_ACTIONS_ENABLED_ENV {
                    Some("1".to_string())
                } else {
                    None
                }
            },
            |command| {
                commands.push(command.clone());
                Ok(())
            },
        )
        .expect("enabled remote power executor");

        assert_eq!(
            commands,
            vec![platform_remote_power_command(
                &mrd_ipc::RemoteDevicePowerAction::Shutdown
            )]
        );
    }
}
