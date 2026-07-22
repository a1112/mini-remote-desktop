# Sidebar Device Actions Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace simulated Sidebar device-menu actions with real or explicitly unavailable behavior.

**Architecture:** Keep Sidebar as the UI owner for context-menu state. Extend `deviceService.unbindDevice` with an optional target device ID so existing logout behavior remains compatible while Sidebar can unbind the selected device.

**Tech Stack:** React, TypeScript, Vitest, Testing Library.

---

### Task 1: Add Sidebar Regression Tests

**Files:**
- Create: `apps/Rdesk/src/app/components/Sidebar.test.tsx`

**Steps:**
1. Mock `useDevices`, `useAuth`, `useTheme`, and `deviceService`.
2. Render `Sidebar` in a `MemoryRouter`.
3. Open the device context menu.
4. Assert unsupported actions are disabled.
5. Assert "退出绑定" calls `deviceService.unbindDevice(userId, deviceId)` for a logged-in user.

### Task 2: Implement Honest Sidebar Actions

**Files:**
- Modify: `apps/Rdesk/src/app/components/Sidebar.tsx`
- Modify: `apps/Rdesk/src/app/services/deviceService.ts`

**Steps:**
1. Add `useAuth` to Sidebar.
2. Replace simulated alert handlers with disabled menu items.
3. Add inline action status text for unbind success/failure.
4. Extend `deviceService.unbindDevice` to accept an optional target device ID.

### Task 3: Verify

**Commands:**
- `pnpm vitest run src/app/components/Sidebar.test.tsx`
- `pnpm type-check`
- `pnpm test`
