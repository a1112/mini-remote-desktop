//! mrd-service machine-service and foreground-console entry points.

#[cfg(any(windows, target_os = "macos"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(windows)]
use mrd_agent_ipc::StopReason;
#[cfg(windows)]
use mrd_service::agent_runtime::{
    eligible_interactive_sessions, installed_session_agent_path, AgentServer, ExecuteGrantIssuer,
    WindowsSessionAgentSupervisor,
};
#[cfg(windows)]
use mrd_service::{
    app_state::{self, AppState},
    ipc_server::IpcServer,
    lan_discovery, relay, security, shell, signaling, wan_session, web_bridge,
    windows_service::{
        ServiceControl as LifecycleControl, ServiceLifecycle,
        SessionChange as LifecycleSessionChange, MRD_WINDOWS_SERVICE_SID,
    },
};
#[cfg(target_os = "macos")]
use mrd_service::{
    app_state::{self, AppState},
    ipc_server::IpcServer,
    lan_discovery, security, shell, web_bridge,
};
#[cfg(windows)]
use ring::rand::{SecureRandom, SystemRandom};
#[cfg(any(windows, target_os = "macos"))]
use std::sync::Arc;
#[cfg(any(windows, target_os = "macos"))]
use tracing::warn;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
enum RuntimeControl {
    Stop,
    PreShutdown,
    SessionChange(LifecycleSessionChange),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Console,
    #[cfg(windows)]
    WindowsService,
}

#[cfg(windows)]
fn requests_existing_store_verification(arguments: &[std::ffi::OsString]) -> Result<bool> {
    const FLAG: &str = "--verify-existing-store";
    if !arguments
        .iter()
        .any(|argument| argument.to_string_lossy().starts_with(FLAG))
    {
        return Ok(false);
    }
    if arguments.len() != 1 || arguments[0] != FLAG {
        anyhow::bail!("--verify-existing-store accepts no additional arguments");
    }
    Ok(true)
}

#[cfg(windows)]
fn run_existing_store_verification() -> Result<()> {
    let verified = (|| -> Result<mrd_store_sqlite::ExistingStoreVerification> {
        let policy =
            security::ProductDirectoryAclPolicy::installed_service(MRD_WINDOWS_SERVICE_SID)
                .map_err(anyhow::Error::msg)?;
        let product_data =
            security::verify_protected_product_data_dir(&policy).map_err(anyhow::Error::msg)?;
        let protector = security::platform_secret_protector().map_err(anyhow::Error::msg)?;
        Ok(
            mrd_store_sqlite::PersistentStore::verify_existing_read_only(
                product_data.join("security-state-v2.sqlite3"),
                protector,
            )?,
        )
    })()
    .map_err(|_| anyhow::anyhow!("existing protected store verification failed"))?;
    println!(
        "{}",
        serde_json::json!({
            "format_version": verified.format_version,
            "identity_initialized": verified.identity_initialized,
        })
    );
    Ok(())
}

fn main() -> Result<()> {
    #[cfg(windows)]
    {
        let arguments: Vec<_> = std::env::args_os().skip(1).collect();
        if requests_existing_store_verification(&arguments)? {
            return run_existing_store_verification();
        }
    }

    // Set the process creation policy before the multi-thread runtime starts:
    // SQLite, WAL/SHM and service-owned state must remain private to this user.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::umask(0o077);
    }
    initialize_logging();

    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--check-macos-permissions")
    {
        anyhow::ensure!(
            arguments.len() == 1,
            "--check-macos-permissions must be run on its own"
        );
        #[cfg(target_os = "macos")]
        {
            // Probe this signed executable, rather than a shell/helper with a
            // different TCC identity. Never prompt, capture, inject, or open DB.
            println!(
                "{}",
                serde_json::json!({
                    "screen_recording": mrd_capture_macos::screen_capture_access_is_granted(),
                    "accessibility": mrd_input::macos::is_accessibility_trusted(),
                })
            );
            return Ok(());
        }
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("--check-macos-permissions is only supported on macOS");
    }
    if arguments
        .iter()
        .any(|argument| argument == "--authorize-keychain")
    {
        anyhow::ensure!(
            arguments.len() == 1,
            "--authorize-keychain must be run on its own"
        );
        #[cfg(target_os = "macos")]
        {
            eprintln!("正在请求 macOS 登录钥匙串授权；请在系统对话框中确认当前 mrd-service。");
            security::authorize_keychain_access().map_err(anyhow::Error::msg)?;
            eprintln!("钥匙串授权检查成功，可以重新启动后台服务。");
            return Ok(());
        }
        #[cfg(not(target_os = "macos"))]
        anyhow::bail!("--authorize-keychain is only supported on macOS");
    }

    #[cfg(windows)]
    if arguments.iter().any(|argument| argument == "--service") {
        info!("mrd-service dispatching to Windows SCM");
        return scm_host::run().context("Windows service dispatcher failed");
    }

    info!("mrd-service starting in foreground console mode");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run_service(RunMode::Console, None, StatusReporter::Console));
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    result
}

