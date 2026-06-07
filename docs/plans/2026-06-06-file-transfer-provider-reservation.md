# File Transfer Provider Reservation

## Context

MRD now has a service-owned local file transfer loop for directory listing, copy, task listing, and cancellation. `G:\Project\R-File` is a full file manager and remote mount system with overlapping responsibilities, so this branch keeps MRD's current path active and reserves integration seams instead of copying R-File modules into MRD.

## Decision

Use MRD's IPC contract as the reservation boundary:

- `FileTransferStartRequest.provider_hint` can name a future provider preference.
- `FileTransferTaskSnapshot.provider_kind` records the provider that actually handled the task.
- `FileTransferTaskSnapshot.provider_capabilities` records stable capability ids for UI and diagnostics.
- The current active provider is `mrd-local` with `service.file_transfer.local`.
- `service.file_transfer.external_bridge` is advertised as `unimplemented` and reserved for a later R-File bridge.
- Non-local `provider_hint` values must fail with `E_FILE_TRANSFER_PROVIDER_UNAVAILABLE` until a provider router is implemented, so reserved R-File/external bridge requests cannot silently run through the MRD-local copier.

## Non-Goals

- Do not import R-File crates or services into MRD in this branch.
- Do not route file transfer through R-File yet.
- Do not replace the current service-owned local copy/list/cancel loop.

## Follow-Up

When the overlap is ready to resolve, add a provider router behind `StartFileTransfer` that can dispatch to `mrd-local` or an external R-File bridge based on availability, peer scope, and `provider_hint`.
