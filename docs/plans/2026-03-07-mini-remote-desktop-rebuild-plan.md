# Mini Remote Desktop Rebuild Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rebuild `mini-remote-desktop` into a clean product workspace with `apps/Rdesk`, `apps/Rdesk-Server`, `apps/realtime-server`, shared `crates/`, `labs/GPUTest`, and `junk/`.

**Architecture:** Use `G:\Project\mini-remote-desktop` as the mainline baseline. Recovered code from `G:\修复\ProjectTest\remote-desktop\mini-remote-desktop` is reference-only. Rebuild proceeds by milestone: structure first, then shared crates, then `Rdesk` host, then realtime/service closure. `GPUTest` only validates shared capabilities.

**Tech Stack:** Rust, Tauri, FastAPI/Python, WebRTC, QUIC, FFmpeg, OpenH264, D3D11, PowerShell.

---

### Task 1: Inventory and freeze the baseline

**Files:**
- Modify: `README.md`
- Create: `docs/plans/2026-03-07-baseline-inventory.md`

**Step 1: Write the failing inventory checklist**

Document the expected current roots:

- `Rdesk`
- `Rdesk-Server`
- `agent-rust`
- `controller-rust`
- `signaling-rs`
- `web`
- `web-client`
- `server`
- `common-control-proto`
- `heartbeat-rs`

**Step 2: Verify the baseline tree**

Run: `Get-ChildItem G:\Project\mini-remote-desktop`

Expected: all current top-level directories are listed and no move has started yet.

**Step 3: Record recovered reference roots**

Document reference-only sources:

- `G:\修复\ProjectTest\remote-desktop\mini-remote-desktop`
- `G:\修复\ProjectTest\remote-desktop\mini-remote-desktop\worktrees\layered-core-migration`

**Step 4: Commit**

```bash
git add README.md docs/plans/2026-03-07-baseline-inventory.md
git commit -m "docs(rebuild): record baseline and recovery sources"
```

### Task 2: Create the new top-level mainline layout

**Files:**
- Create: `apps/.gitkeep`
- Create: `crates/.gitkeep`
- Create: `labs/.gitkeep`
- Create: `junk/.gitkeep`
- Modify: `README.md`

**Step 1: Write the failing structure checklist**

List required target roots:

- `apps/`
- `crates/`
- `labs/`
- `junk/`

**Step 2: Create the target roots**

Create the directories with placeholder files.

**Step 3: Update repository overview**

State that the product mainline is moving to:

- `apps/Rdesk`
- `apps/Rdesk-Server`
- `apps/realtime-server`

**Step 4: Verify**

Run: `Get-ChildItem G:\Project\mini-remote-desktop`

Expected: new target roots exist.

**Step 5: Commit**

```bash
git add apps crates labs junk README.md
git commit -m "chore(rebuild): add new top-level workspace layout"
```

### Task 3: Move product entry points into `apps/`

**Files:**
- Move: `Rdesk` -> `apps/Rdesk`
- Move: `Rdesk-Server` -> `apps/Rdesk-Server`
- Modify: `README.md`
- Modify: `apps/Rdesk/package.json`
- Modify: `apps/Rdesk/src-tauri/tauri.conf.json`
- Modify: `apps/Rdesk-Server/README.md`

**Step 1: Write path-failure checks**

Verify current commands fail against the new paths before the move.

**Step 2: Move the directories**

Move only the current product entry points.

**Step 3: Fix path-dependent configs**

Update any path assumptions in:

- Tauri config
- package scripts
- service docs

**Step 4: Verify**

Run:

- `Get-ChildItem G:\Project\mini-remote-desktop\apps`
- `Test-Path G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri`
- `Test-Path G:\Project\mini-remote-desktop\apps\Rdesk-Server\app`

Expected: all true.

**Step 5: Commit**

```bash
git add apps README.md
git commit -m "refactor(rebuild): move product entrypoints under apps"
```

### Task 4: Rebuild the shared crate workspace under `crates/`

**Files:**
- Create: `Cargo.toml`
- Create: `crates/mrd-proto/...`
- Create: `crates/mrd-session/...`
- Create: `crates/mrd-signal-proto/...`
- Create: `crates/mrd-signal-client/...`
- Create: `crates/mrd-signal-server/...`
- Create: `crates/mrd-pipeline-core/...`
- Create: `crates/mrd-render/...`
- Create: `crates/mrd-render-d3d11/...`
- Create: `crates/mrd-decode/...`
- Create: `crates/mrd-encode/...`
- Create: `crates/mrd-transport-quic/...`
- Create: `crates/mrd-transport-webrtc/...`
- Create: `crates/mrd-capture-d3d11dup/...`

**Step 1: Write failing workspace checks**

Run: `cargo metadata --manifest-path G:\Project\mini-remote-desktop\Cargo.toml`

Expected: fail before workspace file exists.

**Step 2: Recreate workspace membership**

Use recovered `layered-core-migration` structure as reference; rebuild only the intended mainline crates.

**Step 3: Add crate-level smoke tests**

At minimum:

- protocol encode/decode
- session plan construction
- signaling envelope parsing
- renderer registry
- decoder/encoder creation

**Step 4: Verify**

Run targeted crate tests after each crate is restored.

**Step 5: Commit**

```bash
git add Cargo.toml crates
git commit -m "feat(rebuild): restore shared crate workspace"
```

### Task 5: Move `GPUTest` into `labs/` and narrow its responsibility

**Files:**
- Move: `GPUTest` -> `labs/GPUTest`
- Modify: `labs/GPUTest/README.md`
- Modify: `labs/GPUTest/Cargo.toml`

**Step 1: Write the verification-only contract**

Document that `GPUTest`:

- validates shared crates
- does not define product structure
- does not own product entry points