fn initialize_logging() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(windows)]
async fn run_service(
    mode: RunMode,
    mut controls: Option<tokio::sync::mpsc::UnboundedReceiver<RuntimeControl>>,
    reporter: StatusReporter,
) -> Result<()> {
    let tray: app_state::TrayPortRef = if mode == RunMode::WindowsService {
        shell::service_tray()
    } else {
        shell::default_tray()
    };
    let lan_discovery_config = lan_discovery::LanDiscoveryConfig::from_env()?;
    let app_state = open_protected_app_state(tray.clone(), lan_discovery_config, mode)?;
    initialize_application_state(&app_state, &tray).await;

    let issuer = Arc::new(new_execute_grant_issuer()?);
    app_state.bind_console_capture_issuer(Arc::clone(&issuer));
    // AppState owns a fresh, empty registry; the new issuer invalidates grants
    // from earlier service processes. invalidate_all permanently closes admission
    // and is reserved for terminal security failures, not ordinary startup.
    let agent_server = Arc::new(AgentServer::new(Arc::clone(&app_state.agent_registry)));
    app_state.bind_agent_media_server(Arc::clone(&agent_server));
    let mut agents = if mode == RunMode::WindowsService {
        let executable = installed_session_agent_path()?;
        Some(WindowsSessionAgentSupervisor::new(
            Arc::clone(&app_state.agent_registry),
            agent_server,
            issuer,
            executable,
        )?)
    } else {
        info!("Session Agent supervision is disabled in foreground console mode");
        None
    };

    let mut lifecycle = ServiceLifecycle::new();
    let _ = lifecycle.apply(LifecycleControl::Start);
    if let Some(supervisor) = agents.as_mut() {
        for session_id in eligible_interactive_sessions()? {
            let effects = lifecycle.apply(LifecycleControl::SessionChange(
                LifecycleSessionChange::Logon(session_id),
            ));
            if !effects.is_empty() {
                supervisor.launch(session_id).await?;
                info!(
                    session_id,
                    "Session Agent launch requested for active session"
                );
            }
        }
    }

    match lan_discovery::start_lan_discovery(app_state.clone()).await {
        Ok(()) => info!("LAN peer discovery started"),
        Err(error) => {
            let mut shell = app_state.shell.lock().await;
            shell.last_error = Some(format!("LAN discovery failed: {error}"));
            warn!("LAN peer discovery unavailable: {error}");
        }
    }

    if mode == RunMode::Console {
        let initial_model = shell::TrayModel::default();
        if let Err(error) = tray.lock().unwrap().install(initial_model) {
            warn!("Tray not available: {error}");
        }
    }

    let wan_backend = if let Some(config) = wan_session::config::WanSessionBackendConfig::from_env()
        .context("WAN session backend configuration failed")?
    {
        let backend: Arc<dyn wan_session::backend::WanSessionBackend> = Arc::new(
            wan_session::backend::HttpWanSessionBackend::new(config)
                .context("WAN session backend startup failed")?,
        );
        app_state
            .bind_wan_session_backend(Arc::clone(&backend))
            .map_err(anyhow::Error::msg)?;
        info!("Device-authenticated WAN session backend configured");
        Some(backend)
    } else {
        info!("Initial WAN sessions disabled (MRD_WAN_SESSION_API_URL is unset)");
        None
    };

    let relay_client = if let Some(config) =
        relay::RelayClientConfig::from_env().context("relay directory configuration failed")?
    {
        let client = Arc::new(
            relay::RelayDirectoryClient::new(config)
                .context("relay directory client startup failed")?,
        );
        app_state
            .bind_relay_directory_client(client)
            .map_err(anyhow::Error::msg)?;
        info!("Verified multi-region relay directory configured");
        app_state.relay_directory_client()
    } else {
        info!("Multi-region relay directory disabled (MRD_RELAY_DIRECTORY_URL is unset)");
        None
    };

    let signaling_task = signaling::spawn_from_env(Arc::clone(&app_state))
        .await
        .context("authenticated signaling startup failed")?;
    if signaling_task.is_some() {
        info!("Authenticated realtime signaling started");
    } else {
        info!("Authenticated realtime signaling disabled (MRD_SIGNAL_URL is unset)");
    }

    let relay_responder = if let (Some(client), true) = (relay_client, signaling_task.is_some()) {
        let executor: Arc<dyn relay::RelayMigrationExecutor> = Arc::new(
            relay::ServiceRelayMigrationExecutor::new(
                Arc::clone(&app_state),
                std::time::Duration::from_secs(20),
            )
            .context("relay migration executor configuration failed")?,
        );
        let provider: Arc<dyn relay::RelayAccessProvider> = client;
        let input: Arc<dyn relay::RelayInputBarrier> =
            Arc::new(relay::ServiceRelayInputBarrier::new(&app_state));
        let coordinator = Arc::new(
            relay::RelayFailoverCoordinator::new(
                provider,
                executor,
                input,
                Arc::new(relay::SystemRelayClock),
                std::time::Duration::from_secs(3),
            )
            .context("relay failover coordinator configuration failed")?,
        );
        app_state
            .bind_relay_failover_coordinator(Arc::clone(&coordinator))
            .map_err(anyhow::Error::msg)?;
        info!("Authenticated multi-region relay failover runtime started");
        Some(relay::spawn_relay_migration_responder(
            Arc::clone(&app_state),
            coordinator,
        ))
    } else {
        if app_state.relay_directory_client().is_some() {
            warn!("Relay directory is configured but failover is inactive without signaling");
        }
        None
    };

    let wan_session_task = if let (Some(backend), true) = (wan_backend, relay_responder.is_some()) {
        let task = wan_session::service::bind_and_spawn_wan_session_service(
            Arc::clone(&app_state),
            backend,
        )
        .await
        .context("attended WAN session runtime startup failed")?;
        info!("Attended WAN relay session runtime started");
        Some(task)
    } else {
        if app_state.wan_session_backend().is_some() {
            warn!(
                "WAN session backend is configured but initial WAN sessions are inactive without authenticated signaling and relay failover"
            );
        }
        None
    };

    let public_connection_task = mrd_service::public_connection::spawn(Arc::clone(&app_state))
        .await
        .context("resident public connection startup failed")?;
    let ipc_server = IpcServer::new(Arc::clone(&app_state));
    let management_server = IpcServer::new_management(Arc::clone(&app_state));
    let product_server = IpcServer::new_product(Arc::clone(&app_state));
    let _shutdown_runtime = app_state.shutdown.bind_runtime()?;
    let web_bridge_task = web_bridge::spawn_from_env(ipc_server.clone()).await?;
    let web_abort = web_bridge_task
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    let (ipc_ready, ipc_ready_rx) = tokio::sync::oneshot::channel();
    let (management_ready, management_ready_rx) = tokio::sync::oneshot::channel();
    let (product_ready, product_ready_rx) = tokio::sync::oneshot::channel();
    let mut ipc_task = Box::pin(ipc_server.run_with_ready(ipc_ready));
    let mut management_task = Box::pin(management_server.run_with_ready(management_ready));
    let mut product_task = Box::pin(product_server.run_with_ready(product_ready));
    let mut web_task = Box::pin(web_bridge::wait_for_task(web_bridge_task));
    let mut agent_reconcile = tokio::time::interval(std::time::Duration::from_secs(5));
    agent_reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut wan_session_expiry = tokio::time::interval(std::time::Duration::from_secs(1));
    wan_session_expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let startup = async {
        ipc_ready_rx
            .await
            .context("IPC startup failed before readiness")?;
        management_ready_rx
            .await
            .context("Management IPC startup failed before readiness")?;
        product_ready_rx
            .await
            .context("Product IPC startup failed before readiness")?;
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result = startup => result?,
        result = &mut ipc_task => return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("IPC stopped during startup"))),
        result = &mut management_task => return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("Management IPC stopped during startup"))),
        result = &mut product_task => return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("Product IPC stopped during startup"))),
    }
    reporter.running()?;
    info!("mrd-service running");

    let mut runtime_error = None;
    let mut management_available = true;
    let mut shutdown_mode = mrd_ipc::ShutdownMode::Graceful;
    let shutdown_reason = loop {
        tokio::select! {
            requested_mode = mrd_service::shutdown::wait_for_shutdown(&app_state) => {
                shutdown_mode = requested_mode;
                let _ = lifecycle.apply(LifecycleControl::Stop);
                break StopReason::ServiceShutdown;
            }
            result = &mut ipc_task => {
                if let Err(error) = result {
                    runtime_error = Some(error.context("IPC server stopped"));
                }
                break StopReason::ServiceShutdown;
            }
            result = &mut management_task => {
                management_available = false;
                runtime_error = Some(result.err().unwrap_or_else(|| anyhow::anyhow!("management IPC stopped")));
                break StopReason::ServiceShutdown;
            }
            result = &mut product_task => {
                runtime_error = Some(result.err().unwrap_or_else(|| anyhow::anyhow!("Product IPC stopped")));
                break StopReason::ServiceShutdown;
            }
            result = &mut web_task => {
                if let Err(error) = result {
                    runtime_error = Some(error.context("Web bridge stopped"));
                }
                break StopReason::ServiceShutdown;
            }
            control = next_control(&mut controls) => {
                match control {
                    RuntimeControl::Stop => {
                        let _ = lifecycle.apply(LifecycleControl::Stop);
                        break StopReason::ServiceShutdown;
                    }
                    RuntimeControl::PreShutdown => {
                        let _ = lifecycle.apply(LifecycleControl::PreShutdown);
                        break StopReason::ServiceShutdown;
                    }
                    RuntimeControl::SessionChange(change) => {
                        tokio::select! {
                            _ = handle_session_change(&mut lifecycle, agents.as_mut(), change) => {},
                            requested_mode = mrd_service::shutdown::wait_for_shutdown(&app_state) => {
                                shutdown_mode = requested_mode;
                                break StopReason::ServiceShutdown;
                            }
                        }
                    }
                }
            }
            _ = agent_reconcile.tick(), if agents.is_some() => {
                if let Some(supervisor) = agents.as_mut() {
                    tokio::select! {
                        _ = async {
                            app_state.console_capture.retry_pending_cleanup(&app_state).await;
                            supervisor.reconcile().await;
                        } => {},
                        requested_mode = mrd_service::shutdown::wait_for_shutdown(&app_state) => {
                            shutdown_mode = requested_mode;
                            break StopReason::ServiceShutdown;
                        }
                    }
                }
            }
            _ = wan_session_expiry.tick(), if app_state.wan_session_coordinator().is_some() => {
                tokio::select! {
                    _ = mrd_service::wan_session::service::expire_due_wan_sessions(&app_state) => {},
                    requested_mode = mrd_service::shutdown::wait_for_shutdown(&app_state) => {
                        shutdown_mode = requested_mode;
                        break StopReason::ServiceShutdown;
                    }
                }
            }
        }
    };

    reporter.stop_pending()?;
    // SCM/console stop must close the same admission fence as local IPC stop.
    let _ = app_state.shutdown.request(shutdown_mode.clone());
    drop(ipc_task);
    drop(product_task);
    drop(web_task);
    if let Some(abort) = web_abort {
        abort.abort();
    }
    let cleanup = async {
        let sessions_to_close = app_state.sessions.lock().await.list_all();
        for session in sessions_to_close {
            let response =
                mrd_service::handlers::session::stop_session(&app_state, session.session_id).await;
            if let mrd_ipc::IpcResponse::Error { message, .. } = response {
                runtime_error.get_or_insert_with(|| anyhow::anyhow!(message));
            }
        }
        if let Err(error) = public_connection_task.shutdown().await {
            runtime_error.get_or_insert(error);
        }
        if let Some(wan_session_task) = wan_session_task {
            wan_session_task.shutdown().await;
        }
        if let Some(relay_responder) = relay_responder {
            relay_responder.shutdown().await;
        }
        if let Some(signaling_task) = signaling_task {
            signaling_task.shutdown().await;
        }
        if let Err(error) = app_state.webrtc_host.shutdown().await {
            runtime_error.get_or_insert_with(|| error.into());
        }
        if let Some(supervisor) = agents.as_mut() {
            if let Err(error) = supervisor.stop_all(shutdown_reason).await {
                runtime_error.get_or_insert_with(|| error.into());
            }
        }
    };
    let mut cleanup = Box::pin(tokio::time::timeout(
        mrd_service::shutdown::cleanup_timeout(&shutdown_mode),
        cleanup,
    ));
    let cleanup_timed_out = loop {
        tokio::select! {
            result = &mut cleanup => break result.is_err(),
            // Keep lifecycle IPC reachable until resource cleanup is finished.
            result = &mut management_task, if management_available => {
                warn!("Management IPC stopped during cleanup: {result:?}");
                break (&mut cleanup).await.is_err();
            }
            _ = app_state.shutdown.changed() => {
                if shutdown_mode != mrd_ipc::ShutdownMode::Force && app_state.shutdown.ready_mode_at_epoch(usize::MAX, app_state.shutdown.admission_epoch()) == Some(mrd_ipc::ShutdownMode::Force) {
                    break true;
                }
            }
        }
    };
    drop(cleanup);
    if cleanup_timed_out {
        runtime_error.get_or_insert_with(|| anyhow::anyhow!("service cleanup deadline exceeded"));
        warn!("Service cleanup deadline exceeded; terminating remaining runtime tasks");
    }
    drop(agents);
    drop(management_task);
    let _ = tray.lock().unwrap().shutdown();
    reporter.stopped(runtime_error.is_some())?;
    info!(
        cleanup_succeeded = runtime_error.is_none(),
        "mrd-service stopped"
    );
    runtime_error.map_or(Ok(()), Err)
}

