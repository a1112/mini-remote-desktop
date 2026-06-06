# File Transfer Provider Reservation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a reserved MRD file transfer provider boundary so later work can bind either MRD-native transfer or R-File without duplicating feature surfaces.

**Architecture:** Add small DTOs to `mrd-ipc`, return a default reserved provider snapshot from `mrd-service`, expose it through Tauri/bridge adapters, and make the current transfer modal consume the snapshot instead of static demo data.

**Tech Stack:** Rust serde IPC DTOs, Tokio service handler tests, Tauri command adapters, React/Vitest UI tests.

---

### Task 1: IPC Contract

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Test: `crates/mrd-ipc/tests/contracts.rs`

**Step 1: Write the failing test**

Add a contract test that serializes and deserializes `IpcRequest::FileTransferSnapshot` plus `IpcResponse::FileTransferSnapshot { snapshot }`.

**Step 2: Run the red test**

Run: `cargo test -p mrd-ipc serialize_deserialize_file_transfer_provider_reservation -- --nocapture`

Expected: FAIL because the DTOs and enum variants do not exist.

**Step 3: Implement the minimal DTOs**

Add `FileTransferProviderSnapshot`, `FileTransferTaskSnapshot`, and enum variants for snapshot request/response.

**Step 4: Run the green test**

Run the same command and expect PASS.

### Task 2: Service Reserved Snapshot

**Files:**
- Modify: `apps/mrd-service/src/ipc_server.rs`

**Step 1: Write the failing service test**

Add a test that requests `IpcRequest::FileTransferSnapshot` and expects a provider with status `reserved`, provider id `mrd.file_transfer.reserved`, and no tasks.

**Step 2: Run the red test**

Run: `cargo test -p mrd-service file_transfer_snapshot -- --nocapture`

Expected: FAIL because the request is not handled.

**Step 3: Implement handler**

Handle the request by returning the default reserved provider snapshot.

**Step 4: Run the green test**

Run the same command and expect PASS.

### Task 3: Frontend Bridge

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `apps/Rdesk/src/app/adapters/tauri/commands.ts`
- Modify: `apps/Rdesk/src/app/components/FileTransferPage.tsx`
- Test: `apps/Rdesk/src/app/adapters/tauri/contract.test.ts`
- Test: `apps/Rdesk/src/app/components/FileTransferPage.test.tsx`

**Step 1: Write failing frontend tests**

Add adapter and component tests for `ipcFileTransferSnapshot` and reserved-state rendering.

**Step 2: Run red tests**

Run: `pnpm --dir apps/Rdesk test -- src/app/adapters/tauri/contract.test.ts src/app/components/FileTransferPage.test.tsx`

Expected: FAIL because command/types/UI behavior do not exist.

**Step 3: Implement bridge and UI consumption**

Add the Tauri command, TS types, adapter function, and make `TransferModal` fetch the service snapshot when opened.

**Step 4: Run green tests**

Run the same frontend tests and expect PASS.

### Task 4: Final Verification

Run:

```powershell
cargo fmt --check
cargo test -p mrd-ipc serialize_deserialize_file_transfer_provider_reservation -- --nocapture
cargo test -p mrd-service file_transfer_snapshot -- --nocapture
pnpm --dir apps/Rdesk test -- src/app/adapters/tauri/contract.test.ts src/app/components/FileTransferPage.test.tsx
pnpm --dir apps/Rdesk type-check
git diff --check
```

Expected: all commands pass.
