# H264 NVDEC Rollout Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Turn H264 NVDEC into a publishable rollout policy with persistence, runtime fallback, and visible policy state across backend and frontend.

**Architecture:** Add a persisted decoder policy on the Tauri side, load it into `WebrtcHost`, and use that policy to drive stable H264 decoder selection. Keep rollout safe by retaining software-first `auto`, explicit `nvdec` preference with fallback, and clear session diagnostics.

**Tech Stack:** Rust, Tauri, serde, tokio, existing WebRTC host/runtime code, existing React settings/session surfaces.

---

### Task 1: Add persisted decoder policy state

**Files:**
- Create: `apps/Rdesk/src-tauri/src/app_settings.rs`
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Test: `apps/Rdesk/src-tauri/src/app_settings.rs`

**Step 1: Write the failing test**

Add tests for:
- missing settings file defaults to `auto`
- saving then loading preserves `software` and `nvdec`

**Step 2: Run test to verify it fails**

Run: `cargo test -p app app_settings -- --nocapture`
Expected: FAIL because settings module and persistence helpers do not exist yet

**Step 3: Write minimal implementation**

Implement:
- `DecodePolicy`
- `AppSettings`
- load/save helpers under a stable local file path

**Step 4: Run test to verify it passes**

Run: `cargo test -p app app_settings -- --nocapture`
Expected: PASS

### Task 2: Wire decoder policy into `WebrtcHost`

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Test: `apps/Rdesk/src-tauri/src/webrtc_host.rs`

**Step 1: Write the failing test**

Add tests for:
- `auto` resolves to software-first order
- `software` disables NVDEC attempts
- `nvdec` resolves to NVDEC-first order

**Step 2: Run test to verify it fails**

Run: `cargo test -p app h264_decoder_selection -- --nocapture`
Expected: FAIL because the host still uses the old boolean preference model

**Step 3: Write minimal implementation**

Implement:
- policy enum usage inside `WebrtcHost`
- policy-aware backend order helper
- snapshot fields for policy and fallback information

**Step 4: Run test to verify it passes**

Run: `cargo test -p app h264_decoder_selection -- --nocapture`
Expected: PASS

### Task 3: Add runtime fallback accounting and Tauri policy commands

**Files:**
- Modify: `apps/Rdesk/src-tauri/src/main.rs`
- Modify: `apps/Rdesk/src-tauri/src/webrtc_host.rs`
- Test: `apps/Rdesk/src-tauri/src/main.rs`

**Step 1: Write the failing test**

Add tests for:
- Tauri helper roundtrip for decoder policy read/write
- snapshot response includes policy and fallback fields

**Step 2: Run test to verify it fails**

Run: `cargo test -p app nvdec_policy -- --nocapture`
Expected: FAIL because commands/responses do not yet expose the structured policy state

**Step 3: Write minimal implementation**

Implement:
- Tauri commands for reading/updating decoder policy
- startup loading of persisted settings into managed app state
- snapshot response mapping for policy and fallback diagnostics

**Step 4: Run test to verify it passes**

Run: `cargo test -p app nvdec_policy -- --nocapture`
Expected: PASS

### Task 4: Replace experimental toggle with formal decoder policy UI

**Files:**
- Modify: `apps/Rdesk/src/app/services/realtimeService.ts`
- Modify: `apps/Rdesk/src/app/services/realtimeService.test.ts`
- Modify: `apps/Rdesk/src/app/components/SettingsModal.tsx`
- Modify: `apps/Rdesk/src/app/components/RemoteSessionPage.tsx`

**Step 1: Write the failing test**

Update service tests to expect:
- policy read command
- policy update command

**Step 2: Run test to verify it fails**

Run: frontend service tests if available; otherwise rely on TypeScript-facing API mismatch review
Expected: FAIL or static mismatch because the boolean preference API no longer matches

**Step 3: Write minimal implementation**

Implement:
- `DecoderPolicy` type
- service wrappers for read/write
- selector UI in settings and remote session overlay
- fallback metrics rendering

**Step 4: Run test to verify it passes**

Run: frontend tests if available; otherwise keep Rust regressions green and manually inspect the changed TS surfaces
Expected: PASS where runnable

### Task 5: Full verification and cleanup

**Files:**
- Modify: touched files only as needed for cleanup

**Step 1: Format**

Run: `cargo fmt`

**Step 2: Run backend regressions**

Run:
- `cargo test -p app -- --nocapture`
- `cargo test -p mrd-decode-nvdec -- --nocapture`
- `cargo test -p mrd-decode nvdec -- --nocapture`

Expected: all PASS

**Step 3: Summarize frontend verification gap**

Record that `vite` / `vitest` cannot be executed in the current environment if `node_modules` are still unavailable.
