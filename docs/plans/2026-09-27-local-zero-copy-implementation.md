# Local Zero-Copy Implementation Plan

> Execute in this session with independent module owners, regression tests and cross-review. User approved the layered design on 2026-09-27.

**Goal:** Make the local GPU/CPU media paths correct, avoid verified unnecessary copies, and report only observed capabilities and results.

**Architecture:** Keep the thin shell and service architecture. Changes belong in reusable crates and service entrypoints. Preserve the dirty workspace; do not commit unrelated work or remove necessary GPU lifetime copies.

**Tech Stack:** Rust, D3D11/DXGI/WinRT, NVENC/NVDEC, Quinn, PowerShell.

## 1. Capture and encode

Files: `crates/mrd-encode-nvenc/src/lib.rs`, `crates/mrd-encode-nvenc-av1/src/lib.rs`, `vendor/nvenc/src/safe/bitstream.rs`, `crates/mrd-capture-dxgi/src/lib.rs`, `crates/mrd-capture-winrt/src/windows_impl.rs`, focused tests.

1. Regress NVENC output-completion/input-unmap ordering and error cleanup.
2. Correct completion and slot recycling; preserve existing low-delay user changes.
3. Regress producer flush defaults; make shared updates submitted by default.
4. Reuse compatible WinRT staging textures and borrow CPU BGRA input instead of cloning.
5. Run the affected crate unit tests and hardware-dependent probes separately, reporting unavailability explicitly.

## 2. Decode and render

Files: `crates/mrd-decode-nvdec/src/lib.rs`, its probe tests, `crates/mrd-decode/src/lib.rs`, `crates/mrd-render-d3d11/src/lib.rs`.

1. Regress codec-specific diagnostics when runtime libraries are absent.
2. Initialize shared resources before first frame emission and on sequence changes; strict shared mode must not silently emit empty results on fallback.
3. Regress CPU NV12 render input handling and provide reusable GPU plane uploads with existing shaders; cover P010 where supported.
4. Assess shared frame ownership and asynchronous reuse. Implement a bounded correct resource ownership path where feasible; otherwise preserve explicit limitations and reject unsafe success claims.
5. Run unit and probe tests, then real shared-frame tests only if runtime is available.

## 3. QUIC and capability advertisement

Files: `crates/mrd-transport-quic-quinn/src/lib.rs`, `apps/mrd-service/src/lan_discovery/media_capabilities.rs`, focused tests.

1. Add wire-equivalence and buffer-sharing tests for fragmentation/owned decode/reassembly.
2. Write headers with borrowed chunks directly; retain incoming `Bytes` storage and fast-path a complete single fragment.
3. Add capability mapping tests for absent/partial NVIDIA support.
4. Advertise only runtime-supported Windows codec capabilities, with cached probing and truthful fallback claims.
5. Run transport tests and service capability tests.

## 4. Observability and validation tooling

Files: `apps/Rdesk/src-tauri/src/benchmark.rs`, `test_harness.rs`, associated telemetry/report files as necessary.

1. Regress skipped/empty runs claiming zero copy and software encoding incorrectly qualifying.
2. Distinguish requested path from observed path; unknown evidence remains unknown, CPU fallback cannot be true.
3. Ensure summary serialization and CSV/Markdown remain compatible and explain copy scope.
4. Run focused tests and document the precise evidence used.

## 5. Mainline service integration

Files: `apps/mrd-service/src/lan_discovery.rs`, `media_capture_config.rs`, `media_sender.rs`, `media_receiver_runtime.rs`, `media_envelope.rs` and focused tests.

1. Regress software encoder fallback with shared capture and rebuild/re-capture matching CPU memory when selected.
2. Converge mux/v3/v2 complete access units into the existing profile/order/decode pipeline without re-fragmenting in-process.
3. Avoid unconditional access-unit cloning when no agent route exists; preserve authorization and route revocation semantics.
4. Run scoped service tests and compile integration.

## 6. Local evidence and review

1. Record hardware/software inventory and pre-change baselines.
2. Keep GPU benchmarks serial and preserve running user services.
3. Review final diff independently and fix material findings.
4. Record actual test counts, skipped hardware paths, performance results and remaining limitations in a final local report. Do not infer success from conditional early returns.

## Progress

- [x] Repository and hardware inventory; source-path audits.
- [x] Design reviewed and approved by user.
- [x] Capture/encode fixes and regression tests (58 library tests; 7 strict actual GPU tests; first-frame mapping and completion lifetimes independently reviewed).
- [x] Decode/render fixes and regression tests (84 library tests; 6 decoder entry tests; cleanup quarantine independently reviewed).
- [x] QUIC/capability fixes and regression tests (91 transport tests; service capability and real BGRA sharing probes).
- [x] Observability fixes and regression tests (formal app and legacy tests, report compatibility, 3 real local pipeline runs with truthful CPU fallback evidence).
- [x] Mainline integration and regression tests (service: 628 passed, 5 ignored; session-agent runtime fallback: 10 passed).
- [x] Local validation, cross-review and report (final service, session-agent and app checks passed; hardware interop limitation and CPU fallback documented in `2026-09-27-local-zero-copy-results.md`).

The approved implementation and available local verification are complete. The strict NVDEC shared hardware probe still fails with CUDA error 101; this is an explicit unavailable capability, not a passing zero-copy path. Future driver/environment remediation and the untested combinations listed in the results report remain outside this completed implementation.
