# Input Capture and Injection Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build an end-to-end keyboard and mouse capture/injection path for remote desktop sessions.

**Architecture:** Keep the service as the owner of input injection. Reuse `common-control-proto` for wire events and lane selection, add a platform-neutral `mrd-input` crate with Windows `SendInput`, expose service IPC and capability state, then wire UI/native capture into the service path.

**Tech Stack:** Rust workspace crates, Windows `SendInput`, Tauri commands, React/Vitest, existing `common-control-proto` and `mrd-ipc` contracts.

---

### Task 1: Lock Down Control Lane Semantics

**Files:**
- Modify: `common-control-proto/src/lib.rs`

**Step 1: Write failing tests**

Add tests proving:

- `MouseMove` and `MouseWheel` use `ChannelClass::Realtime`
- `MouseButton` and `Key` use `ChannelClass::Reliable`

**Step 2: Run test to verify it fails or proves existing behavior**

Run:

```powershell
cargo test -p common-control-proto control_events_use_expected_channel_classes -- --nocapture
```

Expected: the new test exists and fails only if lane policy is wrong. If it passes immediately because behavior already exists, keep it as a regression test and move on.

**Step 3: Implement minimal code if needed**

Adjust only `ControlEvent::channel_class()` if the test exposes a mismatch.

**Step 4: Verify**

Run:

```powershell
cargo test -p common-control-proto
```

### Task 2: Add `mrd-input` Crate

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/mrd-input/Cargo.toml`
- Create: `crates/mrd-input/src/lib.rs`

**Step 1: Write failing tests**

Add tests for:

- recording injector records keyboard, mouse button, move, and wheel events;
- `release_all()` emits key/button releases for active pressed inputs;
- unsupported injector reports unavailable and rejects injection.

**Step 2: Run red tests**

Run:

```powershell
cargo test -p mrd-input
```

Expected: package initially does not exist, then tests fail until implementation is added.

**Step 3: Implement minimal code**

Define:

- `InputEvent`
- `InputButton`
- `InputKey`
- `InputInjector`
- `RecordingInputInjector`
- `UnsupportedInputInjector`
- `TrackedInputInjector<I>`

Keep wire protocol conversion separate from platform injection.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-input
```

### Task 3: Add Windows SendInput Adapter

**Files:**
- Modify: `crates/mrd-input/Cargo.toml`
- Modify: `crates/mrd-input/src/lib.rs`
- Create: `crates/mrd-input/src/windows.rs`

**Step 1: Write failing tests**

Add unit tests for the pure mapping layer:

- left/right/middle mouse buttons map to down/up commands;
- wheel delta is preserved;
- printable key code or virtual key code maps to a key command;
- invalid or empty key input returns a validation error.

Do not require real desktop injection for normal unit tests.

**Step 2: Run red tests**

Run:

```powershell
cargo test -p mrd-input windows_mapping -- --nocapture
```

Expected: mapping API is missing until implementation.

**Step 3: Implement minimal code**

Behind `cfg(windows)`, implement a `WindowsSendInputInjector` using the `windows` crate. Keep the mapping functions testable without sending input.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-input
```

Optional manual Windows runtime smoke:

```powershell
cargo test -p mrd-input windows_sendinput_mouse_move_smoke_moves_and_restores_cursor -- --ignored --nocapture
cargo test -p mrd-service lan_control_input_sendinput_smoke_moves_cursor_through_udp_handler -- --ignored --nocapture
```

### Task 4: Extend IPC Contracts For Input Events

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `crates/mrd-ipc/tests/contracts.rs`

**Step 1: Write failing tests**

Add contract tests proving round-trip serialization for:

- `IpcRequest::SendControlInput`
- `IpcResponse::ControlInputAccepted`
- optional control injection error response through the existing error envelope.

**Step 2: Run red tests**

Run:

```powershell
cargo test -p mrd-ipc serialize_deserialize_control_input_contracts -- --nocapture
```

Expected: variants are missing.

**Step 3: Implement minimal code**

Add serializable IPC types for:

- session id;
- control event payload;
- selected lane or sequence if required by service;
- response counters if useful.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-ipc
```

### Task 5: Wire Service Capability And Injection State

