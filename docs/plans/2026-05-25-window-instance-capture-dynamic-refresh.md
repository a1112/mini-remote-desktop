# Window Instance Capture And Dynamic Refresh Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement HWND-level software window capture as a first-class LAN/QUIC remote desktop source with dynamic FPS control and multi-window concurrency safeguards.

**Architecture:** Keep capture ownership in `mrd-service`. Rdesk selects a concrete `windows:window:0x...` source, the service stores that source per session, and the LAN media sender builds a WGC/WinRT window capture backend with explicit dynamic FPS pacing and diagnostics. Do not silently fall back to display capture when a selected window fails.

**Tech Stack:** Rust workspace (`mrd-service`, `mrd-capture-winrt`, `mrd-ipc`, `mrd-pipeline-core`), Windows.Graphics.Capture/WinRT, D3D11 shared BGRA frames, NVENC/OpenH264 encoders, QUIC LAN media sender, React/TypeScript/Vitest in `apps/Rdesk`, PowerShell benchmark scripts.

---

## Ground Rules

- Use @superpowers:test-driven-development for implementation tasks: write a failing focused test first, run it, implement the smallest code, run the passing test.
- Do not use `junk/` for architecture decisions.
- Treat `windows:window:0x...` as a concrete HWND instance. Do not implement app-level window following in this plan.
- Do not silently fall back from window capture to display capture.
- Keep queues shallow. Prefer dropping stale captured frames over adding user-visible latency.
- Commit after each task with focused passing tests.

## Current Facts

- `apps/mrd-service/src/capture_source.rs` already enumerates Windows windows as `CaptureSource { source_kind: "window", id: "windows:window:0x..." }`.
- `crates/mrd-capture-winrt/src/lib.rs` already has `WinrtCapture::from_window_handle`, `from_window_handle_shared_texture`, `with_shared_texture_output`, `capture_frame_with_timeout`, and source-closed detection.
- `apps/mrd-service/src/lan_discovery.rs` already stores source selections per session and routes display shared sources to `DxgiSharedTextureCapture`; all other Windows sources currently use `capture_source::create_frame_capture`.
- `prepare_frame_for_h264` currently rejects D3D11 shared captures when the selected profile dimensions differ from captured frame dimensions.
- Existing LAN pacing uses `media_frame_interval`, `sleep_until_media_frame`, high-resolution timer guards, and per-session metrics.
- Rdesk already displays capture source selections and can select a remote source before opening a display window.

---

### Task 1: Window Source Parsing Contract

**Files:**
- Modify: `apps/mrd-service/src/capture_source.rs`

**Step 1: Write the failing tests**

Add focused tests in the existing test module or create one at the bottom of `capture_source.rs`:

```rust
#[cfg(all(windows, test))]
#[test]
fn parse_windows_capture_source_ref_accepts_window_hwnd_hex() {
    assert_eq!(
        parse_windows_capture_source_ref("windows:window:0x1234").unwrap(),
        WindowsCaptureSourceRef::Window(0x1234)
    );
}

#[cfg(all(windows, test))]
#[test]
fn parse_windows_capture_source_ref_rejects_empty_window_hwnd() {
    let error = parse_windows_capture_source_ref("windows:window:")
        .unwrap_err()
        .to_string();

    assert!(error.contains("window"));
}
```

If the helper is not currently visible to tests, keep the tests inside the same module so private items are accessible.

**Step 2: Run test to verify it fails or exposes current behavior**

Run:

```powershell
cargo test -p mrd-service parse_windows_capture_source_ref_ -- --nocapture
```

Expected: Either fail because parsing is incomplete, or pass and document the existing contract.

**Step 3: Implement minimal parsing fixes if needed**

Ensure `parse_windows_capture_source_ref` accepts canonical hex HWND values:

```rust
"window" => {
    let hwnd = parse_window_handle(value)?;
    Ok(WindowsCaptureSourceRef::Window(hwnd))
}
```

The parser must reject empty, zero, and malformed HWND values.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service parse_windows_capture_source_ref_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/capture_source.rs
git commit -m "test: cover windows window source parsing"
```

---

### Task 2: Explicit Windows LAN Capture Backend Selection

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`

**Step 1: Write the failing tests**

Add tests near existing LAN capture backend tests:

