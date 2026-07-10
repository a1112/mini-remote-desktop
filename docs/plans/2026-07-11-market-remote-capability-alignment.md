# Market Remote Capability Alignment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Deliver verified market-level remote access capability across connectivity, trust, unattended access, interaction features, reliability, platform coverage, and advanced product tiers.

**Architecture:** Keep one service-owned session and policy kernel. Use authenticated LAN QUIC for the high-performance route and WebRTC ICE/TURN for public direct and relay routes, both behind a transport-neutral multiplexer. On Windows, separate the machine service from one desktop-bound agent per interactive session so unattended access, UAC, security, and user-session resources have explicit boundaries.

**Tech Stack:** Rust workspace, Tokio, Quinn, webrtc-rs, Axum WebSocket signaling, TURN, Tauri/React/TypeScript, Windows SCM/DPAPI/named pipes, SQLite, PowerShell device-lab scripts, GitHub Actions.

---

## Execution Rules

- Read docs/plans/2026-07-11-market-remote-capability-alignment-design.md before Task 1.
- Execute tasks in order unless a task explicitly states that it can run in parallel.
- Use @superpowers:test-driven-development for every implementation or bug-fix task.
- Use @superpowers:systematic-debugging whenever a test, build, or runtime check fails unexpectedly.
- Use @superpowers:verification-before-completion before each milestone gate and before claiming any capability tier.
- Run work in a dedicated worktree on a codex/ branch.
- Preserve unrelated user changes and do not use junk/ or refs/ as architecture authority.
- A DTO, capability flag, local-only implementation, or synthetic test never upgrades a product capability to available.
- Commit after every task. Do not combine tasks into one commit.

## Milestone Order

1. Gate 0: truthful evidence and fail-closed release verdicts.
2. Secure LAN: persistent identity, authorization, consent, and protected control input.
3. Windows process boundary: machine service plus interactive-session agent.
4. Public connectivity: authenticated signaling, WebRTC direct, TURN relay, migration, and reconnect.
5. P0 feature completion: audio, clipboard, remote files, monitors, unattended policy, privacy, power, UAC.
6. P1 mainstream parity: cross-platform agents/controllers, codecs, displays, recording, privacy, terminal, printing.
7. P2 advanced parity: high refresh/HDR, collaboration, peripherals, multi-region, and enterprise policy.
8. Requirement-by-requirement completion audit.

## Gate 0 — Truthful Evidence

### Task 1: Add the quality-gate crate and stable verdict contract

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-quality-gate/Cargo.toml
- Create: crates/mrd-quality-gate/src/lib.rs
- Create: crates/mrd-quality-gate/tests/verdict_contract.rs

**Step 1: Write the failing verdict test**

~~~rust
use mrd_quality_gate::Verdict;

#[test]
fn verdict_exit_codes_are_stable() {
    assert_eq!(Verdict::Pass.exit_code(), 0);
    assert_eq!(Verdict::AllowedSkip.exit_code(), 0);
    assert_eq!(Verdict::ProductFail.exit_code(), 2);
    assert_eq!(Verdict::InfraFail.exit_code(), 3);
    assert_eq!(Verdict::InvalidArtifact.exit_code(), 4);
}
~~~

**Step 2: Run the test and confirm the crate is absent**

Run: cargo test -p mrd-quality-gate --test verdict_contract

Expected: FAIL because mrd-quality-gate is not a workspace package.

**Step 3: Add the workspace member and minimal verdict implementation**

~~~rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    ProductFail,
    InfraFail,
    InvalidArtifact,
    AllowedSkip,
}

impl Verdict {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Pass | Self::AllowedSkip => 0,
            Self::ProductFail => 2,
            Self::InfraFail => 3,
            Self::InvalidArtifact => 4,
        }
    }
}
~~~

Use serde names PASS, PRODUCT_FAIL, INFRA_FAIL, INVALID_ARTIFACT, and ALLOWED_SKIP.

**Step 4: Run the crate tests**

Run: cargo test -p mrd-quality-gate

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-quality-gate
git commit -m "feat: add canonical quality gate verdicts"
~~~

### Task 2: Define the canonical remote-experience artifact

**Files:**
- Create: crates/mrd-quality-gate/src/artifact.rs
- Modify: crates/mrd-quality-gate/src/lib.rs
- Create: crates/mrd-quality-gate/tests/artifact_validation.rs
- Create: tests/quality-gates/schemas/remote-experience-run.v2.schema.json
- Create: tests/quality-gates/fixtures/valid-direct.json
- Create: tests/quality-gates/fixtures/missing-present.json

**Step 1: Write failing artifact validation tests**

~~~rust
use mrd_quality_gate::{validate_artifact, ArtifactError, RemoteExperienceRun};

#[test]
fn required_present_metric_cannot_be_missing() {
    let raw = include_str!("../../../tests/quality-gates/fixtures/missing-present.json");
    let run: RemoteExperienceRun = serde_json::from_str(raw).unwrap();
    assert_eq!(
        validate_artifact(&run),
        Err(ArtifactError::MissingRequiredMetric("visible_first_frame_ms"))
    );
}

#[test]
fn finite_complete_direct_fixture_is_valid() {
    let raw = include_str!("../../../tests/quality-gates/fixtures/valid-direct.json");
    let run: RemoteExperienceRun = serde_json::from_str(raw).unwrap();
    assert!(validate_artifact(&run).is_ok());
}
~~~

**Step 2: Run the focused tests**

Run: cargo test -p mrd-quality-gate --test artifact_validation

Expected: FAIL because artifact types and validation do not exist.

**Step 3: Implement the v2 artifact model**

Define typed sections for:

- run/build/device/scenario identity;
- authorization transitions and granted scopes;
- requested/selected media profile;
- route candidates and selected path evidence;
- present and input-probe metrics;
- FPS, stalls, freezes, drops, bandwidth, adaptation;
- CPU/GPU/RSS/VRAM samples;
- recovery and injected fault events;
- audit event identifiers;
- producer status and gate verdict.

Validation must reject missing required sections, empty required samples, and non-finite numeric values. Keep content fields such as clipboard text, keystrokes, media payload, password, or file content out of the model.

**Step 4: Run tests and validate the JSON schema fixture**

Run: cargo test -p mrd-quality-gate

Expected: PASS and both fixtures deserialize as intended.

**Step 5: Commit**

~~~powershell
git add crates/mrd-quality-gate tests/quality-gates
git commit -m "feat: define remote experience artifact v2"
~~~

### Task 3: Add policy-driven threshold and skip evaluation

**Files:**
- Create: crates/mrd-quality-gate/src/policy.rs
- Create: crates/mrd-quality-gate/src/evaluator.rs
- Modify: crates/mrd-quality-gate/src/lib.rs
- Create: crates/mrd-quality-gate/tests/policy_evaluation.rs
- Create: tests/quality-gates/policies/strict-required-metrics.v1.json
- Create: tests/quality-gates/policies/diagnostic-allowed-skip.v1.json

**Step 1: Write failing policy tests**

~~~rust
#[test]
fn missing_required_metric_is_invalid_not_skipped() {
    let result = evaluate_fixture("missing-present.json", "strict-required-metrics.v1.json");
    assert_eq!(result.verdict, Verdict::InvalidArtifact);
}

#[test]
fn release_profile_downgrade_is_product_failure() {
    let result = evaluate_with_status("profile_downgraded", "strict-required-metrics.v1.json");
    assert_eq!(result.verdict, Verdict::ProductFail);
}

#[test]
fn explicitly_allowlisted_capability_skip_is_allowed() {
    let result = evaluate_allowlisted_diagnostic_skip();
    assert_eq!(result.verdict, Verdict::AllowedSkip);
}
~~~

**Step 2: Run the policy tests**

Run: cargo test -p mrd-quality-gate --test policy_evaluation

Expected: FAIL because policies and evaluator are absent.

**Step 3: Implement strict evaluation**

The evaluator must:

- validate artifact structure before thresholds;
- distinguish product and infrastructure failures;
- require scenario, capability, and reason allowlists for a skip;
- prohibit skip/profile downgrade for required Windows P0 rows;
- evaluate direct and forced-relay thresholds separately;
- return all failed assertions, not only the first.

**Step 4: Run the quality-gate suite**

Run: cargo test -p mrd-quality-gate

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-quality-gate tests/quality-gates/policies
git commit -m "feat: enforce policy driven experience gates"
~~~

### Task 4: Add the quality-gate CLI and stable exit behavior

**Files:**
- Create: crates/mrd-quality-gate/src/bin/mrd-quality-gate.rs
- Create: crates/mrd-quality-gate/tests/cli_exit_codes.rs
- Modify: crates/mrd-quality-gate/Cargo.toml
- Create: tests/quality-gates/policies/windows-1080p60-direct.v1.json

**Step 1: Write failing CLI integration tests**

