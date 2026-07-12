# Task 24.B3 Native Agent Authority Implementation Plan

> **Execution rule:** Implement one numbered task at a time with test-driven development. Keep Task 24.C service routing and Task 25 media executors out of this change.

**Goal:** Give the production Windows session agent a fail-closed trusted-desktop source, a cancellable local consent surface, and the bootstrap-pinned execute verifier so it can truthfully advertise attended `Consent` and `Input` capabilities on the default desktop.

**Architecture:** Two dedicated Win32 threads remain outside Tokio's control loop. A desktop watcher owns a hidden window, `SetWinEventHook`, WTS notifications, and a cached monotonic desktop snapshot. A separate UI worker owns modeless consent windows and a bounded prompt broker. The runtime remains the sole authorization owner: it releases and pauses remote input while a prompt is visible, closes prompts on desktop changes, validates the resulting scope subset, and installs bindings only after authenticated service requests. The agent receives only the issuer public key; signing and long-term secrets remain service-owned.

**Technology:** Rust, Tokio watch/oneshot channels, `windows` 0.62 Win32 APIs, Ed25519 strict verification, existing `mrd-agent-ipc` and `mrd-input` contracts.

---

## B3.1: Trusted desktop cache and publisher

**Files:**

- Create `apps/mrd-session-agent/src/desktop.rs`
- Modify `apps/mrd-session-agent/src/lib.rs`

1. Add RED unit tests for an initial nonzero snapshot, snapshot-before-notify, notification coalescing with latest-state reads, same-kind trusted transitions, `Default -> nondefault -> Default` ABA, epoch overflow, and sender closure after publisher failure.
2. Implement a single-writer `TrustedDesktopPublisher` and read-only `CachedDesktopStateSource`. The source exposes only an in-memory snapshot and a cloned watch receiver; the publisher owns the sole sender.
3. Make every trusted native transition use `checked_add`, write the complete snapshot first, and notify second. On overflow or publisher loss, clear the snapshot and close the channel.
4. Run `cargo test -p mrd-session-agent desktop::tests` and strict package Clippy.
5. Commit as `feat: add fail-closed desktop state cache`.

## B3.2: Native Windows desktop watcher

**Files:**

- Create `apps/mrd-session-agent/src/windows_desktop.rs`
- Modify `apps/mrd-session-agent/Cargo.toml`
- Modify `apps/mrd-session-agent/src/lib.rs`

1. Add RED tests around an injected event/probe seam for baseline publication, repeated trusted events, WTS lock/disconnect mappings, probe failure, worker exit, and clean shutdown. Native API smoke tests must be opt-in or noninteractive.
2. Start a dedicated OS thread with a hidden HWND and `GetMessageW` loop. Install `WTSRegisterSessionNotification(NOTIFY_FOR_THIS_SESSION)` and an out-of-context `EVENT_SYSTEM_DESKTOPSWITCH` hook before taking the baseline probe.
3. Probe with `OpenInputDesktop(DESKTOP_READOBJECTS)` and `GetUserObjectInformationW(UOI_NAME)`. Map `Default` to `Default`, `Winlogon` to `Winlogon`, and all other names to `Unknown`; WTS lock maps to `Winlogon`, while disconnect/logoff maps to `Unknown`. B3 never switches into or controls a secure desktop.
4. Let callbacks only post a private message. Process transitions serially on the watcher thread. Catch all FFI-boundary panics. Hook, WTS registration, probe, message-loop, or epoch failures clear the cache and terminate the publisher so the runtime fail-stops.
5. On final source drop, post a generation-bound shutdown message, unregister WTS and the hook, destroy the window, and join the worker.
6. Run the targeted tests, `cargo test -p mrd-session-agent`, `cargo clippy -p mrd-session-agent --all-targets --no-deps -- -D warnings`, and `cargo fmt -p mrd-session-agent -- --check`.
7. Commit as `feat: observe the trusted Windows desktop`.

## B3.3: Prompt lifecycle and remote-input exclusion

**Files:**

- Modify `apps/mrd-session-agent/src/consent.rs`
- Modify `apps/mrd-session-agent/src/runtime.rs`
- Modify `apps/mrd-session-agent/tests/consent_routing.rs`
- Modify `apps/mrd-session-agent/tests/input_grants.rs`