```rust
#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_selects_winrt_window_shared_for_window_sources() {
    assert_eq!(
        windows_lan_capture_backend("windows:window:0x1234"),
        WindowsLanCaptureBackend::WinrtWindowShared
    );
}

#[cfg(windows)]
#[test]
fn windows_lan_capture_backend_keeps_dxgi_shared_for_display_shared_sources() {
    assert_eq!(
        windows_lan_capture_backend("windows:display-shared:1"),
        WindowsLanCaptureBackend::DxgiShared
    );
}
```

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service windows_lan_capture_backend_ -- --nocapture
```

Expected: FAIL or compile failure because `windows_lan_capture_backend` currently returns string labels.

**Step 3: Implement typed backend selection**

Replace the string return with a small enum:

```rust
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsLanCaptureBackend {
    DxgiShared,
    WinrtWindowShared,
    Winrt,
}

#[cfg(windows)]
fn windows_lan_capture_backend(source_id: &str) -> WindowsLanCaptureBackend {
    let normalized = source_id.trim().to_ascii_lowercase();
    if normalized.starts_with("windows:display-shared:") {
        WindowsLanCaptureBackend::DxgiShared
    } else if normalized.starts_with("windows:window:") {
        WindowsLanCaptureBackend::WinrtWindowShared
    } else {
        WindowsLanCaptureBackend::Winrt
    }
}
```

Update `create_windows_lan_frame_capture` match arms accordingly.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service windows_lan_capture_backend_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs
git commit -m "refactor: type windows lan capture backend selection"
```

---

### Task 3: Create Shared-Texture Window Capture For LAN Sender

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify if needed: `apps/mrd-service/src/capture_source.rs`

**Step 1: Write the failing tests**

Add pure parser/factory input tests that do not require an actual HWND:

```rust
#[cfg(windows)]
#[test]
fn parse_windows_window_source_id_extracts_hwnd() {
    assert_eq!(
        parse_windows_window_source_id("windows:window:0x1234").unwrap(),
        0x1234
    );
}

#[cfg(windows)]
#[test]
fn parse_windows_window_source_id_rejects_display_source() {
    let error = parse_windows_window_source_id("windows:display-shared:1")
        .unwrap_err()
        .to_string();

    assert!(error.contains("window"));
}
```

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service parse_windows_window_source_id_ -- --nocapture
```

Expected: FAIL because the helper does not exist.

**Step 3: Implement the helper and backend arm**

In `lan_discovery.rs`, add:

```rust
#[cfg(windows)]
fn parse_windows_window_source_id(source_id: &str) -> Result<isize> {
    let trimmed = source_id.trim();
    let handle = trimmed
        .strip_prefix("windows:window:")
        .ok_or_else(|| anyhow::anyhow!("Windows window source id expected, got {source_id}"))?;
    parse_window_handle_value(handle)
}
```

Use the existing parse helper from `capture_source.rs` if it can be cleanly made `pub(crate)`. Avoid duplicate parsing if possible.

In `create_windows_lan_frame_capture`, add the window arm:

```rust
WindowsLanCaptureBackend::WinrtWindowShared => {
    let hwnd = parse_windows_window_source_id(source_id)?;
    let mut capture = mrd_capture_winrt::WinrtCapture::from_window_handle_shared_texture(hwnd)
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .with_context(|| format!("failed to create WinRT shared window capture for {source_id}"))?;
    capture.start().map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(LanFrameCapture::Winrt(capture))
}
```

Keep `WindowsLanCaptureBackend::Winrt` using `crate::capture_source::create_frame_capture(source_id)?`.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service parse_windows_window_source_id_ windows_lan_capture_backend_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs apps/mrd-service/src/capture_source.rs
git commit -m "feat: create shared winrt capture for selected windows"
```

---

### Task 4: Reconcile Window Dimensions Before H.264 Encode

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`

**Step 1: Write the failing tests**

Add tests around `prepare_frame_for_h264` or a new helper:

```rust
#[cfg(windows)]
#[test]
fn h264_target_dimensions_accept_window_native_shared_size() {
    let profile = MediaProfile {
        width: 1920,
        height: 1080,
        fps: 60,
        bitrate_mbps: 20,
        ..MediaProfile::default()
    };
    let frame = CapturedFrame::from_d3d11_shared_bgra(1001, 777, 0, 42, 1001 * 4);

    let result = prepare_frame_for_h264(frame, &profile).unwrap();

    assert_eq!(result.width % 2, 0);
    assert_eq!(result.height % 2, 0);
}
```

If shared texture cropping/scaling cannot be done safely yet, write the test for a helper that decides when to request profile reconfiguration instead of testing actual frame mutation.

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service h264_target_dimensions_accept_window_native_shared_size -- --nocapture
```

