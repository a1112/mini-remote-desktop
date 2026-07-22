use anyhow::Result;

const REMOTE_POWER_ACTIONS_ENABLED_ENV: &str = "MRD_ENABLE_REMOTE_POWER_ACTIONS";

pub(super) fn accept_lan_remote_device_power_action(
    action: &mrd_ipc::RemoteDevicePowerAction,
) -> Result<()> {
    accept_lan_remote_device_power_action_with_runner(
        action,
        |key| std::env::var(key).ok(),
        || anyhow::bail!("legacy unsigned remote power execution is disabled"),
    )
}

// Keep the historic environment and runner inputs at this seam so tests prove
// that neither can authorize or execute an unauthenticated v1 packet.
fn accept_lan_remote_device_power_action_with_runner<E, R>(
    action: &mrd_ipc::RemoteDevicePowerAction,
    env_lookup: E,
    mut runner: R,
) -> Result<()>
where
    E: Fn(&str) -> Option<String>,
    R: FnMut() -> Result<()>,
{
    let _legacy_opt_in = env_lookup(REMOTE_POWER_ACTIONS_ENABLED_ENV);
    let _ = &mut runner;
    reject_legacy_unsigned_remote_power_action(action)
}

fn reject_legacy_unsigned_remote_power_action(
    action: &mrd_ipc::RemoteDevicePowerAction,
) -> Result<()> {
    let action_label = remote_power_action_label(action);
    anyhow::bail!(
        "legacy unsigned remote power action '{action_label}' is disabled; signed authorization is required"
    )
}

fn remote_power_action_label(action: &mrd_ipc::RemoteDevicePowerAction) -> &'static str {
    match action {
        mrd_ipc::RemoteDevicePowerAction::Restart => "restart",
        mrd_ipc::RemoteDevicePowerAction::Shutdown => "shutdown",
    }
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
            || {
                invoked = true;
                Ok(())
            },
        )
        .expect_err("unsigned remote power must be rejected");

        assert!(!invoked);
        assert!(error.to_string().contains("legacy unsigned"));
        assert!(error.to_string().contains("signed authorization"));
    }

    #[test]
    fn remote_power_executor_rejects_even_when_legacy_env_is_enabled() {
        let mut invoked = false;
        let error = accept_lan_remote_device_power_action_with_runner(
            &mrd_ipc::RemoteDevicePowerAction::Shutdown,
            |key| {
                if key == REMOTE_POWER_ACTIONS_ENABLED_ENV {
                    Some("1".to_string())
                } else {
                    None
                }
            },
            || {
                invoked = true;
                Ok(())
            },
        )
        .expect_err("an environment switch must not authorize an unsigned LAN power action");

        assert!(!invoked);
        assert!(error.to_string().contains("legacy unsigned"));
        assert!(error.to_string().contains("signed authorization"));
    }
}
