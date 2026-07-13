use mrd_agent_ipc::{
    validate_execute_command, AgentCommand, DesktopKind, ExecuteCommand, ExecuteGrant,
    ExecuteGrantClaims, ExecuteGrantVerifier, ExecutionContext, GrantAudience, PeerBinding,
    RenderAccessUnit, RenderSurfaceTarget,
};
use mrd_proto::{DeviceId, SessionId};
use mrd_session::{PermissionScope, PermissionScopes};
use mrd_session_agent::{
    capture::UnavailableCaptureAdapter,
    media::{MediaExecutor, MediaResource},
    render::RenderAdapter,
    runtime::AuthorizedCommandExecutor,
};

const REGISTRATION_ID: [u8; 16] = [41; 16];
const RESOURCE_ID: [u8; 16] = [42; 16];
const ISSUER_KEY_ID: [u8; 32] = [43; 32];

struct AcceptVerifier;

impl ExecuteGrantVerifier for AcceptVerifier {
    fn verify(&self, key_id: &[u8; 32], _message: &[u8], _signature: &[u8; 64]) -> bool {
        key_id == &ISSUER_KEY_ID
    }
}

#[derive(Default)]
struct MemoryRender {
    live: bool,
}

impl RenderAdapter for MemoryRender {
    fn is_available(&self) -> bool {
        true
    }

    fn start(&mut self, resource: &MediaResource, session_id: &SessionId) -> bool {
        self.live = resource.session_id() == session_id;
        self.live
    }

    fn push_access_unit(&mut self, _resource: &MediaResource, _unit: &RenderAccessUnit) -> bool {
        self.live
    }

    fn stop(&mut self, resource_id: &[u8; 16], _session_id: &SessionId) -> bool {
        if resource_id != &RESOURCE_ID || !self.live {
            return false;
        }
        self.live = false;
        true
    }
}

fn session() -> SessionId {
    SessionId("grant-bound-render".into())
}

fn peer() -> PeerBinding {
    PeerBinding {
        device_id: DeviceId("peer".into()),
        key_id: [44; 32],
    }
}

fn screen_view() -> PermissionScopes {
    [PermissionScope::ScreenView].into_iter().collect()
}

fn execute(scopes: PermissionScopes) -> ExecuteCommand {
    let command = AgentCommand::StartRender {
        resource_id: RESOURCE_ID,
        display_id: 0,
        surface: RenderSurfaceTarget {
            surface_id: "surface-grant".into(),
            window_handle: 0x4444,
        },
    };
    let mut execute = ExecuteCommand {
        request_token: 1,
        command_id: [45; 16],
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id: [46; 32],
                registration_id: REGISTRATION_ID,
                registration_epoch: 1,
                session_id: session(),
                peer: peer(),
                scopes,
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
    execute
}

fn context() -> ExecutionContext {
    ExecutionContext {
        registration_id: REGISTRATION_ID,
        registration_epoch: 1,
        session_id: session(),
        peer: peer(),
        policy_revision: 1,
        windows_session_id: 7,
        desktop_epoch: 1,
        desktop_kind: DesktopKind::Default,
        now_ms: 1_500,
        expected_issuer_key_id: ISSUER_KEY_ID,
        authorization_scopes: screen_view(),
        authorization_expires_at_ms: 2_000,
    }
}

#[test]
fn render_resource_requires_screen_view_and_revocation_stops_exact_adapter() {
    let unauthorized = execute(PermissionScopes::new());
    assert!(validate_execute_command(&unauthorized, &context(), &AcceptVerifier).is_err());

    let authorized = validate_execute_command(&execute(screen_view()), &context(), &AcceptVerifier)
        .expect("screen.view authorizes StartRender");
    let mut executor = MediaExecutor::new(UnavailableCaptureAdapter, MemoryRender::default());
    assert_eq!(
        executor.execute(authorized),
        mrd_agent_ipc::CommandOutcome::Completed
    );
    assert_eq!(executor.registry().len(), 1);
    assert!(executor.revoke_session(&session()));
    assert!(executor.registry().is_empty());
}
