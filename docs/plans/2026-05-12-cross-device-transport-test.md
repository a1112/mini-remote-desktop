# Cross-Device Transport Test Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add execution-target selection to the Transport test page so local transport tests remain default and discovered LAN devices can run cross-device QUIC/WebRTC validation.

**Architecture:** The Transport page keeps its existing local `testStartRun` path for the default local target. When the user selects a discovered LAN peer, the page reuses `runLanE2EAutomation` with the selected transport and maps the report probe into the existing transport metrics UI. LAN device discovery uses the same `ipcRefreshLanDiscovery` command and local-first target model as Matrix tests.

**Tech Stack:** React, TypeScript, Vitest, Testing Library, Tauri command adapters, existing LAN E2E automation service.

---

### Task 1: Add Failing Component Test

**Files:**
- Create: `apps/Rdesk/src/app/components/TestWorkbench/TransportTestPage.test.tsx`

**Steps:**
1. Render `TransportTestPage`.
2. Assert `执行范围` defaults to `local`.
3. Switch to `cross-device`.
4. Mock `ipc_refresh_lan_discovery` with a Linux peer and select it from `跨设备目标设备`.
5. Click `启动测试`.
6. Assert `ipc_start_lan_remote_session` is called with the selected peer and selected transport.
7. Assert local `test_start_run` is not called.

### Task 2: Implement Execution Target UI

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/TransportTestPage.tsx`

**Steps:**
1. Add `MatrixRunScope`-style state: local/cross-device, LAN peers, selected target, refresh state.
2. Add `ipcRefreshLanDiscovery` effect when cross-device mode is selected.
3. Render an `执行目标` panel before transport selection.
4. Keep default target as local.

### Task 3: Implement Cross-Device Run Path

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/TransportTestPage.tsx`

**Steps:**
1. Import `runLanE2EAutomation` and build `LanE2EAutomationCommands`.
2. In `handleStart`, if a remote LAN target is selected, run `runLanE2EAutomation`.
3. Pass selected transport, a profile derived from test profile, and the selected peer ID.
4. Convert `LanE2EAutomationReport` probe data into `TransportMetrics`.
5. Surface failure messages in `startError`.

### Task 4: Verify

**Commands:**
- `pnpm --dir apps/Rdesk test -- src/app/components/TestWorkbench/TransportTestPage.test.tsx`
- `pnpm --dir apps/Rdesk type-check`
- `pnpm --dir apps/Rdesk build`