~~~rust
#[test]
fn invalid_artifact_exits_four() {
    let output = run_gate("missing-present.json", "windows-1080p60-direct.v1.json");
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn valid_direct_fixture_exits_zero() {
    let output = run_gate("valid-direct.json", "windows-1080p60-direct.v1.json");
    assert_eq!(output.status.code(), Some(0));
}
~~~

**Step 2: Run the CLI tests**

Run: cargo test -p mrd-quality-gate --test cli_exit_codes

Expected: FAIL because the binary does not exist.

**Step 3: Implement the CLI**

Accept:

~~~text
mrd-quality-gate --artifact PATH --policy PATH --output PATH
~~~

Write a machine-readable evaluation JSON even on failure, print one concise summary, and exit with the verdict code. Malformed arguments or unreadable input are INFRA_FAIL; valid JSON missing required evidence is INVALID_ARTIFACT.

**Step 4: Exercise each verdict**

Run: cargo test -p mrd-quality-gate --test cli_exit_codes

Expected: PASS with stable 0, 2, 3, and 4 behavior.

**Step 5: Commit**

~~~powershell
git add crates/mrd-quality-gate
git commit -m "feat: add quality gate command line evaluator"
~~~

### Task 5: Make component-matrix thresholds fail closed

**Files:**
- Modify: tests/component-matrix/scripts/summarize_component_results.ps1
- Modify: tests/component-matrix/scripts/run_component_case.ps1
- Modify: tests/component-matrix/scripts/component_matrix_common.ps1
- Modify: tests/component-matrix/scripts/test_component_matrix_common.ps1
- Create: tests/component-matrix/fixtures/threshold-failure-result.json
- Create: tests/component-matrix/fixtures/null-latency-result.json

**Step 1: Add failing PowerShell regression cases**

Add assertions that:

- a result with passed=false returns exit 2;
- a required null latency returns exit 4;
- a missing output file remains an infrastructure failure;
- a capability-gated optional case returns only an explicit allowed skip.

**Step 2: Run the script tests**

Run: powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/test_component_matrix_common.ps1

Expected: FAIL because threshold failure and null required fields currently do not propagate.

**Step 3: Route component results through the quality gate**

Keep PowerShell responsible for running and collecting. Convert the component result into the canonical artifact or a supported component projection, invoke mrd-quality-gate, and exit with its code.

Do not infer PASS from the cargo process exit code or output-file existence.

**Step 4: Re-run script tests and one real software component**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/test_component_matrix_common.ps1
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/encode.openh264.json
~~~

Expected: helper tests PASS; the real case returns a verdict consistent with its metrics.

**Step 5: Commit**

~~~powershell
git add tests/component-matrix
git commit -m "test: fail closed on component threshold misses"
~~~

### Task 6: Make transport summaries reject null evidence

**Files:**
- Modify: tests/benchmarks/scripts/summarize_transport_results.ps1
- Modify: tests/benchmarks/scripts/transport_matrix_common.ps1
- Modify: tests/benchmarks/scripts/test_transport_matrix_common.ps1
- Create: tests/benchmarks/fixtures/transport-null-required.json
- Create: tests/benchmarks/fixtures/transport-threshold-miss.json

**Step 1: Add failing transport helper tests**

Cover:

- null encode/send/decode/render latency;
- zero samples;
- profile downgrade in a required profile;
- process success with threshold failure;
- explicit optional hardware skip.

**Step 2: Run the helper suite**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1

Expected: FAIL on current permissive null and summary behavior.

**Step 3: Enforce canonical verdicts**

Generate a canonical artifact for every row and evaluate the configured policy. Keep producer_status separate from gate_verdict. Ensure the matrix process exits with the strongest non-zero required-row verdict.

**Step 4: Run tests and the quick software matrix**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 -ScenarioPath tests/benchmarks/scenarios/quick.transport.json
~~~

Expected: helper tests PASS and every accepted quick row has non-null required evidence.

**Step 5: Commit**

~~~powershell
git add tests/benchmarks
git commit -m "test: enforce complete transport benchmark evidence"
~~~

### Task 7: Enforce paired and dual-process verdicts

**Files:**
- Modify: tests/benchmarks/scripts/paired_lan_canary_common.ps1
- Modify: tests/benchmarks/scripts/run_paired_lan_canary.ps1
- Modify: tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1
- Modify: tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
- Create: tests/benchmarks/fixtures/paired-product-fail.json
- Create: tests/benchmarks/fixtures/paired-invalid-route-evidence.json

**Step 1: Add failing paired-script tests**

Assert that:

- a failed required row exits 2 after still writing artifacts;
- missing selected route evidence exits 4;
- forced relay with a non-relay candidate pair fails;
- cleanup failure is a product failure;
- an allowed diagnostic skip exits 0 but is not PASS.

**Step 2: Run the paired helper tests**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1

Expected: FAIL because report rows currently do not control the process exit.

**Step 3: Add the final evaluator step**

Both runners must:

1. finish collection and cleanup;
2. write summary, timeline, metrics, logs, and manifest;
3. invoke mrd-quality-gate;
4. preserve the evaluation artifact;
5. exit with the evaluator code.

**Step 4: Run local dual-process smoke**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p60 -DurationSecs 30
~~~

Expected: helper tests PASS; smoke produces a canonical artifact and honest verdict.

**Step 5: Commit**

~~~powershell
git add tests/benchmarks
git commit -m "test: enforce paired and dual process verdicts"
~~~

### Task 8: Separate frontend producer status from gate verdict and fix the known regression

**Files:**
- Modify: apps/Rdesk/src/app/services/lanE2eAutomationService.ts
- Modify: apps/Rdesk/src/app/services/lanE2eAutomationService.test.ts
- Modify: apps/Rdesk/src/app/services/lanE2eTelemetryService.ts
- Modify: apps/Rdesk/src/app/services/lanE2eTelemetryService.test.ts
- Modify: apps/Rdesk/src/app/components/TestWorkbench/E2ETestPage.tsx
- Modify: apps/Rdesk/src/app/components/TestWorkbench/E2ETestPage.test.tsx

**Step 1: Preserve the failing color-profile regression**

Run:

~~~powershell
pnpm --dir apps/Rdesk exec vitest run src/app/services/lanE2eAutomationService.test.ts -t "fails when the runtime pipeline color profile does not match the requested profile"
~~~

Expected: FAIL because the result is skipped/profile_downgraded instead of failed/media_profile_mismatch.

**Step 2: Add failing producer/gate separation tests**

~~~ts
expect(report.producerStatus).toBe("completed");
expect(report.gateVerdict).toBe("PRODUCT_FAIL");
expect(report.failureReason).toBe("media_profile_mismatch");
~~~

Also prove that an orchestration exception is producerStatus failed and that an allowed skip is not displayed as PASS.

**Step 3: Implement explicit mismatch classes and verdict fields**

Keep negotiated source/profile downgrade distinct from an invalid active pipeline. A runtime color, HDR, bit depth, pixel format, or color-pipeline mismatch is a product failure even outside the exact-profile scenario. Producer completion only means artifact collection ended.

**Step 4: Run targeted frontend verification**

Run:

~~~powershell
pnpm --dir apps/Rdesk test -- --run src/app/services/lanE2eAutomationService.test.ts src/app/services/lanE2eTelemetryService.test.ts src/app/components/TestWorkbench/E2ETestPage.test.tsx
pnpm --dir apps/Rdesk type-check
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk/src/app
git commit -m "fix: separate e2e production from gate verdict"
~~~

### Task 9: Make Gate 0 a required CI check

**Files:**
- Modify: .github/workflows/mainline-e2e.yml
- Create: .github/workflows/quality-gates.yml
- Create: crates/mrd-quality-gate/tests/workflow_contract.rs
- Create: tests/quality-gates/README.md

**Step 1: Add a workflow contract test**

Create a repository test that parses both YAML files and asserts:

- quality-gate tests run on every pull request;
- artifact upload uses an always condition;
- enforcement runs after upload and is not continue-on-error;
- required Windows rows invoke a release policy.

**Step 2: Run the contract test**

Run: cargo test -p mrd-quality-gate workflow_contract

Expected: FAIL until the workflows include the required jobs and order.

**Step 3: Add required jobs**

Add:

- Rust quality-gate tests;
- PowerShell helper tests;
- targeted frontend classification tests;
- canonical fixture evaluations;
- artifact upload followed by non-optional enforcement.

**Step 4: Run the complete Gate 0 suite locally**

Run:

~~~powershell
cargo test -p mrd-quality-gate
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/test_component_matrix_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_transport_matrix_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
pnpm --dir apps/Rdesk test -- --run src/app/services/lanE2eAutomationService.test.ts src/app/services/lanE2eTelemetryService.test.ts
pnpm --dir apps/Rdesk type-check
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add .github/workflows tests/quality-gates crates/mrd-quality-gate/tests/workflow_contract.rs
git commit -m "ci: require truthful remote experience gates"
~~~

## Secure LAN — Identity, Authorization, And Protected Control

### Task 10: Add orthogonal session authorization, route, and media states

**Files:**
- Modify: crates/mrd-session/src/lib.rs
- Create: crates/mrd-session/src/authorization.rs
- Create: crates/mrd-session/src/route.rs
- Create: crates/mrd-session/src/media.rs
- Create: crates/mrd-session/src/remote_session.rs
- Create: crates/mrd-session/tests/remote_session_state.rs

**Step 1: Write failing state-machine tests**

~~~rust
#[test]
fn media_cannot_start_before_authorization() {
    let mut session = RemoteSessionAggregate::new(fixture_plan());
    assert_eq!(
        session.start_media(),
        Err(SessionTransitionError::AuthorizationRequired)
    );
}

#[test]
fn route_migration_preserves_granted_scopes() {
    let mut session = authorized_streaming_session(RouteKind::LanQuic);
    let scopes = session.granted_scopes().clone();
    session.begin_route_migration(RouteKind::WebRtcRelay).unwrap();
    session.complete_route_migration(RouteKind::WebRtcRelay).unwrap();
    assert_eq!(session.granted_scopes(), &scopes);
}
~~~

Cover terminal transitions, reconnect leases, denied authorization, and derived presentation state.

**Step 2: Run the domain tests**

Run: cargo test -p mrd-session --test remote_session_state

Expected: FAIL because the aggregate and sub-states do not exist.

**Step 3: Implement the minimal aggregate**

Store:

- peer binding and role;
- AuthorizationState;
- RouteState;
- MediaState;
- granted scopes;
- policy revision;
- optional reconnect lease;
- stable last failure.

Keep SessionLifecycleState as a compatibility projection until IPC consumers migrate.

**Step 4: Run all session tests**

Run: cargo test -p mrd-session

Expected: PASS, including existing session/scheduler tests.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session
git commit -m "feat: model authorized remote session state"
~~~

### Task 11: Add normalized permission scopes and signed grant payloads

**Files:**
- Create: crates/mrd-session/src/permissions.rs
- Create: crates/mrd-session/src/grant.rs
- Modify: crates/mrd-session/src/lib.rs
- Create: crates/mrd-session/tests/session_grants.rs

**Step 1: Write failing permission intersection tests**

~~~rust
#[test]
fn effective_scopes_are_the_strict_intersection() {
    let effective = EffectiveScopes::resolve(
        scopes(&[ScreenView, InputKeyboard, FileWrite]),
        scopes(&[ScreenView, InputKeyboard]),
        scopes(&[ScreenView]),
        scopes(&[ScreenView, InputKeyboard]),
        scopes(&[ScreenView, InputKeyboard]),
    );
    assert_eq!(effective, scopes(&[ScreenView]));
}

#[test]
fn expired_grant_cannot_authorize_input() {
    let grant = fixture_grant_with_expiry(100);
    assert_eq!(grant.authorize(InputKeyboard, 101), Err(GrantError::Expired));
}
~~~

**Step 2: Run the grant tests**

Run: cargo test -p mrd-session --test session_grants

Expected: FAIL.

**Step 3: Implement scopes and grant body**

Include all design scopes, session/peer IDs, Windows session ID, issue/expiry times, nonce, policy revision, route/profile constraints, and transport fingerprint commitments. The domain crate stores signature bytes but does not perform cryptography.

**Step 4: Run the session crate**

Run: cargo test -p mrd-session

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session
git commit -m "feat: define remote session permission grants"
~~~

### Task 12: Add the device-identity crate and signed session messages

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-identity/Cargo.toml
- Create: crates/mrd-identity/src/lib.rs
- Create: crates/mrd-identity/src/device_key.rs
- Create: crates/mrd-identity/src/session_message.rs
- Create: crates/mrd-identity/tests/signatures.rs

**Step 1: Write failing signature tests**

~~~rust
#[test]
fn signed_intent_rejects_target_or_scope_tampering() {
    let identity = DeviceIdentity::generate(&SystemRandom::new()).unwrap();
    let mut signed = identity.sign_intent(fixture_intent()).unwrap();
    signed.payload.requested_scopes.insert(PermissionScope::FileWrite);
    assert_eq!(signed.verify(), Err(IdentityError::InvalidSignature));
}

#[test]
fn signed_grant_is_bound_to_both_peer_keys() {
    let signed = fixture_signed_grant();
    assert!(signed.verify_for(&controller_public_key(), &target_public_key()).is_ok());
    assert!(signed.verify_for(&other_public_key(), &target_public_key()).is_err());
}
~~~

**Step 2: Run the identity tests**

Run: cargo test -p mrd-identity --test signatures

Expected: FAIL because the crate is absent.

**Step 3: Implement keys and canonical signed bytes**

Add ring 0.17 as an explicit workspace dependency. Use ring::signature::Ed25519KeyPair, ring::rand::SystemRandom, SHA-256, and HMAC with fixed domain separators. Serialize a versioned canonical payload before signing. Never sign an arbitrary serde JSON map whose ordering can vary.

Expose:

- DeviceIdentity generation/load from protected PKCS#8 bytes;
- DevicePublicKey and stable key ID;
- SignedSessionIntent;
- SignedSessionGrant;
- transport fingerprint binding;
- verification errors that do not leak secret material.

**Step 4: Run the identity suite**

Run: cargo test -p mrd-identity

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-identity
git commit -m "feat: add signed device identity protocol"
~~~

### Task 13: Add replay windows, SAS, rotation, and unattended proof contracts

**Files:**
- Create: crates/mrd-identity/src/replay.rs
- Create: crates/mrd-identity/src/sas.rs
- Create: crates/mrd-identity/src/rotation.rs
- Create: crates/mrd-identity/src/unattended.rs
- Modify: crates/mrd-identity/src/lib.rs
- Create: crates/mrd-identity/tests/trust_protocol.rs

**Step 1: Write failing protocol tests**

Cover:

- duplicate nonce rejected;
- counter rollback rejected;
- SAS identical for both peers and changed by transcript tampering;
- rotation requires old-key signature and increasing epoch;
- revoked old key cannot rotate;
- unattended challenge proof is session-transcript bound;
- wrong credential and replayed proof fail.

**Step 2: Run the focused tests**

Run: cargo test -p mrd-identity --test trust_protocol

Expected: FAIL.

**Step 3: Implement the protocols**

Use fixed versioned domain separators for every hash/HMAC/signature purpose. Represent generated unattended credentials as at least 128 random bits. Do not add low-entropy human passwords in this task.

**Step 4: Run identity tests**

Run: cargo test -p mrd-identity

Expected: PASS with no secret values in debug output.

**Step 5: Commit**

~~~powershell
git add crates/mrd-identity
git commit -m "feat: secure identity replay and rotation"
~~~

### Task 14: Add persistent identity, trust, and audit storage

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-store-sqlite/Cargo.toml
- Create: crates/mrd-store-sqlite/src/lib.rs
- Create: crates/mrd-store-sqlite/src/integrity.rs
- Create: crates/mrd-store-sqlite/src/migrations.rs
- Create: crates/mrd-store-sqlite/src/identity_store.rs
- Create: crates/mrd-store-sqlite/src/trust_store.rs
- Create: crates/mrd-store-sqlite/src/audit_store.rs
- Create: crates/mrd-store-sqlite/tests/persistence.rs
- Modify: crates/mrd-identity/src/lib.rs
- Modify: crates/mrd-identity/Cargo.toml
- Modify: crates/mrd-identity/tests/trust_protocol.rs
- Modify: crates/mrd-session/src/authorization.rs
- Modify: crates/mrd-session/src/media.rs
- Modify: crates/mrd-session/src/route.rs
- Modify: apps/mrd-service/Cargo.toml
- Modify: apps/mrd-service/src/lib.rs
- Create: apps/mrd-service/src/security/mod.rs
- Create: apps/mrd-service/src/security/windows_dpapi.rs
- Create: apps/mrd-service/src/security/unsupported.rs

**Step 1: Write failing restart and corruption tests**

Tests must prove:

- generated identity reloads with the same public key;
- trusted/revoked state survives close and reopen;
- monotonic audit sequence survives restart;
- audit integrity-chain tampering is detected;
- corrupt or unreadable identity state prevents new authorization;
- plaintext private-key bytes are absent from the database file.
- identity initialization metadata cannot be reset to silently replace the machine key;
- revoked trust cannot be reactivated, deleted, or injected through direct SQL or a trigger;
- deleting the audit key/events/head cannot restart the audit sequence;
- missing tables or security metadata are not recreated on an existing database;
- identity rows cannot be spliced between stores that share the same test protector;
- concurrent first openers converge on one atomic store birth;
- sensitive plaintext containers have a compiler-resistant zeroize-on-drop contract;
- Windows directory provisioning rejects untrusted owners, files, junctions, ACL drift, and invalid service SIDs.

**Step 2: Run the store tests**

Run: cargo test -p mrd-store-sqlite

Expected: FAIL because the crate is absent.

**Step 3: Implement SQLite stores and SecretProtector**

Use immediate transactions and a sealed schema-v2 store manifest. Protect a dedicated random store-integrity key and commit the store ID, generation, canonical SQLite schema, identity row, sorted full trust snapshot, protected audit-key blob, and sealed audit head under HMAC. Verify the manifest and relevant committed state inside the same read snapshot or `BEGIN IMMEDIATE` write transaction before every operation. Never auto-repair a missing table, manifest, key, or initialized subsystem on an existing database. Reject triggers, views, and table/index drift through the sealed schema commitment so a legal write cannot launder injected state into a fresh manifest.

Schema v2 is the first supported sealed format. The unsealed development-only v1 format is deliberately rejected fail-closed rather than trusted during migration; Task 14 is amended before release, so no supported installation depends on v1 data.

Define a `SecretProtector` port whose plaintext result uses compiler-resistant zeroization on drop. On Windows use store-ID/purpose-bound DPAPI machine scope. Provision the product directory through a no-follow handle, reject reparse points and untrusted pre-existing owners before mutation, set and read back a protected owner/DACL, and require an explicit bootstrap or installed-service-SID policy. Task 26 resolves and configures the actual SCM service SID, then passes it to this adapter. On unsupported platforms fail closed in production. Keep the authenticated fixed-key protector private to store integration tests.

Pin trust to a structurally validated Ed25519 public-key ID/epoch with terminal revocation. Protect a random audit HMAC key, verify the full chain before append, and bind its protected blob and current head into the store manifest so tail deletion or total audit-anchor reset is detected.

The in-database manifest detects partial row/table/schema deletion, modification, splicing, and reset. It cannot distinguish deletion of the entire protected SQLite file or rollback of the file together with its valid manifest; the Windows DACL keeps that operation outside the standard-user threat boundary. Detecting privileged whole-file rollback would require a separate non-rollbackable TPM, remote checkpoint, or equivalent external monotonic anchor and is not claimed by this task.

Never log database rows that contain encrypted secret blobs or credential verifiers.

**Step 4: Run store and Windows adapter tests**

Run:

~~~powershell
cargo test -p mrd-store-sqlite
cargo test -p mrd-service security
~~~

Expected: PASS on Windows; non-Windows builds compile with explicit unsupported capability.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-store-sqlite crates/mrd-identity crates/mrd-session apps/mrd-service/Cargo.toml apps/mrd-service/src/lib.rs apps/mrd-service/src/security docs/plans/2026-07-11-market-remote-capability-alignment.md
git commit -m "feat: persist protected remote trust state"
~~~

### Task 15: Extend IPC for authorization, consent, trust, and route evidence

**Files:**
- Modify: crates/mrd-session/src/permissions.rs
- Modify: crates/mrd-ipc/Cargo.toml
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: crates/mrd-ipc/tests/contracts.rs
- Modify: apps/mrd-service/src/ipc_server/dispatch.rs
- Modify: apps/Rdesk/src-tauri/src/main.rs
- Modify: apps/Rdesk/src/app/adapters/serviceBridge/client.ts
- Modify: apps/Rdesk/src/app/adapters/serviceBridge/client.test.ts
- Modify: apps/Rdesk/src/app/adapters/tauri/types.ts
- Modify: apps/Rdesk/src/app/adapters/tauri/commands.ts
- Modify: apps/Rdesk/src/app/adapters/tauri/contract.test.ts

**Step 1: Add failing round-trip tests**

Add typed requests/responses for:

- GetRemoteSession;
- RequestRemoteSession;
- RespondToConsent;
- Enable/Disable/RotateUnattendedAccess;
- List/Approve/Suspend/Revoke/RotateTrustedDevice;
- ChangeSessionPermissions;
- SubscribeSessionEvents;
- GetRouteEvidence;
- GetAuditEventsV2.

Verify stable serde tags and all reason codes.

**Step 2: Run Rust and frontend contract tests**

Run:

~~~powershell
cargo test -p mrd-ipc
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/contract.test.ts
~~~

Expected: FAIL until contracts and frontend mappings exist.

**Step 3: Implement contracts without business logic**

Reuse mrd-session scope/state DTOs through exhaustive wire projections. Use canonical decimal-string u64 values for revisions, epochs, and cursors. Define `after_sequence` as exclusive and return the greatest delivered value as `next_after_sequence`, with an explicit reset-required state for retention gaps. Use allowlisted typed audit metadata rather than arbitrary key/value details. Do not expose private keys, credentials, proofs, integrity HMACs, or internal database identifiers.

The Tauri passthrough must allowlist exactly these secure-remote requests before opening IPC. Until business handlers land, mrd-service must reject all 15 operations with `E_SECURE_REMOTE_UNAVAILABLE`; it must not fall through to legacy session behavior. Browser-bridge and Tauri errors must preserve the same structured code/message shape.

This Task 15 boundary is not yet a global authorization fence: legacy `StartSession`, remote power, file, and control entrypoints remain migration debt. Later handler tasks must gate or retire them before the secure-session surface is advertised as complete.

**Step 4: Re-run contract tests**

Run the same two commands.

Also run the mrd-session/mrd-identity tests, the focused mrd-service fail-closed test, the Tauri allowlist tests, the service-bridge client test, and frontend type checking because the stable wire boundary spans those adapters.

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session/src/permissions.rs crates/mrd-ipc apps/mrd-service/src/ipc_server/dispatch.rs apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src/app/adapters/serviceBridge apps/Rdesk/src/app/adapters/tauri docs/plans/2026-07-11-market-remote-capability-alignment.md
git commit -m "feat: define secure remote session ipc"
~~~

### Task 16: Replace in-memory trust and audit registries in mrd-service

**Files:**
- Modify: apps/mrd-service/Cargo.toml
- Modify: apps/mrd-service/src/app_state.rs
- Modify: apps/mrd-service/src/app_state/core.rs
- Modify: apps/mrd-service/src/app_state/device_identity_registry.rs
- Modify: apps/mrd-service/src/app_state/audit_log_registry.rs
- Modify: apps/mrd-service/src/handlers/identity.rs
- Modify: apps/mrd-service/src/handlers/telemetry.rs
- Modify: apps/mrd-service/src/ipc_server/audit.rs
- Modify: apps/mrd-service/src/ipc_server/dispatch.rs
- Modify: apps/mrd-service/src/main.rs
- Create: apps/mrd-service/tests/persistent_identity.rs
- Modify: crates/mrd-store-sqlite/src/lib.rs
- Modify: crates/mrd-store-sqlite/src/trust_store.rs
- Modify: crates/mrd-store-sqlite/src/audit_store.rs
- Modify: crates/mrd-store-sqlite/tests/persistence.rs

**Step 1: Write failing service restart tests**

Start a service state with a temporary data directory and authenticated test protector, approve a real Ed25519 peer key, append audit events, destroy the state, reopen it, and assert identity/trust/audit persistence. Prove stale revisions and revoked-key reactivation leave trust unchanged while committing stable denial audits. Prove the legacy DeviceId/fingerprint pairing path fails closed because it has no authenticated peer public key.

**Step 2: Run the service integration test**

Run: cargo test -p mrd-service --test persistent_identity

Expected: FAIL because AppState owns only in-memory registries.

**Step 3: Inject persistent store ports**

Keep in-memory fakes available only in test/debug builds. Production AppState construction must require a protected store path and SecretProtector, and the release entrypoint must verify the fixed protected ProgramData directory before opening the store or any listener. Run synchronous SQLite work behind a blocking boundary. Convert authenticated trust commands to combined trust-plus-audit transactions; expose durable list/suspend/revoke operations, but keep approval unavailable until an authenticated pending public key exists. Never fabricate a public key from a DeviceId or fingerprint. Latch real store failures into unhealthy service state, keep caller validation errors separate, and persist only stable error codes rather than human error text.

**Step 4: Run service tests**

Run:

~~~powershell
cargo test -p mrd-service --test persistent_identity
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-store-sqlite docs/plans/2026-07-11-market-remote-capability-alignment.md
git commit -m "refactor: make trust and audit persistent"
~~~

### Task 17: Sign LAN discovery identity and session bootstrap

**Files:**
- Modify: apps/mrd-service/src/lan_discovery/protocol.rs
- Modify: apps/mrd-service/src/lan_discovery/discovery_identity.rs
- Modify: apps/mrd-service/src/lan_discovery/peer_registry.rs
- Modify: apps/mrd-service/src/lan_discovery/service_identity.rs
- Modify: apps/mrd-service/src/lan_discovery.rs
- Modify: crates/mrd-transport-quic-quinn/src/lib.rs
- Create: apps/mrd-service/tests/signed_lan_identity.rs

**Step 1: Write failing spoofing tests**

Prove that:

- a tampered device ID/name/address invalidates the announcement signature;
- a replayed expired announcement is rejected;
- an untrusted signed peer is discoverable but not controllable;
- a QUIC certificate replacement not signed by the trusted device key fails;
- legacy unsigned peers never enter product sessions.

**Step 2: Run the signed-LAN tests**

Run: cargo test -p mrd-service --test signed_lan_identity

Expected: FAIL because LAN announcements and bootstrap are not identity-bound.

**Step 3: Add signed versioned envelopes**

Sign identity, instance ID, endpoints, capability hash, protocol version, timestamp/expiry, nonce, and current key epoch. Bind the ephemeral QUIC certificate fingerprint in the signed session grant/bootstrap.

Keep unsigned legacy parsing diagnostic-only behind an explicit compatibility setting.

**Step 4: Run LAN and QUIC tests**

Run:

~~~powershell
cargo test -p mrd-service signed_lan
cargo test -p mrd-transport-quic-quinn
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service/src/lan_discovery apps/mrd-service/src/lan_discovery.rs apps/mrd-service/tests crates/mrd-transport-quic-quinn
git commit -m "feat: authenticate lan discovery and quic bootstrap"
~~~

### Task 18: Replace LAN auto-accept with consent or unattended authorization

**Files:**
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-service/src/lan_discovery/session_runtime.rs
- Modify: apps/mrd-service/src/lan_discovery.rs
- Modify: apps/mrd-service/src/ipc_server/dispatch.rs
- Modify: apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx
- Create: apps/Rdesk/src/app/components/IncomingSessionDialog.tsx
- Create: apps/Rdesk/src/app/components/IncomingSessionDialog.test.tsx
- Create: apps/mrd-service/tests/session_authorization.rs

**Step 1: Write failing authorization tests**

Cover:

- trusted peer still requires consent when unattended is disabled;
- untrusted peer cannot use unattended access;
- valid unattended proof grants only configured scopes;
- deny sends a stable denial and starts no capture/media task;
- grant exists before sender/listener begins;
- timeout expires the request.

**Step 2: Run service and UI tests**

Run:

~~~powershell
cargo test -p mrd-service --test session_authorization
pnpm --dir apps/Rdesk test -- --run src/app/components/IncomingSessionDialog.test.tsx
~~~

Expected: FAIL.

**Step 3: Implement the authorization use case**

Route the incoming request to the selected desktop agent/UI consent surface. Remove automatic acceptance from the LAN request handler. Issue a signed grant after consent or unattended proof. Start transport/media only after grant verification.

**Step 4: Run targeted and full service tests**

Run:

~~~powershell
cargo test -p mrd-service --test session_authorization
cargo test -p mrd-service
pnpm --dir apps/Rdesk test -- --run src/app/components/IncomingSessionDialog.test.tsx
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service apps/Rdesk/src/app/components
git commit -m "feat: authorize incoming lan sessions"
~~~

### Task 19: Cryptographically bind control input to grant, peer, scope, and sequence

**Files:**
- Create: crates/mrd-session/src/control_envelope.rs
- Modify: crates/mrd-session/src/lib.rs
- Modify: apps/mrd-service/src/lan_discovery/lan_control_input.rs
- Modify: apps/mrd-service/src/control_input.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: common-control-proto/src/lib.rs
- Create: apps/mrd-service/tests/authorized_control_input.rs

**Step 1: Write failing negative tests**

Cover forged source device, wrong session, missing keyboard scope, stale policy revision, duplicate sequence, out-of-window sequence, revoked grant, and tampered payload. Assert zero injection side effects and one audit decision.

**Step 2: Run the test**

Run: cargo test -p mrd-service --test authorized_control_input

Expected: FAIL because current LAN control relies on an unauthenticated envelope.

**Step 3: Implement ControlEnvelopeV2**

Bind:

- protocol version;
- session and grant IDs;
- source/target key IDs;
- exact scope;
- sequence;
- event ID;
- expiry/policy revision;
- authenticated event bytes.

Keep realtime coalescing after authentication. Reliable key/button events retain retry/ack semantics. Release all pressed input on every authorization or route terminal transition.

**Step 4: Run input and service tests**

Run:

~~~powershell
cargo test -p mrd-service authorized_control_input
cargo test -p mrd-input
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session common-control-proto apps/mrd-service
git commit -m "feat: authenticate remote control input"
~~~

### Task 20: Add secure-LAN product and security-negative gates

**Files:**
- Create: tests/quality-gates/policies/windows-secure-lan.v1.json
- Create: tests/quality-gates/policies/windows-security-negative.v1.json
- Create: tests/quality-gates/fixtures/security-untrusted.json
- Create: tests/quality-gates/fixtures/security-replay.json
- Create: tests/quality-gates/fixtures/security-revoked.json
- Create: tests/quality-gates/fixtures/security-wrong-scope.json
- Create: tests/quality-gates/fixtures/security-certificate-substitution.json
- Modify: tests/benchmarks/scripts/paired_lan_canary_common.ps1
- Modify: tests/benchmarks/scripts/run_paired_lan_canary.ps1
- Create: tests/benchmarks/scripts/run_secure_lan_negative.ps1
- Modify: .github/workflows/mainline-e2e.yml

**Step 1: Add failing policy fixtures**

Create fixtures for untrusted, replayed, revoked, wrong-scope, and certificate-substitution attempts. Every case must require reject=true, media/control side effects zero, and an audit event.

**Step 2: Evaluate fixtures**

Run: cargo run -p mrd-quality-gate -- --artifact tests/quality-gates/fixtures/security-replay.json --policy tests/quality-gates/policies/windows-security-negative.v1.json

Expected: FAIL with a non-zero verdict until the fixture producer and policy are supported.

**Step 3: Add secure-LAN scenarios and script**

The positive route must prove trusted identity, consent/unattended grant, selected QUIC route, real frames, authorized input, and cleanup. Negative scenarios must never start sender/receiver tasks.

**Step 4: Run local and paired secure-LAN suites**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_secure_lan_negative.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 -TargetDeviceId $env:MRD_DEVICE_LAB_TARGET_DEVICE_ID -ScenarioId cross.e2e.secure_remote_display -ProfileId 1080p60
~~~

Expected: positive PASS on the configured device lab; every negative attempt PASSes only by being rejected.

**Step 5: Commit**

~~~powershell
git add tests/quality-gates tests/benchmarks .github/workflows/mainline-e2e.yml
git commit -m "test: gate secure lan remote sessions"
~~~

## Windows Machine Service And Interactive-Session Agent

### Task 21: Add the machine-service/session-agent IPC crate

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-agent-ipc/Cargo.toml
- Create: crates/mrd-agent-ipc/src/lib.rs
- Create: crates/mrd-agent-ipc/src/protocol.rs
- Create: crates/mrd-agent-ipc/src/grant.rs
- Create: crates/mrd-agent-ipc/tests/contracts.rs

**Step 1: Write failing protocol round-trip tests**

Add tests for:

- AgentRegister with process ID, logon SID hash, and Windows session ID;
- AgentChallenge and signed AgentRegistered;
- AgentCapabilitySnapshot;
- ConsentRequest/ConsentResult;
- ExecuteGrant bound to session, peer, scopes, policy revision, Windows session, and expiry;
- Start/Stop capture, input, audio, clipboard, file, and render commands;
- DesktopChanged, Locked, Unlocked, AgentStopping, and AgentCrashed events.

**Step 2: Run the contract tests**

Run: cargo test -p mrd-agent-ipc

Expected: FAIL because the crate is absent.

**Step 3: Implement a framed versioned protocol**

Use length-delimited serde messages with a maximum frame size. Reject unknown protocol major versions, duplicate registration, expired grants, and session mismatch. Keep private-key and unattended-secret material out of all messages.

**Step 4: Run tests**

Run: cargo test -p mrd-agent-ipc

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-agent-ipc
git commit -m "feat: define desktop agent ipc"
~~~

### Task 22: Add the mrd-session-agent application shell

**Files:**
- Modify: Cargo.toml
- Create: apps/mrd-session-agent/Cargo.toml
- Create: apps/mrd-session-agent/src/main.rs
- Create: apps/mrd-session-agent/src/runtime.rs
- Create: apps/mrd-session-agent/src/capabilities.rs
- Create: apps/mrd-session-agent/tests/agent_smoke.rs

**Step 1: Write a failing in-process smoke test**

The test starts a fake machine-service endpoint, launches the agent runtime with a fixed session descriptor, completes registration, returns capabilities, receives StopAgent, and exits cleanly.

**Step 2: Run the smoke test**

Run: cargo test -p mrd-session-agent --test agent_smoke

Expected: FAIL because the app is absent.

**Step 3: Implement the minimal agent**

The initial app owns no product behavior beyond:

- connecting only to the configured private endpoint;
- registering immutable process/session identity;
- reporting truthful platform capabilities;
- accepting only verified ExecuteGrant-bearing commands;
- clean shutdown and heartbeat.

**Step 4: Run agent and workspace checks**

Run:

~~~powershell
cargo test -p mrd-session-agent
cargo check -p mrd-service -p mrd-session-agent
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml apps/mrd-session-agent
git commit -m "feat: add interactive session agent"
~~~

### Task 23: Add authenticated private named-pipe registration and agent registry

**Files:**
- Modify: apps/mrd-service/Cargo.toml
- Create: apps/mrd-service/src/agent_runtime/mod.rs
- Create: apps/mrd-service/src/agent_runtime/registry.rs
- Create: apps/mrd-service/src/agent_runtime/server.rs
- Create: apps/mrd-service/src/agent_runtime/windows_pipe.rs
- Create: apps/mrd-service/src/agent_runtime/unsupported.rs
- Modify: apps/mrd-service/src/app_state/core.rs
- Create: apps/mrd-service/tests/agent_registration.rs

**Step 1: Write failing registration security tests**

Test:

- expected Windows session registers once;
- wrong logon/session identity is rejected;
- anonymous or network identity is rejected;
- stale challenge is rejected;
- duplicate process for the same session follows an explicit replacement policy;
- disconnected agent invalidates outstanding execution grants.

Use a platform-neutral fake verifier for generic CI and Windows token/ACL integration tests behind cfg(windows).

**Step 2: Run the registration tests**

Run: cargo test -p mrd-service --test agent_registration

Expected: FAIL.

**Step 3: Implement the registry and secure pipe**

On Windows create the named pipe with an explicit security descriptor and validate caller PID/token/logon SID/session ID before registration. The agent registry maps Windows session IDs to capability snapshots and health.

**Step 4: Run service tests**

Run:

~~~powershell
cargo test -p mrd-service --test agent_registration
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service
git commit -m "feat: authenticate desktop session agents"
~~~

### Task 24: Route consent and input through the selected session agent

**Files:**
- Create: apps/mrd-session-agent/src/consent.rs
- Create: apps/mrd-session-agent/src/input.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-service/src/control_input.rs
- Modify: apps/mrd-service/src/agent_runtime/registry.rs
- Modify: crates/mrd-input/src/lib.rs
- Modify: crates/mrd-input/src/windows.rs
- Create: apps/mrd-service/tests/agent_input_routing.rs
- Create: apps/mrd-session-agent/tests/input_grants.rs

**Step 1: Write failing routing tests**

Prove:

- consent goes only to the Windows session named in the request;
- input is delivered only after an ExecuteGrant with matching session and scopes;
- fast user switch does not retarget an active session;
- agent disconnect releases all input and pauses control;
- stale agent grants are rejected.

**Step 2: Run the tests**

Run:

~~~powershell
cargo test -p mrd-session-agent --test input_grants
cargo test -p mrd-service --test agent_input_routing
~~~

Expected: FAIL.

**Step 3: Move desktop-bound input execution**

Keep authorization and network control in mrd-service. Move only platform injection and pressed-state tracking into mrd-session-agent. Return structured injection acknowledgments and UIPI/desktop errors.

**Step 4: Run input, agent, and service suites**

Run:

~~~powershell
cargo test -p mrd-input
cargo test -p mrd-session-agent
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-input apps/mrd-session-agent apps/mrd-service
git commit -m "refactor: execute input in desktop session agent"
~~~

### Task 25: Route capture and render-surface work through the session agent

**Files:**
- Create: apps/mrd-session-agent/src/capture.rs
- Create: apps/mrd-session-agent/src/render.rs
- Create: apps/mrd-session-agent/src/media.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: apps/mrd-service/src/lan_discovery/media_frame_capture.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender.rs
- Modify: apps/mrd-service/src/lan_discovery/media_render_worker.rs
- Modify: apps/mrd-service/src/app_state/platform_surface_renderer.rs
- Modify: apps/Rdesk/src-tauri/src/remote_display_surface.rs
- Modify: tests/integration/Cargo.toml
- Create: apps/mrd-session-agent/tests/media_grants.rs
- Create: tests/integration/service_agent_media.rs

**Step 1: Write a failing deterministic media-boundary test**

Use synthetic capture and memory render adapters. Assert:

- capture starts only with screen.view grant;
- encoded access units remain bound to the logical session;
- receiver frames reach the agent render adapter;
- stopping/revoking the grant stops capture/render and clears queues;
- no raw frame flows through Rdesk WebView.

**Step 2: Run the integration test**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test service_agent_media

Expected: FAIL because media remains in the current service process.

**Step 3: Introduce a measured agent media boundary**

Keep the product ownership in mrd-service while executing desktop-bound capture/render in the agent. Prefer encoded access units or shared GPU resources over raw CPU frame copies. Add explicit queue bounds and process-boundary timing metrics.

Do not remove the existing in-process adapter until the agent path passes the same local baseline.

**Step 4: Compare old and new local baselines**

Run:

~~~powershell
cargo test --manifest-path tests/integration/Cargo.toml --test service_agent_media
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p60 -DurationSecs 30
~~~

Expected: tests PASS and the artifact reports process-boundary overhead without hiding regressions.

**Step 5: Commit**

~~~powershell
git add apps/mrd-session-agent apps/mrd-service apps/Rdesk/src-tauri tests/integration
git commit -m "refactor: execute desktop media in session agent"
~~~

### Task 26: Install and supervise mrd-service as a Windows machine service

**Files:**
- Modify: apps/mrd-service/Cargo.toml
- Create: apps/mrd-service/src/windows_service.rs
- Create: apps/mrd-service/src/agent_runtime/windows_sessions.rs
- Modify: apps/mrd-service/src/main.rs
- Modify: apps/mrd-service/src/shell/mod.rs
- Modify: apps/mrd-service/src/shell/windows.rs
- Create: apps/mrd-service/tests/windows_service_contract.rs
- Create: apps/Rdesk/scripts/install-mrd-service.ps1
- Create: apps/Rdesk/scripts/uninstall-mrd-service.ps1

**Step 1: Write failing service-lifecycle tests**

Test the platform-neutral state machine for:

- SCM start/stop/preshutdown;
- clean transport and agent shutdown;
- agent launch for an eligible interactive session;
- user logon/logoff and fast-user-switch events;
- service restart preserving trust and invalidating active execution grants.

**Step 2: Run the lifecycle tests**

Run: cargo test -p mrd-service windows_service_contract

Expected: FAIL.

**Step 3: Implement SCM and session supervision**

Keep console mode for development/tests. Add explicit install/uninstall scripts, service SID/DACL configuration, protected data directory creation, event logging, and deterministic cleanup.

Do not configure an interactive service. Launch session agents through the supported user-session process mechanism.

**Step 4: Verify console and installed modes**

Run:

~~~powershell
cargo test -p mrd-service windows_service_contract
cargo build -p mrd-service -p mrd-session-agent
powershell -ExecutionPolicy Bypass -File apps/Rdesk/scripts/install-mrd-service.ps1 -WhatIf
powershell -ExecutionPolicy Bypass -File apps/Rdesk/scripts/uninstall-mrd-service.ps1 -WhatIf
~~~

Expected: tests/build PASS and WhatIf reports intended SCM/ACL operations without mutation.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service apps/Rdesk/scripts
git commit -m "feat: run mrd-service as windows machine service"
~~~

## Public Connectivity — Signaling, WebRTC, TURN, And Recovery

### Task 27: Version and authenticate the signaling protocol

**Files:**
- Modify: crates/mrd-signal-proto/Cargo.toml
- Modify: crates/mrd-signal-proto/src/lib.rs
- Modify: crates/mrd-signal-client/Cargo.toml
- Modify: crates/mrd-signal-client/src/lib.rs
- Modify: crates/mrd-signal-server/Cargo.toml
- Modify: crates/mrd-signal-server/src/lib.rs
- Create: crates/mrd-signal-proto/tests/authenticated_messages.rs

**Step 1: Write failing signaling-contract tests**

Add versioned types for:

- ServerChallenge;
- AuthenticatedRegister and Registered;
- PresenceHeartbeat;
- Signed SessionIntent/SessionGrant/SessionDeny;
- WebRTC offer/answer/candidate;
- SessionClose;
- ReconnectRequest/ReconnectGrant;
- protocol error with stable reason code.

Tests must reject missing version, wrong intended peer, expired message, invalid signature, and repeated nonce.

**Step 2: Run signaling tests**

Run:

~~~powershell
cargo test -p mrd-signal-proto
cargo test -p mrd-signal-client
cargo test -p mrd-signal-server
~~~

Expected: FAIL until the authenticated protocol exists.

**Step 3: Implement canonical signed messages**

Carry signed mrd-identity payloads without allowing the server to rewrite them. Keep transport candidates outside the authorization decision but bind their accepted fingerprints in the grant.

**Step 4: Run all signaling tests**

Run the same commands.

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-signal-proto crates/mrd-signal-client crates/mrd-signal-server
git commit -m "feat: authenticate signaling messages"
~~~

### Task 28: Harden realtime-server registration, presence, and route authorization

**Files:**
- Modify: apps/realtime-server/Cargo.toml
- Split: apps/realtime-server/src/main.rs
- Create: apps/realtime-server/src/lib.rs
- Create: apps/realtime-server/src/auth.rs
- Create: apps/realtime-server/src/presence.rs
- Create: apps/realtime-server/src/routes.rs
- Create: apps/realtime-server/src/ws.rs
- Create: apps/realtime-server/tests/authenticated_routing.rs

**Step 1: Write failing server integration tests**

Cover:

- register challenge and proof;
- caller-provided device ID without proof rejected;
- expired backend device token rejected;
- session intent routes only to the authorized target;
- random peer cannot inject offer/candidate/close;
- replay and rate-limit failures;
- disconnect removes presence and expires routes;
- duplicate idempotency key does not create duplicate session route.

**Step 2: Run the server test**

Run: cargo test -p realtime-server --test authenticated_routing

Expected: FAIL because registration currently trusts self-reported IDs.

**Step 3: Implement authenticated routing**

Abstract backend token verification behind a port with a deterministic test fake. Require WSS in deployed configuration, configurable bind address, heartbeat TTL, payload limits, and structured audit-safe logs.

**Step 4: Run realtime tests**

Run:

~~~powershell
cargo test -p realtime-server
cargo clippy -p realtime-server -- -D warnings
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/realtime-server
git commit -m "feat: authorize realtime session routing"
~~~

### Task 29: Integrate the authenticated signal client into mrd-service

**Files:**
- Modify: apps/mrd-service/Cargo.toml
- Create: apps/mrd-service/src/signaling/mod.rs
- Create: apps/mrd-service/src/signaling/runtime.rs
- Create: apps/mrd-service/src/signaling/config.rs
- Create: apps/mrd-service/src/signaling/event_mapper.rs
- Modify: apps/mrd-service/src/main.rs
- Modify: apps/mrd-service/src/app_state/core.rs
- Modify: crates/mrd-application/src/lib.rs
- Create: apps/mrd-service/tests/signaling_runtime.rs

**Step 1: Write failing reconnect and mapping tests**

Use a fake signaling server to prove:

- service authenticates using device key and backend token;
- heartbeat and exponential reconnect work;
- duplicate messages are idempotent;
- signed intent becomes an authorization request;
- grant/deny/close map to the correct session aggregate;
- disconnect does not silently authorize or close a valid local route.

**Step 2: Run the service test**

Run: cargo test -p mrd-service --test signaling_runtime

Expected: FAIL.

**Step 3: Add a service-owned signaling runtime**

Do not restore Rdesk Tauri ownership. Add SignalingPort use cases in mrd-application and event mapping into service commands. Expose health and reconnect state in runtime snapshots.

**Step 4: Run application/service tests**

Run:

~~~powershell
cargo test -p mrd-application
cargo test -p mrd-service --test signaling_runtime
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-application apps/mrd-service
git commit -m "feat: connect mrd-service to authenticated signaling"
~~~

### Task 30: Complete the service-owned WebRTC PeerConnection adapter

**Files:**
- Modify: crates/mrd-transport-webrtc/Cargo.toml
- Create: crates/mrd-transport-webrtc/src/peer.rs
- Create: crates/mrd-transport-webrtc/src/config.rs
- Create: crates/mrd-transport-webrtc/src/control.rs
- Create: crates/mrd-transport-webrtc/src/stats.rs
- Modify: crates/mrd-transport-webrtc/src/lib.rs
- Create: crates/mrd-transport-webrtc/tests/peer_connection.rs
- Create: apps/mrd-service/src/transports/webrtc.rs

**Step 1: Write failing loopback PeerConnection tests**

Test:

- offer/answer/candidate exchange;
- H.264 RTP access unit send/receive;
- reliable ordered ctrl_rel channel;
- unordered limited-retransmit ctrl_rt channel;
- selected candidate-pair stats;
- clean close releases tasks;
- unsupported codec/profile fails preflight.

**Step 2: Run transport tests**

Run: cargo test -p mrd-transport-webrtc --test peer_connection

Expected: FAIL because the crate currently provides media helpers rather than the full product PeerConnection adapter.

**Step 3: Implement the adapter**

Move reusable logic from the legacy harness only by reimplementing against the approved service boundary; do not restore legacy architecture. Make ICE server list, transport policy, codec, and channel configuration explicit.

**Step 4: Run WebRTC and service checks**

Run:

~~~powershell
cargo test -p mrd-transport-webrtc
cargo check -p mrd-service --features browser-webrtc-preview
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-transport-webrtc apps/mrd-service/src/transports
git commit -m "feat: add service owned webrtc transport"
~~~

### Task 31: Provision TURN credentials and prove forced relay

**Files:**
- Modify: apps/Rdesk-Server/app/core/config.py
- Create: apps/Rdesk-Server/app/services/turn_credentials.py
- Create: apps/Rdesk-Server/app/api/v1/turn.py
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_turn_credentials.py
- Create: deploy/turn/turnserver.conf.example
- Create: deploy/turn/README.md
- Modify: apps/mrd-service/src/transports/webrtc.rs
- Create: crates/mrd-transport-webrtc/tests/forced_relay.rs

**Step 1: Write failing credential and forced-relay tests**

Backend tests prove authenticated, short-lived, scoped TURN credentials and reject anonymous/expired requests. Transport tests configure relay-only policy and assert the selected candidate pair is relay, not host or server-reflexive.

**Step 2: Run tests**

Run:

~~~powershell
python -m pytest apps/Rdesk-Server/tests/test_turn_credentials.py -q
cargo test -p mrd-transport-webrtc --test forced_relay
~~~

Expected: FAIL.

**Step 3: Implement TURN integration**

Use a standard TURN deployment with UDP, TCP, and TLS listener configuration suitable for self-hosting. Generate short-lived credentials from a server-side secret; never ship static shared credentials in the client. Record relay URL class without logging credentials.

**Step 4: Run backend/transport tests and a local coturn smoke**

Run the same tests, then follow deploy/turn/README.md to run an isolated relay smoke.

Expected: tests PASS and route evidence reports relay.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk-Server deploy/turn crates/mrd-transport-webrtc apps/mrd-service
git commit -m "feat: add authenticated turn relay"
~~~

### Task 32: Introduce TransportMux and migrate LAN/WebRTC adapters

**Files:**
- Create: crates/mrd-application/src/ports/transport_mux.rs
- Modify: crates/mrd-application/src/lib.rs
- Create: apps/mrd-service/src/transports/mod.rs
- Create: apps/mrd-service/src/transports/quic.rs
- Modify: apps/mrd-service/src/transports/webrtc.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender.rs
- Modify: apps/mrd-service/src/lan_discovery/media_receiver.rs
- Create: apps/mrd-service/tests/transport_mux.rs

**Step 1: Write failing adapter-conformance tests**

Run the same suite against fake, QUIC loopback, and WebRTC loopback adapters:

- video send/receive;
- ctrl_rel ordering/reliability;
- ctrl_rt stale replacement;
- independent bulk stream;
- route stats/evidence;
- close and backpressure behavior.

**Step 2: Run the conformance test**

Run: cargo test -p mrd-service --test transport_mux

Expected: FAIL.

**Step 3: Implement the port and adapters**

The application-facing interface exposes logical lanes and route events only. Remove feature code that switches directly on Quinn/WebRTC concrete types. Keep existing QUIC media packetization behind the QUIC adapter.

**Step 4: Run transport and service suites**

Run:

~~~powershell
cargo test -p mrd-application
cargo test -p mrd-transport-quic-quinn
cargo test -p mrd-transport-webrtc
cargo test -p mrd-service
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-application apps/mrd-service
git commit -m "refactor: unify remote transport lanes"
~~~

### Task 33: Add route planning and authenticated candidate racing

**Files:**
- Create: crates/mrd-session/src/route_policy.rs
- Modify: crates/mrd-session/src/route.rs
- Create: crates/mrd-application/src/usecases/plan_route.rs
- Create: crates/mrd-application/src/usecases/connect_route.rs
- Modify: apps/mrd-service/src/handlers/control.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-service/src/capabilities.rs
- Create: apps/mrd-service/tests/route_planner.rs

**Step 1: Write failing route-policy tests**

Cover:

- authenticated LAN QUIC preferred when valid;
- unsigned/spoofed LAN never selected;
- public direct chosen when LAN fails;
- relay chosen when direct fails or policy requires relay;
- required relay rejects non-relay evidence;
- security errors never trigger insecure fallback;
- route choice records reason and candidate evidence.

**Step 2: Run route tests**

Run: cargo test -p mrd-service --test route_planner

Expected: FAIL.

**Step 3: Implement the route planner**

Gather LAN and ICE candidates in parallel after authentication. Start no media until grant. Bound candidate racing and fallback timers by policy. Expose auto, lan, wan, relay-only, and diagnostic intents without allowing UI to bypass trust or capability checks.

**Step 4: Run route and service tests**

Run:

~~~powershell
cargo test -p mrd-session route_policy
cargo test -p mrd-application plan_route
cargo test -p mrd-service --test route_planner
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session crates/mrd-application apps/mrd-service
git commit -m "feat: select authenticated remote routes"
~~~

### Task 34: Implement real reconnect, ICE restart, and direct-to-relay migration

**Files:**
- Modify: crates/mrd-session/src/remote_session.rs
- Modify: crates/mrd-session/src/route.rs
- Create: crates/mrd-application/src/usecases/reconnect.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-service/src/transports/webrtc.rs
- Modify: apps/mrd-service/src/transports/quic.rs
- Create: apps/mrd-service/tests/route_recovery.rs

**Step 1: Write failing recovery tests**

Prove:

- current RecoverSession state-reset behavior is insufficient;
- three-second outage triggers detection, pressed-input release, reconnect, and first recovered present;
- WebRTC direct failure performs ICE restart and selects relay;
- LAN QUIC failure falls back to WebRTC under the same valid grant;
- expired lease or changed policy forces new authorization;
- duplicate reconnect messages are idempotent.

**Step 2: Run recovery tests**

Run: cargo test -p mrd-service --test route_recovery

Expected: FAIL.

**Step 3: Implement reconnect orchestration**

Replace state-only recovery with transport actions, bounded exponential retry, route migration, media restart at a keyframe boundary, queue reset, and explicit recovery telemetry.

**Step 4: Run recovery and fault suites**

Run:

~~~powershell
cargo test -p mrd-service --test route_recovery
cargo test -p mrd-service cross_e2e
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session crates/mrd-application apps/mrd-service
git commit -m "feat: recover and migrate remote routes"
~~~

### Task 35: Add public direct, forced-relay, UDP-blocked, and outage device-lab gates

**Files:**
- Create: tests/quality-gates/policies/windows-1080p60-direct.v1.json
- Create: tests/quality-gates/policies/windows-1080p60-relay.v1.json
- Create: tests/quality-gates/policies/windows-route-recovery.v1.json
- Create: tests/benchmarks/scripts/run_public_route_canary.ps1
- Create: tests/benchmarks/scripts/test_public_route_canary.ps1
- Create: .github/workflows/windows-device-lab.yml
- Modify: .github/workflows/mainline-e2e.yml

**Step 1: Write failing script-contract tests**

Assert:

- direct policy requires direct selected-pair evidence;
- relay policy requires relay evidence;
- UDP-blocked scenario cannot pass with policy-only metadata;
- outage scenario measures detection and first recovered present;
- all routes preserve signed grant and granted scopes;
- cleanup and artifact validity control the process exit.

**Step 2: Run helper tests**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_public_route_canary.ps1

Expected: FAIL.

**Step 3: Implement device-lab orchestration**

Use two independently addressed Windows peers, controlled NAT/firewall profiles, and the configured TURN service. Collect route evidence, visible first frame, input probes, resources, recovery, and audit identifiers into v2 artifacts.

**Step 4: Run each lab route**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_public_route_canary.ps1 -Route direct -Attempts 10
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_public_route_canary.ps1 -Route relay -Attempts 10
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_public_route_canary.ps1 -Route udp-blocked -Attempts 10
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_public_route_canary.ps1 -Route recovery -Attempts 10
~~~

Expected: every configured lab lane produces honest artifacts; unsupported lab configuration is INFRA_FAIL, not product PASS.

**Step 5: Commit**

~~~powershell
git add tests/quality-gates tests/benchmarks .github/workflows
git commit -m "test: gate public direct and relay routes"
~~~

## P0 Windows Market-Core Feature Completion

### Task 36: Add remote Windows system audio over both transports

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-audio/Cargo.toml
- Create: crates/mrd-audio/src/lib.rs
- Create: crates/mrd-audio/src/frame.rs
- Create: crates/mrd-audio/src/codec.rs
- Create: crates/mrd-audio/src/windows.rs
- Create: crates/mrd-audio/src/unsupported.rs
- Create: crates/mrd-audio/tests/audio_pipeline.rs
- Create: apps/mrd-session-agent/src/audio.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: apps/mrd-service/src/capabilities.rs
- Modify: apps/mrd-service/src/transports/quic.rs
- Modify: apps/mrd-service/src/transports/webrtc.rs
- Modify: crates/mrd-ipc/src/lib.rs

**Step 1: Write failing deterministic audio tests**

Use a synthetic PCM source and memory sink to prove:

- Opus encode/decode preserves timing and channel layout;
- audio.listen scope is required;
- mute stops playback without stopping video;
- audio route works through both TransportMux adapters;
- loss/jitter buffer behavior is bounded;
- audio failure produces feature-degraded state, not silent success.

**Step 2: Run audio tests**

Run: cargo test -p mrd-audio

Expected: FAIL because the crate is absent.

**Step 3: Implement WASAPI loopback and playback**

Add Windows WASAPI loopback capture in the target session agent, Opus framing, transport timestamps, controller playback, mute/volume, and synchronization metrics. Keep microphone talk disabled until its separate permission and privacy design is implemented.

**Step 4: Run audio, service, and device smoke**

Run:

~~~powershell
cargo test -p mrd-audio
cargo test -p mrd-session-agent audio
cargo test -p mrd-service audio
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_public_route_canary.ps1 -Route relay -Scenario audio-1080p60 -Attempts 3
~~~

Expected: PASS with non-null audio metrics and audible verification artifact policy.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-audio apps/mrd-session-agent apps/mrd-service crates/mrd-ipc
git commit -m "feat: stream remote system audio"
~~~

### Task 37: Add permissioned bidirectional text clipboard

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-clipboard/Cargo.toml
- Create: crates/mrd-clipboard/src/lib.rs
- Create: crates/mrd-clipboard/src/protocol.rs
- Create: crates/mrd-clipboard/src/windows.rs
- Create: crates/mrd-clipboard/src/unsupported.rs
- Create: crates/mrd-clipboard/tests/synchronization.rs
- Create: apps/mrd-session-agent/src/clipboard.rs
- Modify: apps/mrd-service/src/transports/mod.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx

**Step 1: Write failing clipboard tests**

Cover:

- read/write directions enforced independently;
- loop prevention using origin/update ID;
- repeated same content does not bounce;
- payload size and update-rate limits;
- unsupported MIME rejected;
- content absent from Debug, logs, audit, and telemetry;
- revoke immediately stops synchronization.

**Step 2: Run clipboard tests**

Run: cargo test -p mrd-clipboard

Expected: FAIL.

**Step 3: Implement text clipboard on ctrl_rel**

Use the session agent for OS clipboard access and mrd-service for permission/transport policy. Start with UTF-8 text only. Advertise file/image clipboard as unimplemented until P1.

**Step 4: Run clipboard and UI tests**

Run:

~~~powershell
cargo test -p mrd-clipboard
cargo test -p mrd-session-agent clipboard
cargo test -p mrd-service clipboard
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-clipboard apps/mrd-session-agent apps/mrd-service crates/mrd-ipc apps/Rdesk/src/app/components
git commit -m "feat: synchronize remote text clipboard"
~~~

### Task 38: Replace local copy with resumable remote file transfer

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-file-transfer/Cargo.toml
- Create: crates/mrd-file-transfer/src/lib.rs
- Create: crates/mrd-file-transfer/src/protocol.rs
- Create: crates/mrd-file-transfer/src/chunking.rs
- Create: crates/mrd-file-transfer/src/paths.rs
- Create: crates/mrd-file-transfer/src/resume.rs
- Create: crates/mrd-file-transfer/tests/transfer_protocol.rs
- Create: apps/mrd-session-agent/src/files.rs
- Modify: apps/mrd-service/src/handlers/files.rs
- Modify: apps/mrd-service/src/app_state/file_transfer_registry.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/adapters/tauri/commands.ts
- Modify: apps/Rdesk/src/app/components/DeviceDetailPage.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/remote_file_transfer.rs

**Step 1: Write failing protocol and security tests**

Cover:

- authorized remote directory listing;
- file upload/download over file_bulk;
- file.read/file.write directions;
- traversal, alternate stream, reserved-name, symlink/reparse, and destination escape rejection;
- per-chunk and final hash verification;
- temporary destination and atomic completion;
- cancel, disconnect, resume, overwrite policy, and cleanup;
- large transfer does not block ctrl_rel or ctrl_rt.

**Step 2: Run crate and integration tests**

Run:

~~~powershell
cargo test -p mrd-file-transfer
cargo test --manifest-path tests/integration/Cargo.toml --test remote_file_transfer
~~~

Expected: FAIL because existing file copy is service-local.

**Step 3: Implement the peer provider**

Introduce an mrd-remote provider backed by file_bulk and the target session agent. Keep mrd-local for explicit local administrative use only; never silently satisfy a remote request with local copy. Retain the external provider reservation as unavailable until implemented.

**Step 4: Run file, service, UI, and transport tests**

Run:

~~~powershell
cargo test -p mrd-file-transfer
cargo test -p mrd-service files
cargo test --manifest-path tests/integration/Cargo.toml --test remote_file_transfer
pnpm --dir apps/Rdesk test -- --run src/app/adapters/tauri/commands.fileDirectory.test.ts src/app/components/DeviceDetailPage.test.ts
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-file-transfer apps/mrd-session-agent apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: transfer remote files with resume"
~~~

### Task 39: Complete monitor switching, cursor, and coordinate correctness

**Files:**
- Modify: apps/mrd-service/src/display_mode.rs
- Modify: apps/mrd-service/src/capture_source.rs
- Modify: apps/mrd-service/src/lan_discovery/capture_sources.rs
- Modify: apps/mrd-service/src/lan_discovery/lan_control_input.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-session-agent/src/capture.rs
- Modify: apps/mrd-session-agent/src/input.rs
- Create: apps/mrd-session-agent/src/cursor.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/monitor_switching.rs

**Step 1: Write failing monitor/input tests**

Test:

- sparse monitor indices and negative virtual-desktop origins;
- scaling and aspect mapping;
- portrait/landscape and DPI changes;
- switching source without new trust decision or session ID;
- pressed-state release during switch;
- cursor position/shape update;
- disconnect restores temporary display mode;
- simultaneous multi-view remains explicitly unavailable in P0.

**Step 2: Run integration and frontend tests**

Run:

~~~powershell
cargo test --manifest-path tests/integration/Cargo.toml --test monitor_switching
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx
~~~

Expected: FAIL on the new guarantees.

**Step 3: Implement an atomic display-switch use case**

Pause input, select source, reconcile profile, wait for first present, update geometry/cursor mapping, then resume allowed control. Roll back and report a stable error if any step fails.

**Step 4: Run tests and paired multi-monitor smoke**

Run the same tests, then:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_paired_lan_canary.ps1 -TargetDeviceId $env:MRD_DEVICE_LAB_TARGET_DEVICE_ID -ScenarioId cross.e2e.monitor_switch -ProfileId 1080p60
~~~

Expected: PASS with route/session/grant unchanged and monitor evidence updated.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service apps/mrd-session-agent crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: complete remote monitor switching"
~~~

### Task 40: Complete unattended policy, WOL, power, and privacy controls

**Files:**
- Create: apps/mrd-service/src/policy/mod.rs
- Create: apps/mrd-service/src/policy/unattended.rs
- Create: apps/mrd-service/src/policy/privacy.rs
- Modify: apps/mrd-service/src/lan_discovery/remote_power.rs
- Modify: apps/mrd-service/src/wake_on_lan.rs
- Modify: apps/mrd-service/src/handlers/device.rs
- Modify: apps/mrd-service/src/handlers/identity.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/DeviceDetailPage.tsx
- Create: apps/Rdesk/src/app/components/UnattendedAccessSettings.tsx
- Create: apps/Rdesk/src/app/components/UnattendedAccessSettings.test.tsx
- Create: apps/mrd-service/tests/unattended_policy.rs

**Step 1: Write failing policy tests**

Cover:

- unattended disabled by default;
- generated credential displayed once and not returned again;
- trusted peer allowlist and scope profile;
- failure backoff and lockout persistence;
- rotation/revocation invalidates old proof;
- WOL can precede connection but grants no trust;
- restart/shutdown require explicit scope and target policy;
- session indicator and end-lock policy;
- blank screen/block local input reported unavailable unless safely supported.

**Step 2: Run service and UI tests**

Run:

~~~powershell
cargo test -p mrd-service --test unattended_policy
pnpm --dir apps/Rdesk test -- --run src/app/components/UnattendedAccessSettings.test.tsx
~~~

Expected: FAIL.

**Step 3: Implement policy and UI**

Remove UI-local access-password authority. Store only protected service-side credential material and non-secret policy. Require explicit confirmation for high-risk privacy and power settings.

**Step 4: Run service/frontend/security tests**

Run:

~~~powershell
cargo test -p mrd-service unattended
pnpm --dir apps/Rdesk test -- --run src/app/components/UnattendedAccessSettings.test.tsx src/app/components/DeviceDetailPage.test.ts
pnpm --dir apps/Rdesk type-check
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-ipc apps/Rdesk
git commit -m "feat: enforce unattended and power policy"
~~~

### Task 41: Implement the narrow secure-desktop broker core

**Files:**
- Modify: Cargo.toml
- Create: apps/mrd-secure-desktop-broker/Cargo.toml
- Create: apps/mrd-secure-desktop-broker/src/main.rs
- Create: apps/mrd-secure-desktop-broker/src/protocol.rs
- Create: apps/mrd-secure-desktop-broker/src/windows_desktop.rs
- Create: apps/mrd-secure-desktop-broker/src/windows_pipe.rs
- Create: apps/mrd-secure-desktop-broker/src/grant.rs
- Create: apps/mrd-secure-desktop-broker/tests/grant_contract.rs
- Create: apps/mrd-secure-desktop-broker/tests/pipe_authorization.rs
- Create: docs/security/secure-desktop-broker-threat-model.md
- Modify: apps/mrd-service/src/agent_runtime/mod.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/secure_desktop.rs

**Step 1: Write the threat model and failing broker tests**

Document assets, attackers, process privileges, IPC ACL, confused-deputy risks, downgrade risks, audit, and kill-switch behavior. Tests prove:

- no network listener;
- only the expected machine-service SID, process image, Windows session, and authenticated pipe token can connect;
- grant binds session, peer, scopes, target Windows session/desktop, expiry, and nonce;
- wrong desktop or replay rejected;
- broker has no identity/unattended secret access;
- input released on desktop transition or grant expiry.

**Step 2: Run broker tests**

Run: cargo test -p mrd-secure-desktop-broker

Expected: FAIL because the broker is absent.

**Step 3: Implement the narrow broker core**

Implement only reviewed secure-desktop capture/input operations over a private Windows named pipe with an explicit DACL, peer-token inspection, bounded messages, and one-use grants. Do not disable secure desktop, install a credential provider, capture credential text, or expose a generic privileged command channel.

**Step 4: Run integration and manual Windows matrix**

Run:

~~~powershell
cargo test -p mrd-secure-desktop-broker
cargo test --manifest-path tests/integration/Cargo.toml --test secure_desktop
~~~

Then execute the documented device-lab cases for UAC, lock/unlock, logon, user switch, revoke, and service restart.

Expected: automated tests PASS and every manual/device case emits complete redacted audit evidence.

**Step 5: Commit**

~~~powershell
git add Cargo.toml apps/mrd-secure-desktop-broker apps/mrd-service apps/mrd-session-agent crates/mrd-ipc tests/integration docs/security
git commit -m "feat: add authorized secure desktop broker core"
~~~

### Task 42: Protect, sign, install, and independently review the secure-desktop broker

**Files:**
- Create: apps/mrd-secure-desktop-broker/build.rs
- Modify: apps/mrd-secure-desktop-broker/src/main.rs
- Modify: apps/mrd-secure-desktop-broker/src/windows_pipe.rs
- Create: apps/Rdesk/scripts/install-secure-desktop-broker.ps1
- Create: apps/Rdesk/scripts/uninstall-secure-desktop-broker.ps1
- Create: apps/Rdesk/scripts/verify-secure-desktop-broker.ps1
- Create: tests/windows-security/secure_desktop_broker.ps1
- Create: tests/windows-security/fixtures/wrong-signer.cer
- Create: .github/workflows/windows-security.yml
- Create: docs/security/secure-desktop-broker-security-review.md
- Create: docs/release/secure-desktop-broker-signing.md

**Step 1: Write failing installation and security tests**

The PowerShell security suite must fail unless it proves:

- the release PE has a valid Authenticode signature chaining to the configured production publisher and its hash matches the signed install manifest;
- installation rejects unsigned, wrong-publisher, downgraded, or tampered binaries;
- executable, directory, service, registry, and pipe DACLs deny modification or connection by standard users;
- the process starts under the reviewed least-privileged service identity with only the documented token privileges;
- the named-pipe server validates owner SID, client SID, process image, session ID, integrity level, and grant before impersonating;
- no network socket, shell launch, arbitrary file access, credential access, or generic privileged RPC surface exists;
- crash, upgrade, rollback, and uninstall remove stale pipe state and revoke active grants;
- the independent security-review checklist names a reviewer other than the implementer and records zero unresolved critical/high findings.

**Step 2: Run the Windows security suite**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/windows-security/secure_desktop_broker.ps1 -BrokerPath target/release/mrd-secure-desktop-broker.exe -ExpectedPublisher $env:MRD_WINDOWS_RELEASE_PUBLISHER
~~~

Expected: FAIL because protected installation, release signing verification, ACL enforcement, and review evidence are absent.

**Step 3: Implement protected packaging and least privilege**

Add versioned, atomic install/upgrade/uninstall scripts; fail-closed Authenticode and manifest verification; explicit service and filesystem security descriptors; a restricted service token; pipe client token/process/session checks; and build metadata consumed by the verifier. The development certificate may exercise tests, but no production gate may accept it.

**Step 4: Run packaging, negative, and independent review gates**

Run on a clean Windows VM from an elevated release-job context:

~~~powershell
cargo build -p mrd-secure-desktop-broker --release
powershell -ExecutionPolicy Bypass -File apps/Rdesk/scripts/install-secure-desktop-broker.ps1 -BrokerPath target/release/mrd-secure-desktop-broker.exe -ExpectedPublisher $env:MRD_WINDOWS_RELEASE_PUBLISHER
powershell -ExecutionPolicy Bypass -File apps/Rdesk/scripts/verify-secure-desktop-broker.ps1 -ExpectedPublisher $env:MRD_WINDOWS_RELEASE_PUBLISHER
powershell -ExecutionPolicy Bypass -File tests/windows-security/secure_desktop_broker.ps1 -BrokerPath target/release/mrd-secure-desktop-broker.exe -ExpectedPublisher $env:MRD_WINDOWS_RELEASE_PUBLISHER
~~~

Expected: PASS only for a correctly signed artifact installed with reviewed identity, privileges, and ACLs. The workflow uploads the signed manifest, token/ACL evidence, negative-test results, and completed independent review.

**Step 5: Commit**

~~~powershell
git add apps/mrd-secure-desktop-broker apps/Rdesk/scripts tests/windows-security .github/workflows/windows-security.yml docs/security docs/release
git commit -m "security: harden secure desktop broker delivery"
~~~

### Task 43: Measure true present, input-to-photon, resources, freezes, and adaptation

**Files:**
- Modify: apps/Rdesk/src-tauri/src/benchmark.rs
- Modify: crates/mrd-render-d3d11/src/lib.rs
- Modify: apps/mrd-service/src/resource_monitor.rs
- Modify: apps/mrd-service/src/control_input.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender_telemetry.rs
- Modify: apps/mrd-service/src/lan_discovery/media_render_worker.rs
- Modify: apps/mrd-service/src/media_adaptation.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/services/lanE2eTelemetryService.ts
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/experience_probes.rs

**Step 1: Write failing measurement tests**

Prove:

- visible first frame is controller request to successful present, not first decode;
- input probe ID triggers a deterministic target visual marker and is acknowledged after present;
- one-second FPS windows, frame intervals, stalls, and W3C-style freeze metrics are populated;
- sender/receiver CPU, GPU, RSS, VRAM samples are side-specific and finite;
- adaptation transition time and stall are recorded;
- no cross-machine wall-clock assumption is required for input-to-photon.

**Step 2: Run measurement tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test experience_probes

Expected: FAIL.

**Step 3: Implement canonical probes**

Use monotonic controller timing for the round-trip input marker. Emit present callbacks from the real D3D11 path. Add bounded time series and resource sampling to the v2 artifact.

**Step 4: Run integration and 1080p60 local baseline**

Run:

~~~powershell
cargo test --manifest-path tests/integration/Cargo.toml --test experience_probes
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p60 -DurationSecs 600
~~~

Expected: PASS with every P0 required metric finite and non-empty.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk/src-tauri crates/mrd-render-d3d11 apps/mrd-service crates/mrd-ipc apps/Rdesk/src/app/services tests/integration
git commit -m "feat: measure end to end remote experience"
~~~

### Task 44: Make session restart, concurrency, and failure behavior product-ready

**Files:**
- Modify: crates/mrd-session/src/scheduler.rs
- Modify: crates/mrd-session/src/lib.rs
- Create: crates/mrd-session/tests/scheduler.rs
- Create: crates/mrd-application/src/usecases/preflight.rs
- Create: crates/mrd-application/src/usecases/recover.rs
- Modify: crates/mrd-application/src/lib.rs
- Create: crates/mrd-application/tests/preflight.rs
- Create: crates/mrd-application/tests/recovery.rs
- Modify: apps/mrd-service/src/handlers/preflight.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: apps/mrd-service/src/handlers/shell.rs
- Modify: apps/mrd-service/src/main.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/services/ipcSessionService.ts
- Modify: apps/Rdesk/src/app/services/remoteDisplayLauncher.ts
- Create: apps/Rdesk/src/app/services/sessionRecovery.test.ts
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/ui_service_restart.rs
- Create: tests/integration/session_resource_policy.rs

**Step 1: Write failing lifecycle, resource, and preflight tests**

Prove:

- closing and reopening only the Rdesk UI preserves the service-owned session and rebinds the correct display window;
- a service restart either resumes a session under a valid persisted lease or closes it cleanly with a stable reason and released input/media resources;
- simultaneous sessions on one device have isolated grants, input ownership, queues, metrics, and cleanup;
- configured CPU/GPU/encoder/session limits reject or downgrade new work deterministically without degrading an existing session silently;
- duplicate start/reconnect requests are idempotent and duplicate session IDs cannot alias resources;
- preflight results contain stable code, human-readable cause, actionable remediation, retryability, and any safe downgrade for permission, codec, network, relay, display, audio, and service-state failures.

**Step 2: Run the focused tests**

Run:

~~~powershell
cargo test -p mrd-session --test scheduler
cargo test -p mrd-application --test preflight --test recovery
cargo test --manifest-path tests/integration/Cargo.toml --test ui_service_restart --test session_resource_policy
pnpm --dir apps/Rdesk test -- --run src/app/services/sessionRecovery.test.ts
~~~

Expected: FAIL because shell reattachment, recovery leases, resource scheduling, and actionable preflight results are incomplete.

**Step 3: Implement service-owned recovery and explicit admission control**

Make the service the durable owner of active sessions. Persist only bounded recovery metadata, require fresh peer/grant validation before resume, and expose shell attach/detach over IPC. Add per-device admission control and typed preflight/failure DTOs; never pretend a rejected capability started successfully.

**Step 4: Run lifecycle and crash-recovery matrices**

Run the focused tests from Step 2, then execute device-lab cases for UI crash/restart, service crash/restart, two concurrent controller sessions, encoder exhaustion, display removal, permission denial, and forced relay failure.

Expected: automated tests PASS; every lab case ends in a resumed healthy session or a clean, actionable terminal state with no stuck input, orphan process, leaked grant, or unbounded resource growth.

**Step 5: Commit**

~~~powershell
git add crates/mrd-session crates/mrd-application apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: harden remote session product lifecycle"
~~~

### Task 45: Establish the complete Windows P0 release gate

**Files:**
- Create: tests/quality-gates/policies/windows-p0-market-core.v1.json
- Create: tests/benchmarks/scripts/run_windows_p0_matrix.ps1
- Create: tests/benchmarks/scripts/test_windows_p0_matrix.ps1
- Create: .github/workflows/windows-soak.yml
- Create: .github/workflows/security-negative.yml
- Modify: .github/workflows/windows-device-lab.yml
- Create: docs/release/windows-p0-acceptance.md

**Step 1: Write failing matrix-contract tests**

Require all P0 capabilities and SLO rows:

- attended and unattended;
- LAN, direct, forced relay, UDP blocked;
- 1080p60 video/input/audio;
- clipboard, remote file, monitor switch;
- WOL/power, UAC/secure desktop;
- outage/reconnect/migration;
- moderate/harsh weak network;
- security-negative;
- UI close/reopen and service crash/restart recovery;
- same-device simultaneous sessions and resource admission limits;
- permission, codec, network, relay, display, and audio preflight failures with actionable remediation;
- eight-hour direct and relay soak;
- resource-growth limits.

No required row may skip or downgrade.

**Step 2: Run the matrix-contract test**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_windows_p0_matrix.ps1

Expected: FAIL until all scenarios and policies are wired.

**Step 3: Add the release orchestration**

Aggregate v2 artifacts without averaging away failed attempts. Require rolling connection sample size and preserve route-specific percentiles. Upload all artifacts before final enforcement.

**Step 4: Run the P0 release matrix**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_windows_p0_matrix.ps1 -ControllerDeviceId $env:MRD_P0_CONTROLLER_DEVICE_ID -TargetDeviceId $env:MRD_P0_TARGET_DEVICE_ID -TurnProfile $env:MRD_TURN_PROFILE
~~~

Expected: PASS only when every design P0 requirement and SLO is proven. Otherwise retain the goal and milestone as incomplete.

**Step 5: Commit**

~~~powershell
git add tests/quality-gates tests/benchmarks .github/workflows docs/release
git commit -m "test: define windows p0 market release gate"
~~~

## P1 Mainstream Cross-Platform Parity

### Task 46: Make capability tiers platform- and route-specific

**Files:**
- Modify: apps/mrd-service/src/capabilities.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/services/capabilityMatrix.ts
- Modify: apps/Rdesk/src/app/services/capabilityMatrix.test.ts
- Create: tests/quality-gates/policies/platform-capability-truth.v1.json
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/platform_capabilities.rs

**Step 1: Write failing truthfulness tests**

Assert that every capability has:

- platform;
- role;
- route support;
- implementation status;
- runtime availability;
- permission scope;
- required product gate and last evidence ID.

Mac/Linux must remain partial until their own media/input/feature matrices pass. Planned, protocol-only, or local-only work must not appear available.

**Step 2: Run capability tests**

Run:

~~~powershell
cargo test --manifest-path tests/integration/Cargo.toml --test platform_capabilities
pnpm --dir apps/Rdesk test -- --run src/app/services/capabilityMatrix.test.ts
~~~

Expected: FAIL.

**Step 3: Implement tiered capability evidence**

Add P0/P1/P2 tier and evidence metadata. Generate UI labels from service truth rather than hard-coded feature expectations.

**Step 4: Run service/frontend tests**

Run:

~~~powershell
cargo test -p mrd-service capabilities
pnpm --dir apps/Rdesk test -- --run src/app/services/capabilityMatrix.test.ts
pnpm --dir apps/Rdesk type-check
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-ipc apps/Rdesk tests/quality-gates tests/integration
git commit -m "feat: report evidence backed capability tiers"
~~~

### Task 47: Productize the macOS service and per-user session agent

**Files:**
- Modify: apps/mrd-session-agent/Cargo.toml
- Create: apps/mrd-session-agent/src/platform/mod.rs
- Create: apps/mrd-session-agent/src/platform/macos.rs
- Create: apps/mrd-session-agent/src/platform/macos/lifecycle.rs
- Create: apps/mrd-session-agent/src/platform/macos/permissions.rs
- Create: apps/mrd-session-agent/src/platform/macos/private_ipc.rs
- Create: apps/mrd-session-agent/src/platform/macos/secure_storage.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: crates/mrd-capture-macos/src/lib.rs
- Modify: crates/mrd-render-macos/src/lib.rs
- Modify: crates/mrd-input/src/lib.rs
- Create: crates/mrd-input/src/macos.rs
- Create: packaging/macos/com.mrd.service.plist
- Create: packaging/macos/com.mrd.session-agent.plist
- Create: packaging/macos/install.sh
- Create: packaging/macos/uninstall.sh
- Create: tests/device-lab/macos/run_agent_lifecycle.sh
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/macos_agent_productization.rs

**Step 1: Write failing macOS lifecycle and boundary tests**

Require capture, input, audio, clipboard, file access, monitor enumeration, lock/session events, and render conformance plus:

- launchd starts the machine service at boot and one least-privileged agent in the active GUI login session;
- unattended policy survives reboot without storing trust or access secrets in plist files;
- Keychain items use the designated service identity and are unavailable to unrelated users/processes;
- private Unix-domain IPC checks owner, mode, peer credentials, protocol version, session ID, and grant;
- Screen Recording, Accessibility, Input Monitoring, microphone, and file permissions surface stable remediation and never appear granted prematurely;
- login, logout, fast-user switch, sleep/wake, agent crash, service crash, upgrade, and uninstall terminate or rebind sessions cleanly.

**Step 2: Run macOS contract tests**

Run on the macOS CI/device host:

~~~bash
cargo test -p mrd-session-agent macos
cargo test --manifest-path tests/integration/Cargo.toml --test macos_agent_productization
bash tests/device-lab/macos/run_agent_lifecycle.sh
~~~

Expected: FAIL because launchd lifecycle, secure storage, private IPC, and permission supervision are incomplete.

**Step 3: Implement the macOS product boundary**

Use ScreenCaptureKit/Metal and reviewed native input/audio/clipboard adapters. Install a signed machine service plus per-user launch agent, keep secrets in Keychain, validate private-IPC peer credentials, and bind media/input to the current console user and grant.

**Step 4: Run the macOS device matrix**

Run the commands from Step 2 plus signed install/upgrade/uninstall, attended/unattended, permission-denied, fast-user-switch, sleep/wake, direct/relay, and eight-hour soak cases.

Expected: PASS only for capability rows supported by signed product artifacts; all unsupported rows remain unavailable with actionable reasons.

**Step 5: Commit**

~~~powershell
git add apps/mrd-session-agent crates/mrd-capture-macos crates/mrd-render-macos crates/mrd-input packaging/macos tests/device-lab/macos tests/integration
git commit -m "feat: productize macos remote agent"
~~~

### Task 48: Productize the Linux service and per-user session agent

**Files:**
- Modify: apps/mrd-session-agent/Cargo.toml
- Create: apps/mrd-session-agent/src/platform/linux.rs
- Create: apps/mrd-session-agent/src/platform/linux/lifecycle.rs
- Create: apps/mrd-session-agent/src/platform/linux/permissions.rs
- Create: apps/mrd-session-agent/src/platform/linux/private_ipc.rs
- Create: apps/mrd-session-agent/src/platform/linux/secure_storage.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: crates/mrd-capture-pipewire/src/lib.rs
- Modify: crates/mrd-render-linux/src/lib.rs
- Modify: crates/mrd-input/src/lib.rs
- Create: crates/mrd-input/src/linux.rs
- Create: packaging/linux/mrd-service.service
- Create: packaging/linux/mrd-session-agent.service
- Create: packaging/linux/mrd-session-agent.socket
- Create: packaging/linux/install.sh
- Create: packaging/linux/uninstall.sh
- Create: tests/device-lab/linux/run_agent_lifecycle.sh
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/linux_agent_productization.rs

**Step 1: Write failing Linux lifecycle and boundary tests**

Require media/input/clipboard/file/display conformance on declared Wayland and X11 combinations plus:

- systemd starts the machine service at boot and supervises one user agent per active logind graphical session;
- unattended policy survives reboot while libsecret/kernel-keyring material is inaccessible to unrelated UIDs;
- the private Unix socket has explicit owner/mode/SELinux-or-AppArmor labeling and validates SO_PEERCRED, session, protocol, and grant;
- PipeWire/portal, input, audio, clipboard, and file permission denial returns stable remediation;
- login/logout, seat switch, display-server restart, suspend/resume, agent/service crash, package upgrade, and uninstall clean up every resource.

**Step 2: Run Linux contract tests**

Run on Ubuntu Wayland and X11 device hosts:

~~~bash
cargo test -p mrd-session-agent linux
cargo test --manifest-path tests/integration/Cargo.toml --test linux_agent_productization
bash tests/device-lab/linux/run_agent_lifecycle.sh
~~~

Expected: FAIL because systemd/logind lifecycle, secure storage, private IPC, and permission supervision are incomplete.

**Step 3: Implement the Linux product boundary**

Use PipeWire/xdg-desktop-portal for Wayland and reviewed X11 fallbacks. Supervise service and per-session agents with systemd/logind, protect secrets with native storage, validate Unix peer credentials and LSM labels, and keep advertised capabilities compositor-specific.

**Step 4: Run the Linux device matrix**

Run the commands from Step 2 plus signed package install/upgrade/uninstall, attended/unattended, permission-denied, user/seat switch, suspend/resume, direct/relay, and eight-hour soak cases.

Expected: PASS only for explicitly tested distribution, compositor, route, and capability rows.

**Step 5: Commit**

~~~powershell
git add apps/mrd-session-agent crates/mrd-capture-pipewire crates/mrd-render-linux crates/mrd-input packaging/linux tests/device-lab/linux tests/integration
git commit -m "feat: productize linux remote agent"
~~~

### Task 49: Add the Web viewer/controller client

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-client-proto/Cargo.toml
- Create: crates/mrd-client-proto/src/lib.rs
- Create: apps/Rdesk-Web/package.json
- Create: apps/Rdesk-Web/src/main.tsx
- Create: apps/Rdesk-Web/src/session/client.ts
- Create: apps/Rdesk-Web/src/session/permissions.ts
- Create: apps/Rdesk-Web/src/session/renderer.ts
- Create: apps/Rdesk-Web/src/session/input.ts
- Create: apps/Rdesk-Web/src/session/client.test.ts
- Create: apps/Rdesk-Web/src/session/security.test.ts
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_web_session_bootstrap.py

**Step 1: Write failing browser security and session tests**

Prove authenticated bootstrap, origin/CSRF policy, signed scoped grants, WebRTC-only transport, permission display, keyboard/pointer input, clipboard policy, route/codec downgrade, denial, reconnect, and clean close. Browser code cannot access LAN QUIC, machine identity secrets, unattended credentials, arbitrary TURN credentials, or unapproved origins.

**Step 2: Run Web and server tests**

Run:

~~~text
pnpm --dir apps/Rdesk-Web test
python -m pytest apps/Rdesk-Server/tests/test_web_session_bootstrap.py -q
~~~

Expected: FAIL because the Web client and browser bootstrap contract are absent.

**Step 3: Implement the browser vertical slice**

Use standard browser WebRTC and the shared mrd-client-proto DTOs. Use Canvas/WebCodecs only when runtime capability proves support, keep secrets server-side, display route/permission state, and never claim native high-performance parity.

**Step 4: Run browser compatibility and build gates**

Run the focused tests, production build, and Playwright lanes for current stable Chromium, Firefox, and Safari against direct and forced-TURN sessions.

Expected: PASS with truthful codec/feature downgrades and no browser-accessible machine secret.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-client-proto apps/Rdesk-Web apps/Rdesk-Server
git commit -m "feat: add secure web remote client"
~~~

### Task 50: Add Android and iOS controller clients

**Files:**
- Create: apps/Rdesk-Mobile/README.md
- Create: apps/Rdesk-Mobile/package.json
- Create: apps/Rdesk-Mobile/src/main.tsx
- Create: apps/Rdesk-Mobile/src/session/client.ts
- Create: apps/Rdesk-Mobile/src/session/gestures.ts
- Create: apps/Rdesk-Mobile/src/session/permissions.ts
- Create: apps/Rdesk-Mobile/src/session/client.test.ts
- Create: apps/Rdesk-Mobile/src-tauri/Cargo.toml
- Create: apps/Rdesk-Mobile/src-tauri/src/lib.rs
- Create: apps/Rdesk-Mobile/src-tauri/tauri.conf.json
- Create: apps/Rdesk-Mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml
- Create: apps/Rdesk-Mobile/src-tauri/gen/apple/RdeskMobile/Info.plist
- Create: tests/device-lab/mobile/run_controller_matrix.md

**Step 1: Write failing mobile-controller contract tests**

Run the shared controller contract for authenticated signaling, signed grants, WebRTC direct/relay, touch-to-pointer mapping, keyboard/IME, clipboard policy, audio, orientation, background/foreground, permission denial, reconnect, and clean close. Define Android/iOS OS-version and feature rows; P1 mobile remains controller-only.

**Step 2: Run mobile tests**

Run:

~~~text
pnpm --dir apps/Rdesk-Mobile test
pnpm --dir apps/Rdesk-Mobile tauri android build --debug
pnpm --dir apps/Rdesk-Mobile tauri ios build --debug
~~~

Expected: FAIL because native shells, permissions, and controller lifecycle are absent.

**Step 3: Implement the Tauri mobile controller**

Use mrd-client-proto, platform Keychain/Keystore, WebRTC, native secure storage, explicit permission UX, safe gesture translation, and bounded background behavior. Do not advertise mobile agent or unattended-target capability.

**Step 4: Run Android/iOS device tests**

Run unit/build tests plus physical-device direct, forced relay, rotate, background/foreground, network switch, Bluetooth keyboard, clipboard-denied, audio-route, and thirty-minute thermal/resource cases.

Expected: PASS only for the declared Android/iOS controller matrix.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk-Mobile tests/device-lab/mobile
git commit -m "feat: add android and ios remote controllers"
~~~

### Task 51: Complete and gate the end-to-end P1 media paths

**Files:**
- Modify: apps/mrd-service/src/lan_discovery/media_profile.rs
- Modify: apps/mrd-service/src/lan_discovery/media_frame_preparation.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender.rs
- Modify: apps/mrd-service/src/lan_discovery/media_receiver_decoder.rs
- Modify: apps/mrd-service/src/lan_discovery/media_render_worker.rs
- Modify: apps/mrd-service/src/media_adaptation.rs
- Modify: crates/mrd-capture-dxgi/src/lib.rs
- Modify: crates/mrd-capture-macos/src/lib.rs
- Modify: crates/mrd-capture-pipewire/src/lib.rs
- Modify: crates/mrd-encode-nvenc/src/lib.rs
- Modify: crates/mrd-encode-nvenc-av1/src/lib.rs
- Modify: crates/mrd-decode/src/lib.rs
- Modify: crates/mrd-decode-nvdec/src/lib.rs
- Modify: crates/mrd-render-d3d11/src/lib.rs
- Modify: crates/mrd-render-macos/src/lib.rs
- Modify: crates/mrd-render-linux/src/lib.rs
- Modify: crates/mrd-transport-webrtc/src/peer.rs
- Modify: crates/mrd-transport-webrtc/src/lib.rs
- Create: crates/mrd-transport-webrtc/tests/codec_metadata.rs
- Modify: crates/mrd-pipeline-core/src/lib.rs
- Create: tests/quality-gates/policies/p1-media-2k4k.v1.json
- Create: tests/benchmarks/scenarios/p1.media.2k4k.json
- Create: tests/benchmarks/scripts/run_p1_media_matrix.ps1
- Create: tests/benchmarks/scripts/test_p1_media_matrix.ps1

**Step 1: Add failing negotiation and quality tests**

Cover route/platform codec intersection and an actual frame through capture, pixel-format conversion, encode, packetization, WebRTC SDP/RTP metadata, decode, render, and present. Required rows include H.265 Main/Main10, AV1, 4:4:4, color primaries/transfer/matrix/range, hardware/software fallback labeling, profile transition at a keyframe, decoder reset, and true selected-profile evidence.

**Step 2: Run targeted media tests**

Run:

~~~powershell
cargo test -p mrd-service media_profile
cargo test -p mrd-transport-webrtc --test codec_metadata
cargo test -p mrd-encode-nvenc -p mrd-encode-nvenc-av1 -p mrd-decode -p mrd-decode-nvdec
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_p1_media_matrix.ps1
~~~

Expected: FAIL for the complete P1 matrix.

**Step 3: Complete every media stage**

Preserve negotiated pixel format and color metadata across every stage. Add native H.265/AV1 encode/decode selection and render conversions on declared platforms, negotiate exact WebRTC fmtp/RTP parameters, and promote only combinations that present correctly on the required route and device. Keep unsupported browser/mobile combinations degraded or unavailable.

**Step 4: Run the P1 media device matrix**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p1_media_matrix.ps1

Expected: 2K/4K60 required rows PASS with capture, encoder, transport, decoder, render, present, hardware, and color evidence; visual validation detects chroma or metadata corruption.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-capture-dxgi crates/mrd-capture-macos crates/mrd-capture-pipewire crates/mrd-encode-nvenc crates/mrd-encode-nvenc-av1 crates/mrd-decode crates/mrd-decode-nvdec crates/mrd-render-d3d11 crates/mrd-render-macos crates/mrd-render-linux crates/mrd-transport-webrtc crates/mrd-pipeline-core tests/quality-gates tests/benchmarks
git commit -m "feat: gate mainstream media profiles"
~~~

### Task 52: Add simultaneous physical-display workflows

**Files:**
- Create: crates/mrd-display/Cargo.toml
- Create: crates/mrd-display/src/lib.rs
- Create: crates/mrd-display/src/layout.rs
- Create: crates/mrd-display/src/stream_set.rs
- Modify: Cargo.toml
- Modify: apps/mrd-session-agent/src/capture.rs
- Modify: apps/mrd-session-agent/src/render.rs
- Modify: apps/mrd-service/src/display_mode.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/multi_display.rs

**Step 1: Write failing simultaneous-display tests**

Cover all-monitors composition, independent controller windows, stable per-monitor stream IDs, mixed scale/rotation, negative coordinates, input mapping, hotplug/reorder, dynamic stream add/remove, bandwidth admission, reconnect, and cleanup after crash. Virtual/headless displays are explicitly unavailable in this task.

**Step 2: Run tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test multi_display

Expected: FAIL.

**Step 3: Implement physical display stream sets**

Add one explicit stream/layout model rather than overloading the P0 selected-source field. Bind every input coordinate and media stream to a display generation, and expose per-platform simultaneous-display limits truthfully.

**Step 4: Run multi-display device tests**

Run the integration test and the device-lab simultaneous-display scenario.

Expected: PASS with stream, hotplug, cleanup, bandwidth, and coordinate evidence on declared Windows/macOS/Linux rows.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-display apps/mrd-session-agent apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: add simultaneous physical displays"
~~~

### Task 53: Build and productize the Windows indirect virtual-display driver

**Files:**
- Create: drivers/mrd-virtual-display/mrd-virtual-display.sln
- Create: drivers/mrd-virtual-display/driver/mrd-virtual-display.vcxproj
- Create: drivers/mrd-virtual-display/driver/MrdVirtualDisplay.cpp
- Create: drivers/mrd-virtual-display/driver/MrdVirtualDisplay.inf
- Create: drivers/mrd-virtual-display/driver/MrdVirtualDisplay.man
- Create: drivers/mrd-virtual-display/installer/mrd-virtual-display-installer.vcxproj
- Create: drivers/mrd-virtual-display/installer/main.cpp
- Create: drivers/mrd-virtual-display/tests/driver_contract.cpp
- Create: crates/mrd-display/src/virtual_display.rs
- Create: crates/mrd-display/src/windows_idd.rs
- Create: apps/mrd-service/src/features/virtual_display.rs
- Modify: apps/mrd-service/src/display_mode.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Create: apps/Rdesk/scripts/install-virtual-display-driver.ps1
- Create: apps/Rdesk/scripts/uninstall-virtual-display-driver.ps1
- Create: apps/Rdesk/scripts/verify-virtual-display-driver.ps1
- Create: tests/windows-driver/virtual_display_driver.ps1
- Create: tests/device-lab/windows/run_virtual_display_matrix.ps1
- Create: .github/workflows/windows-driver.yml
- Create: docs/security/virtual-display-driver-threat-model.md
- Create: docs/release/windows-virtual-display-driver.md

**Step 1: Write failing driver, signing, and lifecycle tests**

Require a Windows Indirect Display Driver that proves:

- driver package and catalog signature chain to the approved publisher and reject tampering, downgrade, or test signing in production gates;
- install, upgrade, rollback, disable/enable, and uninstall leave a valid PnP/device state;
- one or more requested virtual monitors expose bounded EDID/mode sets and stable identities without spoofing physical displays;
- headless boot, console lock/unlock, user switch, sleep/resume, GPU reset, DWM restart, agent/service crash, and driver-host crash recover or remove displays cleanly;
- only the privileged service can create/remove displays through a versioned, ACL-protected control surface;
- unsupported OS/GPU/driver states return actionable capability failures and never leave a black phantom display.

**Step 2: Run driver contract and packaging tests**

Run in the WDK Windows driver job:

~~~powershell
msbuild drivers/mrd-virtual-display/mrd-virtual-display.sln /p:Configuration=Release /p:Platform=x64
powershell -ExecutionPolicy Bypass -File tests/windows-driver/virtual_display_driver.ps1 -PackagePath drivers/mrd-virtual-display/out/Release -ExpectedPublisher $env:MRD_WINDOWS_DRIVER_PUBLISHER
~~~

Expected: FAIL because the IDD, catalog signing, protected control surface, and lifecycle automation are absent.

**Step 3: Implement the signed IDD and service adapter**

Use the Windows IddCx model, bounded EDID/modes, a narrow privileged control protocol, explicit ownership, and idempotent create/remove operations. Package with production driver signing and atomic upgrade/rollback; never use an unsigned display emulator in release rows.

**Step 4: Run HLK-oriented and physical-device matrices**

Run build/security tests plus install/upgrade/uninstall, one/two virtual displays, headless reboot, sleep/resume, lock/user-switch, GPU/DWM/driver-host/service crash, 1080p60/4K60, input mapping, and twenty-four-hour churn on supported Windows hardware.

Expected: PASS with signed package evidence, no Code Integrity/PnP errors, correct display/input evidence, and no orphan virtual monitor after cleanup.

**Step 5: Commit**

~~~powershell
git add drivers/mrd-virtual-display crates/mrd-display apps/mrd-service crates/mrd-ipc apps/Rdesk/scripts tests/windows-driver tests/device-lab/windows .github/workflows/windows-driver.yml docs/security docs/release
git commit -m "feat: add signed windows virtual display driver"
~~~

### Task 54: Add policy-aware session recording

**Files:**
- Create: crates/mrd-recording/Cargo.toml
- Create: crates/mrd-recording/src/lib.rs
- Create: crates/mrd-recording/src/container.rs
- Create: crates/mrd-recording/src/policy.rs
- Modify: Cargo.toml
- Create: apps/mrd-service/src/features/recording.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Create: apps/Rdesk/src/app/components/SessionRecordingSettings.tsx
- Create: apps/Rdesk/src/app/components/SessionRecordingSettings.test.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/session_recording.rs

**Step 1: Write failing recording tests**

Test manual and policy-required recording, visible local/remote indication, explicit consent rules, local destination allowlist, free-space and quota limits, audio/video synchronization, pause restrictions, interruption-safe finalization, crash recovery, encryption-at-rest option, retention/deletion, redacted audit, and prohibition of hidden recording.

**Step 2: Run Rust and UI tests**

Run:

~~~text
cargo test --manifest-path tests/integration/Cargo.toml --test session_recording
pnpm --dir apps/Rdesk test -- --run src/app/components/SessionRecordingSettings.test.tsx
~~~

Expected: FAIL.

**Step 3: Implement recording as a scoped media consumer**

Record only authorized session streams through bounded queues. Use an interruption-safe container, visible state, policy/consent checks, destination controls, and deterministic finalization without blocking interactive media.

**Step 4: Run all focused tests**

Run the same commands.

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-recording apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: add policy aware session recording"
~~~

### Task 55: Add privacy screen, local-input blocking, and end-of-session lock

**Files:**
- Create: crates/mrd-privacy/Cargo.toml
- Create: crates/mrd-privacy/src/lib.rs
- Create: crates/mrd-privacy/src/windows.rs
- Create: crates/mrd-privacy/src/macos.rs
- Create: crates/mrd-privacy/src/linux.rs
- Modify: Cargo.toml
- Modify: apps/mrd-service/src/policy/privacy.rs
- Create: apps/mrd-service/src/features/privacy.rs
- Modify: apps/mrd-session-agent/src/runtime.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/DeviceDetailPage.tsx
- Create: apps/Rdesk/src/app/components/PrivacyControls.tsx
- Create: apps/Rdesk/src/app/components/PrivacyControls.test.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/privacy_controls.rs
- Create: docs/security/privacy-controls-threat-model.md

**Step 1: Write failing privacy and fail-safe tests**

Prove distinct grants for privacy screen, local-input block, and end-lock; conspicuous target/controller indication; platform capability truth; local emergency escape; automatic release on disconnect, crash, grant expiry, user switch, or policy change; and audit without keystroke/content capture. A failed privacy operation must abort or visibly downgrade before remote control continues according to policy.

**Step 2: Run privacy tests**

Run:

~~~text
cargo test -p mrd-privacy
cargo test --manifest-path tests/integration/Cargo.toml --test privacy_controls
pnpm --dir apps/Rdesk test -- --run src/app/components/PrivacyControls.test.tsx
~~~

Expected: FAIL because reviewed native adapters and fail-safe lifecycle behavior are absent.

**Step 3: Implement capability-gated native privacy controls**

Use reviewed per-OS APIs; do not simulate privacy by covering only the remote preview. Keep local escape and crash cleanup outside the remote data path, require explicit high-risk confirmation, and expose unsupported/partial states honestly.

**Step 4: Run native privacy device tests**

Run focused tests plus physical-console observation for enable/disable, disconnect, service/agent crash, lock/unlock, user switch, local escape, and unsupported platform cases.

Expected: PASS only where the local console and input behavior are directly observed and cleanup is proven.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-privacy apps/mrd-service apps/mrd-session-agent crates/mrd-ipc apps/Rdesk tests/integration docs/security/privacy-controls-threat-model.md
git commit -m "feat: add fail safe privacy controls"
~~~

### Task 56: Add account MFA and per-device access control

**Files:**
- Modify: apps/Rdesk-Server/app/core/security.py
- Create: apps/Rdesk-Server/app/models/device_access_rule.py
- Create: apps/Rdesk-Server/app/api/v1/mfa.py
- Create: apps/Rdesk-Server/app/api/v1/device_access.py
- Create: apps/Rdesk-Server/app/services/mfa.py
- Create: apps/Rdesk-Server/app/services/device_access.py
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_mfa.py
- Create: apps/Rdesk-Server/tests/test_device_access.py
- Modify: apps/mrd-service/src/handlers/identity.rs
- Create: apps/mrd-service/src/policy/access.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: apps/Rdesk/src/app/components/DeviceDetailPage.tsx
- Create: apps/Rdesk/src/app/components/DeviceAccessRules.test.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/device_access_revocation.rs

**Step 1: Write failing authentication and authorization tests**

Cover MFA enrollment/recovery/replay/rate-limit, step-up for unattended and high-risk changes, per-account/device allow and deny rules, precedence, expiry, concurrent update, target-side cache TTL, offline behavior, session-start enforcement, mid-session revocation, tenant/account isolation, and audit. Account MFA may authorize account operations but cannot replace peer identity or session grants.

**Step 2: Run backend, service, and UI tests**

Run:

~~~text
python -m pytest apps/Rdesk-Server/tests/test_mfa.py apps/Rdesk-Server/tests/test_device_access.py -q
cargo test --manifest-path tests/integration/Cargo.toml --test device_access_revocation
pnpm --dir apps/Rdesk test -- --run src/app/components/DeviceAccessRules.test.tsx
~~~

Expected: FAIL because MFA and device ACL enforcement are absent.

**Step 3: Implement MFA and device ACLs with explicit precedence**

Add phishing-resistant MFA where supported plus recovery controls, signed/versioned device rules, service-side verification, bounded offline cache, target safety ceiling, and immediate revocation propagation. Remove release defaults for JWT secrets or seeded administrators touched by this path.

**Step 4: Run security and revocation suites**

Run the commands from Step 2 plus server security-negative tests for cross-account access, stale/replayed factors, forged rules, and revocation during direct and relayed sessions.

Expected: PASS with deterministic precedence and no rule able to create peer trust silently.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk-Server apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: add mfa and device access control"
~~~

### Task 57: Add file clipboard and drag-and-drop workflows

**Files:**
- Modify: crates/mrd-clipboard/src/protocol.rs
- Modify: crates/mrd-file-transfer/src/protocol.rs
- Create: crates/mrd-file-transfer/src/clipboard.rs
- Create: crates/mrd-file-transfer/src/drag_drop.rs
- Create: apps/mrd-session-agent/src/file_transfer.rs
- Create: apps/mrd-service/src/file_transfer.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Create: apps/Rdesk/src/app/services/fileClipboardService.ts
- Create: apps/Rdesk/src/app/services/fileClipboardService.test.ts
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/file_clipboard.rs

**Step 1: Write failing file-clipboard tests**

Prove distinct file-clipboard and drag/drop scopes, manifest-before-content, canonical path validation, symlink/reparse handling, conflict policy, resume/hash verification, cancellation, quota/free-space limits, malware-scan hook, audit redaction, disconnect/crash cleanup, mixed OS filenames, and isolation from input/media flow control.

**Step 2: Run file-clipboard tests**

Run:

~~~text
cargo test --manifest-path tests/integration/Cargo.toml --test file_clipboard
pnpm --dir apps/Rdesk test -- --run src/app/services/fileClipboardService.test.ts
~~~

Expected: FAIL.

**Step 3: Implement bounded file clipboard and drag/drop**

Extend the existing clipboard/file protocols with a typed manifest and resumable content channel. Resolve targets inside approved roots, require explicit overwrite policy, bound queues and concurrent files, and never materialize untrusted paths before validation.

**Step 4: Run crate, service, UI, and cross-platform tests**

Run:

~~~text
cargo test -p mrd-clipboard -p mrd-file-transfer
cargo test --manifest-path tests/integration/Cargo.toml --test file_clipboard
pnpm --dir apps/Rdesk test -- --run src/app/services/fileClipboardService.test.ts
~~~

Expected: PASS plus Windows/macOS/Linux device rows for Unicode, large files, cancellation, reconnect, hostile paths, and cleanup.

**Step 5: Commit**

~~~powershell
git add crates/mrd-clipboard crates/mrd-file-transfer apps/mrd-session-agent apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration
git commit -m "feat: add remote file clipboard workflows"
~~~

### Task 58: Add scoped remote terminal and TCP tunnel channels

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-terminal/Cargo.toml
- Create: crates/mrd-terminal/src/lib.rs
- Create: crates/mrd-terminal/src/policy.rs
- Create: crates/mrd-tunnel/Cargo.toml
- Create: crates/mrd-tunnel/src/lib.rs
- Create: crates/mrd-tunnel/src/policy.rs
- Create: apps/mrd-session-agent/src/terminal.rs
- Create: apps/mrd-session-agent/src/tunnel.rs
- Create: apps/mrd-service/src/features/terminal.rs
- Create: apps/mrd-service/src/features/tunnel.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/terminal_tunnel.rs
- Create: docs/security/terminal-tunnel-threat-model.md

**Step 1: Write failing authorization and isolation tests**

Require separate terminal and tunnel grants, explicit user/policy enablement, target user identity, command/shell restrictions, destination CIDR/host/port allowlists, DNS-rebinding defense, no loopback/metadata-network bypass, rate/connection/byte/time limits, cancellation, redacted audit, session revocation, cleanup, and independent flow control that cannot block media/input. No generic privileged shell is allowed.

**Step 2: Run feature-channel tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test terminal_tunnel

Expected: FAIL because scoped terminal/tunnel protocols and policy enforcement are absent.

**Step 3: Implement explicit reliable channels**

Run terminal processes only as the bound session user with a constrained environment and lifecycle. Resolve and authorize tunnel destinations before each connect, bind only remote-session streams rather than local listeners by default, and enforce per-channel backpressure and revocation.

**Step 4: Run crate, integration, and security-negative tests**

Run:

~~~text
cargo test -p mrd-terminal -p mrd-tunnel
cargo test --manifest-path tests/integration/Cargo.toml --test terminal_tunnel
~~~

Expected: PASS including forbidden privilege, destination, DNS rebinding, resource exhaustion, replay, disconnect, and concurrent media cases.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-terminal crates/mrd-tunnel apps/mrd-session-agent apps/mrd-service crates/mrd-ipc tests/integration docs/security/terminal-tunnel-threat-model.md
git commit -m "feat: add scoped terminal and tunnel channels"
~~~

### Task 59: Add permissioned remote printing

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-print/Cargo.toml
- Create: crates/mrd-print/src/lib.rs
- Create: crates/mrd-print/src/spool.rs
- Create: crates/mrd-print/src/policy.rs
- Create: apps/mrd-session-agent/src/printing.rs
- Create: apps/mrd-service/src/features/printing.rs
- Modify: crates/mrd-ipc/src/lib.rs
- Create: apps/Rdesk/src/app/components/RemotePrintDialog.tsx
- Create: apps/Rdesk/src/app/components/RemotePrintDialog.test.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/remote_printing.rs
- Create: docs/security/remote-printing-threat-model.md

**Step 1: Write failing print policy and spool tests**

Prove a distinct print scope, explicit printer selection, reviewed PDF/raster spool formats, parser sandbox boundary, size/page/DPI/color limits, cancellation, duplicate-job idempotency, spool encryption/cleanup, printer allowlist, user-visible state, disconnect/crash handling, audit redaction, and isolation from media/input.

**Step 2: Run print tests**

Run:

~~~text
cargo test -p mrd-print
cargo test --manifest-path tests/integration/Cargo.toml --test remote_printing
pnpm --dir apps/Rdesk test -- --run src/app/components/RemotePrintDialog.test.tsx
~~~

Expected: FAIL because the reviewed spool and permission workflow are absent.

**Step 3: Implement the bounded print workflow**

Accept only the reviewed spool representation, validate before spooling, require target printer/user confirmation or fleet policy, execute with the session user printer context, and erase temporary content on every terminal path.

**Step 4: Run physical and virtual printer tests**

Run focused tests plus Windows/macOS/Linux printer-device rows for success, cancel, offline printer, malformed input, oversize job, duplicate/reconnect, service crash, and concurrent interactive media.

Expected: PASS with correct physical/virtual output and no residual spool data after cleanup.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-print apps/mrd-session-agent apps/mrd-service crates/mrd-ipc apps/Rdesk tests/integration docs/security/remote-printing-threat-model.md
git commit -m "feat: add permissioned remote printing"
~~~

### Task 60: Establish the executable P1 cross-platform release gate

**Files:**
- Create: tests/quality-gates/policies/p1-mainstream-parity.v1.json
- Create: tests/benchmarks/scripts/run_p1_platform_matrix.ps1
- Create: tests/benchmarks/scripts/run_p1_platform_matrix_macos.sh
- Create: tests/benchmarks/scripts/run_p1_platform_matrix_linux.sh
- Create: tests/benchmarks/scripts/test_p1_platform_matrix.ps1
- Create: crates/mrd-quality-gate/tests/p1_contract.rs
- Create: .github/workflows/p1-platform-device-lab.yml
- Create: docs/release/p1-mainstream-acceptance.md

**Step 1: Write failing matrix-contract tests**

Require the complete P1 capability set per supported platform/role and route. A platform may be marked partial, but the overall P1 tier cannot pass until all required platform rows pass.

**Step 2: Run exact script and policy contract tests**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_p1_platform_matrix.ps1
cargo test -p mrd-quality-gate --test p1_contract
~~~

Expected: FAIL until scripts/policies cover all rows.

**Step 3: Add device-lab matrix orchestration**

Include Windows, macOS, Linux, Web, and mobile-controller lanes; service/agent install and lifecycle; 2K/4K60 end-to-end codec/color paths; simultaneous and signed virtual displays; recording; privacy; MFA/device ACL; file clipboard; terminal/tunnel; printing; security; weak network; failure recovery; and soak.

**Step 4: Run the P1 matrix**

Run each platform script in its configured device lab.

Expected: PASS only with complete v2 artifacts and canonical verdicts.

**Step 5: Commit**

~~~powershell
git add tests/quality-gates tests/benchmarks crates/mrd-quality-gate/tests/p1_contract.rs .github/workflows docs/release
git commit -m "test: define p1 mainstream release gate"
~~~

## P2 Advanced Capability Parity

### Task 61: Gate high-refresh capture, pacing, codec, and present paths

**Files:**
- Modify: apps/mrd-service/src/lan_discovery/media_profile.rs
- Modify: apps/mrd-service/src/lan_discovery/media_frame_preparation.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender.rs
- Modify: apps/mrd-service/src/lan_discovery/media_receiver_decoder.rs
- Modify: apps/mrd-service/src/lan_discovery/media_render_worker.rs
- Modify: crates/mrd-capture-dxgi/src/lib.rs
- Modify: crates/mrd-encode-nvenc/src/lib.rs
- Modify: crates/mrd-encode-nvenc-av1/src/lib.rs
- Modify: crates/mrd-decode-nvdec/src/lib.rs
- Modify: crates/mrd-render-d3d11/src/lib.rs
- Modify: crates/mrd-pipeline-core/src/lib.rs
- Create: tests/quality-gates/policies/p2-high-refresh.v1.json
- Create: tests/benchmarks/scenarios/p2.high-refresh.json
- Create: tests/benchmarks/scripts/run_p2_high_refresh_matrix.ps1

**Step 1: Write failing refresh and pacing tests**

Cover 1080p144/180/240, 2K144/180, 1600p165, and 4K120/144 where hardware supports them. Prove actual capture cadence, encoder throughput, network pacing, decoder throughput, display refresh, present cadence, frame-time percentiles, queue bounds, latency under overload, no duplicated-frame inflation, and truthful downgrade.

**Step 2: Run focused tests**

Run:

~~~powershell
cargo test -p mrd-service media_profile
cargo test -p mrd-render-d3d11
~~~

Expected: FAIL for the full P2 profile set.

**Step 3: Complete the high-refresh pipeline**

Keep profiles capability-gated. Coordinate capture clocks, hardware encode/decode, transport pacing, bounded drop policy, swapchain timing, and native present. Never use requested FPS, encoder input count, decoded FPS, or WebView preview as high-refresh proof.

**Step 4: Run the high-refresh device matrix**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p2_high_refresh_matrix.ps1

Expected: accepted rows meet present-cadence, latency, frame-time, freeze, resource, and visual-integrity policy on a verified display refresh; unsupported rows are explicit capability results, not fake passes.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-capture-dxgi crates/mrd-encode-nvenc crates/mrd-encode-nvenc-av1 crates/mrd-decode-nvdec crates/mrd-render-d3d11 crates/mrd-pipeline-core tests/quality-gates tests/benchmarks
git commit -m "feat: gate high refresh media paths"
~~~

### Task 62: Complete and gate the end-to-end HDR color pipeline

**Files:**
- Modify: apps/mrd-service/src/lan_discovery/media_profile.rs
- Modify: apps/mrd-service/src/lan_discovery/media_frame_preparation.rs
- Modify: apps/mrd-service/src/lan_discovery/media_sender.rs
- Modify: apps/mrd-service/src/lan_discovery/media_receiver_decoder.rs
- Modify: apps/mrd-service/src/lan_discovery/media_render_worker.rs
- Modify: crates/mrd-capture-dxgi/src/lib.rs
- Modify: crates/mrd-capture-macos/src/lib.rs
- Modify: crates/mrd-capture-pipewire/src/lib.rs
- Modify: crates/mrd-encode-nvenc/src/lib.rs
- Modify: crates/mrd-encode-nvenc-av1/src/lib.rs
- Modify: crates/mrd-decode/src/lib.rs
- Modify: crates/mrd-decode-nvdec/src/lib.rs
- Create: crates/mrd-transport-webrtc/src/color.rs
- Modify: crates/mrd-transport-webrtc/src/lib.rs
- Create: crates/mrd-transport-webrtc/tests/hdr_metadata.rs
- Modify: crates/mrd-render-d3d11/src/lib.rs
- Modify: crates/mrd-render-macos/src/lib.rs
- Modify: crates/mrd-render-linux/src/lib.rs
- Modify: crates/mrd-pipeline-core/src/lib.rs
- Create: tests/quality-gates/policies/p2-hdr-color.v1.json
- Create: tests/benchmarks/scenarios/p2.hdr-color.json
- Create: tests/benchmarks/scripts/run_p2_hdr_matrix.ps1
- Create: tests/benchmarks/scripts/test_p2_hdr_matrix.ps1

**Step 1: Write failing end-to-end HDR integrity tests**

Send known HDR10/Main10 and SDR reference frames through capture, conversion, encode, WebRTC SDP/RTP color metadata, relay/direct transport, decode, render, and present. Verify bit depth, pixel format, mastering display metadata where available, primaries, transfer, matrix, range, MaxCLL/MaxFALL, chroma, tone-map decision, swapchain colorspace, display HDR state, screenshot/probe output, and truthful SDR downgrade without double tone mapping.

**Step 2: Run codec, transport, render, and matrix contract tests**

Run:

~~~powershell
cargo test -p mrd-transport-webrtc --test hdr_metadata
cargo test -p mrd-encode-nvenc -p mrd-encode-nvenc-av1 -p mrd-decode -p mrd-decode-nvdec
cargo test -p mrd-render-d3d11 -p mrd-render-macos -p mrd-render-linux
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_p2_hdr_matrix.ps1
~~~

Expected: FAIL because color state is not preserved and proven across every stage.

**Step 3: Implement explicit color-state transport and presentation**

Make color state a required frame/profile value, map it into exact codec and WebRTC metadata, validate decoder output, select HDR swapchain/colorspace only on a capable active display, and use one documented tone-map path for SDR targets. Reject or downgrade any route/platform combination that loses mandatory metadata.

**Step 4: Run real HDR-display acceptance**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p2_hdr_matrix.ps1

Use calibrated HDR source/target displays and a capture/probe workflow for direct and forced-relay HDR-to-HDR, HDR-to-SDR, SDR-to-HDR, display toggle/hotplug, reconnect, codec switch, and ten-minute scene sequences.

Expected: automated metadata tests PASS and measured/visually reviewed output meets luminance, clipping, banding, hue, chroma, and transition policy. A non-HDR or unverified display cannot satisfy an HDR row.

**Step 5: Commit**

~~~powershell
git add apps/mrd-service crates/mrd-capture-dxgi crates/mrd-capture-macos crates/mrd-capture-pipewire crates/mrd-encode-nvenc crates/mrd-encode-nvenc-av1 crates/mrd-decode crates/mrd-decode-nvdec crates/mrd-transport-webrtc crates/mrd-render-d3d11 crates/mrd-render-macos crates/mrd-render-linux crates/mrd-pipeline-core tests/quality-gates tests/benchmarks
git commit -m "feat: gate end to end hdr color"
~~~

### Task 63: Add multi-controller collaboration and annotation

**Files:**
- Create: crates/mrd-collaboration/Cargo.toml
- Create: crates/mrd-collaboration/src/lib.rs
- Create: crates/mrd-collaboration/src/roles.rs
- Create: crates/mrd-collaboration/src/annotation.rs
- Modify: Cargo.toml
- Modify: crates/mrd-session/src/remote_session.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Create: apps/mrd-service/src/features/collaboration.rs
- Modify: apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx
- Create: apps/Rdesk/src/app/components/CollaborationToolbar.tsx
- Create: apps/Rdesk/src/app/components/CollaborationToolbar.test.tsx
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/collaboration.rs

**Step 1: Write failing role/consistency tests**

Test owner/presenter/viewer/controller roles, explicit control handoff, concurrent pointer identity, annotation ordering, participant removal, permission revocation, and audit. No participant can exceed their grant.

**Step 2: Run tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test collaboration

Expected: FAIL.

**Step 3: Implement collaboration as a scoped feature**

Keep one target session aggregate with multiple participant grants. Use a reliable collaboration channel and independent ephemeral pointer/annotation updates. Require target policy for additional controllers.

**Step 4: Run Rust and UI tests**

Run:

~~~text
cargo test -p mrd-collaboration
cargo test --manifest-path tests/integration/Cargo.toml --test collaboration
pnpm --dir apps/Rdesk test -- --run src/app/components/CollaborationToolbar.test.tsx
~~~

Expected: PASS.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-collaboration crates/mrd-session apps/mrd-service apps/Rdesk tests/integration
git commit -m "feat: add remote collaboration roles"
~~~

### Task 64: Add reviewed semantic peripheral forwarding

**Files:**
- Create: crates/mrd-peripheral/Cargo.toml
- Create: crates/mrd-peripheral/src/lib.rs
- Create: crates/mrd-peripheral/src/gamepad.rs
- Create: crates/mrd-peripheral/src/tablet.rs
- Create: crates/mrd-peripheral/src/camera.rs
- Create: crates/mrd-peripheral/src/microphone.rs
- Modify: Cargo.toml
- Modify: common-control-proto/src/lib.rs
- Create: apps/mrd-session-agent/src/peripheral.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/peripheral_permissions.rs
- Create: docs/security/peripheral-forwarding-threat-model.md

**Step 1: Write threat models and failing permission tests**

Each peripheral has a distinct scope, device allowlist, rate/size limits, user-visible state, explicit start/stop, cleanup, and audit. USB class forwarding is not implemented until each allowed class has a reviewed threat model.

**Step 2: Run tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test peripheral_permissions

Expected: FAIL.

**Step 3: Implement scoped high-level devices first**

Implement gamepad, tablet, microphone, and camera at semantic media/input layers. Isolate each device class, validate descriptors/events, bind it to one grant and target session, and keep generic/raw USB forwarding unavailable.

**Step 4: Run desktop peripheral device matrices**

Run crate/integration tests and configured Windows/macOS/Linux physical gamepad, tablet, microphone, and camera cases, including hot-unplug, permission revoke, malformed events, rate abuse, reconnect, and concurrent media.

Expected: PASS only for reviewed platform/device combinations.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-peripheral common-control-proto apps/mrd-session-agent tests/integration docs/security
git commit -m "feat: add permissioned peripheral forwarding"
~~~

### Task 65: Add Android and iOS target-agent capabilities

**Files:**
- Modify: apps/Rdesk-Mobile/package.json
- Modify: apps/Rdesk-Mobile/src/main.tsx
- Modify: apps/Rdesk-Mobile/src/session/client.ts
- Create: apps/Rdesk-Mobile/src/agent/status.ts
- Create: apps/Rdesk-Mobile/src/agent/status.test.ts
- Modify: apps/Rdesk-Mobile/src-tauri/Cargo.toml
- Modify: apps/Rdesk-Mobile/src-tauri/src/lib.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/mod.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/capture.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/input.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/audio.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/permissions.rs
- Create: apps/Rdesk-Mobile/src-tauri/src/agent/lifecycle.rs
- Modify: apps/Rdesk-Mobile/src-tauri/gen/android/app/src/main/AndroidManifest.xml
- Modify: apps/Rdesk-Mobile/src-tauri/gen/apple/RdeskMobile/Info.plist
- Create: tests/device-lab/mobile/run_agent_matrix.md
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/mobile_agent_capabilities.rs
- Create: docs/security/mobile-agent-threat-model.md

**Step 1: Write failing mobile-agent capability and lifecycle tests**

For each OS/version prove truthful support for screen broadcast/capture, view-only versus input control, audio, foreground service/broadcast indication, consent, permission revoke, background/lock behavior, process death, network switch, reconnect, thermal/resource limits, and cleanup. iOS and Android rows must stay distinct; unavailable OS-level input control can never be represented as supported.

**Step 2: Run shared, Android, and iOS tests**

Run:

~~~text
pnpm --dir apps/Rdesk-Mobile test
cargo test --manifest-path tests/integration/Cargo.toml --test mobile_agent_capabilities
pnpm --dir apps/Rdesk-Mobile tauri android build --debug
pnpm --dir apps/Rdesk-Mobile tauri ios build --debug
~~~

Expected: FAIL because target-agent permission, lifecycle, and capture/control paths are absent.

**Step 3: Implement only OS-approved mobile target paths**

Use Android MediaProjection/foreground-service and approved accessibility or managed-device APIs only under explicit policy and disclosure. Use iOS ReplayKit broadcast capabilities and keep unsupported remote input unavailable. Store identity in Keystore/Keychain and bind every broadcast/control action to visible consent and scoped grants.

**Step 4: Run physical-device target matrices**

Run shared tests plus declared Android/iOS versions for attended start, permission denial/revoke, screen/audio, lock/unlock, background/foreground, process kill, rotate, network switch, relay, thirty-minute thermal load, and cleanup.

Expected: PASS only for directly observed OS-approved capabilities; platform restrictions remain explicit capability results.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk-Mobile tests/device-lab/mobile tests/integration docs/security/mobile-agent-threat-model.md
git commit -m "feat: add truthful mobile target agents"
~~~

### Task 66: Add multi-region relay selection and high availability

**Files:**
- Create: crates/mrd-relay-control/Cargo.toml
- Create: crates/mrd-relay-control/src/lib.rs
- Create: crates/mrd-relay-control/src/selection.rs
- Create: crates/mrd-relay-control/src/health.rs
- Modify: Cargo.toml
- Modify: apps/Rdesk-Server/app/services/turn_credentials.py
- Create: apps/Rdesk-Server/app/services/relay_directory.py
- Create: apps/Rdesk-Server/app/api/v1/relays.py
- Modify: apps/realtime-server/src/presence.rs
- Modify: apps/mrd-service/src/transports/webrtc.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/multi_region_relay.rs
- Create: deploy/turn/regions.example.yaml

**Step 1: Write failing selection/failover tests**

Cover signed relay directory, latency/load/health selection, region policy, credential scope, relay outage before/during session, alternate relay ICE restart, stale directory, and no cross-region policy bypass.

**Step 2: Run tests**

Run: cargo test --manifest-path tests/integration/Cargo.toml --test multi_region_relay

Expected: FAIL.

**Step 3: Implement relay directory and migration**

Keep TURN as the data-plane relay. Add authenticated regional discovery, health, short-lived credentials, selection reason, capacity limits, and failover telemetry.

**Step 4: Run multi-region lab tests**

Run the integration tests and configured two-region outage scenario.

Expected: PASS with actual selected relay evidence before and after migration.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-relay-control apps/Rdesk-Server apps/realtime-server apps/mrd-service tests/integration deploy/turn
git commit -m "feat: add multi region relay failover"
~~~

### Task 67: Add tenant-safe OIDC and SCIM identity lifecycle

**Files:**
- Create: apps/Rdesk-Server/app/models/organization.py
- Create: apps/Rdesk-Server/app/models/enterprise_identity.py
- Create: apps/Rdesk-Server/app/api/v1/organizations.py
- Create: apps/Rdesk-Server/app/api/v1/scim.py
- Create: apps/Rdesk-Server/app/services/oidc.py
- Create: apps/Rdesk-Server/app/services/scim.py
- Modify: apps/Rdesk-Server/app/core/security.py
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_oidc.py
- Create: apps/Rdesk-Server/tests/test_scim.py

**Step 1: Write failing federation and provisioning tests**

Cover OIDC discovery, issuer/audience/nonce/state/PKCE validation, key rotation, login/linking, just-in-time provisioning policy, tenant/domain isolation, SCIM bearer scope, create/update/deactivate/reactivate, group membership, pagination/filtering, idempotency, concurrency, rate limiting, secret rotation, and immediate session/token revocation on deprovisioning.

Federated identity establishes an account/organization principal only; it cannot silently create device trust, peer trust, or remote-session permission.

**Step 2: Run backend identity tests**

Run:

~~~text
python -m pytest apps/Rdesk-Server/tests/test_oidc.py apps/Rdesk-Server/tests/test_scim.py -q
~~~

Expected: FAIL.

**Step 3: Implement tenant-bound OIDC and SCIM**

Pin configured issuers and mappings per organization, validate every OIDC token field, encrypt client/SCIM secrets, make provisioning changes versioned and idempotent, and revoke local sessions/credentials on deactivation. Remove release defaults for JWT secrets and seeded administrator credentials.

**Step 4: Run backend and federation security suites**

Run:

~~~text
python -m pytest apps/Rdesk-Server/tests -q
~~~

Expected: PASS including wrong issuer/tenant, algorithm confusion, replay, stale key, account takeover/linking, SCIM cross-tenant, secret leak, and deprovision/revocation cases.

**Step 5: Commit**

~~~powershell
git add apps/Rdesk-Server
git commit -m "feat: add enterprise oidc and scim identity"
~~~

### Task 68: Add RBAC and signed fleet policy enforcement

**Files:**
- Modify: Cargo.toml
- Create: crates/mrd-policy/Cargo.toml
- Create: crates/mrd-policy/src/lib.rs
- Create: crates/mrd-policy/src/precedence.rs
- Create: crates/mrd-policy/src/signature.rs
- Create: apps/Rdesk-Server/app/models/role.py
- Create: apps/Rdesk-Server/app/models/policy.py
- Create: apps/Rdesk-Server/app/api/v1/policies.py
- Create: apps/Rdesk-Server/app/services/policy_signing.py
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_rbac.py
- Create: apps/Rdesk-Server/tests/test_fleet_policy.py
- Modify: apps/mrd-service/src/policy/mod.rs
- Create: apps/mrd-service/src/policy/fleet.rs
- Modify: apps/mrd-service/src/handlers/session.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/fleet_policy.rs

**Step 1: Write failing RBAC and precedence tests**

Cover tenant isolation, built-in/custom roles, least privilege, role/group/device targeting, deny precedence, signed/versioned policy, rollback protection, local safety ceiling, offline cache TTL/expiry, forced recording/privacy/file/terminal/tunnel/print constraints, concurrent update, direct/relay enforcement, session-start checks, mid-session revocation, and administrator lockout recovery. Fleet policy may restrict local behavior but cannot create peer trust or weaken non-overridable safety rules.

**Step 2: Run backend and service tests**

Run:

~~~text
python -m pytest apps/Rdesk-Server/tests/test_rbac.py apps/Rdesk-Server/tests/test_fleet_policy.py -q
cargo test -p mrd-policy
cargo test --manifest-path tests/integration/Cargo.toml --test fleet_policy
~~~

Expected: FAIL because signed fleet policy and deterministic precedence are absent.

**Step 3: Implement signed, explicit policy evaluation**

Use one pure policy evaluator shared by admission and active-session enforcement. Sign revisions server-side, verify and persist bounded cache service-side, reject rollback/unknown critical fields, expose the winning rule and safe remediation, and revoke affected scopes without over-revoking unrelated sessions.

**Step 4: Run policy security and outage suites**

Run focused tests plus forged/stale/conflicting policy, tenant escape, backend outage, clock skew, cache expiry, mid-session change, direct/relay, and rollback cases.

Expected: PASS with identical authoritative decisions in server, service, and audit evidence.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-policy apps/Rdesk-Server apps/mrd-service tests/integration
git commit -m "feat: enforce rbac and signed fleet policy"
~~~

### Task 69: Add tamper-evident enterprise audit retention and export

**Files:**
- Create: apps/Rdesk-Server/app/models/audit_event.py
- Create: apps/Rdesk-Server/app/api/v1/audit.py
- Create: apps/Rdesk-Server/app/services/audit_ingest.py
- Create: apps/Rdesk-Server/app/services/audit_export.py
- Modify: apps/Rdesk-Server/app/api/v1/router.py
- Create: apps/Rdesk-Server/tests/test_audit_ingest.py
- Create: apps/Rdesk-Server/tests/test_audit_export.py
- Create: crates/mrd-audit/Cargo.toml
- Create: crates/mrd-audit/src/lib.rs
- Create: crates/mrd-audit/src/chain.rs
- Modify: Cargo.toml
- Create: apps/mrd-service/src/audit.rs
- Modify: tests/integration/Cargo.toml
- Create: tests/integration/enterprise_audit.rs
- Create: docs/security/audit-data-classification.md

**Step 1: Write failing integrity, privacy, and retention tests**

Cover stable event schema, tenant/device/session correlation, sequence/hash-chain integrity, authenticated ingestion, replay/duplicate/out-of-order handling, local offline queue limits, redaction of secrets/content/clipboard/filenames as configured, clock uncertainty, retention/legal hold, deletion authorization, paginated export, export signing, access audit, tenant isolation, SIEM delivery retry, and verification after partial corruption.

**Step 2: Run audit tests**

Run:

~~~text
python -m pytest apps/Rdesk-Server/tests/test_audit_ingest.py apps/Rdesk-Server/tests/test_audit_export.py -q
cargo test -p mrd-audit
cargo test --manifest-path tests/integration/Cargo.toml --test enterprise_audit
~~~

Expected: FAIL because canonical tamper evidence, privacy rules, retention, and export are absent.

**Step 3: Implement bounded tamper-evident audit flow**

Emit typed redacted events from authoritative service transitions, chain and queue them locally with bounded storage, ingest idempotently per tenant/device, enforce retention/legal hold, and export signed verifiable batches. Treat audit outage according to explicit fail-open/fail-closed policy for each regulated feature.

**Step 4: Run outage, privacy, and export verification**

Run focused tests plus offline/backfill, server failover, duplicate/reorder, tamper, retention expiry, legal hold, cross-tenant export, large export, SIEM outage, and redaction scans over generated artifacts.

Expected: PASS with verifiable chain/export evidence and no prohibited content or secret in logs, queues, APIs, or artifacts.

**Step 5: Commit**

~~~powershell
git add Cargo.toml crates/mrd-audit apps/Rdesk-Server apps/mrd-service tests/integration docs/security/audit-data-classification.md
git commit -m "feat: add tamper evident enterprise audit"
~~~

### Task 70: Establish the executable P2 advanced release gate

**Files:**
- Create: tests/quality-gates/policies/p2-advanced-parity.v1.json
- Create: tests/benchmarks/scripts/run_p2_advanced_matrix.ps1
- Create: tests/benchmarks/scripts/test_p2_advanced_matrix.ps1
- Create: crates/mrd-quality-gate/tests/p2_contract.rs
- Create: .github/workflows/p2-advanced-device-lab.yml
- Create: docs/release/p2-advanced-acceptance.md

**Step 1: Write failing gate-contract tests**

Require high-refresh present evidence, full HDR/color evidence on a real HDR display, collaboration, reviewed semantic peripherals, truthful Android/iOS target agents, multi-region relay, OIDC/SCIM, RBAC/fleet policy, tamper-evident audit, fault recovery, privacy, security-negative, and soak evidence.

**Step 2: Run gate-contract tests**

Run:

~~~powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_p2_advanced_matrix.ps1
cargo test -p mrd-quality-gate --test p2_contract
~~~

Expected: FAIL until every required P2 row is present and enforced.

**Step 3: Add the P2 orchestrator**

Compose route/platform/capability matrices without treating unsupported rows as PASS. Preserve per-feature evidence and privacy-safe artifacts.

**Step 4: Run the configured P2 device lab**

Run: powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p2_advanced_matrix.ps1

Expected: PASS only when the complete advanced policy is proven.

**Step 5: Commit**

~~~powershell
git add tests/quality-gates tests/benchmarks crates/mrd-quality-gate/tests/p2_contract.rs .github/workflows docs/release
git commit -m "test: define p2 advanced release gate"
~~~

## Final Completion Audit

### Task 71: Audit every approved requirement before completing the goal

**Files:**
- Create: docs/release/market-capability-completion-audit.md
- Create: tests/quality-gates/manifests/market-capability-requirements.v1.json
- Create: crates/mrd-quality-gate/src/completion.rs
- Create: crates/mrd-quality-gate/tests/completion_audit.rs
- Modify: docs/plans/2026-07-11-market-remote-capability-alignment-design.md

**Step 1: Encode every design requirement**

Create one stable requirement ID for each:

- architecture invariant;
- P0/P1/P2 capability;
- security rule;
- route requirement;
- process boundary;
- SLO;
- weak-network case;
- CI/device-lab/soak gate;
- completion rule.

Each entry names authoritative artifact queries and required verdicts. No requirement may be satisfied by a plan, code search, or narrower test.

**Step 2: Write a failing completeness test**

~~~rust
#[test]
fn every_required_capability_has_current_passing_evidence() {
    let audit = load_completion_audit();
    let result = evaluate_completion(&audit).unwrap();
    assert!(result.missing.is_empty());
    assert!(result.contradicted.is_empty());
    assert!(result.weak_or_indirect.is_empty());
    assert!(result.invalid.is_empty());
}
~~~

Run: cargo test -p mrd-quality-gate --test completion_audit

Expected: FAIL until every requirement has current valid evidence.

**Step 3: Perform the evidence audit**

For each requirement classify evidence as:

- proves completion;
- contradicts completion;
- incomplete;
- weak/indirect;
- missing;
- invalid/stale.

Re-run or repair every incomplete row. Do not waive requirements because implementation exists or a different scenario passes.

**Step 4: Run full release verification**

Run:

~~~text
cargo test --workspace
pnpm --dir apps/Rdesk test
pnpm --dir apps/Rdesk type-check
python -m pytest apps/Rdesk-Server/tests -q
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_windows_p0_matrix.ps1 -ControllerDeviceId $env:MRD_P0_CONTROLLER_DEVICE_ID -TargetDeviceId $env:MRD_P0_TARGET_DEVICE_ID -TurnProfile $env:MRD_TURN_PROFILE
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p1_platform_matrix.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_p2_advanced_matrix.ps1
cargo test -p mrd-quality-gate --test completion_audit
~~~

Expected: all required suites PASS and completion audit contains no missing, contradicted, weak, stale, or invalid requirement.

**Step 5: Commit the audit**

~~~powershell
git add docs/release tests/quality-gates crates/mrd-quality-gate docs/plans/2026-07-11-market-remote-capability-alignment-design.md
git commit -m "docs: prove market remote capability alignment"
~~~

Only after this commit and an independent review of the artifacts may the active goal be marked complete.

## Execution Handoff

The plan is intentionally large because the approved goal spans a full remote-access product, not one feature. Execute milestone by milestone and keep the full objective active across sessions.

Recommended first execution batch:

1. Tasks 1–4: canonical quality-gate crate, artifact, policy, and CLI.
2. Tasks 5–9: fail-closed script/frontend/CI integration.
3. Run the complete Gate 0 milestone verification.
4. Review evidence before starting Task 10.

Do not start public connectivity or feature work while Gate 0 can still report a failed product as successful.
