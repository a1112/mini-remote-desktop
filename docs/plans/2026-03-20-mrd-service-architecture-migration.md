# mrd-service Architecture Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restructure `mini-remote-desktop` so `Rdesk` becomes a local shell and `mrd-service` becomes the only local session orchestration runtime.

**Architecture:** Introduce a local service app plus stable IPC, move session orchestration into `mrd-application`, and gradually migrate `Rdesk` away from direct transport/signaling/runtime ownership. Reuse existing QUIC/WebRTC/media adapters where possible and move boundaries before changing behavior.

**Tech Stack:** Rust workspace, Tauri, Tokio, existing `mrd-*` crates, QUIC Quinn, WebRTC, DXGI/NVENC/OpenH264, local IPC

---

### Task 1: Freeze the migration target in repository docs

**Files:**
- Create: `G:\Project\mini-remote-desktop\docs\plans\2026-03-20-mrd-service-architecture-migration-design.md`
- Modify: `G:\Project\mini-remote-desktop\README.md`
- Modify: `G:\Project\mini-remote-desktop\ARCHITECTURE.md`

**Step 1: Write the doc updates**

Add a concise repository-level statement that:

- `Rdesk` is becoming a shell
- `mrd-service` is the future mainline
- media/transport crates remain infrastructure adapters

**Step 2: Verify docs render sensibly**

Run: `Get-Content G:\Project\mini-remote-desktop\README.md -TotalCount 120`

Expected: top-level architecture notes mention `mrd-service`.

**Step 3: Commit**

```bash
git -C G:\Project\mini-remote-desktop add README.md ARCHITECTURE.md docs/plans/2026-03-20-mrd-service-architecture-migration-design.md
git -C G:\Project\mini-remote-desktop commit -m "docs: define mrd-service target architecture"
```

### Task 2: Add the new workspace crates and service shell

**Files:**
- Modify: `G:\Project\mini-remote-desktop\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\main.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-ipc\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-ipc\src\lib.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-application\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-application\src\lib.rs`

**Step 1: Add failing workspace members**

Update workspace members first and create minimal crate manifests.

**Step 2: Run workspace check to verify it fails usefully**

Run: `cargo check --workspace`

Expected: FAIL until stub source files exist.

**Step 3: Add minimal compiling stubs**

Implement:

- `apps/mrd-service/src/main.rs` with a basic Tokio runtime entrypoint
- `mrd-ipc` with request/response enums
- `mrd-application` with placeholder use case module exports

**Step 4: Run workspace check**

Run: `cargo check --workspace`

Expected: PASS

**Step 5: Commit**

```bash
git -C G:\Project\mini-remote-desktop add Cargo.toml apps/mrd-service crates/mrd-ipc crates/mrd-application
git -C G:\Project\mini-remote-desktop commit -m "feat: add mrd-service and application workspace scaffolding"
```

### Task 3: Define stable local IPC contracts

**Files:**
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-ipc\src\lib.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-ipc\tests\contracts.rs`

**Step 1: Write failing contract tests**

Define tests for serializing/deserializing commands such as:

- `StartSession`
- `AcceptSession`
- `SessionRuntimeSnapshot`
- `StartSender`
- `StopSession`

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-ipc`

Expected: FAIL until contracts exist.

**Step 3: Implement minimal IPC DTOs**

Add:

- command enum
- response enum
- session snapshot DTO
- transport/bootstrap DTO

Use serde and keep the contract independent from Tauri types.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-ipc`

Expected: PASS

**Step 5: Commit**

```bash
git -C G:\Project\mini-remote-desktop add crates/mrd-ipc
git -C G:\Project\mini-remote-desktop commit -m "feat: define local ipc contracts"
```

### Task 4: Move session orchestration out of Tauri main

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-application\src\usecases\start_session.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-application\src\usecases\accept_session.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-application\src\usecases\sync_runtime.rs`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-application\src\lib.rs`

**Step 1: Write a failing integration-style test around orchestration**

Target current helpers such as:

- `apply_realtime_events_to_session_coordinators`
- `prepare_quic_accept_with`
- `sync_quic_host_from_session_snapshot_with`

Move behavior expectations into a test under the new application crate.

**Step 2: Run targeted tests**

Run: `cargo test -p apps-Rdesk-src-tauri` or equivalent targeted module tests if extraction is incremental

Expected: FAIL or compile-break until orchestration is extracted.

**Step 3: Extract orchestration**

Introduce port traits in `mrd-application` and move the orchestration logic into use case modules.

`main.rs` should call those use cases instead of owning the logic.

**Step 4: Re-run tests**

Run:

```bash
cargo test -p mrd-application
cargo check -p app
```

Expected: PASS

**Step 5: Commit**

```bash
git -C G:\Project\mini-remote-desktop add apps/Rdesk/src-tauri/src/main.rs crates/mrd-application
git -C G:\Project\mini-remote-desktop commit -m "refactor: extract session orchestration into application layer"
```

### Task 5: Move QUIC session metadata into the session domain

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\quic_session.rs`
- Modify: `G:\Project\mini-remote-desktop\crates\mrd-session\src\lib.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-session\src\quic.rs`
- Create: `G:\Project\mini-remote-desktop\crates\mrd-session\tests\session_runtime.rs`

