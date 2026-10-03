# Resident Service Management Implementation Plan

> **For Codex:** Use the executing-plans workflow to implement and verify each task.

**Goal:** Make Windows background-service autostart, shutdown, and UI lifecycle controls work and report real outcomes.

**Architecture:** The installed MiniRemoteDesktop SCM service remains the background owner. Autostart reads and changes SCM startup configuration. A service-owned shutdown coordinator closes session admission, waits for the IPC acknowledgement, then drains or cancels runtime resources. UI commands expose the actual outcome and bootstrap starts the installed service when present.

**Tech Stack:** Rust, Tokio, windows-service, Tauri, TypeScript, React, Vitest.

### Task 1: Real Windows autostart

- Modify `apps/mrd-service/src/shell/mod.rs`, `apps/mrd-service/src/handlers/shell.rs` and add an SCM adapter module if useful.
- Add failing tests for actual state readback, unsupported operation, query/change failures, and Automatic/DemandStart mapping.
- Implement query/change through SCM, preserve unrelated service configuration, and refresh shell status from the real port.
- Run `cargo test -p mrd-service --lib shell` and focused integration tests.

### Task 2: Service-owned shutdown

- Create `apps/mrd-service/src/shutdown.rs`; modify app state, IPC dispatch/connection, session entry points, and `apps/mrd-service/src/main.rs`.
- Add failing tests for acknowledgement before exit, unavailable runtime, drain admission, terminal sessions, and escalation.
- Bind the coordinator in the service runtime; route Graceful, AfterSessions and Force to bounded cleanup. Reject new session admission after a shutdown request. Keep IPC alive long enough to deliver the response.
- Run focused shutdown tests, IPC tests, Windows service contract tests, and `cargo check -p mrd-service`.

### Task 3: UI and restart behavior

- Modify Tauri shell commands/bootstrap, adapters, lifecycle service and settings modal.
- Add failing tests for errors keeping the UI open, stop-before-start restart ordering, healthy timeout, and autostart read/write/readback.
- Wire a real background-autostart switch, wait for confirmed stop before restarting, and propagate shutdown failures instead of silently exiting.
- Run focused Vitest tests, `pnpm type-check`, `pnpm build`, and native `cargo check`.

### Task 4: Integration and delivery

- Review diffs and preserve the three pre-existing agent/runtime edits in the primary checkout.
- Run relevant regressions and a real isolated IPC acknowledgement/admission smoke test.
- Build deployable Windows artifacts. Update release notes with verified behavior and any remaining activation requirement.
- Current shell is not elevated. Updating the installed service in Program Files requires an elevated installer; complete code, tests, and deployable artifacts before reporting that activation requirement.
