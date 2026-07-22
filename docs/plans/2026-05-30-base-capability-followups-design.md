# Base Capability Followups Design

## Goal

Close the capability and benchmark contract gaps found after the performance-test followup branch.

## Scope

- Keep this as a contract and orchestration cleanup.
- Do not introduce new capture, codec, render, or transport backends.
- Make existing FFmpeg, render pacing, capability, and benchmark metadata easier to compare and safer to consume.

## Design

1. Benchmark summary schema becomes the source-compatible contract for all fields currently emitted by the Tauri benchmark writer and the transport summary post-processor. The schema should include render pacing counters, swapchain pacing metadata, NVDEC capability metadata, derived render rates, and `run_status`.
2. FFmpeg CLI decode is classified as software decode in frontend comparison data. It is external tooling, but it is still CPU/software from the acceleration perspective and should not land in `unknown`.
3. Local transport test decoder selection should use the same preference as matrix testing for H.264-capable Windows paths: hardware decode first, FFmpeg H.264 fallback second, software fallback third, and `none` only when no decoder is available.
4. Capability snapshots should not mark planned-only native decode work as runnable support. Planned native macOS VideoToolbox decode remains visible, but as unimplemented until the service-owned path is actually wired.
5. Transport benchmark command execution should gain the same basic safety properties as component matrix runs: timeout support, captured stdout/stderr, and process cleanup on timeout.

## Testing

- Add frontend tests for FFmpeg decode classification and transport decoder selection.
- Add service capability tests for planned-only VideoToolbox decode status.
- Add benchmark PowerShell tests for schema coverage and transport timeout behavior.
- Run targeted frontend, PowerShell, and Rust tests before committing.
