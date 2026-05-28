# FFmpeg Optional Tooling Integration Design

**Date:** 2026-05-28

## Goal

Add FFmpeg as an optional local tool dependency that can be detected, downloaded, configured with golden defaults, and surfaced through the service capability snapshot.

This phase does not make FFmpeg the primary media decoder. It prepares a reliable tool-management path first so future decoder fallback work can depend on a known FFmpeg installation contract.

## Context

The active mainline uses the thin shell plus local service architecture:

- `apps/Rdesk` is the Tauri shell and owns local UI settings.
- `apps/mrd-service` owns service-side capability snapshots.
- `crates/mrd-decode` owns decoder descriptors and factories.
- Windows high-performance decode currently goes through direct NVDEC probing and implementation.
- Software H.264 decode currently goes through OpenH264.

FFmpeg is only referenced in historical Python transport diagnostics and design notes. There is no active FFmpeg dependency in the Rust mainline.

The official FFmpeg download page states that FFmpeg provides source code and links to third-party Windows executable builds, including gyan.dev. The gyan.dev builds page provides Windows binaries containing `ffmpeg`, `ffprobe`, and `ffplay`, including `ffmpeg-release-essentials.zip` and adjacent `.sha256` metadata. Those links are a practical default for a Windows optional download path.

## Chosen Approach

Create a new `mrd-ffmpeg` crate for optional external tool management.

Responsibilities:

- store FFmpeg settings with safe golden defaults;
- resolve configured, bundled, and PATH-based tool locations;
- probe `ffmpeg` and `ffprobe` by running their version commands;
- download the configured Windows archive;
- verify SHA256 before extraction;
- extract into a managed application tools directory;
- report a structured probe result that can be mapped into service capabilities and Tauri responses.

The crate owns only external tool lifecycle. `mrd-decode` should not depend on it in this phase.

## Alternatives Considered

### Direct FFmpeg Decoder Backend

This would spawn FFmpeg from `mrd-decode` and pipe encoded access units to raw video output.

Pros:

- immediately provides a runtime decode fallback;
- aligns with older design notes that mentioned FFmpeg-backed software decode.

Cons:

- broadens scope into process lifetime, pipe framing, backpressure, pixel format conversion, and latency behavior;
- risks changing decode behavior before the tool installation path is deterministic;
- needs a separate performance and recovery design.

### Package FFmpeg at Build Time

This would vendor FFmpeg during packaging and avoid runtime downloads.

Pros:

- deterministic releases;
- no user-facing download flow.

Cons:

- larger installer;
- less optional;
- makes license and distribution choices part of every build.

### Optional Tooling First

This is the chosen path.

Pros:

- satisfies current detection, download, golden settings, and capability requirements;
- keeps existing decode behavior unchanged;
- gives later FFmpeg decoder work a stable installation contract.

Cons:

- does not yet make FFmpeg an actual decode fallback.

## Components

### `crates/mrd-ffmpeg`

Public model:

- `FfmpegSettings`
- `FfmpegDownloadSource`
- `FfmpegGoldenSettings`
- `FfmpegProbeResult`
- `FfmpegToolPath`
- `FfmpegInstallResult`

Public functions:

- `golden_settings() -> FfmpegSettings`
- `default_managed_install_dir() -> PathBuf`
- `probe_ffmpeg(settings: &FfmpegSettings) -> FfmpegProbeResult`
- `download_ffmpeg(settings: &FfmpegSettings) -> Result<FfmpegInstallResult, FfmpegError>`

Probe order:

1. explicit `settings.ffmpeg_path` and `settings.ffprobe_path`;
2. managed install directory from settings;
3. `PATH`.

The probe is successful only when both `ffmpeg` and `ffprobe` run and report version output.

Download behavior:

1. download archive to a temporary file;
2. download or use the configured SHA256;
3. verify archive hash;
4. extract to a temporary directory;
5. find `bin/ffmpeg.exe` and `bin/ffprobe.exe`;
6. atomically replace or create the managed install directory;
7. probe the extracted tools before reporting success.

### `apps/Rdesk/src-tauri`

Extend `AppSettings` with `ffmpeg: FfmpegSettings`.

Add Tauri commands:

- `ffmpeg_probe`
- `ffmpeg_download`
- `ffmpeg_reset_golden_settings`

The commands operate through local settings and return structured JSON serializable responses. Download errors must be actionable and must distinguish network, checksum, archive, extraction, and probe failures.

### `apps/mrd-service`

Add `mrd-ffmpeg` as a dependency and include FFmpeg in capability snapshots.

Capability IDs:

- `service.ffmpeg`
- `decode.ffmpeg_h264`
- `decode.ffmpeg_hevc`

The `service.ffmpeg` item represents tool availability. Decode items are marked available only when the tool probe succeeds, but they should remain optional and must not become required by existing profiles in this phase.

Static capability snapshots should report FFmpeg as supported/degraded rather than requiring a runtime probe.

### Frontend Service Mapping

Add the new IDs to `apps/Rdesk/src/app/services/capabilityMatrix.ts` so the UI classifies FFmpeg consistently.

No large UI screen is required in this phase. Existing settings/test workbench surfaces can consume the new Tauri commands later.

## Settings

Golden defaults for Windows:

- enabled: true;
- channel: `release-essentials`;
- archive URL: `https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip`;
- SHA256 URL: `https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256`;
- install directory: `%APPDATA%/mini-remote-desktop/tools/ffmpeg/release-essentials`;
- require checksum: true.

Non-Windows defaults:

- enabled: true;
- no managed download source in this phase;
- probe PATH and configured paths only.

## Error Handling

Probe errors should identify which tool failed and where it was searched.

Download errors should be specific:

- unsupported platform;
- request failed;
- checksum missing;
- checksum mismatch;
- archive extraction failed;
- expected executable missing;
- post-install probe failed.

Checksum mismatch must stop installation and leave the previous managed install untouched.

## Testing Strategy

Follow test-driven implementation.

`mrd-ffmpeg` unit tests:

- golden Windows settings include the managed download URL, SHA256 URL, and managed install directory;
- probe succeeds against fake `ffmpeg` and `ffprobe` executables in a temporary directory;
- probe fails when either tool is missing;
- SHA256 parsing accepts common `<hash>  <filename>` and plain hash formats;
- checksum mismatch prevents installation.

`apps/Rdesk/src-tauri` tests:

- app settings round-trip persists FFmpeg settings;
- reset golden settings returns deterministic defaults.

`apps/mrd-service` tests:

- static snapshot includes `service.ffmpeg`;
- runtime snapshot maps missing FFmpeg to a non-running status instead of crashing;
- existing profiles do not require FFmpeg in this phase.

Frontend tests:

- new FFmpeg capability IDs normalize to expected domains and statuses.

## Success Criteria

This phase is complete when:

- the workspace has a reusable `mrd-ffmpeg` crate;
- FFmpeg availability can be probed locally;
- FFmpeg can be downloaded and installed into a managed directory on Windows with SHA256 verification;
- Rdesk settings include FFmpeg golden defaults and persist user overrides;
- Tauri exposes probe, download, and reset commands;
- mrd-service capability snapshots include FFmpeg availability;
- existing decode paths and profiles continue to work without requiring FFmpeg;
- targeted Rust and frontend tests pass.
