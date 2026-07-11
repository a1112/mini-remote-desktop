#[cfg(windows)]
mod windows_process_bootstrap {
    use mrd_agent_ipc::{
        derive_execute_grant_issuer_key_id, derive_registration_public_key,
        windows_agent_bootstrap_pipe_name, write_agent_bootstrap, AgentBootstrap,
        BoundEd25519RegistrationVerifier, ServiceToAgent, StopAgent, StopReason,
    };
    use mrd_service::agent_runtime::{
        inspect_windows_process, AgentConnectionExit, AgentConnectionId, AgentRegistry,
        AgentServer, ExpectedAgentSession, ReplacementPolicy, WindowsAgentPipe,
    };
    use ring::signature::KeyPair;
    use std::{
        process::Stdio,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use tokio::{process::Command, time::timeout};
    use zeroize::Zeroizing;

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn real_agent_uses_only_authenticated_bootstrap_then_heartbeats_and_stops() {
        let executable = env!("CARGO_BIN_EXE_mrd-session-agent");
        let mut command = Command::new(executable);
        command
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("launch real agent binary");
        let child_id = child.id().expect("child process id");
        let child_process = inspect_windows_process(child_id).expect("inspect launched child");
        assert!(child_process.holds_process_object());

        let bootstrap_name = windows_agent_bootstrap_pipe_name(
            child_process.windows_session_id(),
            child_id,
            child_process.process_creation_time(),
        );
        let control_name = format!(
            r"\\.\pipe\mrd-agent-control-v1-service-{}-child-{child_id}",
            std::process::id()
        );
        let mut bootstrap_pipe =
            WindowsAgentPipe::create_for_process(&bootstrap_name, &child_process)
                .expect("protected bootstrap pipe");
        let mut control_pipe = WindowsAgentPipe::create_for_process(&control_name, &child_process)
            .expect("protected control pipe");

        let seed = [73_u8; 32];
        let registration_key =
            derive_registration_public_key(&seed).expect("derive real Ed25519 key");
        let execute_grant_signer =
            ring::signature::Ed25519KeyPair::from_seed_unchecked(&[74; 32]).unwrap();
        let execute_grant_public_key: [u8; 32] = execute_grant_signer
            .public_key()
            .as_ref()
            .try_into()
            .unwrap();
        let execute_grant_issuer_key_id =
            derive_execute_grant_issuer_key_id(&execute_grant_public_key);
        let verifier = Arc::new(
            BoundEd25519RegistrationVerifier::new(
                registration_key.key_id,
                registration_key.public_key,
            )
            .expect("bound verifier"),
        );
        let registry = Arc::new(AgentRegistry::default());
        registry
            .expect_session(
                ExpectedAgentSession {
                    windows_session_id: child_process.windows_session_id(),
                    logon_sid_hash: *child_process.logon_sid_hash(),
                    process_id: child_id,
                    process_creation_time: child_process.process_creation_time(),
                    agent_key_id: registration_key.key_id,
                    expires_at_ms: now_ms() + 30_000,
                    replacement_policy: ReplacementPolicy::RejectExisting,
                },
                verifier,
            )
            .expect("launcher admission");
        let server = Arc::new(AgentServer::new(Arc::clone(&registry)));

        timeout(Duration::from_secs(10), bootstrap_pipe.connect())
            .await
            .expect("bootstrap connect timeout")
            .expect("bootstrap connection");
        let bootstrap_peer = bootstrap_pipe
            .inspect_peer()
            .expect("bootstrap peer identity");
        assert_eq!(bootstrap_peer.identity().process_id, child_id);
        let service_process = inspect_windows_process(std::process::id()).unwrap();
        let mut bootstrap_stream = bootstrap_pipe.into_stream();
        write_agent_bootstrap(
            &mut bootstrap_stream,
            AgentBootstrap {
                control_endpoint: &control_name,
                service_process_id: std::process::id(),
                service_process_creation_time: service_process.process_creation_time(),
                heartbeat_interval_ms: 20,
                handshake_timeout_ms: 5_000,
                registration_seed: Zeroizing::new(seed),
                expected_agent_key_id: registration_key.key_id,
                execute_grant_issuer_key_id,
                execute_grant_public_key,
            },
        )
        .await
        .expect("send protected bootstrap");
        drop(bootstrap_stream);
        drop(bootstrap_peer);

        timeout(Duration::from_secs(10), control_pipe.connect())
            .await
            .expect("control connect timeout")
            .expect("control connection");
        let control_peer = control_pipe.inspect_peer().expect("control peer identity");
        assert_eq!(control_peer.identity().process_id, child_id);
        let connection_id = AgentConnectionId::from_bytes([71; 16]).unwrap();
        let serving = tokio::spawn({
            let server = Arc::clone(&server);
            let observed = control_peer.cloned_identity();
            async move {
                let _process_guard = control_peer;
                server
                    .serve_connection(control_pipe.into_stream(), connection_id, observed)
                    .await
            }
        });

        timeout(Duration::from_secs(10), async {
            loop {
                if registry
                    .active_for_session_at(child_process.windows_session_id(), now_ms())
                    .is_some_and(|snapshot| snapshot.last_event_sequence >= 1)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("agent heartbeat");

        server
            .send_to_connection(
                connection_id,
                ServiceToAgent::StopAgent(StopAgent {
                    request_id: [81; 16],
                    deadline_ms: now_ms() + 5_000,
                    reason: StopReason::ServiceShutdown,
                }),
            )
            .expect("send StopAgent");
        assert_eq!(
            timeout(Duration::from_secs(10), serving)
                .await
                .expect("server stop timeout")
                .expect("server task")
                .expect("agent server result"),
            AgentConnectionExit::Stopped
        );
        let status = timeout(Duration::from_secs(10), child.wait())
            .await
            .expect("child exit timeout")
            .expect("child status");
        assert!(status.success(), "agent exit was {status}");
    }

    #[tokio::test]
    async fn agent_ignores_legacy_environment_and_argv_canary_endpoints() {
        let canary_name = format!(r"\\.\pipe\mrd-agent-legacy-canary-{}", std::process::id());
        let current_process = inspect_windows_process(std::process::id()).expect("test process");
        let mut canary = WindowsAgentPipe::create_for_process(&canary_name, &current_process)
            .expect("canary pipe");
        let mut command = Command::new(env!("CARGO_BIN_EXE_mrd-session-agent"));
        command
            .env("MRD_AGENT_PRIVATE_ENDPOINT", &canary_name)
            .arg("--agent-endpoint")
            .arg(&canary_name)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("launch canary agent");

        assert!(
            timeout(Duration::from_secs(2), canary.connect())
                .await
                .is_err(),
            "agent must not connect to an env/argv-selected endpoint"
        );
        child.kill().await.expect("terminate canary agent");
        let status = child.wait().await.expect("reap canary agent");
        assert!(!status.success());
    }
}
