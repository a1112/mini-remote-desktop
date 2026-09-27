# Local 2560×1440 / 144 Hz low-latency optimization

User request: continue the approved local optimization work at 2K144. User selected low-latency remote control and smooth interaction. This is an iteration of the approved layered design, executed in the current dirty workspace without reverting unrelated work or changing system display drivers.

## Success criteria

- Measure actual 2560×1440 input/output, target 144 FPS (6.944 ms frame interval), actual submissions/presents, stage p50/p95 and queue/drop/fallback evidence.
- Separate requested FPS, processing throughput, accepted presents and unique desktop updates. Repeated cached captures cannot prove new 144 FPS content or display scan-out.
- Apply and compare optimizations on the same machine/build/configuration; preserve a pre-change baseline. State debug/release and loopback/real transport explicitly.
- Keep failed CUDA/D3D11 interop explicit. CPU fallback remains valid and must remain distinguishable from shared decode.

## Approach

First measure the existing path. Prefer removing demonstrated CPU conversion/copy or repeated failed setup before changing scheduling. The current D3D11 renderer accepts planar NV12/P010, while some Rdesk adapters still expand CPU decoder output to BGRA; validate this boundary before choosing the patch. Keep preview conversion and unsupported renderer compatibility correct.

Evaluate capture/encode and service pacing in parallel through source review. Change defaults only with measured evidence; preserve explicit overrides and bounded frame ownership. A lightweight component probe can isolate GPU capability, but cannot stand in for a complete service or cross-machine session.

## Implementation and verification tasks

1. **Baseline:** rebuild the current app test executable (including the previous first-map fix); run serial 2K144 H.264/NVDEC cases and save source/executable hashes, dimensions and raw summaries under `artifacts/local-zero-copy/2k144/`.
2. **Identify bottleneck:** trace measured capture/encode/decode/render/queue costs to the actual implementation. Inspect service/Agent paths separately from harness compatibility paths. Record the root cause before edits.
3. **Targeted implementation:** write a reproducing regression for each behavior change, then implement the smallest change. Primary candidates are renderer capability-aware planar frame forwarding and avoiding repeated unsupported shared setup. Do not change unrelated formats, network authorization or driver configuration.
4. **Capture evidence:** establish whether a capture is a new desktop update or a cached frame using actual duplication metadata, where available. Keep this separate from processing FPS; add targeted verification if telemetry changes.
5. **Comparison:** repeat equivalent local runs after the patch; inspect real frames and truthful zero-copy evidence, not just exit status. Use optimized component measurements as needed and state their scope.
6. **Integration:** focused Rust/application and report tests, final compilation of affected app/service/Agent entrypoints, independent review, and results with the actual limiting stage and unresolved hardware constraints.

## Progress

- [x] User preference and native 2560×1440@144 display verified.
- [x] Current-source diagnostic baseline and bottleneck evidence (non-quiescent debug harness; see results).
- [ ] Targeted code changes and regression tests.
- [ ] Comparable post-change runs and capability results.
- [ ] Final integration, review and report.

Interrupted by repeated OS restarts and active SATA I/O errors. Results and remaining work are recorded in `2026-09-27-local-2k144-results.md`; do not treat this progress checkpoint as a completed 144 FPS acceptance.
