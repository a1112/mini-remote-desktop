# Display Mode and Aspect Adaptation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add non-destructive Windows LAN display-mode control and make the receiver UI/rendering path adapt to non-16:9 remote sources.

**Architecture:** The peer's display mode is treated as negotiated session state rather than a media error. `mrd-service` owns display mode enumeration/change/restore and exposes it through IPC plus LAN packets; Rdesk remains a thin shell that requests a temporary mode for tests and renders selected profiles with the correct aspect ratio. Canary scripts may temporarily switch the peer to a requested profile and must restore the original mode after each row.

**Tech Stack:** Rust IPC/service code, Windows display APIs behind a service adapter, Tauri commands, React/Vitest, PowerShell benchmark scripts.

---

### Task 1: IPC Types and Service State

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `apps/mrd-service/src/app_state.rs`

**Steps:**
1. Add failing Rust tests that require `DisplayMode`, `DisplayModeChange`, and restore state to round-trip through IPC serialization and service state.
2. Add minimal structs and IPC request/response variants.
3. Add a per-session display mode registry for requested/current/original mode.
4. Run `cargo test -p mrd-ipc` and focused `mrd-service` tests.

### Task 2: Local and LAN Display Mode Control

**Files:**
- Create or modify: `apps/mrd-service/src/display_mode.rs`
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify: `apps/mrd-service/src/ipc_server.rs`

**Steps:**
1. Add failing tests for mode selection, temporary restore bookkeeping, and LAN request/ack classification.
2. Implement Windows mode enumeration/change/restore behind a small adapter; non-Windows returns unsupported.
3. Add LAN packets for list/set/restore so the controller can request changes on the peer.
4. Keep failures classified as unsupported or restore failed, not media failure.

### Task 3: Receiver Aspect Adaptation

**Files:**
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Steps:**
1. Add failing Vitest coverage for 16:10 selected profiles rendering without forced `aspect-video`.
2. Add render fit mode state: fit, fill, original.
3. Derive preview/native surface aspect from negotiated selected profile or probe dimensions.
4. Verify no stretching and no layout overflow.

### Task 4: Canary Integration

**Files:**
- Modify: `tests/benchmarks/scripts/run_paired_lan_canary.ps1`
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.ts`
- Modify: related tests under `apps/Rdesk/src/app/services` and `apps/Rdesk/src-tauri`

**Steps:**
1. Add failing tests for `displayModePolicy=temporary` propagation.
2. Add URL/env plumbing for display mode policy.
3. Before each cross-device row, request peer display mode if supported; after row, restore.
4. Report `display_mode_changed`, `display_mode_unsupported`, and `display_mode_restore_failed`.

### Task 5: Verification

**Commands:**
- `cargo fmt --all -- --check`
- `cargo test -p mrd-service display_mode -- --nocapture`
- `cargo test -p app lan_e2e_autorun_route_uses_env_configuration -- --nocapture`
- `pnpm test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/services/lanE2eAutomationService.test.ts`
- `cargo build -p app -p mrd-service`
- Paired LAN canary with `-DisplayModePolicy temporary`.

**Acceptance:**
- A 16:10 peer can either be rendered correctly as 16:10 or temporarily switched to exact 16:9 for strict parity tests.
- Strict profile failures are separated from display mode limitations.
- Test runs restore the peer's original display mode.
