# Android Bidirectional Remote Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Install an Android application on the Xiaomi 14 Pro that can control a Windows desktop and, with Android consent, be controlled from the desktop.

**Architecture:** Add reusable gateway logic under `crates/mrd-mobile-gateway`, expose it from `mrd-service`, and run `apps/mrd-mobile-gateway` in the interactive Windows session for desktop capture. Build an independent Android application under `apps/Rdesk-Mobile`. Reuse Windows capture and input adapters; use Android MediaProjection and AccessibilityService for the reverse direction. Pair each side with a secret and terminate streams on disconnect.

**Tech Stack:** Rust, axum WebSocket, DXGI, Windows SendInput, Java Android SDK 36, Gradle 8.13/Android Gradle Plugin 8.13.2, JUnit/Rust tests, ADB.

---

### Task 1: Protocol and security

**Files:** Create `crates/mrd-mobile-gateway/src/lib.rs` and `apps/mrd-mobile-gateway/src/main.rs`; modify `apps/mrd-service/src/lib.rs`, `apps/mrd-service/src/web_bridge.rs`, `apps/mrd-service/Cargo.toml`.

1. Write failing Rust tests for token validation, role authorization, normalized coordinate bounds, oversized JPEG rejection, and frame dimensions.
2. Run `cargo test -p mrd-mobile-gateway` and confirm the expected failures.
3. Implement bounded protocol parsing and guarded WebSocket routes; default all mobile endpoints off.
4. Rerun focused tests and `cargo check -p mrd-service --lib`.

### Task 2: Desktop to phone media and input

**Files:** Modify `crates/mrd-mobile-gateway/src/lib.rs`; create `crates/mrd-mobile-gateway/src/mobile_phone_page.html` and `tests/mobile-gateway/smoke.py`.

1. Test that only one publishing phone supplies the current frame, stale frames disappear on disconnect, and control events go only to the active phone.
2. Implement relay and browser page with scaled touch mapping, keyboard/system actions, explicit disconnect, and connection feedback.
3. Run focused Rust tests and the live WebSocket smoke test.

### Task 3: Phone to desktop media and input

**Files:** Modify `crates/mrd-mobile-gateway/src/lib.rs`.

1. Test control event mapping and refusal without an authenticated active controller.
2. Capture the primary display with `mrd-capture-dxgi`, encode bounded JPEG frames, and inject validated input through `mrd-input`.
3. Run Rust tests and verify actual screen frames from a local WebSocket client.

### Task 4: Android controller

**Files:** Create `apps/Rdesk-Mobile/settings.gradle`, `build.gradle`, `app/build.gradle`, manifest, Java classes and resources.

1. Test protocol and coordinate mapping with local JVM tests where possible.
2. Build native pairing/settings UI and desktop viewer with frame decode, touch, scroll and keyboard controls.
3. Build debug APK with Gradle; install and launch on device via ADB; inspect logcat.

### Task 5: Android target

**Files:** Create `ProjectionService.java`, `RemoteAccessibilityService.java`, service metadata and permission UI.

1. Test session state and input validation without requiring privileged Android services.
2. Implement MediaProjection foreground service, ImageReader/JPEG stream and consent revocation cleanup.
3. Implement AccessibilityService gesture and system action dispatch, gated by active approved capture session.
4. Build and install updated APK; request manual Android consent and accessibility enablement; verify PC sees and controls live phone screen.

### Task 5a: LAN discovery

**Files:** Modify `apps/mrd-mobile-gateway/src/main.rs`; create `apps/Rdesk-Mobile/app/src/main/java/com/a1112/rdeskmobile/LanDiscovery.java`.

1. Write failing discovery response tests, including a check that no pairing secret is advertised.
2. Add UDP discovery on port 9535, and Android broadcast plus saved-address unicast probes.
3. Verify a live response from the Xiaomi phone over Wi-Fi; retain manual address entry for segmented networks.

### Task 6: End-to-end verification

**Files:** Update user-facing setup instructions under `apps/Rdesk-Mobile/README.md`.

1. Run focused tests, Android build, `adb install`, `adb shell am start`, and crash log inspection.
2. Verify both directions on real device, authentication denial, cleanup, and report measured limits.
3. Review the diff for security and lifecycle issues, fixing any found.
