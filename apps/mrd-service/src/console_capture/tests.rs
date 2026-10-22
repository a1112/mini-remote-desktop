use super::*;
use crate::{
    agent_runtime::{
        AgentCallerKind, AgentConnectionId, AgentServer, ExpectedAgentSession, ReplacementPolicy,
    },
    session_authorization::{VerifiedIncomingAuthorizationRequest, VerifiedSessionGrant},
    transports::{memory::MemoryTransportMux, TransportMuxConfig},
};
use mrd_agent_ipc::{
    derive_registration_public_key, BoundEd25519ExecuteGrantVerifier,
    BoundEd25519RegistrationVerifier,
};
use mrd_application::ports::{TransportLane, TransportMuxPort, TransportSendOutcome};
use mrd_pipeline_core::VideoDecoder;
use mrd_proto::DeviceId;
use mrd_session_agent::{
    bootstrap::OneShotEd25519Signer,
    capture::WindowsDxgiOpenH264CaptureAdapter,
    consent::{
        ConsentAbortReason, ConsentBackend, ConsentBackendDecision, ConsentBackendFuture,
        ConsentPrompt,
    },
    media::{MediaExecutor, MediaResource},
    render::RenderAdapter,
    runtime::{
        AgentRuntime, AgentRuntimeConfig, SessionDescriptor, TrustedDesktopState,
        TrustedDesktopStateSource,
    },
};
use tokio::sync::watch;

#[derive(Default)]
struct ResponseFaults {
    drop_input_ack: std::sync::atomic::AtomicBool,
    drop_input_stop: std::sync::atomic::AtomicBool,
    stop_ids: std::sync::Mutex<std::collections::HashSet<[u8; 16]>>,
}

