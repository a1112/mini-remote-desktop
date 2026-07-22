# Rdesk Hard-Cut Service Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the old in-process remote-control mainline from `apps/Rdesk` and make `apps/mrd-service` the only owner of session orchestration, transport runtime, and runtime snapshots.

**Architecture:** The migration is a hard cut. `Rdesk` keeps only shell responsibilities, service lifecycle control, IPC calls, and UI/render shell concerns. `mrd-service` owns session state, signaling application, transport host lifecycle, sender/receiver control, and all runtime/probe snapshots. There must be no direct fallback path left in `Rdesk` after the cutover.

**Tech Stack:** Rust, Tauri, Tokio, local IPC via `mrd-ipc`, application/domain crates (`mrd-application`, `mrd-session`), existing realtime/quic/webrtc/media adapters.

---

### Task 1: Expand the IPC contract to cover the hard cut

**Files:**
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ipc\src\lib.rs`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ipc\tests\contracts.rs`
- Test: `G:\Project\mini-remote-desktop\crates\mrd-ipc\tests\contracts.rs`

**Step 1: Write failing contract tests for the missing request/response surface**

Add serialization tests for:
- `ListSessions`
- `GetRuntimeSnapshot`
- `GetProbeSnapshot`
- `ServiceHealth`
- `SessionRuntimeSnapshot.last_error`
- `RuntimeSnapshot`
- `ServiceStatus`

**Step 2: Run the new contract tests to verify they fail**

Run: `cargo test -p mrd-ipc --test contracts`
Expected: FAIL because the new request/response variants and DTO fields do not exist yet.

**Step 3: Extend the IPC request/response enums and DTOs**

Add the missing request/response variants and DTOs in `mrd-ipc`, keeping them independent from Tauri types.

Minimum DTO surface:

```rust
pub struct SessionRuntimeSnapshot {
    pub session_id: SessionId,
    pub role: String,
    pub state: String,
    pub transport_kind: String,
    pub local_bootstrap: Option<SessionBootstrap>,
    pub remote_bootstrap: Option<SessionBootstrap>,
    pub last_error: Option<String>,
}
```

```rust
pub struct ServiceStatus {
    pub running: bool,
    pub healthy: bool,
    pub pid: Option<u32>,
}
```

**Step 4: Run the contract tests to verify they pass**

Run: `cargo test -p mrd-ipc --test contracts`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/mrd-ipc/src/lib.rs crates/mrd-ipc/tests/contracts.rs
git commit -m "feat: expand IPC contracts for hard-cut migration"
```

### Task 2: Make `mrd-service` own a real runtime app state

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\app_state.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\main.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\ipc_server.rs`
- Test: `G:\Project\mini-remote-desktop\apps\mrd-service\src\ipc_server.rs`

**Step 1: Write a failing test proving the service must expose shared runtime state**

Add a service test that:
- constructs one `AppState`
- serves `StartSession`
- then reads `SessionRuntimeSnapshot`
- expects the same shared runtime to be visible across requests

**Step 2: Run the service test to verify it fails**

Run: `cargo test -p mrd-service session_snapshot_returns_shared_state`
Expected: FAIL because `ipc_server` still uses its own in-memory store rather than service-owned runtime state.

**Step 3: Create `mrd-service::AppState`**

Move ownership of service-side runtime objects into a shared state object, for example:

```rust
pub struct AppState {
    pub realtime_runtime: RealtimeRuntime,
    pub webrtc_host: Arc<Mutex<WebrtcHost>>,
    pub quic_host: Arc<Mutex<QuicHost>>,
    pub webrtc_sessions: Arc<Mutex<WebrtcSessionCoordinator>>,
    pub quic_sessions: Arc<Mutex<QuicSessionCoordinator>>,
    pub probe_registry: Arc<ProbeRegistry>,
}
```

**Step 4: Inject `AppState` into `IpcServer`**

Replace `IpcSessionStore` as the source of truth. `ipc_server` should read/write through service-owned state, not its own session hash map.

**Step 5: Run the service tests to verify shared state works**

Run: `cargo test -p mrd-service`
Expected: PASS for the new shared-state test.

**Step 6: Commit**

```bash
git add apps/mrd-service/src/app_state.rs apps/mrd-service/src/main.rs apps/mrd-service/src/ipc_server.rs
git commit -m "refactor: add shared service app state"
```