Expected: FAIL with current exact-dimension rejection or compile failure if helper is needed.

**Step 3: Implement the smallest safe behavior**

Preferred first implementation:

- For D3D11 shared window frames, do not CPU-copy or stretch.
- If dimensions differ only by odd width/height, crop to even dimensions using the existing WinRT shared texture target dimensions before capture.
- If profile dimensions differ materially from captured window dimensions, trigger profile reconciliation before encoder creation rather than failing inside `prepare_frame_for_h264`.

Add a helper:

```rust
fn window_h264_capture_dimensions(width: usize, height: usize) -> (usize, usize) {
    (even_dimension(width).max(2), even_dimension(height).max(2))
}
```

Use it when creating the window capture:

```rust
let target_width = even_dimension(profile.width as usize).max(2);
let target_height = even_dimension(profile.height as usize).max(2);
capture.set_target_dimensions(target_width, target_height);
```

If exact profile dimensions are not viable for arbitrary windows, add a follow-up task before enabling hardware shared path broadly.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service h264_target_dimensions_accept_window_native_shared_size -- --nocapture
cargo test -p mrd-service prepare_frame_for_h264 -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs
git commit -m "feat: reconcile window capture dimensions for h264"
```

---

### Task 5: Dynamic FPS Policy Unit

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`

**Step 1: Write the failing tests**

Add a small pure policy type and tests:

```rust
#[test]
fn dynamic_window_fps_enters_active_tier_on_changed_frame() {
    let mut policy = DynamicWindowFpsPolicy::new(120);

    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: true,
        input_active: false,
        source_available: true,
        active_window_capture_count: 1,
    });

    assert_eq!(decision.tier, DynamicWindowFpsTier::Active);
    assert_eq!(decision.target_fps, 120);
}

#[test]
fn dynamic_window_fps_caps_idle_window() {
    let mut policy = DynamicWindowFpsPolicy::new(120);

    for _ in 0..10 {
        policy.update(DynamicWindowFpsInput {
            frame_changed: false,
            input_active: false,
            source_available: true,
            active_window_capture_count: 1,
        });
    }

    let decision = policy.current();
    assert_eq!(decision.tier, DynamicWindowFpsTier::Idle);
    assert!(decision.target_fps <= 15);
}

#[test]
fn dynamic_window_fps_reduces_background_fps_under_multi_window_pressure() {
    let mut policy = DynamicWindowFpsPolicy::new(144);

    let decision = policy.update(DynamicWindowFpsInput {
        frame_changed: true,
        input_active: false,
        source_available: true,
        active_window_capture_count: 3,
    });

    assert!(decision.target_fps <= 60);
}
```

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service dynamic_window_fps_ -- --nocapture
```

Expected: FAIL because the policy type does not exist.

**Step 3: Implement minimal policy**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicWindowFpsTier {
    Active,
    Warm,
    Idle,
    Suspended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DynamicWindowFpsDecision {
    tier: DynamicWindowFpsTier,
    target_fps: u32,
}
```

Implement simple hysteresis with counters:

- `frame_changed || input_active`: active.
- After several quiet updates: warm, then idle.
- `!source_available`: suspended.
- If `active_window_capture_count >= 3`, cap active target at 60 for the first version.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service dynamic_window_fps_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs
git commit -m "feat: add dynamic window fps policy"
```

---

### Task 6: Integrate Dynamic FPS Into LAN Sender Loop

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify if needed: `apps/mrd-service/src/app_state.rs`
- Modify if needed: `crates/mrd-ipc/src/lib.rs`

**Step 1: Write the failing tests**

Add pure tests for interval selection:

```rust
#[test]
fn media_frame_interval_uses_dynamic_window_target_when_present() {
    let profile = MediaProfile {
        fps: 144,
        ..MediaProfile::default()
    };
    let decision = DynamicWindowFpsDecision {
        tier: DynamicWindowFpsTier::Idle,
        target_fps: 12,
    };

    assert_eq!(
        media_frame_interval_for_dynamic_decision(&profile, Some(decision)),
        Duration::from_micros(83_333)
    );
}
```

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service media_frame_interval_uses_dynamic_window_target -- --nocapture
```

Expected: FAIL because the helper does not exist.

**Step 3: Implement interval helper and sender state**

Add:

