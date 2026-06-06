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

## Non-Goals

- Do not copy R-File source into MRD.
- Do not implement real upload/download/cancel/retry behavior in this slice.
- Do not change LAN media transport behavior.
- Do not introduce new filesystem mutation surfaces.

## Testing

Use IPC contract tests for serialization, service tests for the default
reserved snapshot, and frontend tests for adapter/UI behavior.
