# WAN Authority Deadlines Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Allow a fresh, verified WAN grant to establish its bounded session lease while retaining the initial confirmation deadline and all revocation checks.

**Architecture:** Keep setup and active authority separate. Only the private boundary combining an authenticated backend record with a signed grant may establish the active lease; raw snapshots and later polls cannot extend it. Keep existing wire, database schema, API policy TTLs and Agent limits.

**Tech Stack:** Rust service/coordinator and authorization registry, Vitest browser peer, installed Windows Agent IPC, existing HTTPS API and Rust signaling server.

---

### Task 1: Reproduce and implement coordinator authority

Files: `apps/mrd-service/src/wan_session/model.rs`, `coordinator.rs`, their existing tests, and any narrowly required service-only proof module.

1. Add meaningful failing tests for a fresh 30-second grant message and 600-second exact backend policy after a 60-second setup request. Capture RED before implementation.
2. Introduce a private proof that cannot be constructed from a public raw snapshot alone. Bind exact identities, request/intent/grant commitments, revision and both deadlines. Reject expired/future/unbounded authority and expired setup before activating it.
3. Use the proof-derived deadline for already authorized state transitions, negotiation, runtime watchdogs and cleanup. Keep setup waits bounded by the original request. Preserve exact duplicate retention and reject any renewal through duplicate or poll.
4. Run targeted Rust tests; confirm GREEN and unchanged invalid-binding/replay behavior. Record actual command, exit code, counts and hashes under `target/self-web-performance-20261013/browser-grant-deadline`.

### Task 2: Bind service, registry and browser watch

Files: `apps/mrd-service/src/wan_session/service.rs`, `browser_authority.rs`, `session_authorization.rs`, `wan_session/target_authority_tests.rs` and existing registry tests.

1. Add RED tests proving a verified long WAN lease may be installed while confirmation remains current, a raw lease cannot, and a lock/await crossing the immutable confirmation cutoff is rejected.
2. Pass the private proof through target/controller installation. Re-read reliable current time under the final registry lock. Retain local policy revision, current peer key, scopes and role checks; leave LAN/general install behavior intact.
3. Remove only comparisons that confuse message delivery expiry with active lease. Replace them with proof, pending cutoff and fresh message checks. Recheck all callers of `identity.deadline_unix_ms()` and document every remaining use.
4. Have browser authority watch compare exact backend grant expiry as well as all existing fields. Never extend a running lease; terminate at the immutable proof deadline.
5. Run focused target, controller, registry, browser watcher and WAN coordinator suites. Have separate specification and code-quality reviews resolve findings before deployment.

### Task 3: Build, upgrade and verify

1. Run `cargo test -p mrd-service --lib wan_session` and the changed registry test group, scoped Rust formatting and `git diff --check`. Retain the existing browser peer's 48 passing tests and successful production Vite build.
2. Commit the verified service change; build exact committed source with `cargo build -p mrd-service --release`. Prepare a reversible installed-service binary transaction preserving original configuration, machine identities, paired policy, sealed state and audit. Keep unchanged Agent/UI provenance explicit.
3. Stop/start through the normal SCM lifecycle, verify installed hash, supervised Agent version and public API/signaling/temporary-access readiness.
4. Retry actual local webpage with the same browser identity and current password. Confirm no repeated local prompt for saved pairing, advancing decoded video and remote pointer/keyboard input in a separate empty Notepad tab.
5. After that proof, run moving-source 30/60/120 profiles with bounded samples; record actual decoded/source frame rates, route, CPU/GPU and drops without equating 60 Hz source refresh with 120 unique frames. Continue the existing peer Windows conversation only after local E2E succeeds.
