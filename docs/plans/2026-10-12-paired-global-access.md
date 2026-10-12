# Paired Global Access Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Own paired computers and browsers can reconnect through authenticated P2P or relay without repeating local screen/keyboard/pointer confirmation; the owner can revoke access.

**Architecture:** Introduce explicit `paired` access mode, separate from both attended consent and the unfinished shared-credential unattended mode. A stable controller Ed25519 key must match an enabled, sealed target-local policy. Each connection retains signed intent, fresh backend authorization, target grant, route proof and a separately signed, short-lived Agent authority bound to the exact active unlocked Default desktop. Unknown keys require one explicit local pairing action.

**Tech Stack:** Rust service/Agent, sealed SQLite v3 and DPAPI, Ed25519, FastAPI/PostgreSQL, React/TypeScript, WebCrypto/IndexedDB.

## Contract and boundaries

- `paired` participates in the existing canonical request commitment; attended bytes remain unchanged.
- Pairing is a distinct local installed-UI policy action, never synthetic native consent. Only the kernel-verified installed UI on the current Agent logon may enroll an authenticated pending controller key. It grants only selected `screen.view`, `input.pointer`, `input.keyboard` scopes. It cannot enroll arbitrary caller-supplied public keys.
- Preserve old trust with paired access disabled. Suspended/revoked keys cannot enroll; policy revision and trust revision must match at approval and grant installation.
- Durable policy uses a protected blob plus an atomic dedicated audit commitment in existing sealed v3 storage. Complete snapshot and the latest per-peer policy commitment must verify; missing, replaced or rolled-back blobs fail closed. Do not infer permissions from ordinary audit messages.
- Agent protocol minor 9 installs a distinct signed authority; existing attended bindings and command validation remain intact. Registration, peer, session, policy, scopes, desktop epoch and bounded expiry are verified independently.
- Durable ownership is bound to the kernel-verified Windows TokenUser SID. Each live session is independently bound to the current Agent registration, logon SID and active unlocked desktop; login or restart never preserves a live grant.
- Browser keys are non-exportable and stable per origin in IndexedDB. Backend browser principals/tokens remain short-lived and session-bound. Password/account authentication still applies; pairing controls target permission.
- Automatic confirmation lasts only while the target password material is unchanged. Each sealed pairing binds the current positive temporary-password generation and target device authentication version. A password refresh, resident restart which generates a new password, disabled access, or device credential change makes the old pairing ineffective for new/in-flight approvals. Signed publication lease renewal preserves the same generation and does not invalidate pairing. Explicit local re-pairing advances the policy revision and binds the new versions; legacy policies without a password binding require this confirmation. Active grants retain their existing rotation behavior; disabling access continues to revoke them. The unfinished fixed unattended password feature must not be reported as implemented by this change.
- Physical paired sessions and account browser sessions require the same owner. Guest browser requests still require valid temporary-password authority plus the target-local exact-key pairing policy.
- Revoke persists before terminalizing matching pending/active authorizations and WAN/LAN resources, releasing capture and held input. Restarts preserve only durable policy, never old session grants.

## Task 1: Sealed peer policy

**Files:** `crates/mrd-store-sqlite/src/paired_access_store.rs`, `src/lib.rs`, existing trust/audit helpers; integration tests in the same crate.

1. Write failing tests for explicit enrollment, reopen, missing policy, stale revision, revoked/suspended trust, mismatched key, scope expansion, deleted/replaced protected blob and audit tampering.
2. Run `cargo test -p mrd-store-sqlite paired` and record the expected failures.
3. Implement typed policy enroll/get/list/revoke transactions using verified v3 snapshots, a purpose-bound protected blob and the exact latest dedicated audit commitment.
4. Run the targeted tests and existing store integrity tests; inspect every result.

## Task 2: Independent Agent paired authority

**Files:** `crates/mrd-agent-ipc/src/paired_authority.rs`, grant/protocol exports; `apps/mrd-session-agent/src/runtime.rs` and tests.

1. Write signature, replay, expiry, cross-registration/session/peer/desktop and revoke tests; run them RED.
2. Add signed install/ack authority protocol and a private Agent registry, bound to the pinned service issuer and current trusted desktop. Reuse execution/input checks with explicit authority selection.
3. Run `cargo test -p mrd-agent-ipc` and `cargo test -p mrd-session-agent`; retain attended regression results.

## Task 3: WAN and backend contract

**Files:** `crates/mrd-signal-proto/src/initial_v3.rs`; backend schemas/models/device_sessions/browser_sessions and a repeatable database constraint migration.

1. Write paired canonical vectors and owner/key/mode rejection tests; run them RED.
2. Add `WanAccessModeV3::Paired`, backend `paired` validation and strict physical/account owner constraints. Keep temporary-password guest authentication and short-lived browser principal constraints.
3. Run Rust protocol tests and relevant backend pytest tests; verify attended canonical bytes and historical rows.

## Task 4: Service policy and actual capture/input route

**Files:** `crates/mrd-ipc/src/lib.rs`; `apps/mrd-service/src/session_authorization.rs`, `ipc_server/paired_access.rs`, `console_capture.rs`, `agent_runtime/`, WAN coordinator/backend/service and IPC product/dispatch; Tauri command bridge.

1. Write registry tests proving unknown/unbound peer rejection, exact scopes and policy revision, paired authorization and revoked grant rejection. Run them RED.
2. Publish the request with its signed mode, bind the independently authenticated public key before any local decision, and either wait for explicit first pairing or verify the durable policy automatically.
3. Expose `PairPendingSession`, `ListPairedAccessPolicies`, `RevokePairedAccess` only through verified product UI. Pair uses the pending signed key and current owner logon, never an arbitrary key provided by the frontend.
4. Install a signed Agent authority and exact console target before allowing the WAN target grant. Recheck sealed policy under the authorization security gate after awaits and before grant installation. Fail and clean up if policy or desktop changes.
5. Run service focused tests, IPC contracts and required service regression tests.

## Task 5: Browser identity and user flow

**Files:** `apps/Rdesk/src/app/services/browserSigningIdentity.ts`, browser service/protocol, incoming consent host and pair-list UI; adapter types/commands and tests.

1. Write IndexedDB concurrent identity, persisted reuse, corruption/unavailable storage and paired canonical/UI tests; run RED.
2. Persist a non-exportable signing key and expose an explicit paired connection choice. Add local `pair and remember` action and a revocation list using the formal service APIs.
3. Run relevant Vitest, `pnpm type-check`, `pnpm build` and Tauri bridge checks.

## Task 6: Review, release and real verification

1. Review cross-component policy, signature, race, cleanup and compatibility boundaries independently. Repair findings and rerun only affected checks.
2. Build clean reviewed release artifacts, record commit/tree/hash provenance and preserve identity/config/ACL/audit data during the existing authorized installer flow.
3. Deploy the coordinated backend/protocol/web contract using existing authorized server access; validate health and migration outcome.
4. In the installed local UI, explicitly pair the stable local-preview browser once. Verify reconnect without native confirmation, changing frames, pointer and text input into an empty scratch window. Verify revoke immediately stops frames/input and prevents reconnect; re-pair via the same formal UI.
5. Only after genuine frames and input work, benchmark 30/60/120 fps profiles with measured delivered frames, latency, CPU/GPU and bitrate. Global dual-Windows P2P/relay acceptance remains a separate real test; do not substitute synthetic encoding throughput.

## Completion evidence

Report implementation and tests separately from installed/runtime proof. No video/input or performance success claim without actual browser observations. A stopped computer-use session or unavailable host is a runtime limitation, not permission to bypass the native/tool authority boundary.
