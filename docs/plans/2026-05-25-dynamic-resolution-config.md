# Dynamic Resolution Config Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add an explicit dynamic resolution switch that lowers encoded sampling resolution only when enabled and never crops the source image.

**Architecture:** Extend the IPC adaptive media config with a default-off `dynamic_resolution_enabled` flag. Keep the adaptation controller's existing bitrate/FPS ladder, but freeze width/height across rungs unless the flag is enabled. On Windows, avoid crop-prone shared capture backends when a selected profile is smaller than the selected source.

**Tech Stack:** Rust workspace (`mrd-ipc`, `mrd-service`), Tauri TypeScript adapter/types, Vitest contract tests, Cargo unit/contract tests.

---

### Task 1: IPC Config Field

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `crates/mrd-ipc/tests/contracts.rs`
- Modify: `apps/Rdesk/src/app/adapters/tauri/types.ts`
- Modify: `apps/Rdesk/src/app/adapters/tauri/contract.test.ts`

**Step 1: Write the failing test**

Add a contract assertion that `AdaptiveMediaConfig` serializes `dynamic_resolution_enabled` when true and deserializes old JSON without the field as `false`.

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-ipc serialize_deserialize_configure_media_adaptation -- --nocapture`

Expected: FAIL because `AdaptiveMediaConfig` has no `dynamic_resolution_enabled` field.

**Step 3: Implement minimal IPC support**

Add `#[serde(default)] pub dynamic_resolution_enabled: bool` to `AdaptiveMediaConfig`. Mirror the field in TypeScript `AdaptiveMediaConfig`.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mrd-ipc serialize_deserialize_configure_media_adaptation -- --nocapture`

Expected: PASS.

**Step 5: Commit**

Commit message: `feat: expose dynamic resolution config`

### Task 2: Adaptation Ladder Semantics

**Files:**
- Modify: `apps/mrd-service/src/media_adaptation.rs`

**Step 1: Write failing tests**

Add one test proving `effective_ladder` keeps all rung widths/heights equal to the current profile when `dynamic_resolution_enabled` is false. Add another test proving enabling the flag keeps existing lower-resolution rungs for the selected source aspect ratio.

**Step 2: Run tests to verify failure**

Run: `cargo test -p mrd-service media_adaptation::tests::effective_ladder_keeps_resolution_fixed_when_dynamic_resolution_is_disabled media_adaptation::tests::effective_ladder_allows_lower_resolution_when_dynamic_resolution_is_enabled -- --nocapture`

Expected: first test FAIL because current default ladder lowers resolution.

**Step 3: Implement minimal ladder shaping**

After building the effective ladder, if `dynamic_resolution_enabled` is false, copy `current_profile.width` and `current_profile.height` to every rung, then sanitize/deduplicate. If enabled, keep existing source-aspect resolution ladder behavior.

**Step 4: Run tests to verify pass**

Run: `cargo test -p mrd-service media_adaptation::tests::effective_ladder_ -- --nocapture`

Expected: PASS.

**Step 5: Commit**

Commit message: `feat: gate adaptive resolution changes`

### Task 3: Windows No-Crop Capture Backend

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`

**Step 1: Write failing tests**

Add tests for Windows backend selection:
- full-size display-shared profile keeps `DxgiShared`;
- reduced display-shared profile uses `Winrt`;
- full-size window profile can keep `WinrtWindowShared` when NVENC is available;
- reduced window profile uses `Winrt`.

**Step 2: Run tests to verify failure**

Run: `cargo test -p mrd-service windows_lan_capture_backend -- --nocapture`

Expected: reduced-profile tests FAIL because current backend selection ignores selected profile dimensions.

**Step 3: Implement minimal backend selection**

Introduce a helper that compares selected profile dimensions against the selected capture source dimensions. Use shared texture backends only when the selected profile is at least the source dimensions after even-dimension reconciliation. Use the existing WinRT CPU capture path for reduced profiles so `prepare_frame_for_h264` performs proportional full-frame scaling.

**Step 4: Run tests to verify pass**

Run: `cargo test -p mrd-service windows_lan_capture_backend -- --nocapture`

Expected: PASS.

**Step 5: Commit**

Commit message: `fix: avoid crop backends for reduced dynamic resolution`

### Task 4: UI/Automation Wiring

**Files:**
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.ts`
- Modify: `apps/Rdesk/src/app/services/lanE2eAutomationService.test.ts`
- Modify as needed: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`

**Step 1: Write failing test**

Add a service test that `buildAdaptiveMediaConfig` does not enable dynamic resolution by default and honors an override with `dynamic_resolution_enabled: true`.

**Step 2: Run test to verify failure**

Run: `pnpm --dir apps/Rdesk test -- lanE2eAutomationService.test.ts`

Expected: FAIL because the field is not present in generated config.

**Step 3: Implement minimal UI wiring**

Add `dynamic_resolution_enabled: false` to generated adaptive config. Preserve override behavior so explicit UI/test settings can turn it on.

**Step 4: Run test to verify pass**

Run: `pnpm --dir apps/Rdesk test -- lanE2eAutomationService.test.ts`

Expected: PASS.

**Step 5: Commit**

Commit message: `feat: wire explicit dynamic resolution toggle`

### Task 5: Final Verification

**Files:**
- All modified files.

**Step 1: Format**

Run: `cargo fmt --all -- --check`

Expected: PASS. If it fails, run `cargo fmt --all` and repeat the check.

**Step 2: Rust tests**

Run:
- `cargo test -p mrd-ipc`
- `cargo test -p mrd-service --lib`

Expected: PASS.

**Step 3: Frontend checks**

Run:
- `pnpm --dir apps/Rdesk test -- lanE2eAutomationService.test.ts`
- `pnpm --dir apps/Rdesk type-check`

Expected: PASS.

**Step 4: Commit any verification-only fixes**

Commit message if needed: `test: cover dynamic resolution config`
