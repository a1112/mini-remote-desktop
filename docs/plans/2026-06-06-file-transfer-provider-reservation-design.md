# File Transfer Provider Reservation Design

Date: 2026-06-06
Branch: `codex/file-transfer-provider-reservation`

## Goal

Reserve a service-owned file transfer provider boundary in MRD without merging
R-File's overlapping file manager, bridge, watch, or mount implementation.

## Context

`G:\Project\R-File` already owns a richer file product surface: watch service,
bridge service, remote client DTOs, QUIC copy DTOs, transfer jobs, mount, and UI
panes. MRD currently has a transfer modal with static demo data and no stable
local IPC contract for file transfer status or provider ownership.

## Architecture

MRD will expose a narrow IPC snapshot for file transfer capability:

- provider id and display name
- provider status such as `reserved`, `available`, or `unavailable`
- supported capabilities and action names
- task snapshots using MRD snake_case wire fields
- an optional provider detail string for future R-File bridge binding

The first implementation returns a reserved provider from `mrd-service`. It
does not start copies, mount paths, enumerate files, or import R-File crates.
The Rdesk UI reads this snapshot and shows an empty or reserved state instead
of hard-coded transfer examples.

## R-File Capability Mapping

Local review of `G:\Project\R-File` shows that R-File already has several
separable file capabilities that can be bound behind the MRD provider boundary:

| R-File capability | Source location | MRD provider capability |
| --- | --- | --- |
| QUIC upload/download/remote-copy over bidirectional streams | `services/rfile-watch/src/quic_transfer.rs` | `file.transfer.rfile.quic_stream` |
| HTTP remote client with retry policy and aggregate timing/byte counters | `crates/rfile-remote-client/src/lib.rs` | `file.transfer.rfile.http_client_stats` |
| Remote mount surface and cross-platform smoke/perf scripts | `services/rfile-mount`, `services/rfile-fuse`, `docs/remote-mount-three-platform-capability-matrix.md` | `file.transfer.rfile.remote_mount` |
| Direct-vs-mounted and optional SMB/rclone comparison scripts | `bin/Windows/test/local-mount-perf.ps1`, `bin/macOS/test/local-mount-perf.sh`, `bin/Linux/test/local-mount-perf.sh` | `file.transfer.perf_baseline` |

The current MRD snapshot exposes these as capability strings plus the actions
`compare_provider` and `bind_external_provider`. Those actions are intentionally
declarative in this slice: they document the available provider work without
opening filesystem mutation commands before an authorization and lifecycle model
exists in MRD.

## Performance Comparison Plan

Use R-File as the first baseline provider rather than copying its internals into
MRD:

1. Run R-File's direct/mounted scripts with the same payload sizes and target
   disk/network path that MRD will use.
2. Add an MRD provider benchmark command that records throughput, elapsed time,
   total bytes, retry count, failure count, and active task count using the same
   units as `RemoteClientStats`.
3. Compare MRD-native, R-File QUIC stream, R-File HTTP, and optional SMB/rclone
   paths in one report before enabling user-visible transfer actions.

This keeps R-File responsible for its richer file-manager and mount behavior,
while MRD owns session context, provider selection, status reporting, and UI
integration.

## Non-Goals

- Do not copy R-File source into MRD.
- Do not implement real upload/download/cancel/retry behavior in this slice.
- Do not change LAN media transport behavior.
- Do not introduce new filesystem mutation surfaces.

## Testing

Use IPC contract tests for serialization, service tests for the default
reserved snapshot, and frontend tests for adapter/UI behavior.