```rust
fn media_frame_interval_for_dynamic_decision(
    profile: &MediaProfile,
    decision: Option<DynamicWindowFpsDecision>,
) -> Duration {
    let fps = decision
        .map(|decision| decision.target_fps)
        .unwrap_or(profile.fps)
        .max(1);
    Duration::from_micros((1_000_000 / u64::from(fps)).max(1))
}
```

In the sender loop:

- Detect whether selected source kind is `window`.
- Maintain `DynamicWindowFpsPolicy` only for window sources.
- Update it after capture attempts using a first-pass `frame_changed` signal:
  - true when capture succeeds and timestamp/dimensions differ from the previous sent frame.
  - false on repeated timeout or same dimensions/timestamp.
- Use the dynamic interval when scheduling the next frame.

Keep the existing precise sleep helper by constructing a temporary FPS value or adding a helper that accepts target FPS.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service dynamic_window_fps_ media_frame_interval_uses_dynamic_window_target -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs apps/mrd-service/src/app_state.rs crates/mrd-ipc/src/lib.rs
git commit -m "feat: pace window capture with dynamic fps"
```

---

### Task 7: Window Source Loss Does Not Fall Back To Display

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify if needed: `apps/mrd-service/src/capture_source.rs`

**Step 1: Write the failing tests**

Add a pure test for error classification:

```rust
#[cfg(windows)]
#[test]
fn invalid_window_source_error_is_source_loss_not_display_fallback() {
    let error = window_capture_source_error("windows:window:0x0", "window hwnd must not be zero");

    assert_eq!(error.code, "WINDOW_CAPTURE_SOURCE_NOT_FOUND");
    assert!(!error.message.contains("display"));
}
```

If no error type exists, create a small internal struct for tests before wiring it to session failure.

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service invalid_window_source_error_is_source_loss -- --nocapture
```

Expected: FAIL because the classification helper does not exist.

**Step 3: Implement explicit source-loss classification**

Add a helper that maps window-specific failures to clear codes:

```rust
struct CaptureSourceFailure {
    code: &'static str,
    message: String,
}
```

Use it when `create_windows_lan_frame_capture` fails for `WinrtWindowShared` and when `capture.capture_frame()` repeatedly reports item closed.

Mark the session failed with a message naming the selected source id. Do not call `default_capture_source` from any window failure path.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service invalid_window_source_error_is_source_loss -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs apps/mrd-service/src/capture_source.rs
git commit -m "fix: report lost window sources without display fallback"
```

---

### Task 8: Diagnostics For Window Capture And Dynamic FPS

**Files:**
- Modify: `crates/mrd-ipc/src/lib.rs`
- Modify: `apps/mrd-service/src/app_state.rs`
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Test: `crates/mrd-ipc/tests/contracts.rs`

**Step 1: Write the failing IPC contract test**

Add diagnostics fields to the relevant snapshot type. If `MediaSenderTransportSnapshot` is the current best home, add a contract test:

```rust
#[test]
fn media_sender_snapshot_serializes_window_dynamic_fps_fields() {
    let snapshot = MediaSenderTransportSnapshot {
        capture_source_id: Some("windows:window:0x1234".to_string()),
        capture_source_kind: Some("window".to_string()),
        capture_memory_path: Some("d3d11_shared_bgra".to_string()),
        dynamic_fps_tier: Some("active".to_string()),
        target_fps: Some(120),
        ..test_media_sender_snapshot()
    };

    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains("windows:window:0x1234"));
    assert!(json.contains("dynamic_fps_tier"));
}
```

Use existing test helpers and field names if the snapshot already has similar fields.

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-ipc media_sender_snapshot_serializes_window_dynamic_fps_fields -- --nocapture
```

Expected: FAIL because fields do not exist.

**Step 3: Implement diagnostics fields**

Add optional fields with serde defaults:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub capture_source_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub capture_source_kind: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub capture_memory_path: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub dynamic_fps_tier: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub target_fps: Option<u32>,
```

Wire them from the sender loop metrics. Use existing `MediaStageMetrics` if it is the actual snapshot carrier.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-ipc media_sender_snapshot_serializes_window_dynamic_fps_fields -- --nocapture
cargo test -p mrd-service dynamic_window_fps_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add crates/mrd-ipc/src/lib.rs crates/mrd-ipc/tests/contracts.rs apps/mrd-service/src/app_state.rs apps/mrd-service/src/lan_discovery.rs
git commit -m "feat: expose window capture dynamic fps diagnostics"
```

---

### Task 9: Rdesk Window Source Selection Coverage

