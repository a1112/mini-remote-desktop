# Task 24 Agent Routing Completion Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Complete Task 24 by routing attended consent and authenticated incoming input through one exact interactive-session agent generation, with no service-process input injection fallback.

**Architecture:** `mrd-service` remains the authorization and network owner. It selects and persists an exact `AgentBinding`, signs short-lived `ExecuteGrant` commands, and correlates consent, command, and input responses over the authenticated private connection. `mrd-session-agent` owns the consent surface, input-resource state, and platform injection; registration replacement, desktop changes, disconnects, and grant changes fail closed without automatic retargeting.

**Tech Stack:** Rust, Tokio bounded channels, Ed25519 execute grants, Windows named pipes, `mrd-agent-ipc`, `mrd-session`, `mrd-input`.

---

### Task 1: Correlated consent and execute requests

**Files:**
- Modify: `apps/mrd-service/src/agent_runtime/server.rs`
- Modify: `apps/mrd-service/src/agent_runtime/registry.rs`
- Modify: `apps/mrd-service/tests/agent_input_routing.rs`

1. Add failing tests showing that `ConsentRequest` is sent only to its named Windows session and that a result from another connection, generation, request, or expired request cannot complete it.
2. Add failing tests showing that `ExecuteCommand` completion is correlated to the exact connection, registration, and command id; cancellation, timeout, revocation, and late results cannot cause delivery or key reuse.
3. Run `cargo test -p mrd-service --test agent_input_routing` and confirm the new tests fail because only input requests are correlated.
4. Implement bounded `request_consent` and `request_execute` paths using the same cancellation, write interruption, exact-route revalidation, and retired-key rules as input.
5. Run the targeted service tests and keep all existing input-routing tests green.

### Task 2: Native agent consent and trusted session bindings

**Files:**
- Create: `apps/mrd-session-agent/src/consent.rs`
- Modify: `apps/mrd-session-agent/src/lib.rs`
- Modify: `apps/mrd-session-agent/src/runtime.rs`
- Modify: `apps/mrd-session-agent/src/bootstrap.rs`
- Create: `apps/mrd-session-agent/tests/consent_routing.rs`

1. Add failing tests for exact Windows-session validation, request expiry, approved-scope subset enforcement, duplicate request replay, desktop changes, and binding creation only from an authenticated service request.
2. Add a bounded consent backend that returns structured decisions and never supplies scopes not present in the request. The Windows adapter runs outside the async control loop; unsupported platforms and unavailable desktops dismiss or expire fail-closed.
3. Persist the service-authenticated session/peer/policy binding in the consent manager's crate-private registry and make that registry the sole source for subsequent execute/input validation.
4. Release input and invalidate live bindings on disconnect, StopAgent, desktop generation change, and binding expiry.
5. Run `cargo test -p mrd-session-agent` and strict Clippy.

### Task 3: Service-owned execute-grant issuer and input resource router

**Files:**
- Modify: `apps/mrd-service/src/control_input.rs`
- Modify: `apps/mrd-service/src/app_state/core.rs`
- Modify: `apps/mrd-service/src/session_authorization.rs`
- Modify: `apps/mrd-service/tests/agent_input_routing.rs`

1. Add failing tests for a persisted Windows-session/generation binding, matching input scopes, monotonic per-resource sequence, fast-user-switch non-retargeting, disconnect pause, grant expiry, policy change, and cleanup.
2. Add a service-owned execute-grant issuer backed by the machine identity and a per-session agent-input resource registry.
3. Start one resource with `StartInput`, route mapped `InputEventEnvelope` values through `AgentServer::request_input`, and stop/release it on terminal or invalidating transitions.
4. Record only coarse structured failures; do not log input payloads or native platform strings.
5. Run the targeted routing, authorization, and control-input tests.

### Task 4: Replace incoming LAN local injection and route attended consent

**Files:**
- Modify: `apps/mrd-service/src/handlers/session.rs`
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify: `apps/mrd-service/src/lan_discovery/lan_control_input.rs`
- Modify: `apps/mrd-service/src/capabilities.rs`
- Modify: `apps/mrd-service/src/session_authorization.rs`

1. Add failing tests proving secure incoming LAN input never calls the service-process injector and pauses when no exact agent route exists.
2. Select a consent-capable Windows session once, persist it in the authorization/grant, and route consent through that exact agent. Multiple or unavailable candidates fail closed until explicit session supervision selects one.
3. Make the Rdesk consent response path incapable of bypassing agent consent for secure incoming sessions.
4. Replace `ControlInputRegistry::handle_authenticated_session_event` in the incoming LAN path with the agent-input router. Keep local injection only for explicit non-remote administrative/test paths.
5. Derive input capability truth from a healthy exact agent, not from service-local `SendInput` availability.

### Task 5: Completion audit and commit

**Files:**
- Verify every Task 24 file and acceptance statement from `docs/plans/2026-07-11-market-remote-capability-alignment.md`.

1. Run the Task 24 targeted tests, package tests, and strict Clippy.
2. Run `cargo test -p mrd-service`; classify only independently reproduced pre-existing platform failures, and fix every Task 24 regression.
3. Request a specification review, then a code-quality/security review; resolve all Critical and Important findings and re-review.
4. Run `git diff --check` and verify the worktree diff contains no unrelated changes.
5. Commit with `git commit -m "fix: complete desktop agent input routing"`.
