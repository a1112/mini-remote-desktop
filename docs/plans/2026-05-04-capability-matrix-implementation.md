# Capability Matrix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add the first structured capability matrix layer while preserving existing `test_get_capabilities` and LAN E2E behavior.

**Architecture:** Start in the Rdesk frontend with a compatibility model that converts existing legacy arrays into structured capability items. Add a pure evaluator for constraints and profiles, then wire Test Workbench and LAN E2E to consume the structured model without breaking current Tauri commands. mrd-service structured LAN peer snapshots are a follow-up once the local model is stable.

**Tech Stack:** TypeScript, React, Vitest, Tauri command adapter contracts, existing Rdesk Test Workbench.

---

### Task 1: Add Structured Capability Types

**Files:**
- Create: `apps/Rdesk/src/app/services/capabilityMatrix.ts`
- Test: `apps/Rdesk/src/app/services/capabilityMatrix.test.ts`
- Reference: `apps/Rdesk/src/app/adapters/tauri/types.ts`

**Step 1: Write failing tests**

Create tests for:

- converting legacy `EnvironmentSnapshot` arrays into structured `CapabilitySnapshot`
- classifying known Windows capabilities as `available`
- preserving unknown values as `unknown`
- including domains for capture, encode, decode, render, memory, transport, control, service

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
```

Expected: FAIL because `capabilityMatrix.ts` does not exist.

**Step 2: Implement minimal model**

Add:

- `CapabilityStatus`
- `CapabilityDomain`
- `CapabilityItem`
- `CapabilityConstraint`
- `CapabilityProfile`
- `ProfileProbeResult`
- `CapabilitySnapshot`
- `buildCapabilitySnapshotFromEnvironment(environment)`

Do not change existing adapter types in this task.

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/services/capabilityMatrix.ts apps/Rdesk/src/app/services/capabilityMatrix.test.ts
git commit -m "feat: add structured capability matrix model"
```

### Task 2: Add Constraint Evaluation

**Files:**
- Modify: `apps/Rdesk/src/app/services/capabilityMatrix.ts`
- Test: `apps/Rdesk/src/app/services/capabilityMatrix.test.ts`

**Step 1: Write failing tests**

Cover these rules:

- `openh264` blocks `d3d11_shared` when no CPU copy step is declared.
- `d3d12_native` is `unimplemented` for mainline remote display.
- `webview` render is `degraded`, not native parity.
- `display_shared` is preferred over `display`, then `window`.

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
```

Expected: FAIL because evaluator does not exist.

**Step 2: Implement evaluator**

Add:

- `evaluateCapabilityCombination(request, snapshot)`
- `pickPreferredCaptureSourceKind(items)`
- `CapabilityEvaluation`

Return:

- `status: "ready" | "blocked" | "degraded" | "skipped"`
- `reasons: string[]`
- `requiredFallbacks: string[]`

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/services/capabilityMatrix.ts apps/Rdesk/src/app/services/capabilityMatrix.test.ts
git commit -m "feat: evaluate capability combinations"
```

### Task 3: Add Performance Profile Evaluation

**Files:**
- Modify: `apps/Rdesk/src/app/services/capabilityMatrix.ts`
- Test: `apps/Rdesk/src/app/services/capabilityMatrix.test.ts`

**Step 1: Write failing tests**

Cover:

- `lan.2k144` requires QUIC datagram media, profile control, 2560x1440, 144 FPS, H264.
- static support can be `ready` while measured probe result is `failed`.
- runtime profile mismatch returns a deterministic reason.

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
```

Expected: FAIL.

**Step 2: Implement profiles**

Add:

- built-in profiles: `smoke.720p30`, `interactive.1080p60`, `lan.2k144`, `quality.4k60`, `diagnostic.software`
- `evaluateProfileSupport(profileId, snapshot)`
- `evaluateProfileProbe(profile, probeSnapshot)`

Use existing LAN E2E probe fields as the first runtime input.

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/capabilityMatrix.test.ts
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/services/capabilityMatrix.ts apps/Rdesk/src/app/services/capabilityMatrix.test.ts
git commit -m "feat: evaluate capability performance profiles"
```

### Task 4: Wire Test Workbench Overview

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/OverviewPage.tsx`
- Test: `apps/Rdesk/src/app/components/TestWorkbench/OverviewPage.test.tsx`
- Use: `apps/Rdesk/src/app/services/capabilityMatrix.ts`

**Step 1: Write failing component test**

Test that Overview shows:

- structured domain groups
- status labels such as `available`, `degraded`, `unimplemented`
- 2K144 profile readiness
- reason text for unavailable capability

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/components/TestWorkbench/OverviewPage.test.tsx
```

Expected: FAIL.

**Step 2: Implement UI**

Keep current quick actions and recent runs. Add a compact capability section below the environment summary. Do not redesign the whole page.

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/components/TestWorkbench/OverviewPage.test.tsx
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/components/TestWorkbench/OverviewPage.tsx apps/Rdesk/src/app/components/TestWorkbench/OverviewPage.test.tsx
git commit -m "feat: show structured capability matrix overview"
```

### Task 5: Use Capability Evaluation In Matrix Rows

**Files:**
- Modify: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx`
- Test: `apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx`
- Use: `apps/Rdesk/src/app/services/capabilityMatrix.ts`

**Step 1: Write failing tests**

Cover:

- invalid OpenH264 + D3D11 shared rows are `skipped`
- unimplemented renderer rows are `skipped`
- skipped rows include reason text
- skipped rows are counted as terminal rows, not `running`

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/components/TestWorkbench/MatrixTestPage.test.tsx
```

Expected: FAIL.

**Step 2: Implement minimal integration**

Evaluate rows before `testStartRun`. Mark blocked rows as `skipped` with reason. Do not move matrix execution to backend yet.

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/components/TestWorkbench/MatrixTestPage.test.tsx
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.tsx apps/Rdesk/src/app/components/TestWorkbench/MatrixTestPage.test.tsx
git commit -m "feat: skip invalid matrix capability combinations"
```

### Task 6: Feed LAN E2E From Capability Profiles

**Files:**
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.ts`
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.test.ts`
- Use: `apps/Rdesk/src/app/services/capabilityMatrix.ts`

**Step 1: Write failing tests**

Cover:

- LAN E2E preflight rejects peer missing `lan.2k144` support.
- runtime probe mismatch uses shared profile evaluator.
- report includes profile evaluation result.

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/lanE2eAutomationService.test.ts
```

Expected: FAIL.

**Step 2: Implement integration**

Reuse `evaluateProfileProbe` for runtime validation. Keep legacy peer transport checks until mrd-service structured snapshots exist.

**Step 3: Verify**

Run:

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/lanE2eAutomationService.test.ts
pnpm type-check
```

Expected: PASS.

**Step 4: Commit**

```powershell
git add apps/Rdesk/src/app/services/lanE2eAutomationService.ts apps/Rdesk/src/app/services/lanE2eAutomationService.test.ts
git commit -m "feat: validate lan e2e with capability profiles"
```

### Final Verification

Run:

```powershell
cd apps/Rdesk
pnpm type-check
pnpm test
pnpm build
```

Expected:

- type-check passes
- all Vitest tests pass
- Vite build passes, existing chunk-size warning is acceptable

Then run:

```powershell
git status --short
```

Expected: clean except intentional changes.

### Later Rust/mrd-service Follow-Up

After the frontend compatibility layer is stable:

- Add shared Rust capability schema under `crates/` or `crates/mrd-ipc`.
- Add `capability_get_snapshot` IPC.
- Add structured capability snapshot to LAN discovery.
- Keep legacy string transports until both local machines are updated.