**Files:**
- Modify: `apps/mrd-service/Cargo.toml`
- Modify: `apps/mrd-service/src/capabilities.rs`
- Modify: `apps/mrd-service/src/ipc_server.rs`
- Create or modify service input module under `apps/mrd-service/src/`

**Step 1: Write failing tests**

Add tests proving:

- `control.keyboard_mouse` is available when the injector probe is available;
- the same capability is unsupported/unavailable when the injector is unavailable;
- sending input updates service counters;
- sending input with unavailable injector returns an error and records the last error.

**Step 2: Run red tests**

Run:

```powershell
cargo test -p mrd-service control_input -- --nocapture
```

Expected: missing service control input path.

**Step 3: Implement minimal code**

Add service-owned input state with:

- injector availability probe;
- counters;
- last error;
- `release_all()` hook for cleanup;
- `IpcRequest::SendControlInput` handling.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-service capabilities control_input -- --nocapture
```

### Task 6: Add Frontend Tauri Adapter For Control Input

**Files:**
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `apps/Rdesk/src/app/adapters/tauri/commands.ts`
- Test: existing adapter tests if present, otherwise add focused test near adapter tests.

**Step 1: Write failing tests**

Test that `sendControlInput()` sends the expected command/bridge payload for key, mouse button, move, and wheel events.

**Step 2: Run red tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- src/app/adapters/tauri
```

Expected: new adapter is missing.

**Step 3: Implement minimal code**

Add typed frontend control event definitions and a command wrapper.

**Step 4: Verify**

Run the same frontend adapter test.

### Task 7: Capture Input In Remote Display UI

**Files:**
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Modify or add: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`

**Step 1: Write failing tests**

Add tests proving:

- render area is focusable when control is available;
- pointer move maps client coordinates into remote frame coordinates;
- mouse button down/up dispatch reliable button events;
- wheel dispatches wheel events;
- key down/up dispatch key events only while focused;
- blur releases active keys/buttons.

**Step 2: Run red tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- src/app/components/RemoteDisplayWindowPage.test.tsx
```

Expected: no dispatch/capture behavior exists.

**Step 3: Implement minimal code**

Wire event handlers into the render area and use the Tauri adapter. Avoid visible instructional UI text.

**Step 4: Verify**

Run the same test plus:

```powershell
pnpm --dir apps/Rdesk type-check
```

### Task 8: Forward Windows Native Surface Input

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/remote_display_surface.rs`
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Test: Rust unit tests for pure Windows message mapping helpers where possible.

**Step 1: Write failing tests**

Extract pure mapping helpers and test:

- mouse move coordinates;
- button down/up;
- wheel delta;
- key down/up;
- focus loss cleanup command.

**Step 2: Run red tests**

Run:

```powershell
cargo test -p app remote_display_surface_input -- --nocapture
```

Expected: helpers are missing.

**Step 3: Implement minimal code**

Forward Windows child HWND messages to the same control-input service path. Keep unsafe code narrow and keep unsupported platforms unchanged.

**Step 4: Verify**

Run:

```powershell
cargo test -p app remote_display_surface_input -- --nocapture
```

### Task 9: Transport Lane Follow-Up

**Files:**
- Modify as needed after inspecting active LAN/WebRTC control receivers.

**Step 1: Write focused tests**

Add tests proving reliable events are not routed through lossy lanes and realtime pointer movement can use lossy lanes.

**Step 2: Implement minimal routing**

Prefer typed lane naming:

- `ctrl_rel`
- `ctrl_rt`

Avoid breaking existing media reliable streams.

**Step 3: Verify**

Run relevant transport tests discovered during implementation.

### Task 10: Final Verification And Merge Readiness

Run:

```powershell
cargo fmt --check
cargo test -p common-control-proto
cargo test -p mrd-input
cargo test -p mrd-ipc
cargo test -p mrd-service
cargo test -p app remote_display_surface_input -- --nocapture
pnpm --dir apps/Rdesk test -- src/app/components/RemoteDisplayWindowPage.test.tsx
pnpm --dir apps/Rdesk type-check
git diff --check
```

Then inspect:

```powershell
git status --short --branch
```

Document any unsupported-platform limits in the final response.