#[cfg(target_os = "macos")]
async fn run_service(
    _mode: RunMode,
    _controls: Option<()>,
    reporter: StatusReporter,
) -> Result<()> {
    let tray = shell::default_tray();
    let directory = security::ensure_protected_product_data_dir().map_err(anyhow::Error::msg)?;
    let protector = security::platform_secret_protector().map_err(anyhow::Error::msg)?;
    let app_state = Arc::new(
        AppState::open_persistent_with_tray_and_lan_discovery_config(
            tray.clone(),
            lan_discovery::LanDiscoveryConfig::from_env()?,
            directory.join("security-state-v2.sqlite3"),
            protector.clone(),
        )?,
    );
    security::verify_protected_product_data_dir().map_err(anyhow::Error::msg)?;
    app_state
        .public_connection
        .configure_persistence(
            directory,
            protector,
            app_state
                .device_identities
                .machine_key_id()
                .ok_or_else(|| anyhow::anyhow!("Persistent machine identity is unavailable"))?,
        )
        .context("public device credential persistence initialization failed")?;
    initialize_application_state(&app_state, &tray).await;
    if let Err(error) = lan_discovery::start_lan_discovery(app_state.clone()).await {
        app_state.shell.lock().await.last_error = Some(format!("LAN discovery failed: {error}"));
        warn!("LAN peer discovery unavailable: {error}");
    }
    if let Err(error) = tray.lock().unwrap().install(shell::TrayModel::default()) {
        warn!("Tray unavailable: {error}");
    }
    // macOS runs as the logged-in user and uses its native media adapters. It
    // does not impersonate or launch the Windows interactive Session Agent.
    let public_connection_task = mrd_service::public_connection::spawn(app_state.clone()).await?;
    let ipc_server = IpcServer::new(app_state.clone());
    let management_server = IpcServer::new_management(app_state.clone());
    let _shutdown_runtime = app_state.shutdown.bind_runtime()?;
    let web_bridge_task = web_bridge::spawn_from_env(ipc_server.clone()).await?;
    let web_abort = web_bridge_task
        .as_ref()
        .map(tokio::task::JoinHandle::abort_handle);
    let (ipc_ready, ipc_ready_rx) = tokio::sync::oneshot::channel();
    let (management_ready, management_ready_rx) = tokio::sync::oneshot::channel();
    let mut ipc_task = Box::pin(ipc_server.run_with_ready(ipc_ready));
    let mut management_task = Box::pin(management_server.run_with_ready(management_ready));
    let mut web_task = Box::pin(web_bridge::wait_for_task(web_bridge_task));
    let startup = async {
        ipc_ready_rx
            .await
            .context("IPC startup failed before readiness")?;
        management_ready_rx
            .await
            .context("Management IPC startup failed before readiness")?;
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result = tokio::time::timeout(std::time::Duration::from_secs(10), startup) => result.context("IPC startup deadline exceeded")??,
        result = &mut ipc_task => return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("IPC stopped during startup"))),
        result = &mut management_task => return Err(result.err().unwrap_or_else(|| anyhow::anyhow!("Management IPC stopped during startup"))),
    }
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut expiry = tokio::time::interval(std::time::Duration::from_secs(1));
    expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reporter.running()?;
    info!("mrd-service running in the macOS user session");
    let mut runtime_error = None;
    let mut management_available = true;
    let mut shutdown_mode = mrd_ipc::ShutdownMode::Graceful;
    loop {
        tokio::select! {
            requested = mrd_service::shutdown::wait_for_shutdown(&app_state) => {
                shutdown_mode = requested;
                break;
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = terminate.recv() => break,
            result = &mut ipc_task => {
                runtime_error = Some(result.err().unwrap_or_else(|| anyhow::anyhow!("IPC server stopped")));
                break;
            }
            result = &mut management_task => {
                management_available = false;
                runtime_error = Some(result.err().unwrap_or_else(|| anyhow::anyhow!("Management IPC stopped")));
                break;
            }
            result = &mut web_task => {
                if let Err(error) = result { runtime_error = Some(error.context("Web bridge stopped")); }
                break;
            }
            _ = expiry.tick(), if app_state.wan_session_coordinator().is_some() => {
                tokio::select! {
                    _ = mrd_service::wan_session::service::expire_due_wan_sessions(&app_state) => {},
                    requested = mrd_service::shutdown::wait_for_shutdown(&app_state) => { shutdown_mode = requested; break; }
                }
            }
        }
    }
    reporter.stop_pending()?;
    let _ = app_state.shutdown.request(shutdown_mode.clone());
    drop(ipc_task);
    drop(web_task);
    if let Some(abort) = web_abort {
        abort.abort();
    }
    let cleanup = async {
        let sessions = app_state.sessions.lock().await.list_all();
        for session in sessions {
            if let mrd_ipc::IpcResponse::Error { message, .. } =
                mrd_service::handlers::session::stop_session(&app_state, session.session_id).await
            {
                runtime_error.get_or_insert_with(|| anyhow::anyhow!(message));
            }
        }
        if let Err(error) = public_connection_task.shutdown().await {
            runtime_error.get_or_insert(error);
        }
        if let Err(error) = app_state.webrtc_host.shutdown().await {
            runtime_error.get_or_insert_with(|| error.into());
        }
    };
    let mut cleanup = Box::pin(tokio::time::timeout(
        mrd_service::shutdown::cleanup_timeout(&shutdown_mode),
        cleanup,
    ));
    let timed_out = loop {
        tokio::select! {
            result = &mut cleanup => break result.is_err(),
            result = &mut management_task, if management_available => {
                warn!("Management IPC stopped during cleanup: {result:?}");
                break (&mut cleanup).await.is_err();
            }
            _ = app_state.shutdown.changed() => {
                if shutdown_mode != mrd_ipc::ShutdownMode::Force
                    && app_state.shutdown.ready_mode_at_epoch(usize::MAX, app_state.shutdown.admission_epoch()) == Some(mrd_ipc::ShutdownMode::Force)
                { break true; }
            }
        }
    };
    drop(cleanup);
    if timed_out {
        runtime_error.get_or_insert_with(|| anyhow::anyhow!("service cleanup deadline exceeded"));
    }
    drop(management_task);
    let _ = tray.lock().unwrap().shutdown();
    reporter.stopped(runtime_error.is_some())?;
    info!(
        cleanup_succeeded = runtime_error.is_none(),
        "mrd-service stopped"
    );
    runtime_error.map_or(Ok(()), Err)
}

