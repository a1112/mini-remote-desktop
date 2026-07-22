# NVENC 720p Decode Matrix Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a fixed-sample `NVENC 1280x720` decode component-matrix case so decode latency and throughput are measured independently from encode and transport.

**Architecture:** Keep `mrd-decode` as the decode boundary owner. Generate a deterministic `NVENC 720p` H264 access-unit sample set inside `mrd-decode` test support, then add a new ignored perf test that measures `push_access_unit + drain_decoded_frames` against those samples. Wire the case into `tests/component-matrix` so the result lands in the same artifact layout and uses the same summary pipeline as existing decode cases.

**Tech Stack:** Rust, `mrd-decode`, `mrd-encode-nvenc`, PowerShell component-matrix scripts, JSON case manifests.

---

### Task 1: Add the failing NVENC 720p decode perf test

**Files:**
- Modify: `crates/mrd-decode/tests/perf_decode.rs`

**Step 1: Write the failing test**

Add a new ignored test named `perf_nvenc_720p_decode_reports_latency_distribution` that:
- reads `MRD_COMPONENT_CASE_NAME`, defaulting to `decode.nvenc_720p`
- creates the decoder with `create_decoder("h264_software")`
- requests a fixed sample set from a helper like `nvenc_720p_access_units()`
- asserts the result metadata is for `1280x720`
- writes a `ComponentResult`

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode perf_nvenc_720p_decode_reports_latency_distribution -- --ignored --nocapture`

Expected: FAIL because the fixed-sample helper does not exist yet.

**Step 3: Write minimal implementation**

Add the smallest helper stub in the same test file or test helper module so the test compiles but still needs real sample generation.

**Step 4: Run test to verify the failure is now about missing behavior**

Run the same command.

Expected: FAIL because no valid `NVENC 720p` access-unit sample set is returned yet.

**Step 5: Commit**

```bash
git add crates/mrd-decode/tests/perf_decode.rs
git commit -m "test: add nvenc 720p decode perf case scaffold"
```

### Task 2: Implement fixed NVENC 720p sample generation for decode tests

**Files:**
- Modify: `crates/mrd-decode/tests/perf_decode.rs`
- Modify: `crates/mrd-encode-nvenc/src/lib.rs` only if test support needs a stable constructor that already exists but lacks visibility

**Step 1: Write the failing test for the helper**

Add a focused helper test that:
- generates or loads `NVENC 720p` access units
- expects at least one sample
- expects at least one keyframe-capable AU in the set

**Step 2: Run test to verify it fails**

Run: `cargo test -p mrd-decode nvenc_720p_sample_set_is_usable_for_decode_perf -- --nocapture`

Expected: FAIL because the helper still returns no usable samples.

**Step 3: Write minimal implementation**

Implement a deterministic helper inside `perf_decode.rs` or a nearby test helper that:
- tries `NvencH264Encoder::new_baseline(1280, 720, 30)`
- builds a small synthetic BGRA frame pattern
- encodes a bounded number of frames
- collects non-empty H264 access units into a `Vec<Vec<u8>>`
- returns early if NVENC is unavailable, consistent with current hardware-gated matrix behavior

**Step 4: Run tests to verify they pass**

Run:
- `cargo test -p mrd-decode nvenc_720p_sample_set_is_usable_for_decode_perf -- --nocapture`
- `cargo test -p mrd-decode perf_nvenc_720p_decode_reports_latency_distribution -- --ignored --nocapture`

Expected: PASS on supported NVIDIA hosts; on unsupported hosts, the perf result should still be emitted with zero/empty throughput behavior rather than crashing.

**Step 5: Commit**

```bash
git add crates/mrd-decode/tests/perf_decode.rs crates/mrd-encode-nvenc/src/lib.rs
git commit -m "feat: add nvenc 720p decode perf samples"
```

### Task 3: Wire the new decode case into the component matrix

**Files:**
- Create: `tests/component-matrix/cases/decode.nvenc_720p.json`
- Modify: `tests/component-matrix/scripts/run_component_matrix.ps1`
- Modify: `tests/component-matrix/README.md`

**Step 1: Write the failing matrix wiring**

Add `decode.nvenc_720p.json` with:
- `component = "decode"`
- `crate = "mrd-decode"`
- `backend = "nvenc_720p"`
- `test_name = "perf_nvenc_720p_decode_reports_latency_distribution"`
- `case_name = "decode.nvenc_720p"`

Append it to `run_component_matrix.ps1`.

**Step 2: Run the single case to verify failure mode**

Run: `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/decode.nvenc_720p.json -RepoRoot .`

Expected: PASS if the perf test already works; otherwise fail with the test error, not with harness errors.

**Step 3: Update docs minimally**

Document the new decode case in `tests/component-matrix/README.md` and note that it is hardware-gated because the sample set is NVENC-backed.

**Step 4: Run the case and full decode verification**

Run:
- `cargo test -p mrd-decode -- --nocapture`
- `cargo test -p mrd-decode perf_nvenc_720p_decode_reports_latency_distribution -- --ignored --nocapture`
- `powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 -CasePath tests/component-matrix/cases/decode.nvenc_720p.json -RepoRoot .`

Expected:
- decode crate tests stay green
- the new case writes artifacts under `artifacts/component-matrix/.../decode/...`

**Step 5: Commit**

```bash
git add tests/component-matrix/cases/decode.nvenc_720p.json tests/component-matrix/scripts/run_component_matrix.ps1 tests/component-matrix/README.md
git commit -m "feat: add nvenc 720p decode component matrix case"
```