**Files:**
- Modify: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx`
- Modify if needed: `apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx`
- Modify if needed: `apps/Rdesk/src/app/services/remoteDisplayLauncher.test.ts`

**Step 1: Write the failing component test**

Add a fixture:

```ts
const remoteWindowSource = {
  id: "windows:window:0x1234",
  platform: "windows",
  source_kind: "window",
  title: "Code - mini-remote-desktop",
  class_name: "Chrome_WidgetWin_1",
  width: 1600,
  height: 900,
  process_id: 4242,
  app_name: "Code",
  bundle_identifier: null,
  preview_data_url: null,
  preview_width: null,
  preview_height: null,
};
```

Test selector behavior:

```ts
it("selects a remote software window capture source", async () => {
  const mockInvoke = getMockInvoke();
  mockInvoke.mockImplementation((command: string) => {
    if (command === "ipc_list_remote_capture_sources") return Promise.resolve([remoteWindowSource]);
    if (command === "ipc_select_remote_capture_source") {
      return Promise.resolve({
        session_id: "session-1",
        source: remoteWindowSource,
        status: "selected",
        reason: null,
      });
    }
    return Promise.resolve(null);
  });

  renderRemoteDisplay("session-1");
  fireEvent.click(await screen.findByRole("button", { name: /刷新捕获源/ }));
  fireEvent.click(await screen.findByText(/Code - mini-remote-desktop/));

  expect(mockInvoke).toHaveBeenCalledWith("ipc_select_remote_capture_source", {
    sessionId: "session-1",
    sourceId: "windows:window:0x1234",
  });
});
```

Adjust labels to match existing component helpers.

**Step 2: Run test to verify it fails or exposes current behavior**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx -t "software window capture source"
```

Expected: FAIL if the UI hides or mishandles window sources.

**Step 3: Implement minimal UI fixes**

Ensure:

- `captureSourceKindLabel("window")` renders a clear window label.
- Window sources remain selectable in dropdown and modal modes.
- The selected source status uses the window title and dimensions.

Do not change source priority unless the user explicitly requests window-first defaults.