### Task 3: Move session control handlers into `mrd-service`

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\handlers\session.rs`
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\handlers\transport.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\ipc_server.rs`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-application\src\lib.rs`
- Test: `G:\Project\mini-remote-desktop\apps\mrd-service\tests\ipc_session_flow.rs`

**Step 1: Write a failing end-to-end service test for session control**

Create a test that sends:
- `StartSession`
- `AcceptSession`
- `StartSender`
- `StartReceiver`
- `StopSession`

through IPC server handlers and verifies the service mutates shared session state.

**Step 2: Run the test to verify it fails**

Run: `cargo test -p mrd-service --test ipc_session_flow`
Expected: FAIL because the current handlers only return placeholder success for sender/receiver and do not use shared orchestrators.

**Step 3: Replace placeholder handler logic**

Extract the request handling into focused handler modules and route them through application/domain objects instead of ad-hoc DTO mutation.

Use `mrd-application` as the orchestration entry point where practical; if a use case is empty today, implement only the minimum logic needed for the hard cut.

**Step 4: Make sender/receiver operations mutate service-owned runtime**

Remove the current `TODO`-only success path. Sender and receiver start commands must reflect real service state, not synthetic acks.

**Step 5: Run the new flow test**

Run: `cargo test -p mrd-service --test ipc_session_flow`
Expected: PASS

**Step 6: Commit**

```bash
git add apps/mrd-service/src/handlers/session.rs apps/mrd-service/src/handlers/transport.rs apps/mrd-service/src/ipc_server.rs crates/mrd-application/src/lib.rs apps/mrd-service/tests/ipc_session_flow.rs
git commit -m "feat: move session control into mrd-service handlers"
```

### Task 4: Make service snapshots authoritative

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\handlers\telemetry.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\ipc_server.rs`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-session\src\lib.rs`
- Test: `G:\Project\mini-remote-desktop\apps\mrd-service\tests\snapshot_semantics.rs`

**Step 1: Write failing tests for snapshot semantics**

Cover:
- controller vs agent role
- created vs listening vs connecting vs connected
- `last_error`
- runtime/probe snapshot fetching through IPC

**Step 2: Run the snapshot tests to verify they fail**

Run: `cargo test -p mrd-service --test snapshot_semantics`
Expected: FAIL because snapshot generation still fabricates role/state and does not expose all required snapshot types.

**Step 3: Move role/state semantics into service/domain state**

Stop inferring role and lifecycle state from “presence of bootstrap fields”.

Add explicit lifecycle fields in service/domain-facing state, for example:

```rust
pub enum SessionLifecycleState {
    Created,
    Listening,
    Connecting,
    Connected,
    Streaming,
    Failed { message: String },
    Closed,
}
```

**Step 4: Make IPC snapshots a projection of service-owned truth**

`ipc_server` should project domain/service runtime state into DTOs without inventing semantics in the handler itself.

**Step 5: Run snapshot tests again**

Run: `cargo test -p mrd-service --test snapshot_semantics`
Expected: PASS

**Step 6: Commit**

```bash
git add apps/mrd-service/src/handlers/telemetry.rs apps/mrd-service/src/ipc_server.rs crates/mrd-session/src/lib.rs apps/mrd-service/tests/snapshot_semantics.rs
git commit -m "feat: make service snapshots authoritative"
```

### Task 5: Switch `Rdesk` commands to IPC-only

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\ipc_client.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\service_manager.rs`
- Test: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write failing command tests for IPC-backed flows**

Add tests for:
- `ipc_start_session`
- `ipc_accept_session`
- `ipc_session_snapshot`
- `ipc_start_sender`
- `ipc_start_receiver`
- `ipc_stop_session`

The tests should assert command behavior through IPC and not through direct host/session access.

**Step 2: Run the command tests to verify they fail**

Run: `cargo test -p Rdesk ipc_`
Expected: FAIL or require new tests because the shell still mixes IPC and direct runtime ownership.

**Step 3: Replace old command handlers with IPC-backed handlers**

Every session/signaling/transport control command still exposed to the frontend must call `mrd-ipc`.

Do not add adapter shims that route back into old shell-owned runtime.

**Step 4: Fix service control commands to use shared manager state**

Keep a single `ServiceManager` in `AppState` and return actual results from:
- `service_status`
- `service_pid`
- `service_start`
- `service_stop`
- `service_restart`

**Step 5: Run the command tests again**

