use std::sync::{Arc, Mutex};

use mrd_ipc::IpcResponse;
use mrd_service::{
    handlers::shell::{autostart_status, refresh_autostart_state, set_autostart},
    AppState, AutostartPort, AutostartPortRef, NoOpAutostart,
};

struct TestAutostart {
    enabled: Mutex<bool>,
    query_fails: bool,
    change_fails: bool,
    ignores_changes: bool,
}

impl AutostartPort for TestAutostart {
    fn is_enabled(&self) -> anyhow::Result<bool> {
        anyhow::ensure!(!self.query_fails, "SCM query denied");
        Ok(*self.enabled.lock().unwrap())
    }

    fn set_enabled(&self, enabled: bool) -> anyhow::Result<()> {
        anyhow::ensure!(!self.change_fails, "SCM change denied");
        if !self.ignores_changes {
            *self.enabled.lock().unwrap() = enabled;
        }
        Ok(())
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn get_entry_name(&self) -> &str {
        "MiniRemoteDesktop"
    }
}

fn autostart(query_fails: bool, change_fails: bool, ignores_changes: bool) -> AutostartPortRef {
    Arc::new(Mutex::new(TestAutostart {
        enabled: Mutex::new(false),
        query_fails,
        change_fails,
        ignores_changes,
    }))
}

fn assert_error(response: IpcResponse, expected_code: &str, expected_message: &str) {
    match response {
        IpcResponse::Error { code, message } => {
            assert_eq!(code, expected_code);
            assert!(message.contains(expected_message), "{message}");
        }
        response => panic!("expected an error, received {response:?}"),
    }
}

#[test]
fn autostart_query_failure_is_reported_instead_of_disabled() {
    assert_error(
        autostart_status(&autostart(true, false, false)),
        "E500",
        "SCM query denied",
    );
}

#[test]
fn unsupported_autostart_status_is_explicit() {
    let port: AutostartPortRef = Arc::new(Mutex::new(NoOpAutostart::new("test")));
    assert!(matches!(
        autostart_status(&port),
        IpcResponse::AutostartStatus {
            enabled: false,
            supported: false
        }
    ));
}

#[cfg(windows)]
#[test]
fn windows_autostart_uses_the_installed_service_name() {
    let port = mrd_service::WindowsAutostart::new("mrd-service");
    assert_eq!(
        port.get_entry_name(),
        mrd_service::windows_service::MRD_WINDOWS_SERVICE_NAME
    );
}

#[cfg(windows)]
#[test]
#[ignore = "Requires the installed MiniRemoteDesktop service; queries SCM without changing it"]
fn windows_autostart_queries_the_live_service_configuration() {
    use windows_service::{
        service::{ServiceAccess, ServiceStartType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let port = mrd_service::WindowsAutostart::new("mrd-service");
    let manager =
        ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).unwrap();
    let service = manager
        .open_service(
            mrd_service::windows_service::MRD_WINDOWS_SERVICE_NAME,
            ServiceAccess::QUERY_CONFIG,
        )
        .unwrap();
    let expected = service.query_config().unwrap().start_type == ServiceStartType::AutoStart;
    assert!(port.is_supported());
    assert_eq!(port.is_enabled().unwrap(), expected);
}

#[tokio::test]
async fn unsupported_autostart_change_is_rejected_as_unsupported() {
    let state = Arc::new(AppState::new());
    let port: AutostartPortRef = Arc::new(Mutex::new(NoOpAutostart::new("test")));
    assert_error(
        set_autostart(&state, &port, true).await,
        "E501",
        "not supported",
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, None);
}

#[tokio::test]
async fn autostart_change_failure_is_reported_without_claiming_success() {
    let state = Arc::new(AppState::new());
    assert_error(
        set_autostart(&state, &autostart(false, true, false), true).await,
        "E500",
        "SCM change denied",
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, None);
}

#[tokio::test]
async fn autostart_change_requires_successful_readback() {
    let state = Arc::new(AppState::new());
    assert_error(
        set_autostart(&state, &autostart(true, false, false), true).await,
        "E500",
        "SCM query denied",
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, None);
}

#[tokio::test]
async fn autostart_change_reports_when_configuration_does_not_match() {
    let state = Arc::new(AppState::new());
    assert_error(
        set_autostart(&state, &autostart(false, false, true), true).await,
        "E500",
        "requested state",
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, Some(false));
}

#[tokio::test]
async fn autostart_change_reads_and_caches_the_actual_configuration() {
    let state = Arc::new(AppState::new());
    let port = autostart(false, false, false);
    assert!(matches!(
        set_autostart(&state, &port, true).await,
        IpcResponse::Ack
    ));
    assert_eq!(state.shell.lock().await.autostart_enabled, Some(true));
    assert!(matches!(
        set_autostart(&state, &port, false).await,
        IpcResponse::Ack
    ));
    assert_eq!(state.shell.lock().await.autostart_enabled, Some(false));
}

#[tokio::test]
async fn shell_snapshot_refresh_observes_configuration_changes() {
    let state = Arc::new(AppState::new());
    let port = autostart(false, false, false);
    assert_eq!(
        refresh_autostart_state(&state, &port).await.unwrap(),
        Some(false)
    );
    port.lock().unwrap().set_enabled(true).unwrap();
    assert_eq!(
        refresh_autostart_state(&state, &port).await.unwrap(),
        Some(true)
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, Some(true));
}

#[tokio::test]
async fn shell_snapshot_refresh_clears_stale_state_after_query_failure() {
    let state = Arc::new(AppState::new());
    state.shell.lock().await.autostart_enabled = Some(true);
    assert!(
        refresh_autostart_state(&state, &autostart(true, false, false))
            .await
            .is_err()
    );
    assert_eq!(state.shell.lock().await.autostart_enabled, None);
}
