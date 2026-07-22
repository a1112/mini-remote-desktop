# Windows Low-Latency Rust Matrix Design

**Goal:** Define the Windows-first low-latency streaming matrix for this repo, including which Rust ecosystem implementations should become first-class matrix entries and which should remain secondary candidates.

**Scope:** Windows host capture, H264 encode/decode, WebRTC and QUIC transport, and D3D11 rendering. This design intentionally excludes cross-platform-first expansion, AV1-first work, and broad UI/runtime concerns.

---

## Current Mainline

The repo already has a usable Windows-first baseline:

- `mrd-capture-dxgi`
- `mrd-encode-openh264`
- `mrd-decode`
- `mrd-transport-webrtc`
- `mrd-render-d3d11`
- `mrd-observability`
- `tests/component-matrix/`
- single-process composed pipeline tests inside `apps/Rdesk/src-tauri/src/webrtc_host.rs`

This baseline proves the current architecture is correct:

- single-component validation
- host-orchestrated composed validation
- benchmark-style result capture

The next step is not to replace this path. The next step is to extend it with additional implementations that fit the same validation model.

---

## Design Principles

1. Windows low-latency path stays primary.
2. Every new backend must enter the same validation ladder:
   - component matrix
   - single-process composed pipeline
   - benchmark harness
3. New implementations integrate behind `crates/*` interfaces, not directly inside `src-tauri`.
4. Existing working paths remain as the baseline reference for regressions.
5. Candidate crates are judged by fit for this product, not by ecosystem popularity.

---

## Candidate Classification

### First-Class Matrix Targets

These should be integrated into the actual matrix and composed tests.

#### Transport

- `webrtc-rs`
  - Keep as the WebRTC baseline.
  - Already integrated and validated in the repo.

- `quinn`
  - Make this the first QUIC implementation for the mainline matrix.
  - Reason:
    - pure Rust
    - async-first
    - stable Windows support
    - straightforward datagram support for low-latency media transport
    - lower integration cost than `quiche` or `s2n-quic`

#### Encode / Decode

- `OpenH264`
  - Keep as the CPU baseline for H264 encode/decode comparisons.
  - Reason:
    - working now
    - deterministic enough for matrix baselines
    - useful fallback when hardware paths are unavailable

#### Capture / Render

- `DXGI`
  - Keep as the primary capture path.

- `D3D11`
  - Keep as the primary render path.

### Secondary Candidates

These should stay in the roadmap, but should not displace the first implementation wave.

#### Transport

- `quiche`
  - Strong candidate for a second QUIC backend after `quinn`.
  - Better treated as a comparative transport candidate, not the first integration.

- `s2n-quic`
  - Strong engineering quality, but not the best first fit for this repo’s immediate Windows streaming path.

#### Capture

- Windows Graphics Capture wrappers
  - Good second capture path after DXGI.
  - Integrate only after QUIC is in the matrix.

#### Hardware Codec Paths

- NVENC / NVDEC Rust bindings
  - Worth integrating, but behind repo-owned crates like:
    - `mrd-encode-nvenc`
    - `mrd-decode-nvdec`
  - Do not make the repo depend directly on an ecosystem crate shape as the public boundary.

### Not Recommended As Primary Mainline Backends

- generic `ffmpeg`-centric transport/media shells
- `wgpu` as the first Windows low-latency render path
- cross-platform abstractions that weaken direct DXGI/D3D11 control

These may still be useful in tooling or compatibility layers, but they should not define the low-latency mainline.

---

## Target Matrix

The intended matrix should evolve to this shape.

### Capture

- `dxgi`
- `wgc` later

### Encode

- `openh264`
- `nvenc` later

### Decode

- `h264_software`
- `nvdec` later

### Transport

- `webrtc`
- `quic_quinn`
- `quic_quiche` later

### Render

- `d3d11`

---

## Validation Model

Every implementation must integrate into the same 3 layers.

### 1. Component Matrix

Single capability validation:

- capture
- encode
- decode
- transport sender
- transport receiver
- render

Required outputs:

- throughput
- latency `avg/p50/p95/p99/max`
- component-specific counters

### 2. Single-Process Composed Pipeline

Host orchestration validation:

- sender host
- receiver host
- in-memory signaling
- decode into frame sink
- probe stages populated

Required outcomes:

- remote frames arrive
- key probe stages are populated
- composed pipeline does not stall over a fixed duration

### 3. Benchmarks

Scenario-level result capture:

- quick
- steady
- stress

Required outputs:

- benchmark artifacts
- logs
- summary json/csv/markdown

---

## Integration Order

### Phase 1

- keep `DXGI + OpenH264 + WebRTC + D3D11` as baseline
- design and implement `mrd-transport-quic-quinn`
- add QUIC sender/receiver component-matrix coverage
- add QUIC single-process composed coverage

### Phase 2

- add `mrd-capture-wgc`
- compare `dxgi` vs `wgc` using the existing matrix

### Phase 3

- add `mrd-encode-nvenc`
- add `mrd-decode-nvdec`
- compare hardware vs software codec paths using the same matrix

### Phase 4

- add `quiche` as secondary QUIC candidate if `quinn` is already stable in the matrix

---

## Repository Implications

The repo should evolve with these concrete crate targets:

- `crates/mrd-transport-quic-quinn`
- `crates/mrd-capture-wgc`
- `crates/mrd-encode-nvenc`
- `crates/mrd-decode-nvdec`

Each new crate must:

- expose shared pipeline-facing interfaces
- emit observability-compatible metrics
- add component-matrix tests
- add composed-pipeline coverage where applicable

---

## Decision

The mainline strategy is:

- keep the current WebRTC/H264/DXGI/D3D11 path as the baseline
- integrate `quinn` as the first QUIC backend
- keep `quiche`, `s2n-quic`, WGC, and hardware codec paths as next-wave candidates
- require all future optimization work to pass through the matrix-based validation model

This keeps the architecture coherent while still allowing broad backend integration over time.
