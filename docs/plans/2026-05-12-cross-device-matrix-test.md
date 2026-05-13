# Cross Device Matrix Test Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a cross-device matrix mode to the Test Workbench Matrix page with LAN peer discovery, a device selector that defaults to the local machine, and LAN E2E smoke execution for selected remote peers.

**Architecture:** Keep the existing local matrix path unchanged. Add a run-scope selector to `MatrixTestPage`; local scope runs existing `test_start_run` combinations, while cross-device scope reuses `runLanE2EAutomation` and maps matrix profile dimensions into LAN media profiles. Device discovery comes from the existing `ipc_refresh_lan_discovery` command.

**Tech Stack:** React, Vitest, Testing Library, Tauri adapter commands, existing LAN E2E automation service.

---

### Task 1: Cross-Device Matrix UI Test

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

**Step 1: Write failing tests**

Add tests that render `MatrixTestPage`, verify the scope selector defaults to local, refresh LAN discovery populates a remote peer option, selecting that peer and starting the matrix calls LAN IPC commands instead of `test_start_run`, and the requested LAN profile reflects selected resolution/fps/bitrate/duration.

**Step 2: Run red test**

Run:
`pnpm --dir apps/Rdesk test -- src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

Expected: FAIL because the scope selector and cross-device matrix path do not exist.

### Task 2: Matrix Page Implementation

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx`

**Step 1: Add state and discovery**

Add state for `runScope`, `lanPeers`, `selectedLanTargetId`, and `crossDeviceNotice`. Load peers with `commands.ipcRefreshLanDiscovery`; expose a selector whose first option is local.

**Step 2: Add cross-device execution**

Add a `runCrossDeviceMatrixTests` path that converts each `MatrixTest` into `runLanE2EAutomation` options:
- `targetDeviceId`: selected peer id
- `transportKind`: selected matrix transport, defaulting to QUIC for remote
- `requestedProfile`: selected resolution/fps/bitrate converted to Mbps
- `timeoutMs`: selected duration plus sample margin
- `scenarioId`: `cross.e2e.remote_display_smoke`

Map `LanE2EAutomationReport` to existing row status and failure fields.

**Step 3: Preserve local behavior**

Keep the existing `handleStart` local behavior for the default local scope.

### Task 3: Verification

**Files:**
- Test: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

**Step 1: Run targeted tests**

Run:
`pnpm --dir apps/Rdesk test -- src/app/components/TestWorkbench/MatrixTestPage.test.tsx`

Expected: PASS.

**Step 2: Run type check**

Run:
`pnpm --dir apps/Rdesk type-check`

Expected: PASS.
