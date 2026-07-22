# Mainline Refs And Roadmap

## Context

This plan compares the two May 2026 research reports and turns them into tracked development work. The combined direction is to keep `mini-remote-desktop` on a low-latency media engine mainline while borrowing proven product and control-plane patterns from established projects.

The merge target for the current media work is `main`. Third-party projects are pinned under `/refs/projects/*` as Git submodules. They are reference-only inputs, not architecture facts and not implementation sources.

## Combined Direction

The two reports agree on the same split:

- Media performance remains native and service-owned: capture, encode, QUIC datagram transport, decode, and native render.
- Rdesk stays a thin UI shell for windows, controls, native surface lifecycle, and state.
- Device identity, pairing, approval, and revocation need a first-class control/security design.
- Browser and relay compatibility should be explicit product modes with separate acceptance criteria.
- Benchmarks must report negotiated runtime profiles and stage metrics, not only pass/fail rows.

## Task Labels

Use these labels for issues, roadmap rows, and reference notes:

- `priority/P0`, `priority/P1`, `priority/P2`
- `area/media`, `area/control`, `area/security`, `area/refs`, `area/benchmark`, `area/product`
- `type/design`, `type/implementation`, `type/test`, `type/research`
- `ref/rustdesk`, `ref/obs-studio`, `ref/scrcpy`, `ref/syncthing`, `ref/guacamole-client`, `ref/guacamole-server`, `ref/xrdp`, `ref/srt`, `ref/pipewire`, `ref/kdeconnect-kde`

## Development Tasks

| Priority | Area | Type | Task | Reference Tags | Acceptance |
| --- | --- | --- | --- | --- | --- |
| P0 | media | implementation | Continue LAN QUIC media mainline: `DXGI shared -> NVENC H.264 -> QUIC DATAGRAM -> NVDEC -> D3D11 native`. | `ref/obs-studio`, `ref/scrcpy`, `ref/pipewire` | Matrix proves 1080p144/180/249 selected FPS is not clamped by refresh rate or fallback pacing. |
| P0 | benchmark | test | Solidify paired local/cross canary scripts and reports. | `ref/srt`, `ref/scrcpy` | Emits local, cross, and comparison JSON/MD with `threshold_miss`, `profile_downgraded`, `decode_error`, and `transport_loss` classes. |
| P0 | security | design | Design device ID, pairing approval, certificate fingerprint, revoke, and reconnect state machine. | `ref/syncthing`, `ref/kdeconnect-kde` | Device trust changes are explicit, auditable, and survive service restart. |
| P1 | control | design | Split control plane and media plane into reliable control channel and lossy media channel. | `ref/kdeconnect-kde`, `ref/guacamole-client` | Keyboard, mouse, clipboard, authorization, and audit use reliable ordered semantics independent of media loss. |
| P1 | product | research | Define self-hosted, relay, browser gateway, RDP compatibility, and degraded-network product boundaries. | `ref/rustdesk`, `ref/guacamole-server`, `ref/xrdp`, `ref/srt` | Each mode has clear latency, security, and feature acceptance criteria. |
| P1 | refs | research | Maintain one reference note per submodule with usage, license risk, and task tags. | `ref/rustdesk`, `ref/obs-studio`, `ref/scrcpy`, `ref/syncthing`, `ref/guacamole-client`, `ref/guacamole-server`, `ref/xrdp`, `ref/srt`, `ref/pipewire`, `ref/kdeconnect-kde` | Every submodule has a note and appears in `refs/reference-tags.lock.json`. |
| P2 | product | research | Write browser gateway compatibility experiments after the native LAN path is stable. | `ref/guacamole-client`, `ref/guacamole-server`, `ref/xrdp` | Browser mode is reported separately and never blocks LAN native acceptance. |

## Near-Term Execution

1. Land the LAN media matrix parity branch on `main`.
2. Pin reference projects under `/refs/projects/*`.
3. Keep `/refs/reference-tags.lock.json` in sync with actual submodule HEADs.
4. Add paired canary scripts as the next P0 benchmark task.
5. Start the P0 security design before expanding relay or browser gateway scope.

## Guardrails

GPL and AGPL projects may be used for architecture, interface, test, and operational comparisons. Do not copy GPL or AGPL implementation code into `apps/`, `crates/`, or `tests/` unless the repository intentionally adopts the license obligations.

The `/refs` tree is excluded from architecture authority. If a reference changes a design decision, the decision must be written back into `docs/plans/` before code follows it.
