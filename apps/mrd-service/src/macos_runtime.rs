//! Foreground/LaunchAgent runtime in the logged-in macOS user's security context.
use crate::{app_state, ipc_server::IpcServer, lan_discovery, security, shell, AppState};
use anyhow::{Context, Result};
use nix::fcntl::{Flock, FlockArg};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, sync::Arc};

pub async fn run() -> Result<()> {
    // Set before creating the persistent database, journals, and IPC socket.
    unsafe {
        nix::libc::umask(0o077);
    }
    let directory = security::protected_product_data_dir().map_err(anyhow::Error::msg)?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW)
        .open(directory.join("service.lock"))?;
    let _instance =
        Flock::lock(lock_file, FlockArg::LockExclusiveNonblock).map_err(|(_, error)| {
            anyhow::anyhow!("Rdesk Service is already running or cannot acquire its lock: {error}")
        })?;
    let tray = shell::default_tray();
    let state = Arc::new(
        AppState::open_persistent_with_tray_and_lan_discovery_config(
            tray.clone(),
            lan_discovery::LanDiscoveryConfig::from_env()?,
            directory.join("security-state-v2.sqlite3"),
            security::platform_secret_protector().map_err(anyhow::Error::msg)?,
        )?,
    );
    let (id, name) = app_state::default_lan_device_identity();
    let id = if std::env::var("MRD_LAN_DEVICE_ID")
        .ok()
        .is_some_and(|id| !id.trim().is_empty())
    {
        id
    } else {
        device_id_from_key(
            state
                .device_identities
                .machine_key_id()
                .context("persistent device identity is unavailable")?,
        )
    };
    let name = if name == "Rdesk LAN Device" {
        let mut hostname = [0_u8; 256];
        if unsafe { nix::libc::gethostname(hostname.as_mut_ptr().cast(), hostname.len()) } == 0 {
            String::from_utf8_lossy(
                &hostname[..hostname
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(hostname.len())],
            )
            .into_owned()
        } else {
            name
        }
    } else {
        name
    };
    state
        .devices
        .lock()
        .await
        .register_if_unregistered(id, name);
    state.shell.lock().await.tray_available = tray.lock().unwrap().is_available();
    state.refresh_capability_snapshot_in_background();
    lan_discovery::start_lan_discovery(state.clone())
        .await
        .context("start authenticated LAN discovery")?;
    let ipc = IpcServer::new(state.clone());
    let management = IpcServer::new_management(state.clone());
    let _shutdown_runtime = state.shutdown.bind_runtime()?;
    let bridge = crate::web_bridge::spawn_from_env(ipc.clone()).await?;
    let web_abort = bridge.as_ref().map(tokio::task::JoinHandle::abort_handle);
    let mut ipc_task = Box::pin(ipc.run());
    let mut management_task = Box::pin(management.run());
    let mut web_task = Box::pin(crate::web_bridge::wait_for_task(bridge));
    tracing::info!(
        input_available = state.control_input.lock().await.is_available(),
        "macOS LAN service initialized with persistent Keychain identity"
    );
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut shutdown_mode = mrd_ipc::ShutdownMode::Graceful;
    let mut management_available = true;
    let result = tokio::select! {
        result = &mut ipc_task => result,
        result = &mut management_task => {
            management_available = false;
            result
        },
        result = &mut web_task => result,
        mode = crate::shutdown::wait_for_shutdown(&state) => {
            shutdown_mode = mode;
            Ok(())
        },
        result = tokio::signal::ctrl_c() => result.map_err(Into::into),
        _ = terminate.recv() => Ok(()),
    };
    let _ = state.shutdown.request(shutdown_mode.clone());
    drop(ipc_task);
    drop(web_task);
    if let Some(abort) = web_abort {
        abort.abort();
    }
    let cleanup = async {
        let sessions = state.sessions.lock().await.list_all();
        for session in sessions {
            let response = crate::handlers::session::stop_session(&state, session.session_id).await;
            if let mrd_ipc::IpcResponse::Error { message, .. } = response {
                tracing::warn!(%message, "session cleanup failed during macOS shutdown");
            }
        }
        state.session_audio.lock().await.stop_all();
        state
            .control_input
            .lock()
            .await
            .release_all_sessions()
            .context("release remote input on shutdown")?;
        state.webrtc_host.shutdown().await?;
        Ok::<(), anyhow::Error>(())
    };
    let mut cleanup = Box::pin(tokio::time::timeout(
        crate::shutdown::cleanup_timeout(&shutdown_mode),
        cleanup,
    ));
    let cleanup_result = loop {
        tokio::select! {
            result = &mut cleanup => break result.context("macOS service cleanup deadline exceeded").and_then(|result| result),
            // Lifecycle status remains reachable while session resources are released.
            result = &mut management_task, if management_available => {
                management_available = false;
                tracing::warn!(?result, "Management IPC stopped during macOS cleanup");
            },
            _ = state.shutdown.changed() => {
                if shutdown_mode != mrd_ipc::ShutdownMode::Force
                    && state.shutdown.ready_mode_at_epoch(usize::MAX, state.shutdown.admission_epoch()) == Some(mrd_ipc::ShutdownMode::Force)
                {
                    break Err(anyhow::anyhow!("macOS service cleanup interrupted by forced shutdown"));
                }
            },
        }
    };
    drop(cleanup);
    drop(management_task);
    let tray_result = tray.lock().unwrap().shutdown().map_err(Into::into);
    tracing::info!(
        cleanup_succeeded = result.is_ok() && cleanup_result.is_ok() && tray_result.is_ok(),
        "macOS service stopped"
    );
    result.and(cleanup_result).and(tray_result)
}

fn device_id_from_key(key_id: &str) -> mrd_proto::DeviceId {
    mrd_proto::DeviceId(format!(
        "lan-{}",
        key_id
            .rsplit(':')
            .next()
            .unwrap_or(key_id)
            .chars()
            .take(24)
            .collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finder_launched_macs_have_stable_distinct_device_ids_without_hostname_environment() {
        let a = device_id_from_key(&"ab".repeat(32));
        let b = device_id_from_key(&"cd".repeat(32));
        assert_ne!(a, b);
        assert_eq!(a, device_id_from_key(&"ab".repeat(32)));
        assert_eq!(a.0.len(), 28);
    }
}