Run: `cargo test -p Rdesk ipc_`
Expected: PASS for the IPC-backed command coverage.

**Step 6: Commit**

```bash
git add apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src-tauri/src/ipc_client.rs apps/Rdesk/src-tauri/src/service_manager.rs
git commit -m "refactor: switch Rdesk session commands to IPC only"
```

### Task 6: Delete the old shell-owned mainline

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`
- Delete or stop importing direct runtime modules from `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\`
- Test: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write a failing architectural guard**

Add a test or static assertion strategy that fails if `AppState` still owns:
- `RealtimeRuntime`
- `WebrtcHost`
- `QuicHost`
- `WebrtcSessionCoordinator`
- `QuicSessionCoordinator`

If no clean compile-time guard exists, use a grep-style verification step in CI/manual verification instead.

**Step 2: Remove old AppState ownership**

Delete those fields from `Rdesk::AppState`.

**Step 3: Remove old direct command helpers and registrations**

Delete the old session/signaling/transport control code paths from `main.rs`.

Expected retained shell state:

```rust
struct AppState {
    service_manager: Arc<ServiceManager>,
    service_client: ServiceClient,
    render_windows: Arc<Mutex<RenderWindowRegistry>>,
    settings_path: PathBuf,
}
```

**Step 4: Rebuild and fix fallout**

Run: `cargo check -p Rdesk`
Expected: compile errors from dead imports/usages

Remove dead modules/imports/usages until `Rdesk` compiles as a shell-only app.

**Step 5: Run the shell build/tests**

Run: `cargo test -p Rdesk`
Expected: PASS

**Step 6: Commit**

```bash
git add apps/Rdesk/src-tauri/src/main.rs apps/Rdesk/src-tauri/src/*.rs
git commit -m "refactor: remove old in-process shell runtime"
```

### Task 7: Add hard-cut regression coverage

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\tests\hard_cut_smoke.rs`
- Create: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\tests\ipc_shell_smoke.rs`
- Modify: `G:\Project\mini-remote-desktop\docs\plans\2026-03-20-rdesk-hard-cut-service-migration-design.md`

**Step 1: Write a failing local smoke test**

Cover this minimum flow:
- start service
- register/list devices
- start session
- accept session
- fetch snapshot
- stop session

All control calls must go through IPC.

**Step 2: Run the smoke tests to verify they fail**

Run: `cargo test -p mrd-service --test hard_cut_smoke`
Run: `cargo test -p Rdesk --test ipc_shell_smoke`
Expected: FAIL before all wiring is complete.

**Step 3: Implement the missing glue**

Fill any remaining gaps exposed by the smoke tests without reintroducing direct shell ownership.

**Step 4: Run all targeted migration tests**

Run:

```bash
cargo test -p mrd-ipc -p mrd-session -p mrd-application
cargo test -p mrd-service
cargo test -p Rdesk
```

Expected: PASS

**Step 5: Update the design doc with actual completion notes if needed**

Only adjust the design doc if implementation changed a real architectural choice.

**Step 6: Commit**

```bash
git add apps/mrd-service/tests/hard_cut_smoke.rs apps/Rdesk/src-tauri/tests/ipc_shell_smoke.rs docs/plans/2026-03-20-rdesk-hard-cut-service-migration-design.md
git commit -m "test: add hard-cut migration regression coverage"
```

### Task 8: Final verification and cleanup

**Files:**
- Modify only as needed based on verification fallout

**Step 1: Run full relevant verification**

Run:

```bash
cargo test -p mrd-ipc -p mrd-session -p mrd-application
cargo test -p mrd-service
cargo test -p Rdesk
git grep -n "RealtimeRuntime\\|WebrtcHost\\|QuicHost\\|WebrtcSessionCoordinator\\|QuicSessionCoordinator" -- apps/Rdesk/src-tauri/src
```

Expected:
- all targeted tests pass
- `git grep` returns no remaining shell-owned runtime usage, except comments/tests explicitly marked as historical if any

**Step 2: Run an end-to-end manual smoke check**

1. start `mrd-service`
2. call shell IPC-backed commands
3. verify service lifecycle commands report real status
4. verify runtime snapshot is fetched through IPC

**Step 3: Fix any final regressions**

Do not restore direct shell fallback to make a test pass.

**Step 4: Commit**

```bash
git add .
git commit -m "chore: finalize hard-cut service migration"
```