**Step 4: Run tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/services/remoteDisplayLauncher.test.ts
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/Rdesk/src/app/components/RemoteDisplayWindowPage.tsx apps/Rdesk/src/app/components/RemoteDisplayWindowPage.test.tsx apps/Rdesk/src/app/services/remoteDisplayLauncher.test.ts
git commit -m "test: cover remote software window source selection"
```

---

### Task 10: Multi-Window Concurrency Coverage

**Files:**
- Modify: `apps/mrd-service/src/lan_discovery.rs`
- Modify if needed: `crates/mrd-session/src/scheduler.rs`

**Step 1: Write the failing tests**

Add service-level pure tests:

```rust
#[tokio::test]
async fn capture_source_selection_tracks_different_windows_per_session() {
    let app_state = Arc::new(AppState::default());
    let session_a = SessionId("window-a".to_string());
    let session_b = SessionId("window-b".to_string());

    store_capture_source_selection(
        &app_state,
        &session_a,
        CaptureSourceSelection {
            session_id: session_a.clone(),
            source: test_window_capture_source("windows:window:0x1111"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    store_capture_source_selection(
        &app_state,
        &session_b,
        CaptureSourceSelection {
            session_id: session_b.clone(),
            source: test_window_capture_source("windows:window:0x2222"),
            status: "selected".to_string(),
            reason: None,
        },
    )
    .await;

    assert_eq!(
        selected_capture_source_id(&app_state, &session_a).await.unwrap(),
        "windows:window:0x1111"
    );
    assert_eq!(
        selected_capture_source_id(&app_state, &session_b).await.unwrap(),
        "windows:window:0x2222"
    );
}
```

**Step 2: Run test to verify it fails**

Run:

```powershell
cargo test -p mrd-service capture_source_selection_tracks_different_windows_per_session -- --nocapture
```

Expected: FAIL only if helper fixtures or selection behavior are missing.

**Step 3: Implement missing fixture/helper and concurrency cap**

Add `test_window_capture_source(id: &str) -> CaptureSource` in tests.

If active-window capture count is needed by dynamic FPS policy, derive it from `app_state.capture_sources` plus active sessions. Keep the first version simple:

```rust
async fn active_window_capture_count(app_state: &Arc<AppState>) -> usize {
    app_state
        .capture_sources
        .lock()
        .await
        .selections()
        .filter(|selection| selection.source.source_kind == "window")
        .count()
}
```

Use existing registry APIs if names differ.

**Step 4: Run tests**

Run:

```powershell
cargo test -p mrd-service capture_source_selection_tracks_different_windows_per_session dynamic_window_fps_ -- --nocapture
```

Expected: PASS.

**Step 5: Commit**

```powershell
git add apps/mrd-service/src/lan_discovery.rs crates/mrd-session/src/scheduler.rs
git commit -m "feat: account for concurrent window capture sessions"
```

---

### Task 11: Windows Manual Canaries

**Files:**
- Modify if needed: `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1`
- Modify if needed: `tests/benchmarks/scripts/paired_lan_canary_common.ps1`
- Optional doc update: `docs/plans/2026-05-25-window-instance-capture-dynamic-refresh-design.md`

**Step 1: Run script unit tests**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: PASS.

**Step 2: Build service and Rdesk backend**

Run:

```powershell
cargo build -p mrd-service -p app
```

Expected: PASS.

**Step 3: Enumerate capture sources**

Use Rdesk capture source UI or IPC to find a real window source id. Record exact source ids:

```text
windows:window:0x...
```

Expected: at least one visible software window source is available.

**Step 4: Run single-window canary**

If the benchmark script already accepts `-CaptureSourceId`, run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p60 -Codec h264 -CaptureSourceId windows:window:0x1234 -DurationSecs 10 -NoBuild
```

Expected:

- Report contains requested and actual capture source id.
- Actual source kind is `window`.
- No display fallback message.
- Capture/encode/send p95 are present.

**Step 5: Run two-window canary**

Start two sessions or run the available multi-terminal stress harness with two different window source ids. If no script exists, document the gap and do not claim multi-window hardware validation.

Expected:

- Both sessions report different `windows:window:*` source ids.
- Dynamic FPS tiers and observed FPS are visible.
- No unbounded queue growth or severe latency buildup.

**Step 6: Commit only if scripts/docs changed**

```powershell
git add tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 tests/benchmarks/scripts/paired_lan_canary_common.ps1 docs/plans/2026-05-25-window-instance-capture-dynamic-refresh-design.md
git commit -m "test: document window capture canaries"
```

---

### Task 12: Full Verification

**Files:** no edits unless verification reveals a bug.

**Step 1: Format**

Run:

```powershell
cargo fmt --all -- --check
```

Expected: PASS. If it fails, run `cargo fmt --all`, inspect `git diff`, and commit formatting with the relevant task.

**Step 2: Rust service tests**

Run:

```powershell
cargo test -p mrd-service windows_lan_capture_backend_ parse_windows_window_source_id_ dynamic_window_fps_ -- --nocapture
cargo test -p mrd-service capture_source_selection_tracks_different_windows_per_session -- --nocapture
```

Expected: PASS.

**Step 3: IPC contract tests**

Run:

```powershell
cargo test -p mrd-ipc media_sender_snapshot_serializes_window_dynamic_fps_fields -- --nocapture
cargo test -p mrd-ipc
```

Expected: PASS.

**Step 4: Frontend tests**

Run:

```powershell
pnpm --dir apps/Rdesk test -- --run src/app/components/RemoteDisplayWindowPage.test.tsx src/app/services/remoteDisplayLauncher.test.ts
pnpm --dir apps/Rdesk type-check
```

Expected: PASS.

**Step 5: Benchmark script tests**

Run:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
```

Expected: PASS.

**Step 6: Hardware canaries**

Run the single-window and two-window manual canaries from Task 11 on a Windows machine with visible software windows. Record exact HWND source ids, observed FPS, dynamic FPS tier behavior, and p95 timings.

Expected: PASS or clearly documented hardware/environment blocker.

**Step 7: Final commit**

If verification produces any final fixes:

```powershell
git status --short
git add <changed-files>
git commit -m "feat: implement window instance capture"
```

Expected: clean working tree except intentionally untracked reports.

---

## Acceptance Criteria

- Rdesk can select a concrete `windows:window:0x...` software window source for a remote session.
- `mrd-service` creates a WGC/WinRT window capture backend for that source.
- The selected window stream does not include unrelated desktop contents.
- Window capture failure or closure reports source loss and does not fall back to display capture.
- Dynamic FPS reduces idle window work and restores active FPS quickly.
- Multiple window sessions can run at the same time with bounded queues and per-session diagnostics.
- Focused tests and type checks pass.
- Windows manual canary results document exact source ids, FPS behavior, and latency metrics.
