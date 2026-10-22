# Manual Temporary Password Implementation Plan

> Execute in the attached `codex/mac-native-input` worktree with independent agents owning disjoint files and root integration review.

**Goal:** Keep a resident temporary password stable until explicit manual refresh, while maintaining bounded publications and existing session authority.

**Architecture:** The service owns the secret and renews only its signed publication lease. The FastAPI backend accepts strictly matching monotonic renewal without revoking or extending existing sessions. IPC metadata lets the thin UI distinguish updated and legacy services.

**Tech Stack:** Rust/Tokio/serde/mrd-ipc, FastAPI/SQLAlchemy/pytest, React/TypeScript/Vitest.

### Task 1: Resident custody and lease renewal

Files: `apps/mrd-service/src/temporary_access.rs`, `apps/mrd-service/src/public_connection/temporary.rs`, `crates/mrd-ipc/src/lib.rs`, directly affected Rust status fixtures.

1. Write failing tests that cross the old lease deadline without changing the secret/generation and require renewal.
2. Prove red with `cargo test -p mrd-service temporary_access` or the relevant existing harness.
3. Retain secret on lease expiry but keep access fail-closed until a fresh matching publication ACK; add bounded renewal before expiry. Explicit refresh/disable and identity fences preserve existing behavior.
4. Add optional backwards-compatible refresh-mode metadata; local status reports manual.
5. Verify explicit refresh, disabled state, stale ACK, concurrent operation epoch, generation monotonicity, offline gap and retry. Run appropriate Rust checks.

### Task 2: Backend monotonic renewal

Files: `apps/Rdesk-Server/app/services/temporary_access.py`, existing temporary-access publication and guest-session tests.

1. Write a signed same-generation renewal regression and prove its current rejection.
2. Under existing device/state row locks and pinned key checks, accept strictly later expiry bounded to server now+600000 only for equal enabled material/auth/key/scopes.
3. Preserve the existing publication proof format and pepper-protected digest/verifier storage. Reject any same-generation material change, shorter/expired lease, disabled resurrection or identity mismatch.
4. Update only publication expiry/digest/update time on renewal. Do not alter existing guest authority, grants, reservation deadlines or revoke session requests.
5. Run temporary-access and guest-browser security tests; include concurrent disable/renew race and stale replay cases.

### Task 3: Accurate native UI mode

Files: `apps/Rdesk/src/app/adapters/tauri/types.ts`, `commands.ts`, `commands.temporary-access.test.ts`, `components/TemporaryAccessPasswordCard.tsx` and tests.

1. Add regression for manual metadata, no rotation countdown or timer-triggered rotate, and explicit refresh invocation.
2. Validate optional `refresh_mode`; preserve legacy absence and reject unsupported values.
3. Show manual refresh instructions only for updated service metadata. Keep status polling, secret hiding, background/unmount erasure, copy and explicit rotate/disable operations.
4. Run targeted Vitest and TypeScript checks.

### Task 4: Integration and delivery

Root reviews disjoint diffs and independent security review findings; commits scoped coherent changes, integrates main and pushes. Deploy the backend renewal before rolling out service renewal. Build and verify the exact native candidates before replacing installed software. Preserve device identities and stores. Keep existing Mac OS permissions and genuine local consent; report actual video/input evidence separately from unit/build success.
