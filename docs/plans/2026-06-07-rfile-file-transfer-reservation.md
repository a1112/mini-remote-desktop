# R-File File Transfer Reservation

Date: 2026-06-07

## Decision

MRD keeps the current service-owned local file copy/list/cancel path as the only
active file transfer implementation. R-File is reserved as an external provider
handoff target instead of duplicating its file-manager, remote-mount, and
transfer-history feature set inside MRD.

## Reservation Contract

- MRD provider id: `r-file`
- MRD capability id: `service.file_transfer.external_bridge`
- External app: `R-File`
- External bridge service: `rfile-bridge`
- Control endpoint hint: `http://127.0.0.1:18100`
- Data endpoint hint: `http://127.0.0.1:18080`
- Reserved external capabilities:
  - `rfile.bridge.session_v1`
  - `rfile.watch.http_v1`
  - `rfile.remote_mount.v1`
  - `rfile.transfer_history.v1`

## Runtime Behavior

`mrd-local` remains available and executable. `r-file` is visible in provider
discovery as `unimplemented`, and explicit `provider_hint = "r-file"` requests
are rejected before any copy starts. This keeps the product surface stable while
leaving a clean integration point for a future R-File handoff.

## Future Work

The next integration should make MRD discover a running `rfile-bridge`, exchange
session/device context, and hand off file actions to R-File through the reserved
provider contract. MRD should not add another remote mount engine unless R-File
cannot provide the required behavior.