**Step 2: Move the project**

Move `GPUTest` under `labs/`.

**Step 3: Rewire references**

Point it at the rebuilt `crates/` workspace as appropriate.

**Step 4: Verify**

Run one minimal check per restored shared capability.

**Step 5: Commit**

```bash
git add labs/GPUTest
git commit -m "refactor(rebuild): move gputest into labs verification role"
```

### Task 6: Move historical projects into `junk/`

**Files:**
- Move: `agent-rust` -> `junk/agent-rust`
- Move: `controller-rust` -> `junk/controller-rust`
- Move: `signaling-rs` -> `junk/signaling-rs`
- Move: `web` -> `junk/web`
- Move: `web-client` -> `junk/web-client`
- Move: `server` -> `junk/server`
- Move: `agent-python` -> `junk/agent-python`
- Move: `client-qt` -> `junk/client-qt`
- Modify: `README.md`

**Step 1: Write the exclusion checklist**

Document that these are not the mainline runtime anymore.

**Step 2: Move the directories**

Move them intact; do not refactor contents in this task.

**Step 3: Update docs**

Mark them as historical/reference-only.

**Step 4: Verify**

Run: `Get-ChildItem G:\Project\mini-remote-desktop\junk`

Expected: all historical projects are present.

**Step 5: Commit**

```bash
git add junk README.md
git commit -m "refactor(rebuild): move historical projects into junk"
```

### Task 7: Restore `apps/realtime-server` as the sidecar product

**Files:**
- Create: `apps/realtime-server/Cargo.toml`
- Create: `apps/realtime-server/src/main.rs`
- Modify: `apps/Rdesk-Server/app/api/v1/realtime.py`
- Modify: `apps/Rdesk-Server/app/core/config.py`

**Step 1: Write failing sidecar host tests**

Expected behaviors:

- `/health`
- `/ws`
- session-routed signaling

**Step 2: Rebuild the Rust sidecar app**

Use the recovered realtime-server implementation as reference only.

**Step 3: Reconnect FastAPI management**

Update management API to point at `apps/realtime-server`.

**Step 4: Verify**

Run:

- sidecar tests
- FastAPI realtime tests

**Step 5: Commit**

```bash
git add apps/realtime-server apps/Rdesk-Server
git commit -m "feat(rebuild): restore realtime sidecar product app"
```

### Task 8: Restore `apps/Rdesk` runtime host and render shell

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Modify: `apps/Rdesk/src-tauri/src/session_manager.rs`
- Modify: `apps/Rdesk/src-tauri/src/runtime_host.rs`
- Modify: `apps/Rdesk/src-tauri/src/pipeline_host.rs`
- Modify: `apps/Rdesk/src-tauri/src/realtime_client.rs`
- Modify: `apps/Rdesk/src-tauri/src/realtime_management.rs`
- Modify: `apps/Rdesk/src/...`

**Step 1: Write failing host tests**

Minimum expected behavior:

- session lifecycle
- realtime management status/start/stop/restart
- render window attach
- decoded frame routing

**Step 2: Rebuild the host layer**

Restore only the clean architecture:

- session manager
- runtime host
- pipeline host
- render tick scheduler
- realtime client
- management client

**Step 3: Rebuild the minimal front-end shell**

Restore enough UI to drive host commands and open render windows.

**Step 4: Verify**

Run:

- `cargo test -p app`
- frontend build/dev smoke checks

**Step 5: Commit**

```bash
git add apps/Rdesk
git commit -m "feat(rebuild): restore rdesk host and render shell"
```

### Task 9: Restore the minimal real media path

**Files:**
- Modify: `crates/mrd-render/...`
- Modify: `crates/mrd-render-d3d11/...`
- Modify: `crates/mrd-decode/...`
- Modify: `crates/mrd-encode/...`
- Modify: `apps/Rdesk/src-tauri/...`
- Modify: `labs/GPUTest/...`

**Step 1: Write failing media-path tests**

Expected path:

- encoded frame
- decode
- decoded frame route
- render upload
- present

**Step 2: Restore render first**

Bring back:

- D3D11 attach
- clear/present fallback
- BGRA upload

**Step 3: Restore decode and encode**

Bring back:

- FFmpeg-backed H264 software decode
- OpenH264 software encode

**Step 4: Reattach runtime path**

Reconnect the minimal media path into `apps/Rdesk`.

**Step 5: Verify**

Run:

- crate tests
- host tests
- `labs/GPUTest` validation tests

**Step 6: Commit**

```bash
git add crates apps/Rdesk labs/GPUTest
git commit -m "feat(rebuild): restore minimal real media pipeline"
```

### Task 10: Restore transport closure and server/client runtime integration

**Files:**
- Modify: `crates/mrd-transport-quic/...`
- Modify: `crates/mrd-transport-webrtc/...`
- Modify: `apps/Rdesk/src-tauri/...`
- Modify: `apps/realtime-server/...`
- Modify: `apps/Rdesk-Server/...`

**Step 1: Write failing ingress tests**

Expected behavior:

- QUIC ingress delivers encoded frames to session sink
- WebRTC RTP ingress delivers H264 AU to session sink
- signaling routes by `sessionId`

**Step 2: Restore QUIC ingress**

Reconnect encoded-frame ingress to the runtime session sink.

**Step 3: Restore WebRTC ingress**

Restore:

- signaling offer/answer/ice flow
- track ingress adapter
- session coordinator

**Step 4: Verify**

Run:

- transport crate tests
- `apps/Rdesk` host tests
- `apps/realtime-server` signaling tests
- `apps/Rdesk-Server` unittest suite

**Step 5: Commit**

```bash
git add crates apps/Rdesk apps/realtime-server apps/Rdesk-Server
git commit -m "feat(rebuild): restore transport and realtime integration"
```