**Step 1: Write the failing domain test**

Test that a session aggregate can record:

- source/target device ids
- transport kind
- local bootstrap
- remote bootstrap

without depending on Tauri or Quinn concrete types.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-session`

Expected: FAIL

**Step 3: Implement domain state**

Move or mirror `QuicSessionSnapshot` semantics into domain types inside `mrd-session`.

Update `quic_session.rs` to become a temporary adapter or to call into the domain crate.

**Step 4: Run test**

Run: `cargo test -p mrd-session`

Expected: PASS

**Step 5: Commit**

```bash
git -C G:\Project\mini-remote-desktop add apps/Rdesk/src-tauri/src/quic_session.rs crates/mrd-session
git -C G:\Project\mini-remote-desktop commit -m "refactor: move quic session state into domain layer"
```

### Task 6: Introduce `mrd-service` IPC server and wire one end-to-end use case

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\mrd-service\src\main.rs`
- Create: `G:\Project\mini-remote-desktop\apps\mrd-service\src\ipc_server.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Implement a single vertical slice**

Choose one use case first:

- `session_runtime_snapshot`

The shell should call IPC; the service should answer from application-layer state.

**Step 2: Run targeted verification**

Run:

```bash
cargo check -p mrd-service
cargo check -p app
```

Expected: PASS

**Step 3: Add a smoke test if practical**

Test shell-side serialization and service-side handler invocation with an in-process stub.

**Step 4: Commit**

```bash
git -C G:\Project\mini-remote-desktop add apps/mrd-service apps/Rdesk/src-tauri/src/main.rs
git -C G:\Project\mini-remote-desktop commit -m "feat: add first shell-to-service ipc flow"
```

### Task 7: Migrate remaining Tauri commands off direct runtime ownership

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`
- Create/Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\commands\*.rs`
- Create/Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\state\*.rs`

**Step 1: Move commands into thin wrappers**

Each command should:

- deserialize UI input
- call IPC client
- map response DTOs

It should not touch `QuicHost`, `RealtimeRuntime`, `WebrtcHost`, or session coordinators directly.

**Step 2: Run compile**

Run: `cargo check -p app`

Expected: PASS

**Step 3: Commit**

```bash
git -C G:\Project\mini-remote-desktop add apps/Rdesk/src-tauri/src
git -C G:\Project\mini-remote-desktop commit -m "refactor: thin tauri shell over local service"
```

### Task 8: Add regression tests around multi-session orchestration

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`
- Create: `G:\Project\mini-remote-desktop\tests\integration\multi_session_runtime.rs`

**Step 1: Add failing regression tests**

Cover:

- multiple realtime events in one drain cycle
- session reconnect / listener reset
- accept/connect failures surfacing into snapshots or responses

**Step 2: Run tests**

Run: `cargo test --workspace`

Expected: FAIL until gaps are fixed.

**Step 3: Fix regression behavior in the new architecture**

Implement the smallest changes necessary so:

- errors are surfaced
- stale session state is cleared
- multi-session sync is not silently collapsed to the last session

**Step 4: Re-run tests**

Run: `cargo test --workspace`

Expected: PASS

**Step 5: Commit**

```bash
git -C G:\Project\mini-remote-desktop add tests apps/Rdesk/src-tauri/src/main.rs apps/mrd-service crates/mrd-application crates/mrd-session
git -C G:\Project\mini-remote-desktop commit -m "test: add multi-session migration regressions"
```
