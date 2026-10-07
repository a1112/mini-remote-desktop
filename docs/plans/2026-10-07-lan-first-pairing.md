# First LAN Pairing Implementation Plan

**Goal:** Allow an explicitly confirmed, verified installed UI to establish screen-view-only trust for a freshly signed LAN peer, without granting a remote session.

**Architecture:** Reuse signed discovery and kernel-verified product callers. Store permission policy in the sealed trust store and enforce it in LAN authorization. Preserve Agent native session consent and generic IPC denials.

**Tech Stack:** Rust, existing SQLite integrity and audit, Tauri IPC, React and Vitest.

## 1. Baseline and proof of the missing UI flow

- Run the existing device identity registry tests against the clean fixed worktree.
- Add a failing component test for an explicit pending-peer confirmation that displays the device, endpoint, epoch and public fingerprint and sends only screen.view after the user's click.
- Define `LanPairingCandidate`, `LanPairingApproval`, `ListLanPairingCandidates`, `LanPairingCandidateList` and `ApproveLanPairing` in `crates/mrd-ipc/src/lib.rs`; approval reuses `TrustedDeviceUpdated`.

## 2. Authoritative persistent permission policy

- Change only `crates/mrd-store-sqlite/` for atomic sealed v2-to-v3 migration and scope storage.
- Expose `insert_trusted_device_with_policy_and_audit(...)` and `trust_permission_ceiling(peer_key_id)`; canonical string scopes are bounded and validated.
- Write and run failing positive/negative tests before implementation: policy survives restart, old valid store migrates without promoting permissions, tamper and audit failure leave no trust, revoked and duplicate peers are not overwritten.
- Run the complete store tests and provide exact evidence before integrating.

## 3. Fresh signed candidates and installed UI admission

- In `apps/mrd-service/src/lan_discovery.rs` and a focused pairing module, retain a bounded current signed candidate lease after verification and replay acceptance.
- Add candidate renewal, expired/rebound/ambiguous-peer rejection and exact-match approval tests before implementation.
- In `apps/mrd-service/src/ipc_server/dispatch.rs` and `product.rs`, admit the new explicit request only with the verified installed active-desktop caller. Continue denying generic `ApproveTrustedDevice` and legacy trust mutations.
- Approve only the stored public key of a current untrusted candidate. Under the security gate, atomically persist screen.view ceiling and audit, then refresh the trusted peer projection.
- Expose authoritative permission policy through the registry and trusted-device projection.

## 4. Session ceiling enforcement

- Apply the durable peer ceiling to both LAN outgoing and incoming authorization instead of request/runtime scopes.
- Add a failing test for an otherwise valid signed controller grant that exceeds the stored/requested ceiling; fix `install_verified_grant` to check all relevant ceilings.
- Keep session-agent native consent and capture grants unchanged. Run session authorization, signed LAN and product boundary tests.

## 5. Explicit product UI

- Add a focused `LanPairingPanel` and its tests to `apps/Rdesk/src/app/components/`, using typed Tauri adapters and the new candidate DTO.
- Surface it in the normal device view. Display errors, expiry and the public fingerprint; cancellation and expiry never send an approval.
- Do not automatically pair, broaden scopes, call the old generic method or create a session.
- Run targeted component/contract tests and type checking using the existing node_modules junction without reinstalling packages.

## 6. Independent review and handoff

- Request a security/code review of the uncommitted diff against base 5d4ac814.
- Fix material findings, run relevant full checks, and export a reviewable patch with its digest.
- Report exactly what is automated versus still waiting for each user's real UI/native consent clicks. Do not commit or push. Do not reinstall a patched binary on only one machine and call the versions unified.
