# Rdesk Legacy Harness Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Remove the legacy direct-control runtime from the `app` binary by extracting it into a separate `rdesk-legacy-harness` package while keeping shell-only behavior in `apps/Rdesk/src-tauri`.

**Architecture:** The extraction is mechanical, not a redesign. `app` keeps shell/service/IPC/render responsibilities only. The old QUIC/WebRTC/realtime runtime, session coordinators, benchmark helpers, and legacy integration tests move into a new workspace package that exists only for validation/reference until it can be retired.

**Tech Stack:** Rust workspace packages, Tauri shell crate (`app`), shared local crates (`mrd-*`), workspace tests, existing legacy runtime modules.

---

### Task 1: Create the legacy harness package

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\Cargo.toml`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\lib.rs`
- Modify: `G:\Project\mini-remote-desktop\Cargo.toml`

**Step 1: Write the failing workspace expectation**

Decide the new package name: `rdesk-legacy-harness`.

Add it to the workspace members in the root `Cargo.toml`, but do not create all sources yet.

**Step 2: Run metadata to verify the package is missing**

Run: `cargo metadata --no-deps --format-version 1`
Expected: FAIL or package path issues until the new package files exist.

**Step 3: Create the new package skeleton**

Use a minimal library crate:

```toml
[package]
name = "rdesk-legacy-harness"
version = "0.1.0"
edition = "2021"

[dependencies]
```

and:

```rust
pub fn package_marker() -> &'static str {
    "rdesk-legacy-harness"
}
```

**Step 4: Run metadata again**

Run: `cargo metadata --no-deps --format-version 1`
Expected: PASS and the new package appears in workspace members.

**Step 5: Commit**

```bash
git add Cargo.toml apps/rdesk-legacy-harness/Cargo.toml apps/rdesk-legacy-harness/src/lib.rs
git commit -m "feat: add legacy harness workspace package"
```

### Task 2: Move legacy runtime modules out of `app`

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\benchmark.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\quic_host.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\quic_session.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\realtime_client.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\realtime_runtime.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\session_lifecycle.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\session_runtime.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\webrtc_host.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\webrtc_media.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\webrtc_session.rs`
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\quic_transport_harness.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\src\lib.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\Cargo.toml`
- Modify: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\Cargo.toml`

**Step 1: Write a failing compile check for `app` after removing legacy modules**

Remove one legacy `mod` declaration temporarily in `main.rs` and confirm it breaks current tests/helpers, proving the old code is still structurally coupled.

Run: `cargo check -p app`
Expected: FAIL until the moved code has a new home.

**Step 2: Copy the legacy modules into the new package**

Move them as-is first. Do not refactor internals.

Expose them from `lib.rs`:

```rust
pub mod benchmark;
pub mod quic_host;
pub mod quic_session;
pub mod realtime_client;
pub mod realtime_runtime;
pub mod session_lifecycle;
pub mod session_runtime;
pub mod webrtc_host;
pub mod webrtc_media;
pub mod webrtc_session;
pub mod quic_transport_harness;
```

**Step 3: Port the package dependencies**

Bring over the dependencies currently required by those modules from `apps/Rdesk/src-tauri/Cargo.toml`.

**Step 4: Run harness compile check**

Run: `cargo check -p rdesk-legacy-harness`
Expected: FAIL initially on missing imports or path assumptions.

**Step 5: Fix imports/paths until harness compiles**

Keep changes mechanical. Prefer updating module paths over redesigning code.

**Step 6: Commit**

```bash
git add apps/rdesk-legacy-harness apps/Rdesk/src-tauri/Cargo.toml
git commit -m "refactor: move legacy runtime modules into harness package"
```

### Task 3: Move legacy tests and benchmark helpers out of `main.rs`

**Files:**
- Create: `G:\Project\mini-remote-desktop\apps\rdesk-legacy-harness\tests\legacy_runtime.rs`
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Write the failing harness test target**

Create `legacy_runtime.rs` and migrate one representative legacy test from the giant `#[cfg(test)] mod tests` block in `main.rs`.

**Step 2: Run the new test**

Run: `cargo test -p rdesk-legacy-harness --test legacy_runtime`
Expected: FAIL until helper imports and module visibility are corrected.

**Step 3: Move the remaining legacy tests in batches**

Prioritize:
- direct realtime/session flow tests
- legacy QUIC/WebRTC integration tests
- benchmark tests that still depend on old direct runtime helpers

**Step 4: Remove migrated test code from `main.rs`**

Delete only the moved tests from `app`; leave shell-only tests in place.

**Step 5: Run the harness tests again**

Run: `cargo test -p rdesk-legacy-harness`
Expected: PASS for migrated tests.

**Step 6: Commit**

```bash
git add apps/rdesk-legacy-harness/tests/legacy_runtime.rs apps/Rdesk/src-tauri/src/main.rs
git commit -m "test: move legacy runtime tests into harness package"
```

### Task 4: Strip legacy module declarations from `app`

**Files:**
- Modify: `G:\Project\mini-remote-desktop\apps\Rdesk\src-tauri\src\main.rs`

**Step 1: Remove global dead-code suppression**

Delete:

```rust
#![allow(dead_code)]
```

**Step 2: Remove legacy `mod` declarations**

Delete:
- `mod benchmark;`
- `mod quic_host;`
- `mod quic_session;`
- `mod realtime_client;`
- `mod realtime_runtime;`
- `mod session_lifecycle;`
- `mod session_runtime;`
- `mod webrtc_host;`
- `mod webrtc_media;`
- `mod webrtc_session;`
- `mod quic_transport_harness;`

Keep only shell-necessary modules.

**Step 3: Remove legacy `use` items**

Delete imports for:
- `QuicHost`
- `QuicSessionCoordinator`
- `RealtimeRuntime`
- `SessionLifecycleCoordinator`
- `WebrtcHost`
- `WebrtcSessionCoordinator`
- their snapshot/helper types if no longer used by shell commands

**Step 4: Run shell compile check**

Run: `cargo check -p app`
Expected: FAIL on remaining references to moved helpers.

**Step 5: Remove remaining shell references to moved code**

Delete or replace any leftover helper functions that only existed to support the legacy direct runtime.

**Step 6: Run shell compile check again**

Run: `cargo check -p app`
Expected: PASS

**Step 7: Commit**

```bash
git add apps/Rdesk/src-tauri/src/main.rs
git commit -m "refactor: remove legacy runtime from shell binary"
```

### Task 5: Verify shell-only boundaries

**Files:**
- Modify only if verification reveals fallout

**Step 1: Run package-level checks**

Run:

```bash
cargo check -p app
cargo test -p app
cargo check -p rdesk-legacy-harness
cargo test -p rdesk-legacy-harness
```

Expected: PASS

**Step 2: Run grep-based architecture verification**

Run:

```bash
git grep -n "mod quic_host\\|mod realtime_runtime\\|mod webrtc_host\\|mod quic_session\\|mod webrtc_session\\|mod session_lifecycle\\|mod session_runtime\\|mod benchmark" -- apps/Rdesk/src-tauri/src
```

Expected: no matches in `apps/Rdesk/src-tauri/src/main.rs` for the extracted legacy modules.

Run:

```bash
git grep -n "#!\\[allow(dead_code)\\]" -- apps/Rdesk/src-tauri/src
```

Expected: no matches in the shell package root.

**Step 3: Fix any verification fallout**

Do not reintroduce dead-code suppression or move legacy modules back into `app`.

**Step 4: Commit**

```bash
git add .
git commit -m "chore: finalize legacy harness extraction"
```