#[derive(Default)]
struct TestAgentServerClock(std::sync::atomic::AtomicU64);
impl crate::agent_runtime::AgentServerClock for TestAgentServerClock {
    fn now_ms(&self) -> u64 {
        now_ms().saturating_add(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}
impl mrd_session_agent::runtime::AgentClock for TestAgentServerClock {
    fn now_ms(&self) -> u64 {
        crate::agent_runtime::AgentServerClock::now_ms(self)
    }
}

fn fault_bridge(
    service: tokio::io::DuplexStream,
    agent: tokio::io::DuplexStream,
    faults: Arc<ResponseFaults>,
) -> [tokio::task::JoinHandle<()>; 2] {
    let (mut service_reader, mut service_writer) = tokio::io::split(service);
    let (mut agent_reader, mut agent_writer) = tokio::io::split(agent);
    let requests = tokio::spawn({
        let faults = faults.clone();
        async move {
            while let Ok(frame) =
                mrd_agent_ipc::read_frame::<_, mrd_agent_ipc::ServiceToAgent>(&mut service_reader)
                    .await
            {
                if let mrd_agent_ipc::ServiceToAgent::Execute(execute) = &frame.message {
                    if matches!(execute.command, AgentCommand::StopInput { .. }) {
                        faults.stop_ids.lock().unwrap().insert(execute.command_id);
                    }
                }
                if mrd_agent_ipc::write_frame(&mut agent_writer, &frame.message)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    });
    let replies = tokio::spawn(async move {
        while let Ok(frame) =
            mrd_agent_ipc::read_frame::<_, mrd_agent_ipc::AgentToService>(&mut agent_reader).await
        {
            let drop_reply = match &frame.message {
                mrd_agent_ipc::AgentToService::InputAck(_) => faults
                    .drop_input_ack
                    .load(std::sync::atomic::Ordering::SeqCst),
                mrd_agent_ipc::AgentToService::CommandResult(result) => {
                    faults
                        .drop_input_stop
                        .load(std::sync::atomic::Ordering::SeqCst)
                        && faults.stop_ids.lock().unwrap().contains(&result.command_id)
                }
                _ => false,
            };
            if !drop_reply
                && mrd_agent_ipc::write_frame(&mut service_writer, &frame.message)
                    .await
                    .is_err()
            {
                break;
            }
        }
    });
    [requests, replies]
}

struct TestRecordingInput(Arc<std::sync::Mutex<Vec<mrd_input::InputEvent>>>);
impl mrd_input::InputInjector for TestRecordingInput {
    fn is_available(&self) -> bool {
        true
    }
    fn inject(&mut self, event: &mrd_input::InputEvent) -> Result<(), mrd_input::InputError> {
        self.0.lock().unwrap().push(*event);
        Ok(())
    }
}

// Controlled adapters exist only in the test binary. Production always uses
// the native surface and independently observed desktop watcher.
struct TestConsent;
impl ConsentBackend for TestConsent {
    fn is_available(&self) -> bool {
        true
    }
    fn prompt(
        &self,
        prompt: ConsentPrompt,
        _: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        Box::pin(async move { ConsentBackendDecision::Approved(prompt.requested_scopes().clone()) })
    }
}

struct TestDesktop(watch::Sender<()>, std::sync::atomic::AtomicU64);
impl TrustedDesktopStateSource for TestDesktop {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        Some(TrustedDesktopState {
            desktop_epoch: self.1.load(std::sync::atomic::Ordering::SeqCst),
            desktop_kind: DesktopKind::Default,
        })
    }
    fn subscribe(&self) -> watch::Receiver<()> {
        self.0.subscribe()
    }
}

struct NoRender;
impl RenderAdapter for NoRender {
    fn is_available(&self) -> bool {
        false
    }
    fn start(&mut self, _: &MediaResource, _: &SessionId) -> bool {
        false
    }
    fn push_access_unit(&mut self, _: &MediaResource, _: &mrd_agent_ipc::RenderAccessUnit) -> bool {
        false
    }
    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        false
    }
}

fn profile() -> MediaProfile {
    MediaProfile {
        width: 320,
        height: 180,
        fps: 10,
        bitrate_mbps: 1,
        codec: "h264".into(),
        ..MediaProfile::default()
    }
}

#[test]
fn pointer_mapping_uses_authenticated_source_rect_and_exact_scaled_profile() {
    let resource = Resource {
        id: [3; 16],
        peer: PeerBinding {
            device_id: DeviceId("peer".into()),
            key_id: [4; 32],
        },
        policy_revision: 1,
        expires_at_ms: 10_000,
        profile: MediaProfile {
            width: 1280,
            height: 720,
            ..profile()
        },
        source_bounds: Some(CaptureSourceBounds {
            left: -1920,
            top: 100,
            width: 1920,
            height: 1200,
        }),
        stop_command: Arc::new(Mutex::new(None)),
    };
    for (point, expected) in [
        ((-100, -50), (-1920, 100)),
        ((1279, 719), (-1, 1299)),
        ((5000, 5000), (-1, 1299)),
        ((640, 360), (-960, 700)),
    ] {
        assert_eq!(
            map_pointer_to_capture_source(
                mrd_agent_ipc::InputEventPayload::MouseMove {
                    x: point.0,
                    y: point.1
                },
                Some(&resource)
            )
            .unwrap(),
            mrd_agent_ipc::InputEventPayload::MouseMove {
                x: expected.0,
                y: expected.1
            }
        );
    }
    let mut missing = resource;
    missing.source_bounds = None;
    assert!(map_pointer_to_capture_source(
        mrd_agent_ipc::InputEventPayload::MouseMove { x: 10, y: 20 },
        Some(&missing)
    )
    .is_err());
    assert_eq!(
        map_pointer_to_capture_source(mrd_agent_ipc::InputEventPayload::ReleaseAll, None).unwrap(),
        mrd_agent_ipc::InputEventPayload::ReleaseAll
    );
}

async fn pending_target(
    state: &AppState,
    session_id: &SessionId,
    scopes: Vec<RemotePermissionScope>,
) -> ConsentResponse {
    let now = now_ms();
    let pending = state
        .session_authorizations
        .begin_verified_incoming(VerifiedIncomingAuthorizationRequest {
            session_id: session_id.clone(),
            peer_device_id: DeviceId("test-controller".into()),
            peer_key_id: "12".repeat(32),
            peer_key_epoch: 1,
            access_mode: RemoteAccessMode::Attended,
            requested_scopes: scopes.clone(),
            peer_permission_ceiling: scopes.clone(),
            machine_permission_ceiling: scopes.clone(),
            runtime_capabilities: scopes.clone(),
            transport_kind: "quic".into(),
            request_nonce: random_id().unwrap(),
            created_at_ms: now,
            expires_at_ms: now + 30_000,
        })
        .await
        .unwrap();
    ConsentResponse {
        session_id: session_id.clone(),
        decision: ConsentDecision::Approve,
        approved_scopes: scopes,
        expected_policy_revision: pending.policy_revision,
    }
}

async fn grant_target(state: &AppState, response: ConsentResponse) {
    let session_id = response.session_id.clone();
    let approved = state
        .session_authorizations
        .respond_to_consent(response, now_ms())
        .await
        .unwrap();
    let now = now_ms();
    state
        .session_authorizations
        .install_verified_grant(
            VerifiedSessionGrant {
                grant_id: format!("sha256:{}", "34".repeat(32)),
                session_id,
                granted_scopes: approved.granted_scopes,
                issued_at_ms: now,
                expires_at_ms: now + 120_000,
                policy_revision: approved.policy_revision.get(),
                route_constraint: "quic".into(),
                transport_fingerprint_sha256: [5; 32],
            },
            now,
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn approved_screen_grant_without_exact_desktop_binding_cannot_start_capture() {
    let state = AppState::new();
    state.bind_console_capture_issuer(Arc::new(ExecuteGrantIssuer::from_seed([0x46; 32]).unwrap()));
    assert!(state.console_capture.is_enabled());
    let id = SessionId("approved-without-agent".into());
    let response = pending_target(&state, &id, vec![RemotePermissionScope::ScreenView]).await;
    grant_target(&state, response).await;
    let failure = state
        .console_capture
        .start(&state, &id, &profile())
        .await
        .unwrap_err();
    assert!(failure
        .to_string()
        .contains("did not bind a target desktop"));
    assert!(!state.agent_media_ingress.lock().await.has_session(&id.0));
}

#[tokio::test]
#[ignore = "captures the current physical Windows desktop with a controlled test-only consent surface"]
async fn real_console_agent_signed_capture_reaches_service_mux_and_decodes_then_stops() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let state = Arc::new(AppState::new());
    let process = crate::agent_runtime::inspect_windows_process(std::process::id()).unwrap();
    assert_eq!(process.windows_session_id(), active_console().unwrap());
    let observed = ObservedAgentIdentity {
        caller_kind: AgentCallerKind::InteractiveUser,
        process_id: process.process_id(),
        process_creation_time: process.process_creation_time(),
        logon_sid_hash: *process.logon_sid_hash(),
        windows_session_id: process.windows_session_id(),
    };
    let seed = [0x39; 32];
    let key = derive_registration_public_key(&seed).unwrap();
    state
        .agent_registry
        .expect_session_at(
            ExpectedAgentSession {
                windows_session_id: observed.windows_session_id,
                logon_sid_hash: observed.logon_sid_hash,
                process_id: observed.process_id,
                process_creation_time: observed.process_creation_time,
                agent_key_id: key.key_id,
                expires_at_ms: now_ms() + 30_000,
                replacement_policy: ReplacementPolicy::RejectExisting,
            },
            Arc::new(BoundEd25519RegistrationVerifier::new(key.key_id, key.public_key).unwrap()),
            now_ms(),
        )
        .unwrap();
    let issuer = Arc::new(ExecuteGrantIssuer::from_seed([0x47; 32]).unwrap());
    state.bind_console_capture_issuer(issuer.clone());
    let test_clock = Arc::new(TestAgentServerClock::default());
    let server = Arc::new(AgentServer::with_clock_and_request_timeout(
        state.agent_registry.clone(),
        test_clock.clone(),
        Duration::from_millis(300),
    ));
    state.bind_agent_media_server(server.clone());
    let desktop = Arc::new(TestDesktop(
        watch::channel(()).0,
        std::sync::atomic::AtomicU64::new(1),
    ));
    let recorded_input = Arc::new(std::sync::Mutex::new(Vec::new()));
    let runtime = AgentRuntime::new(
        AgentRuntimeConfig {
            session: SessionDescriptor::new(
                [1; 16],
                observed.process_id,
                observed.process_creation_time,
                observed.logon_sid_hash,
                observed.windows_session_id,
                [2; 32],
                1,
            )
            .unwrap(),
            heartbeat_interval: Duration::from_millis(100),
            handshake_timeout: Duration::from_secs(3),
        },
        test_clock.clone(),
        Arc::new(OneShotEd25519Signer::new(zeroize::Zeroizing::new(seed), key.key_id).unwrap()),
    )
    .unwrap()
    .with_attended_authority(
        Arc::new(TestConsent),
        Arc::new(
            BoundEd25519ExecuteGrantVerifier::new(issuer.key_id(), issuer.public_key()).unwrap(),
        ),
        desktop.clone(),
        issuer.key_id(),
        Box::new(MediaExecutor::new(
            WindowsDxgiOpenH264CaptureAdapter::new(),
            NoRender,
        )),
    )
    .unwrap()
    .with_input_backend(Box::new(
        mrd_session_agent::input::InputResourceManager::new(TestRecordingInput(
            recorded_input.clone(),
        )),
    ));
    let (service_stream, proxy_service) = tokio::io::duplex(1024 * 1024);
    let (proxy_agent, agent_stream) = tokio::io::duplex(1024 * 1024);
    let faults = Arc::new(ResponseFaults::default());
    let bridge_tasks = fault_bridge(proxy_service, proxy_agent, faults.clone());
    let server_task = tokio::spawn({
        let server = server.clone();
        let observed = observed.clone();
        async move {
            server
                .serve_connection(
                    service_stream,
                    AgentConnectionId::from_bytes([3; 16]).unwrap(),
                    observed,
                )
                .await
        }
    });
    let agent_task = tokio::spawn(runtime.run(agent_stream));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .agent_registry
                .active_for_session_at(observed.windows_session_id, now_ms())
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    let id = SessionId("real-console-capture".into());
    let response = pending_target(
        &state,
        &id,
        vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputPointer,
            RemotePermissionScope::InputKeyboard,
        ],
    )
    .await;
    state
        .console_capture
        .prepare_consent(&state, &observed, &response)
        .await
        .unwrap();
    grant_target(&state, response).await;
    state
        .console_capture
        .start(&state, &id, &profile())
        .await
        .unwrap();
    let (sender, receiver) = MemoryTransportMux::pair(id.clone(), TransportMuxConfig::default());
    let mut decoder = mrd_decode::H264SoftwareDecoder::new().unwrap();
    let decoded = tokio::time::timeout(Duration::from_secs(10), async {
        'frame: loop {
            let units = state.console_capture.drain(&state, &id, 8).await.unwrap();
            for unit in units {
                let validated =
                    crate::lan_discovery::media_sender::validate_agent_access_unit(unit).unwrap();
                let encoded = crate::lan_discovery::media_sender::prepare_agent_transport_unit(
                    validated,
                    crate::lan_discovery::media_sender::LanAccessUnitCodec::H264,
                )
                .unwrap();
                let envelope =
                    crate::lan_discovery::media_sender::transport_envelope_from_agent_unit(
                        &id,
                        1,
                        profile().width,
                        profile().height,
                        encoded,
                    );
                assert!(matches!(
                    sender.send(envelope).await.unwrap(),
                    TransportSendOutcome::Enqueued | TransportSendOutcome::ReplacedStale
                ));
                let received = receiver.recv(TransportLane::Video).await.unwrap().unwrap();
                assert_eq!(received.session_id, id);
                let metadata = received.video.unwrap();
                assert_eq!((metadata.width, metadata.height), (320, 180));
                decoder.push_access_unit(&received.payload).unwrap();
                if let Some(frame) = decoder.drain_decoded_frames().into_iter().next() {
                    break 'frame frame;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!((decoded.width, decoded.height), (320, 180));
    let operation = state.console_capture.operation.lock().await;
    let delayed_input = tokio::spawn({
        let state = state.clone();
        let id = id.clone();
        let deadline = now_ms() + 50;
        async move {
            state
                .console_capture
                .apply_input(
                    &state,
                    &id,
                    RemotePermissionScope::InputKeyboard,
                    42,
                    deadline,
                    mrd_agent_ipc::InputEventPayload::Key {
                        key: mrd_agent_ipc::InputKey::VirtualKey { code: 67 },
                        pressed: true,
                    },
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(80)).await;
    drop(operation);
    assert!(delayed_input
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("Signed remote input expired"));
    assert!(recorded_input.lock().unwrap().is_empty());
    let pointer_ack = state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputPointer,
            1,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::MouseMove { x: 10, y: 20 },
        )
        .await
        .unwrap();
    let source = state
        .console_capture
        .targets
        .lock()
        .await
        .get(&id)
        .unwrap()
        .resource
        .clone()
        .unwrap();
    let expected_pointer = map_pointer_to_capture_source(
        mrd_agent_ipc::InputEventPayload::MouseMove { x: 10, y: 20 },
        Some(&source),
    )
    .unwrap();
    let mrd_agent_ipc::InputEventPayload::MouseMove { x, y } = expected_pointer else {
        panic!("mapped pointer");
    };
    assert!(recorded_input
        .lock()
        .unwrap()
        .contains(&mrd_input::InputEvent::MouseMove { x, y }));
    let keyboard_ack = state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputKeyboard,
            1,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::Key {
                key: mrd_agent_ipc::InputKey::VirtualKey { code: 65 },
                pressed: true,
            },
        )
        .await
        .unwrap();
    assert_ne!(
        pointer_ack.unwrap().resource_id,
        keyboard_ack.unwrap().resource_id
    );
    let pointer_reliable_ack = state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputPointer,
            1,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::MouseButton {
                button: mrd_agent_ipc::InputButton::Left,
                pressed: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(pointer_reliable_ack.unwrap().sequence, 2);
    assert!(state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputKeyboard,
            2,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::MouseMove { x: 5, y: 7 }
        )
        .await
        .is_err());
    state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputPointer,
            2,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::ReleaseAll,
        )
        .await
        .unwrap();
    assert!(!recorded_input
        .lock()
        .unwrap()
        .iter()
        .any(|event| matches!(event, mrd_input::InputEvent::Key { pressed: false, .. })));
    state.console_capture.stop_input(&state, &id).await.unwrap();
    assert!(recorded_input.lock().unwrap().iter().any(|event| matches!(
        event,
        mrd_input::InputEvent::Key {
            key: mrd_input::InputKey::VirtualKey(65),
            pressed: false
        }
    )));
    let target = state
        .console_capture
        .targets
        .lock()
        .await
        .get(&id)
        .cloned()
        .unwrap();
    faults
        .drop_input_ack
        .store(true, std::sync::atomic::Ordering::SeqCst);
    faults
        .drop_input_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputKeyboard,
            2,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::Key {
                key: mrd_agent_ipc::InputKey::VirtualKey { code: 66 },
                pressed: true
            }
        )
        .await
        .is_err());
    let pending_id = {
        let targets = state.console_capture.targets.lock().await;
        let input = targets
            .get(&id)
            .unwrap()
            .input
            .get(&PermissionScope::InputKeyboard)
            .unwrap();
        assert!(input.cleanup_pending);
        input.id
    };
    assert!(state
        .console_capture
        .apply_input(
            &state,
            &id,
            RemotePermissionScope::InputPointer,
            3,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::MouseMove { x: 20, y: 30 }
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("cleanup is pending"));
    state.console_capture.stop(&state, &id).await;
    {
        let targets = state.console_capture.targets.lock().await;
        let pending = targets
            .get(&id)
            .expect("unacknowledged cleanup must retain exact ownership");
        assert!(pending.closing);
        assert_eq!(
            pending
                .input
                .get(&PermissionScope::InputKeyboard)
                .unwrap()
                .id,
            pending_id
        );
    }
    faults
        .drop_input_ack
        .store(false, std::sync::atomic::Ordering::SeqCst);
    faults
        .drop_input_stop
        .store(false, std::sync::atomic::Ordering::SeqCst);
    state.console_capture.retry_pending_cleanup(&state).await;
    assert!(state
        .console_capture
        .targets
        .lock()
        .await
        .get(&id)
        .is_none());

    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!state.agent_media_ingress.lock().await.has_session(&id.0));
    assert_eq!(state.agent_media_ingress.lock().await.session_len(&id.0), 0);
    assert!(state
        .agent_registry
        .resolve_exact(&target.binding, AgentCapability::Capture, now_ms())
        .is_ok());
    let expired_id = SessionId("real-console-expired-cleanup".into());
    let expired_response = pending_target(
        &state,
        &expired_id,
        vec![
            RemotePermissionScope::ScreenView,
            RemotePermissionScope::InputKeyboard,
        ],
    )
    .await;
    state
        .console_capture
        .prepare_consent(&state, &observed, &expired_response)
        .await
        .unwrap();
    grant_target(&state, expired_response).await;
    state
        .console_capture
        .start(&state, &expired_id, &profile())
        .await
        .unwrap();
    state
        .console_capture
        .apply_input(
            &state,
            &expired_id,
            RemotePermissionScope::InputKeyboard,
            1,
            now_ms() + 5_000,
            mrd_agent_ipc::InputEventPayload::Key {
                key: mrd_agent_ipc::InputKey::VirtualKey { code: 68 },
                pressed: true,
            },
        )
        .await
        .unwrap();
    test_clock.0.store(
        mrd_agent_ipc::AGENT_CONSENT_MAX_LIFETIME_MS + 1_000,
        std::sync::atomic::Ordering::SeqCst,
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while !recorded_input
            .lock()
            .unwrap()
            .contains(&mrd_input::InputEvent::Key {
                key: mrd_input::InputKey::VirtualKey(68),
                pressed: false,
            })
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("expiry must actually release the held key");
    desktop.1.store(2, std::sync::atomic::Ordering::SeqCst);
    desktop.0.send_replace(());
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .agent_registry
                .active_for_session_at(
                    observed.windows_session_id,
                    crate::agent_runtime::AgentServerClock::now_ms(&*test_clock),
                )
                .is_some_and(|active| active.capabilities.desktop_epoch == 2)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("real registered IPC must observe the changed desktop before cleanup");
    state
        .console_capture
        .stop_input(&state, &expired_id)
        .await
        .expect("pre-signed exact input cleanup must be confirmed after native expiry");
    state.console_capture.stop(&state, &expired_id).await;
    assert!(
        state
            .console_capture
            .targets
            .lock()
            .await
            .get(&expired_id)
            .is_none(),
        "pre-signed exact capture cleanup must be confirmed after native expiry"
    );
    server_task.abort();
    agent_task.abort();
    for task in bridge_tasks {
        task.abort();
        let _ = task.await;
    }
    let _ = server_task.await;
    let _ = agent_task.await;
    println!("Real DXGI desktop -> signed Agent capture -> authenticated IPC -> service transport mux -> H264 decode: 320x180; physical source coordinate mapping verified; expired queued input has zero injections; independent pointer/keyboard resources ack and scope-specific release; dropped input/stop replies retain exact frozen ownership; background retry confirms the original signed stop; no queued late frames");
}