1. Add RED integration tests proving that existing pressed input is released before a consent backend starts, every remote input event is rejected while a surface is active or closing, desktop change immediately aborts the surface, queued prompts bound to the old desktop are dismissed, and a return to `Default` cannot revive an old prompt.
2. Add `ConsentAbortReason::DesktopChanged` and crate-private manager observations/actions for active prompt state and desktop invalidation. Keep registration, issuer, desktop, policy, and timestamps outside the backend boundary.
3. Release all input before calling `ConsentManager::begin`. Reject incoming input while the manager has an active/closing prompt. Preserve the existing bounded single-prompt queue and exact cancellation semantics.
4. On a trusted desktop transition, invalidate bindings and old-desktop prompts, send coarse dismissal results, close the visible surface, and wait for its terminal completion without accepting its decision.
5. Run the two targeted integration-test binaries, the package suite, strict Clippy, and formatting.
6. Commit as `fix: isolate local consent from remote input`.

## B3.4: Bounded native Windows consent backend

**Files:**

- Create `apps/mrd-session-agent/src/native_consent.rs`
- Create `apps/mrd-session-agent/src/windows_consent.rs`
- Modify `apps/mrd-session-agent/src/lib.rs`
- Modify `apps/mrd-session-agent/Cargo.toml`

1. Add RED deterministic tests with a fake surface driver for first-poll laziness, one in-flight bounded prompt, exact and subset approvals, scope-escalation rejection, deny, dismiss, abort, future drop, stale-generation messages, worker startup failure, worker exit, and final worker reclamation.
2. Implement a platform-neutral async adapter whose `is_available()` is one atomic read and whose `prompt()` performs no work before first poll. Use a generation-bound bounded broker; abort/drop posts close and decisions complete only after the surface reports destruction.
3. Implement the Windows driver on a second dedicated message-loop thread. Build a modeless top-level window with standard static text, one checkbox per requested scope, and explicit Deny/Allow buttons. Deny receives initial/default focus; Escape, `WM_CLOSE`, empty selection, unknown commands, and native failures fail closed.
4. Render only sanitized display data: replace NUL/control/bidi formatting characters, bound UTF-16 lengths, show the full peer key fingerprint, and use fixed local labels for scopes. No registration, policy, desktop, issuer, timestamps, token, or raw native errors cross into the UI.
5. Make all button/close commands prompt-generation-bound so HWND reuse and late messages cannot decide another request. On backend drop, close any surface, stop the UI thread, and join it.
6. Run targeted tests, the package suite, strict Clippy, and formatting.
7. Commit as `feat: add native Windows attended consent`.

## B3.5: Production bootstrap assembly

**Files:**

- Modify `apps/mrd-session-agent/src/bootstrap.rs`
- Modify `apps/mrd-session-agent/src/capabilities.rs`
- Modify `apps/mrd-session-agent/tests/process_bootstrap.rs`

1. Add RED assembly tests proving the authenticated bootstrap key id/public key constructs `BoundEd25519ExecuteGrantVerifier`, malformed or mismatched verifier material fails before connecting the control runtime, and the production empty executor advertises no Task 25 capabilities.
2. Replace the unused execute-key fields with the concrete bound verifier and issuer id. Never construct or retain an execute signing key in the agent.
3. Add a private `EmptyAuthorizedCommandExecutor` that returns `AgentCapabilities::empty()` and rejects every unexpected generic command.
4. Construct one native desktop source and one native consent backend, then call `with_attended_authority(backend, verifier, desktop, issuer_id, empty_executor)` followed by the existing `WindowsSendInputInjector` backend.
5. Extend the Windows process bootstrap test to assert the first default-desktop capability snapshot contains exactly `Consent` and `Input`, has a nonzero desktop epoch, then accepts `StopAgent` without displaying UI. Add a fail-closed test seam for non-default/unavailable desktop assembly.
6. Run `cargo test -p mrd-session-agent --test process_bootstrap`, the package suite, strict Clippy, and formatting.
7. Commit as `feat: wire production attended agent authority`.

## B3.6: Completion verification and review

1. Run:

   - `cargo test -p mrd-session-agent`
   - `cargo test -p mrd-agent-ipc`
   - `cargo test -p mrd-service --test agent_input_routing`
   - `cargo check -p mrd-service`
   - `cargo clippy -p mrd-session-agent --all-targets --no-deps -- -D warnings`
   - `cargo fmt -p mrd-session-agent -- --check`
   - `git diff --check`

2. Request a fresh specification review and an independent code-quality/security review. Resolve every Critical and Important finding and re-run affected tests.
3. Verify no production path imports `junk/`, no long-term secret reaches the agent, no real dialog appears in ordinary automated tests, and the worktree contains no unrelated changes.
4. Record UAC, lock/unlock, RDP reconnect, fast-user-switch, high-DPI, high-contrast, Narrator, and keyboard-only checks as device-lab/manual follow-ups; these do not weaken automated fail-closed gates.
5. Only after B3 passes, advance to Task 24.C service issuer/resource routing. Do not start Task 25 media work early.
