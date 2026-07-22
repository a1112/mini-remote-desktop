# Session Agent Native Render Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Deliver a truthful Windows Session Agent Render capability that receives authorized H.264 access units, prefers NVDEC/D3D11 shared textures, falls back to software decode, and presents into the exact Rdesk-owned HWND.

**Architecture:** Add a production render adapter behind the existing `RenderAdapter` port. Each authorized render resource owns a bounded worker, decoder, and D3D11 renderer; production bootstrap advertises Render only when the adapter factory is viable, while `mrd-service` retains all session and route policy.

**Tech Stack:** Rust, Tokio agent IPC, `mrd-decode`, `mrd-pipeline-core`, `mrd-render`, `mrd-render-d3d11`, Windows HWND/D3D11, Cargo tests, PowerShell canary.

---

### Task 1: Make media capabilities adapter-truthful

**Files:**
- Modify: `apps/mrd-session-agent/src/capture.rs`
- Modify: `apps/mrd-session-agent/src/render.rs`
- Modify: `apps/mrd-session-agent/src/media.rs`

**Steps:**

1. Add failing tests proving `MediaExecutor` advertises only capabilities reported available by its adapters.
2. Run `cargo test -p mrd-session-agent media::tests::capabilities --lib` and verify the fixed Capture+Render implementation fails.
3. Add `is_available()` to both adapter ports and build capabilities from those results.
4. Re-run the targeted and full media unit tests.
5. Commit with `refactor: report truthful agent media capabilities`.

### Task 2: Define decoded-frame to render-frame conversion

**Files:**
- Create: `apps/mrd-session-agent/src/windows_render.rs`
- Modify: `apps/mrd-session-agent/src/lib.rs`
- Modify: `apps/mrd-session-agent/Cargo.toml`

**Steps:**

1. Add failing Windows unit tests for RGB24, BGRA32, shared NV12, shared P010, and unsupported CPU I420/NV12 conversion behavior.
2. Run the exact `windows_render::tests::decoded_frame_conversion` tests and verify the conversion API is missing.
3. Add dependencies on `mrd-decode`, `mrd-pipeline-core`, `mrd-render`, and `mrd-render-d3d11` under Windows.
4. Implement lossless mappings for renderer-native formats; keep software YUV conversion isolated behind a converter port so unsupported data fails closed rather than being mislabeled.
5. Re-run tests and commit with `feat: map decoded frames for agent rendering`.

### Task 3: Select NVDEC with software fallback

**Files:**
- Modify: `apps/mrd-session-agent/src/windows_render.rs`

**Steps:**

1. Add failing tests with injected factories proving `nvdec_d3d11_shared` is tried first, `h264_software` is used only after hardware initialization failure, and total failure makes the factory unavailable.
2. Run the targeted selection tests and observe failure.
3. Implement a decoder-factory port plus production factory using `mrd_decode::create_decoder`.
4. Add the software I420-to-BGRA conversion required by the fallback and test odd/invalid dimensions and undersized planes.
5. Re-run tests and commit with `feat: add hybrid agent render decoder selection`.

### Task 4: Own HWND rendering in a bounded worker

**Files:**
- Modify: `apps/mrd-session-agent/src/windows_render.rs`
- Modify: `apps/mrd-session-agent/src/render.rs`

**Steps:**

1. Add failing tests for exact HWND attachment, successful frame presentation, queue saturation/replacement policy, keyframe preservation, worker failure, exact stop, and session/resource mismatch.
2. Run targeted worker tests and verify failure.
3. Implement a renderer-factory port and production `D3d11RendererFactory`; start one worker per resource and wait for initialization acknowledgement before returning success.
4. Use an explicitly bounded queue and expose enqueue/replacement/decode/present counters without copying payloads for diagnostics.
5. Make stop join the exact worker and make failures permanently reject subsequent units.
6. Re-run tests and commit with `feat: render agent media into authorized HWND`.

### Task 5: Assemble the production executor

**Files:**
- Modify: `apps/mrd-session-agent/src/bootstrap.rs`
- Modify: `apps/mrd-session-agent/src/capture.rs`
- Modify: `apps/mrd-session-agent/src/media.rs`
- Test: `apps/mrd-session-agent/tests/process_bootstrap.rs`

**Steps:**

1. Add failing bootstrap tests proving a real Windows process advertises Render only when production render initialization succeeds and never advertises unassembled Capture.
2. Run the targeted bootstrap test and verify the empty executor fails the expected capability assertion.
3. Replace `EmptyAuthorizedCommandExecutor` with `MediaExecutor<UnavailableCaptureAdapter, WindowsRenderAdapter>` on Windows; retain the empty executor for unsupported platforms.
4. Ensure runtime shutdown and authority revocation stop and join all render workers.
5. Run all `mrd-session-agent` tests and commit with `feat: enable truthful session agent rendering`.

### Task 6: Prove the dual-process boundary

**Files:**
- Create: `apps/mrd-session-agent/tests/media_grants.rs`
- Create: `tests/integration/service_agent_media.rs`
- Modify: `tests/integration/Cargo.toml`
- Modify: `apps/mrd-service/src/lan_discovery/media_render_worker.rs`

**Steps:**

1. Add a failing deterministic integration test using synthetic decoder/renderer factories. Prove authorized encoded units reach the exact agent resource, revocation clears queued work, and rejected agent ownership never falls back to local decode.
2. Run `cargo test --manifest-path tests/integration/Cargo.toml --test service_agent_media` and verify failure for the missing harness or behavior.
3. Add only the integration seams needed to run the service-agent pair and surface boundary metrics.
4. Re-run the integration test and the service route tests.
5. Commit with `test: cover dual-process agent media rendering`.

### Task 7: Benchmark and final verification

**Files:**
- Modify: `tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1`
- Modify: relevant benchmark artifact schema/tests if required

**Steps:**

1. Add a failing script/schema assertion requiring agent boundary enqueue, decode, present, replacement, and fallback-mode fields.
2. Implement the metric collection and artifact output.
3. Run `cargo test --manifest-path tests/integration/Cargo.toml --test service_agent_media`.
4. Run `powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p60 -DurationSecs 30` on an interactive Windows desktop.
5. Run `cargo test -p mrd-agent-ipc`, `cargo test -p mrd-session-agent`, `cargo test -p mrd-service --lib`, relevant integration tests, `cargo check`, `cargo fmt --all -- --check`, and `git diff --check`.
6. Record unsupported hardware/environment gates explicitly; do not infer runtime success from unit tests.
7. Commit with `refactor: execute desktop rendering in session agent`.

