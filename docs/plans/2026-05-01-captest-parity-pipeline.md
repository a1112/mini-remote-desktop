# CapTest Parity Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bring mini-remote-desktop/Rdesk Windows capture, encode, decode, and render paths to CapTest parity, with reusable pipeline comparison metrics.

**Architecture:** Keep CapTest and mini-remote-desktop independent, but use compatible metric field names so results can be compared side by side. In mini-remote-desktop, make `mrd-pipeline-core` and the existing Rdesk test harness the shared contract for tests and UI, then remove CPU fallback from the high-performance AV1 path by supporting D3D11 shared texture input and direct D3D11 shared BGRA rendering.

**Tech Stack:** Rust workspace, D3D11 shared textures, NVENC AV1, NVDEC, Rdesk Tauri test harness, CapTest C++ D3D11/D3D12 comparison tools.

---

### Task 1: Document and Lock the Parity Contract

**Files:**
- Create: `docs/plans/2026-05-01-captest-parity-pipeline.md`
- Modify later: `crates/mrd-pipeline-core/src/lib.rs`

**Step 1: Capture existing gaps**

Record the current gaps:
- `mrd-encode-nvenc-av1` uploads CPU BGRA via `UpdateSubresource`.
- Rdesk rejects `zero_copy=true` with `EncoderType::NvencAv1`.
- Direct captured shared BGRA frames cannot be converted to `RenderFrame`.
- Windows capture-render demo has no Windows direct visual path.

**Step 2: Use compatible result fields**

The reusable comparison output should preserve these CapTest-compatible fields when possible:
- `pipeline`
- `codec`
- `memory_path`
- `frames`
- `encoded_units`
- `decoded_frames`
- `encode_failures`
- `decode_failures`
- `avg_capture_time_ms`
- `avg_encode_time_ms`
- `avg_decode_time_ms`
- `avg_render_time_ms`
- `avg_present_time_ms`
- `total_bitstream_bytes`

**Step 3: Commit the design checkpoint**

Run:

```powershell
git add docs/plans/2026-05-01-captest-parity-pipeline.md
git commit -m "docs: plan captest parity pipeline"
```

Expected: the documentation checkpoint is isolated from implementation changes.

### Task 2: Add Tests for AV1 Shared Texture Input

**Files:**
- Modify: `crates/mrd-encode-nvenc-av1/src/lib.rs`
- Compare: `crates/mrd-encode-nvenc/src/lib.rs`

**Step 1: Write the failing test**

Add a non-hardware unit test that asserts the Windows `NvencAv1Encoder` advertises `FrameMemoryKind::D3D11SharedBgra` through the `VideoEncoder` implementation contract. This should fail because the current implementation does not override `input_memory_kind()`.

**Step 2: Implement shared input support**

Port the H.264 shared texture input pattern:
- Add a `SharedInputResource` field.
- Open the incoming `D3D11SharedBgraFrame` handle with `ID3D11Device::OpenSharedResource`.
- Register the shared texture with NVENC using ARGB/BGRA-compatible buffer format.
- Use map/encode/unmap on the shared resource instead of uploading CPU BGRA.
- Keep CPU fallback for synthetic tests and non-zero-copy modes.

**Step 3: Remove Rdesk guard**

Remove the explicit Rdesk error for `EncoderType::NvencAv1` with `zero_copy=true`.

**Step 4: Verify**

Run:

```powershell
cargo test -p mrd-encode-nvenc-av1
cargo test -p Rdesk test_harness
```

Expected: unit tests pass; hardware-dependent tests remain ignored unless explicitly requested.

### Task 3: Add Direct Shared BGRA Render Support

**Files:**
- Modify: `crates/mrd-render/src/lib.rs`
- Modify: `crates/mrd-render-d3d11/src/lib.rs`
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write failing tests**

Add contract tests for:
- `RenderPixelFormat::D3D11SharedBgra` appears in the D3D11 descriptor on Windows.
- `CapturedFrame::from_d3d11_shared_bgra(...)` can become a renderable `RenderFrame` without CPU data.

**Step 2: Implement render frame type**

Add:
- `RenderPixelFormat::D3D11SharedBgra`
- `RenderFrameData::D3D11SharedBgra { shared_handle, width, height, row_pitch }`
- `RenderFrame::from_d3d11_shared_bgra(...)`

**Step 3: Implement D3D11 presentation path**

Open the shared BGRA texture in `mrd-render-d3d11` and render or copy it to the swapchain backbuffer on the GPU. Do not map to CPU.

**Step 4: Wire Rdesk direct path**

Update `captured_frame_to_render_frame` so shared BGRA captured frames go to the new render type.

**Step 5: Verify**

Run:

```powershell
cargo test -p mrd-render
cargo test -p mrd-render-d3d11
cargo test -p Rdesk test_harness
```

Expected: shared BGRA contract tests pass.

### Task 4: Add Reusable Pipeline Comparison Runner

**Files:**
- Modify: `crates/mrd-pipeline-core/src/lib.rs`
- Create or modify: `tests/pipeline-compare`
- Modify: `apps/Rdesk/src-tauri/src/test_harness.rs`

**Step 1: Write tests for metric serialization**

Add tests that serialize direct, encode-only, and encode-decode-render results with CapTest-compatible field names.

**Step 2: Implement shared metric structs**

Add a reusable comparison result schema in a core crate or test support crate that can be used by CLI tests and Rdesk commands.

**Step 3: Add Windows runner**

Implement pipeline modes:
- `capture-render`
- `capture-encode`
- `capture-encode-decode-render`

Each mode should record capture/encode/decode/render timings, frame counts, failures, memory path, and codec.

**Step 4: Verify**

Run the runner with DXGI shared capture, NVENC AV1, NVDEC AV1, and D3D11 renderer when hardware supports it.

### Task 5: Keep CapTest Independent but Comparable

**Files:**
- Modify in `D:\Project\TestProject\CapTest` only where missing fields or scripts are needed.

**Step 1: Preserve CapTest implementation**

Do not make CapTest depend on mini-remote-desktop. Keep the C++ D3D11/D3D12/NVENC test base independent.

**Step 2: Align result fields**

Add any missing metric fields required to compare with mini-remote-desktop output.

**Step 3: Run both sides**

Run CapTest C++ pipeline comparison and mini-remote-desktop Rust/Rdesk comparison on the same machine.

**Step 4: Report**

Summarize:
- direct capture-render latency/fps
- AV1 encode latency/fps
- AV1 decode-render latency/fps
- GPU memory path vs CPU fallback
- hardware capability gaps
