use super::*;
use mrd_agent_ipc::{
    validate_execute_command, AgentCaptureProfile, DesktopKind, ExecuteCommand, ExecuteGrant,
    ExecuteGrantClaims, ExecuteGrantVerifier, ExecutionContext, GrantAudience, PeerBinding,
};
use mrd_proto::DeviceId;
use mrd_session::{PermissionScope, PermissionScopes};

const FIRST: [u8; 16] = [61; 16];
const SECOND: [u8; 16] = [62; 16];
const ISSUER: [u8; 32] = [63; 32];
const REGISTRATION: [u8; 16] = [64; 16];

struct TestVerifier;
impl ExecuteGrantVerifier for TestVerifier {
    fn verify(&self, key: &[u8; 32], _: &[u8], _: &[u8; 64]) -> bool {
        key == &ISSUER
    }
}

fn authorize(owner: &str, command_id: u8, command: AgentCommand) -> AuthorizedCommand {
    let session_id = SessionId(owner.into());
    let peer = PeerBinding {
        device_id: DeviceId("capture-peer".into()),
        key_id: [65; 32],
    };
    let scopes: PermissionScopes = [PermissionScope::ScreenView].into_iter().collect();
    let mut execute = ExecuteCommand {
        request_token: u64::from(command_id),
        command_id: [command_id; 16],
        command,
        grant: ExecuteGrant {
            claims: ExecuteGrantClaims {
                grant_id: [command_id; 32],
                registration_id: REGISTRATION,
                registration_epoch: 1,
                session_id: session_id.clone(),
                peer: peer.clone(),
                scopes: scopes.clone(),
                policy_revision: 1,
                windows_session_id: 7,
                desktop_epoch: 1,
                desktop_kind: DesktopKind::Default,
                issued_at_ms: 1000,
                not_before_ms: 1000,
                expires_at_ms: 2000,
                command_digest: [0; 32],
                audience: GrantAudience::SessionAgent,
            },
            issuer_key_id: ISSUER,
            signature: [1; 64],
        },
    };
    execute.grant.claims.command_digest = execute.command_digest();
    validate_execute_command(
        &execute,
        &ExecutionContext {
            registration_id: REGISTRATION,
            registration_epoch: 1,
            session_id,
            peer,
            policy_revision: 1,
            windows_session_id: 7,
            desktop_epoch: 1,
            desktop_kind: DesktopKind::Default,
            now_ms: 1500,
            expected_issuer_key_id: ISSUER,
            authorization_scopes: scopes,
            authorization_expires_at_ms: 2000,
        },
        &TestVerifier,
    )
    .expect("test command must pass the real grant validation")
}

fn start(id: [u8; 16], display_id: u32) -> AgentCommand {
    AgentCommand::StartCapture {
        resource_id: id,
        display_id,
        profile: Some(AgentCaptureProfile {
            width: 1920,
            height: 1080,
            fps: 60,
            bitrate_bps: 4_000_000,
        }),
    }
}