#[cfg(all(not(windows), not(target_os = "macos")))]
async fn run_service(
    _mode: RunMode,
    _controls: Option<()>,
    _reporter: StatusReporter,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "mrd-service production runtime requires a supported native secret protector"
    ))
}

#[cfg(windows)]
async fn handle_session_change(
    lifecycle: &mut ServiceLifecycle,
    supervisor: Option<&mut WindowsSessionAgentSupervisor>,
    change: LifecycleSessionChange,
) {
    use mrd_service::windows_service::ServiceEffect;
    let effects = lifecycle.apply(LifecycleControl::SessionChange(change));
    let Some(supervisor) = supervisor else { return };
    for effect in effects {
        let result = match effect {
            ServiceEffect::LaunchAgent(session_id) => supervisor.launch(session_id).await,
            ServiceEffect::RevokeAgentSession(session_id) => {
                supervisor
                    .stop_session(session_id, StopReason::SessionEnding)
                    .await
            }
            _ => continue,
        };
        if let Err(error) = result {
            warn!("Session Agent reconciliation failed: {error}");
        }
    }
}

#[cfg(windows)]
async fn next_control(
    controls: &mut Option<tokio::sync::mpsc::UnboundedReceiver<RuntimeControl>>,
) -> RuntimeControl {
    match controls {
        Some(receiver) => receiver.recv().await.unwrap_or(RuntimeControl::Stop),
        None => {
            let _ = tokio::signal::ctrl_c().await;
            RuntimeControl::Stop
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn initialize_application_state(app_state: &Arc<AppState>, tray: &app_state::TrayPortRef) {
    let (device_id, device_name) = app_state::default_lan_device_identity();
    let mut devices = app_state.devices.lock().await;
    if let Some((registered_id, registered_name)) =
        devices.register_if_unregistered(device_id, device_name)
    {
        info!(
            "Default LAN device registered: {} ({})",
            registered_id.0, registered_name
        );
    }
    drop(devices);
    let tray_available = tray.lock().unwrap().is_available();
    app_state.shell.lock().await.tray_available = tray_available;
    app_state.refresh_capability_snapshot_in_background();
}

#[cfg(windows)]
fn new_execute_grant_issuer() -> Result<ExecuteGrantIssuer> {
    let mut seed = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut seed)
        .map_err(|_| anyhow::anyhow!("execute-grant issuer entropy unavailable"))?;
    ExecuteGrantIssuer::from_seed(seed)
        .ok_or_else(|| anyhow::anyhow!("execute-grant issuer seed is invalid"))
}

#[cfg(windows)]
fn open_protected_app_state(
    tray: app_state::TrayPortRef,
    lan_discovery_config: lan_discovery::LanDiscoveryConfig,
    mode: RunMode,
) -> Result<Arc<AppState>> {
    let policy = if mode == RunMode::WindowsService {
        security::ProductDirectoryAclPolicy::installed_service(MRD_WINDOWS_SERVICE_SID)
            .map_err(anyhow::Error::msg)?
    } else {
        security::ProductDirectoryAclPolicy::bootstrap()
    };
    let product_data =
        security::verify_protected_product_data_dir(&policy).map_err(anyhow::Error::msg)?;
    let protector = security::platform_secret_protector().map_err(anyhow::Error::msg)?;
    let state = Arc::new(
        AppState::open_persistent_with_tray_and_lan_discovery_config(
            tray,
            lan_discovery_config,
            product_data.join("security-state-v2.sqlite3"),
            Arc::clone(&protector),
        )?,
    );
    state.public_connection.configure_persistence(
        product_data,
        protector,
        state
            .device_identities
            .machine_key_id()
            .context("machine key is unavailable")?,
    )?;
    Ok(state)
}

enum StatusReporter {
    Console,
    #[cfg(windows)]
    Scm(windows_service::service_control_handler::ServiceStatusHandle),
}

#[cfg(any(windows, target_os = "macos"))]
impl StatusReporter {
    fn running(&self) -> Result<()> {
        #[cfg(windows)]
        if let Self::Scm(handle) = self {
            return scm_host::set_status(
                handle,
                windows_service::service::ServiceState::Running,
                0,
            )
            .map_err(Into::into);
        }
        Ok(())
    }

    fn stop_pending(&self) -> Result<()> {
        #[cfg(windows)]
        if let Self::Scm(handle) = self {
            return scm_host::set_status(
                handle,
                windows_service::service::ServiceState::StopPending,
                1,
            )
            .map_err(Into::into);
        }
        Ok(())
    }

    fn stopped(&self, _failed: bool) -> Result<()> {
        #[cfg(windows)]
        if let Self::Scm(handle) = self {
            if _failed {
                return scm_host::set_failure(handle).map_err(Into::into);
            }
            return scm_host::set_status(
                handle,
                windows_service::service::ServiceState::Stopped,
                0,
            )
            .map_err(Into::into);
        }
        Ok(())
    }
}

#[cfg(windows)]
mod scm_host {
    use super::{initialize_logging, run_service, RunMode, RuntimeControl, StatusReporter};
    use mrd_service::windows_service::SessionChange;
    use std::{ffi::OsString, time::Duration};
    use windows_service::{
        define_windows_service,
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType, SessionChangeReason,
        },
        service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
        service_dispatcher,
    };

    const SERVICE_NAME: &str = mrd_service::windows_service::MRD_WINDOWS_SERVICE_NAME;
    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> windows_service::Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    fn service_main(_arguments: Vec<OsString>) {
        initialize_logging();
        if let Err(error) = run_registered_service() {
            tracing::error!("Windows service failed: {error}");
            write_event_log(&format!("MiniRemoteDesktop service failed: {error}"), true);
        }
    }

    fn run_registered_service() -> anyhow::Result<()> {
        let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
        let handler = move |control| match map_control(control) {
            Some(control) => {
                let _ = control_tx.send(control);
                ServiceControlHandlerResult::NoError
            }
            None if matches!(control, ServiceControl::Interrogate) => {
                ServiceControlHandlerResult::NoError
            }
            None => ServiceControlHandlerResult::NotImplemented,
        };
        let status = service_control_handler::register(SERVICE_NAME, handler)?;
        set_status(&status, ServiceState::StartPending, 1)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let result = runtime.block_on(run_service(
            RunMode::WindowsService,
            Some(control_rx),
            StatusReporter::Scm(status),
        ));
        runtime.shutdown_timeout(Duration::from_secs(2));
        if result.is_err() {
            let _ = set_failure(&status);
        }
        result
    }

    fn map_control(control: ServiceControl) -> Option<RuntimeControl> {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => Some(RuntimeControl::Stop),
            ServiceControl::Preshutdown => Some(RuntimeControl::PreShutdown),
            ServiceControl::SessionChange(change) => {
                let session_id = change.notification.session_id;
                let change = match change.reason {
                    SessionChangeReason::SessionLogon => SessionChange::Logon(session_id),
                    SessionChangeReason::ConsoleConnect | SessionChangeReason::RemoteConnect => {
                        SessionChange::Connect(session_id)
                    }
                    SessionChangeReason::ConsoleDisconnect
                    | SessionChangeReason::RemoteDisconnect => {
                        SessionChange::Disconnect(session_id)
                    }
                    SessionChangeReason::SessionLogoff | SessionChangeReason::SessionTerminate => {
                        SessionChange::Logoff(session_id)
                    }
                    _ => return None,
                };
                Some(RuntimeControl::SessionChange(change))
            }
            _ => None,
        }
    }

    pub fn set_status(
        handle: &ServiceStatusHandle,
        state: ServiceState,
        checkpoint: u32,
    ) -> windows_service::Result<()> {
        handle.set_service_status(service_status(state, checkpoint, false))?;
        match state {
            ServiceState::Running => write_event_log("MiniRemoteDesktop service is running", false),
            ServiceState::Stopped => {
                write_event_log("MiniRemoteDesktop service stopped cleanly", false)
            }
            _ => {}
        }
        Ok(())
    }

    pub fn set_failure(handle: &ServiceStatusHandle) -> windows_service::Result<()> {
        handle.set_service_status(service_status(ServiceState::Stopped, 0, true))?;
        write_event_log(
            "MiniRemoteDesktop service stopped after a runtime failure",
            true,
        );
        Ok(())
    }

    fn service_status(state: ServiceState, checkpoint: u32, failed: bool) -> ServiceStatus {
        let controls_accepted = if state == ServiceState::Running {
            ServiceControlAccept::STOP
                | ServiceControlAccept::PRESHUTDOWN
                | ServiceControlAccept::SESSION_CHANGE
        } else {
            ServiceControlAccept::empty()
        };
        ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted,
            exit_code: if failed {
                ServiceExitCode::ServiceSpecific(1)
            } else {
                ServiceExitCode::Win32(0)
            },
            checkpoint,
            wait_hint: if matches!(
                state,
                ServiceState::StartPending | ServiceState::StopPending
            ) {
                Duration::from_secs(30)
            } else {
                Duration::ZERO
            },
            process_id: None,
        }
    }

    #[cfg(test)]
    #[test]
    fn runtime_failure_reports_nonzero_exit_to_enable_installed_recovery() {
        let failed = service_status(ServiceState::Stopped, 0, true);
        assert!(matches!(
            failed.exit_code,
            ServiceExitCode::ServiceSpecific(1)
        ));
        assert!(failed.controls_accepted.is_empty());
        let normal = service_status(ServiceState::Stopped, 0, false);
        assert!(matches!(normal.exit_code, ServiceExitCode::Win32(0)));
    }

    fn write_event_log(message: &str, is_error: bool) {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::System::EventLog::{
                DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_ERROR_TYPE,
                EVENTLOG_INFORMATION_TYPE,
            },
        };

        let source: Vec<u16> = std::ffi::OsStr::new(SERVICE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let message: Vec<u16> = std::ffi::OsStr::new(message)
            .encode_wide()
            .chain(Some(0))
            .collect();
        if let Ok(handle) = unsafe { RegisterEventSourceW(PCWSTR::null(), PCWSTR(source.as_ptr())) }
        {
            let strings = [PCWSTR(message.as_ptr())];
            let event_type = if is_error {
                EVENTLOG_ERROR_TYPE
            } else {
                EVENTLOG_INFORMATION_TYPE
            };
            let _ =
                unsafe { ReportEventW(handle, event_type, 0, 1, None, 0, Some(&strings), None) };
            let _ = unsafe { DeregisterEventSource(handle) };
        }
    }
}

#[cfg(all(test, windows))]
mod existing_store_preflight_tests {
    use super::*;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn existing_store_preflight_accepts_only_the_exact_sole_argument() {
        assert!(requests_existing_store_verification(&args(&["--verify-existing-store"])).unwrap());
        for arguments in [
            vec!["--verify-existing-store", "--service"],
            vec!["--service", "--verify-existing-store"],
            vec!["--verify-existing-store", "arbitrary.sqlite"],
            vec!["--verify-existing-store", "--verify-existing-store"],
            vec!["--verify-existing-store=arbitrary.sqlite"],
        ] {
            assert!(requests_existing_store_verification(&args(&arguments)).is_err());
        }
    }

    #[test]
    fn existing_store_preflight_preserves_ordinary_console_and_service_routing() {
        for arguments in [vec![], vec!["--service"], vec!["--console"]] {
            assert!(!requests_existing_store_verification(&args(&arguments)).unwrap());
        }
    }
}
