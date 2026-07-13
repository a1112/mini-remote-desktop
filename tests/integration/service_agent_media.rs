use mrd_agent_ipc::{
    decode_frame, encode_frame, validate_execute_command, AgentCommand, AuthorizedCommand,
    DesktopKind, ExecuteCommand, ExecuteGrant, ExecuteGrantClaims, ExecuteGrantVerifier,
    ExecutionContext, GrantAudience, MediaCodec, PeerBinding, RenderAccessUnit,
    RenderSurfaceTarget, ServiceToAgent,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_service::agent_runtime::{AgentRenderDispatch, AgentRenderRouteRegistry};
use mrd_session::{PermissionScope, PermissionScopes};
use mrd_session_agent::{
    capture::UnavailableCaptureAdapter,
    media::{MediaExecutor, MediaResource},
    render::RenderAdapter,
    runtime::AuthorizedCommandExecutor,
};
use std::sync::{Arc, Mutex};

const RESOURCE_ID: [u8; 16] = [31; 16];
const REGISTRATION_ID: [u8; 16] = [32; 16];
const ISSUER_KEY_ID: [u8; 32] = [33; 32];

#[derive(Default)]
struct RenderState {
    started: usize,
    stopped: usize,
    units: Vec<RenderAccessUnit>,
}

struct MemoryRenderAdapter(Arc<Mutex<RenderState>>);

impl RenderAdapter for MemoryRenderAdapter {
    fn is_available(&self) -> bool {
        true
    }

    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool {
        if resource.session_id() != session_id
            || resource
                .render_surface()
                .map(|surface| surface.window_handle)
                != Some(0x1234)
        {
            return false;
        }
        self.0.lock().unwrap().started += 1;
        true
    }

    fn push_access_unit(&mut self, _resource: &MediaResource, unit: &RenderAccessUnit) -> bool {
        self.0.lock().unwrap().units.push(unit.clone());
        true
    }

    fn stop(&mut self, resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
        if resource_id != &RESOURCE_ID {
            return false;
        }
        self.0.lock().unwrap().stopped += 1;
        true
    }
}

struct AcceptVerifier;

impl ExecuteGrantVerifier for AcceptVerifier {
    fn verify(&self, issuer_key_id: &[u8; 32], _message: &[u8], _signature: &[u8; 64]) -> bool {
        issuer_key_id == &ISSUER_KEY_ID
    }
}

fn scopes() -> PermissionScopes {
    [PermissionScope::ScreenView].into_iter().collect()
}

fn session_id() -> SessionId {
    SessionId("dual-process-render".into())
}

fn peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId("controller".into()),
        key_id: [34; 32],
    }
}

fn authorized_start_render() -> AuthorizedCommand {
    let command = AgentCommand::StartRender {
        resource_id: RESOURCE_ID,
        display_id: 0,
        surface: RenderSurfaceTarget {
            surface_id: "surface-1".into(),
            window_handle: 0x1234,
        },
    };
    let mut execute = ExecuteCommand {
        request_token: 1,
        command_id: [35; 16],
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id: [36; 32],
                registration_id: REGISTRATION_ID,
                registration_epoch: 1,
                session_id: session_id(),
                peer: peer(),
                scopes: scopes(),
                policy_revision: 1,
                windows_session_id: 7,
                desktop_epoch: 1,
                desktop_kind: DesktopKind::Default,
                issued_at_ms: 1_000,
                not_before_ms: 1_000,
                expires_at_ms: 2_000,
                command_digest: [0; 32],
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: ISSUER_KEY_ID,
            signature: [1; 64],
        },
        command,
    };
    execute.grant.claims.command_digest = execute.command_digest();
    validate_execute_command(
        &execute,
        &ExecutionContext {
            registration_id: REGISTRATION_ID,
            registration_epoch: 1,
            session_id: session_id(),
            peer: peer(),
            policy_revision: 1,
            windows_session_id: 7,
            desktop_epoch: 1,
            desktop_kind: DesktopKind::Default,
            now_ms: 1_500,
            expected_issuer_key_id: ISSUER_KEY_ID,
            authorization_scopes: scopes(),
            authorization_expires_at_ms: 2_000,
        },
        &AcceptVerifier,
    )
    .expect("authorized StartRender")
}

#[test]
fn service_encoded_units_cross_wire_to_exact_agent_resource_and_revoke_cleanly() {
    let state = Arc::new(Mutex::new(RenderState::default()));
    let mut executor = MediaExecutor::new(
        UnavailableCaptureAdapter,
        MemoryRenderAdapter(Arc::clone(&state)),
    );
    assert_eq!(
        executor.execute(authorized_start_render()),
        mrd_agent_ipc::CommandOutcome::Completed
    );

    let mut routes = AgentRenderRouteRegistry::new(1).expect("bounded service routes");
    routes
        .install(session_id(), "agent-binding", RESOURCE_ID)
        .expect("active route");
    let prepared = routes
        .prepare(
            &session_id(),
            1,
            42,
            MediaCodec::H264,
            true,
            vec![0, 0, 0, 1, 0x65],
        )
        .expect("service render unit");
    let wire = encode_frame(&ServiceToAgent::RenderAccessUnit(prepared.unit().clone()))
        .expect("bounded IPC frame");
    let decoded = decode_frame::<ServiceToAgent>(&wire)
        .expect("agent IPC decode")
        .message;
    let ServiceToAgent::RenderAccessUnit(unit) = decoded else {
        panic!("expected encoded render access unit")
    };
    assert!(executor.render_access_unit(unit));
    assert_eq!(state.lock().unwrap().units.len(), 1);

    assert!(executor.revoke_session(&session_id()));
    assert_eq!(state.lock().unwrap().stopped, 1);
    assert!(!executor.render_access_unit(prepared.unit().clone()));

    assert!(AgentRenderDispatch::Unavailable.allows_local_render_fallback());
    assert!(!AgentRenderDispatch::Delivered.allows_local_render_fallback());
    assert!(!AgentRenderDispatch::Rejected.allows_local_render_fallback());
}