#[derive(Default)]
struct CountingCapture {
    starts: Vec<MediaResource>,
    stops: Vec<[u8; 16]>,
    fail_next: bool,
}
impl CaptureAdapter for CountingCapture {
    fn is_available(&self) -> bool {
        true
    }
    fn start(&mut self, resource: &MediaResource, owner: &SessionId) -> bool {
        assert_eq!(resource.session_id(), owner);
        self.starts.push(resource.clone());
        !std::mem::take(&mut self.fail_next)
    }
    fn stop(&mut self, id: &[u8; 16], _: &SessionId) -> bool {
        self.stops.push(*id);
        true
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
    fn push_access_unit(&mut self, _: &MediaResource, _: &RenderAccessUnit) -> bool {
        false
    }
    fn stop(&mut self, _: &[u8; 16], _: &SessionId) -> bool {
        false
    }
}

fn executor_with_first() -> MediaExecutor<CountingCapture, NoRender> {
    let mut executor = MediaExecutor::new(CountingCapture::default(), NoRender);
    assert_eq!(
        executor.execute(authorize("old-owner", 1, start(FIRST, 0))),
        CommandOutcome::Completed
    );
    executor
}

#[test]
fn capture_admission_cross_session_display_busy_never_calls_second_factory_or_changes_owner() {
    let mut executor = executor_with_first();
    let existing = executor.registry.get(&FIRST).unwrap().clone();
    let outcome = executor.execute(authorize("new-owner", 2, start(SECOND, 0)));
    assert_eq!(
        executor.capture.starts.len(),
        1,
        "busy display must not construct a second capture"
    );
    assert_eq!(outcome, CommandOutcome::Rejected);
    assert_eq!(executor.registry.len(), 1);
    assert_eq!(executor.registry.get(&FIRST), Some(&existing));
    assert!(executor.registry.get(&SECOND).is_none());
    assert!(executor.capture.stops.is_empty());
    assert!(executor.capture_access_units().unwrap().is_empty());
}

#[test]
fn capture_admission_same_session_new_resource_cannot_duplicate_the_display() {
    let mut executor = executor_with_first();
    assert_eq!(
        executor.execute(authorize("old-owner", 2, start(SECOND, 0))),
        CommandOutcome::Rejected
    );
    assert_eq!(executor.capture.starts.len(), 1);
    assert_eq!(executor.registry.len(), 1);
    assert!(executor.capture.stops.is_empty());
}

#[test]
fn capture_admission_different_display_remains_available_to_another_session() {
    let mut executor = executor_with_first();
    assert_eq!(
        executor.execute(authorize("new-owner", 2, start(SECOND, 1))),
        CommandOutcome::Completed
    );
    assert_eq!(executor.capture.starts.len(), 2);
    assert_eq!(executor.capture.starts[0].display_id(), 0);
    assert_eq!(executor.capture.starts[1].display_id(), 1);
    assert_eq!(executor.registry.len(), 2);
    assert!(executor.capture.stops.is_empty());
    assert!(executor.capture_access_units().unwrap().is_empty());
}

#[test]
fn capture_admission_wrong_session_stop_cannot_unlock_a_busy_display() {
    let mut executor = executor_with_first();
    assert_eq!(
        executor.execute(authorize(
            "new-owner",
            2,
            AgentCommand::StopCapture { resource_id: FIRST }
        )),
        CommandOutcome::Rejected
    );
    assert!(executor.capture.stops.is_empty());
    assert_eq!(
        executor.execute(authorize("new-owner", 3, start(SECOND, 0))),
        CommandOutcome::Rejected
    );
    assert_eq!(executor.capture.starts.len(), 1);
    assert_eq!(
        executor.registry.get(&FIRST).unwrap().session_id(),
        &SessionId("old-owner".into())
    );
}

#[test]
fn capture_admission_exact_stop_allows_a_new_command_to_retry_without_stealing_resources() {
    let mut executor = executor_with_first();
    assert_eq!(
        executor.execute(authorize("new-owner", 2, start(SECOND, 0))),
        CommandOutcome::Rejected
    );
    assert_eq!(
        executor.execute(authorize(
            "old-owner",
            3,
            AgentCommand::StopCapture { resource_id: FIRST }
        )),
        CommandOutcome::Completed
    );
    assert_eq!(executor.capture.stops, vec![FIRST]);
    assert!(executor.registry.is_empty());
    assert_eq!(
        executor.execute(authorize("new-owner", 4, start(SECOND, 0))),
        CommandOutcome::Completed
    );
    assert_eq!(executor.capture.starts.len(), 2);
    assert_eq!(executor.registry.len(), 1);
    assert_eq!(
        executor.registry.get(&SECOND).unwrap().session_id(),
        &SessionId("new-owner".into())
    );
}

#[test]
fn capture_admission_factory_failure_rolls_back_only_the_new_resource() {
    let mut executor = executor_with_first();
    let existing = executor.registry.get(&FIRST).unwrap().clone();
    executor.capture.fail_next = true;
    assert_eq!(
        executor.execute(authorize("new-owner", 2, start(SECOND, 1))),
        CommandOutcome::Failed
    );
    assert_eq!(executor.capture.starts.len(), 2);
    assert_eq!(executor.registry.len(), 1);
    assert_eq!(executor.registry.get(&FIRST), Some(&existing));
    assert!(executor.registry.get(&SECOND).is_none());
    assert!(executor.capture.stops.is_empty());
    assert!(executor.capture_access_units().unwrap().is_empty());
}
