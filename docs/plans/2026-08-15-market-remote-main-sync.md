# Market Remote Main Sync Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Produce a conflict-free, tested integration branch containing both the remote-capability work and the latest `main`.

**Architecture:** Merge `origin/main` once into an isolated integration branch and resolve conflicts by preserving compatible behavior from both histories. Validate at protocol boundaries first, then run repository-level gates before publishing a draft PR.

**Tech Stack:** Git, Rust/Cargo, React/TypeScript/pnpm, Python/pytest, PowerShell benchmark contracts, GitHub.

---

### Task 1: Reproduce and Inventory the Merge

**Files:**
- Inspect: all files reported by `git diff --name-only --diff-filter=U`

**Step 1: Merge current main without committing**

Run: `git merge --no-commit --no-ff origin/main`

Expected: the known conflicts are reproduced in the isolated worktree.

**Step 2: Record the exact conflict set**

Run: `git diff --name-only --diff-filter=U`

Expected: every conflicted path is explicit and no primary-worktree path is touched.

### Task 2: Resolve Configuration and Frontend Contracts

**Files:**
- Modify: `apps/Rdesk-Server/app/core/config.py`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/CustomTestPage.test.tsx`
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.ts`

**Step 1: Compare base, feature, and main versions**

Run: `git show :1:<path>`, `git show :2:<path>`, and `git show :3:<path>` for each file.

Expected: independent additions and incompatible edits are identified before editing.

**Step 2: Merge public shapes additively**

Preserve security/configuration additions from `main`, remote-display behavior from the feature branch, and tests for both contracts.

**Step 3: Run focused frontend checks**

Run: `pnpm type-check` and focused Vitest files under `apps/Rdesk`.

Expected: no TypeScript errors and all focused tests pass.

### Task 3: Resolve Service and Protocol Conflicts

**Files:**
- Modify: `apps/mrd-service/src/app_state/media_pipeline_registry.rs`
- Modify: `apps/mrd-service/src/app_state/tests.rs`
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify: `apps/mrd-service/src/lan_discovery/discovery_config.rs`
- Modify: `apps/mrd-service/src/lan_discovery/media_capabilities.rs`
- Modify: `apps/mrd-service/src/lan_discovery/protocol.rs`
- Modify: `apps/mrd-service/src/lan_discovery/tests.rs`
- Modify: `apps/realtime-server/src/main.rs`
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `crates/mrd-ipc/tests/contracts.rs`

**Step 1: Trace each conflict to its public contract**

Compare merge stages and search call sites with `rg` before choosing a resolution.

**Step 2: Preserve both independent protocol capabilities**

Keep additive request/response variants, serialization defaults, security validation, and observability fields. Remove only duplicate imports, definitions, or mutually exclusive legacy paths.

**Step 3: Run focused service and IPC tests**

Run: `cargo test -p mrd-ipc -p mrd-service -p realtime-server`.

Expected: all non-hardware tests pass.

### Task 4: Resolve QUIC Transport Conflicts

**Files:**
- Modify: `crates/mrd-transport-quic-quinn/src/lib.rs`
- Modify: `crates/mrd-transport-quic-quinn/tests/loopback.rs`

**Step 1: Compare transport lifecycle changes**

Trace connection setup, authentication, stream limits, shutdown, and loopback assertions across all three merge stages.

**Step 2: Compose the transport behavior**

Keep the newest `main` transport correctness changes together with the feature branch's authenticated transport and performance semantics.

**Step 3: Run transport tests**

Run: `cargo test -p mrd-transport-quic-quinn`.

Expected: unit and loopback tests pass.

### Task 5: Complete Merge Verification and Publication

**Files:**
- Verify: complete merged repository

**Step 1: Confirm conflict cleanup**

Run: `git diff --name-only --diff-filter=U` and `rg -n "^(<<<<<<<|=======|>>>>>>>)"`.

Expected: no unresolved paths or conflict markers.

**Step 2: Run repository gates**

Run Rust formatting/tests, Rdesk type-check/tests/build, Rdesk Server pytest, and PowerShell benchmark contract tests.

Expected: all required checks pass; hardware-only unsupported cases are explicitly classified.

**Step 3: Commit the merge**

Run: `git commit -m "merge: sync remote capability branch with main"`.

Expected: one merge commit with both parents.

**Step 4: Push and open a draft PR**

Push `codex/market-remote-capability-alignment-main-sync` and create a draft PR targeting `main` with verification evidence.

**Step 5: Audit remote merge readiness**

Confirm the remote head SHA, PR mergeability, and available CI status before reporting completion.
