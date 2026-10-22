# Settings and Console Windows Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Finish usable core settings and prevent background diagnostic commands from creating Windows console windows.

**Architecture:** Rdesk retains UI preferences and window management. Existing IPC owns service health and lifecycle; no management permissions are bypassed. A shared settings component presents only supported actions, with separate loading/error state and protection against stale async results.

**Tech Stack:** React, TypeScript, Vitest, Tauri 2, Rust, Windows process creation flags.

User approved the recommended design on 2026-10-09. Use the managed settings-console-fix worktree. Independent tasks have disjoint file ownership; integrate and review in this session. Preserve the original workspace's unrelated untracked files. Keep reviewable local changes without automatic merge or deployment.

## Task 1: Persist native close behavior

Files: `apps/Rdesk/src-tauri/src/app_settings.rs`, `apps/Rdesk/src-tauri/src/main.rs`.

1. Add failing Rust tests for old settings defaulting to `hide_to_tray`, round-trip `exit_ui`, rejected invalid values, and concurrent updates preserving decode/FFmpeg settings.
2. Run `cargo test -p app --bin app app_settings` and verify the expected missing behavior.
3. Add serde-default `CloseBehavior`, a UI-preferences response and a serialized read/update/write helper. Add `get_ui_preferences` and `set_close_behavior` commands. Both the close command and native CloseRequested handler use the persisted decision; exit closes UI-owned display windows and detaches UI without stopping the service. Default remains tray hiding.
4. Route all app-settings writers, including FFmpeg reset/download and decode save, through the serialized update helper. Propagate save failures before reporting a confirmed state.
5. Rerun Rust tests and `cargo check -p app --bin app`.

## Task 2: Suppress proven background consoles

Files: `crates/mrd-ffmpeg/src/lib.rs`, `crates/mrd-decode/src/lib.rs`, `apps/Rdesk/src-tauri/src/resource_monitor.rs`, `apps/Rdesk/src-tauri/src/device_info.rs`, `apps/mrd-service/src/resource_monitor.rs`; a small reusable command helper under `crates/` if justified.

1. Add a failing Windows subprocess test: a diagnostic executable reports `GetConsoleWindow() == 0`, stdout/stderr remain captured, nonzero exit remains observable.
2. Run the test and record the failure before changing creation flags.
3. Apply `CREATE_NO_WINDOW` to confirmed FFmpeg/ffprobe, NVIDIA, WMIC and background decoder command paths without changing output or lifecycle behavior. Reuse an existing small helper if available; do not alter interactive terminal launchers or unrelated historical hardware code.
4. Rerun focused tests and check affected Rust packages. Observe repeated resource sampling and media probing for new console windows with the new binary.

## Task 3: Accurate adapters and service state

Files: `apps/Rdesk/src/app/adapters/tauri/{types,commands,index}.ts`, their tests, `apps/Rdesk/src/app/services/serviceLifecycleService{,.test}.ts`.

1. Add failing tests asserting `{running:true, healthy:false,pid:123}` remains unhealthy, endpoint absence becomes stopped, permission failures propagate, restart does not start after a failed stop, and UI-preference commands carry the expected arguments.
2. Run targeted Vitest tests to confirm the missing behavior.
3. Expose `ipc_service_health` via the existing adapter mechanism and one `getServiceHealth` query. Compatibility status/health/PID functions read that real response. Expose `getUiPreferences` and `setCloseBehavior`; unavailable web-bridge UI preference operations report unsupported rather than pretending to persist.
4. Rerun focused service and adapter tests.

## Task 4: Shared usable settings

Files: `apps/Rdesk/src/app/components/SettingsContent.tsx`, `SettingsModal.tsx`, `SettingsPage.tsx`, `AuthContext.tsx`, settings and auth tests.

1. Write failing user-visible tests for persisted close behavior, real unhealthy service state, unsupported controls, absence of fake network metrics, actual logout preserving preferences, independent FFmpeg/policy errors, failed-save readback, and ignoring old responses after reopen.
2. Run tests and verify expected failures.
3. Extract one shared content component with seven accessible categories. Modal handles dialog, Escape/overlay close, focus restoration and size; old page reuses content.
4. Keep ThemeContext persistence; save close behavior through native preferences. Autostart displays confirmed status and actual errors. Query real health once per refresh and use existing lifecycle commands; failed operations remain visible.
5. Display decode-policy migration as unsupported without a save/applied claim. Keep real FFmpeg probe/download/reset with mutual exclusion and independent errors. Remove constant speed results and no-op controls; explain unsupported categories concisely.
6. Account logout calls AuthContext logout and dispatches `rdesk-auth-changed`; do not unbind the device or stop service. Disable unsupported account actions. Use per-mounted-instance cancellation and request generation to guard async updates.
7. Update older tests to match display-category media settings and actual health command. Remove vacuous conditional assertions. Run targeted component/service/contract tests.

## Task 5: Integration and review

1. Run focused frontend tests, TypeScript checking, production Vite build and affected Rust checks/tests. Capture exit codes and completed results; a timeout is not success.
2. Independently review requirements first, then code quality. Fix findings and rerun the affected checks.
3. Launch the updated UI with a hidden process, inspect all settings categories and repeated media/resource probes, and verify no recurring terminal growth. Close only owned verification instances afterward.
4. Record delivered behavior, verification evidence, and real limitations. Return changed-file links and the managed worktree location. Do not claim unsupported backend features were implemented.
