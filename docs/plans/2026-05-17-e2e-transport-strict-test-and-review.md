# E2E Transport Strict Test And Review

Source archive: `docs/research/2026-05-17-end-to-end-remote-desktop-transport-review.md`

## Review Position

The archived review is intentionally strict: the current project should not claim production-grade WAN E2E remote desktop until the test evidence covers NAT, relay fallback, device trust, reliable control semantics, and long-running failure recovery. The current strongest implementation remains the native LAN media path:

`DXGI shared -> NVENC H.264 -> QUIC datagram media v3 -> NVDEC D3D11 shared -> D3D11 native render`

The local dual-process LAN test bed added on this branch is the right next evidence layer because it runs two separate `mrd-service` processes, distinct IPC endpoints, real LAN discovery, real QUIC media transport, native render attach, and optional media datagram impairment. It does not prove WAN product readiness by itself.

## Current Branch Review

| Area | Finding | Severity | Required Gate |
| --- | --- | --- | --- |
| LAN media path | Receiver now defaults to `nvdec_d3d11_shared` and reports `receiver.format.d3d11_shared_nv12`; CPU preview is no longer required for pass. | P0 | No accepted LAN row may use CPU/PNG preview as the primary frame path. |
| Display refresh | 180/249 FPS rows can be selected in the runtime request but are environment-limited if the active display mode is only 144 Hz. | P0 | Report `display_refresh_limited` and mark the row non-comparable instead of passing or failing performance. |
| Test impairment | Delay/jitter must not sleep the capture/encode loop; impairment must act like network delivery delay. | P0 | `sender.send_datagram` p95 must stay sub-millisecond when only synthetic delay/jitter is enabled. |
| QUIC media envelope | H.264 access units must route by v3 media envelope and never fall back to legacy probe parsing. | P0 | No report may contain `invalid magic` or `legacy probe fallback` for accepted media rows. |
| Cross-device parity | Cross-device Windows peer must be compared only when local and peer selected profiles match. | P0 | Paired comparison uses selected profile equality and `>=80%` of local baseline FPS. |
| WAN readiness | TURN/relay, NAT traversal, and identity pinning are not proven by this branch. | P0 product gap | WAN rows remain out of acceptance until a separate relay/WebRTC/TURN plan lands. |

## Strict Acceptance Matrix

### Local Single Process Baseline

Purpose: establish the upper bound of the local media components without service/process/network overhead.

Required rows:

| Profile | Chain | Minimum Result |
| --- | --- | --- |
| 1080p60 | `dxgi/nvenc_h264/quic/nvdec/d3d11_shared` | completed, decoded FPS >= 55 |
| 2k60 | same | completed, decoded FPS >= 45 |
| 1080p144 | same | completed, decoded FPS >= 115 |
| 1080p180 | same | completed only when active display mode >= 180 Hz; otherwise `display_refresh_limited` |
| 1080p249 | same | completed only when active display mode >= 249 Hz; otherwise `display_refresh_limited` |

### Local Dual-Process LAN E2E

Purpose: reproduce the real service boundary and LAN QUIC media path on one machine before cross-device testing.

Command:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 `
  -ProfileId 1080p60,2k60,1080p144,1080p180,1080p249 `
  -DurationSecs 30 `
  -NoBuild
```

Required evidence:

| Metric | Gate |
| --- | --- |
| `active_decoder` | `nvdec_d3d11_shared` |
| `active_renderer` | `d3d11` |
| `receiver.format.d3d11_shared_nv12` | present |
| `receiver.record` p95 | <= 2 ms for shared frames |
| `render_present` p95 | <= 3 ms for shared frames |
| `sender.send_datagram` p95 | <= 2 ms without impairment |
| `queue_depth` | <= 1 steady state |
| `invalid magic` / legacy probe fallback | absent |
| display-limited high FPS rows | `skipped/display_refresh_limited`, not completed |

### Local Dual-Process With Impairment

Purpose: validate that synthetic loss/delay/MTU affects media delivery without blocking producer scheduling.

Command:

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 `
  -ProfileId 1080p144 `
  -DurationSecs 30 `
  -NoBuild `
  -LossPct 1 `
  -BaseDelayMs 2 `
  -JitterMs 3 `
  -MtuBytes 1200
```

Required evidence:

| Metric | Gate |
| --- | --- |
| `test_impairment.datagrams_dropped` | greater than 0 when loss is enabled |
| `test_impairment.datagrams_delayed` | greater than 0 when delay/jitter is enabled |
| `sender.send_datagram` p95 | remains <= 2 ms |
| `receiver.reassemble` p95 | <= 1 ms |
| `receiver.decode` p95 | <= 3 ms |
| classification | `completed` or clear `transport_loss`; never `service_crash` |

### Cross-Device Windows LAN

Purpose: prove LAN Windows peer parity against local baseline.

Required peer capabilities:

- same branch and service build id
- media protocol version `>= 3`
- `dxgi_capture`
- `nvenc_h264`
- `quic_datagram_media_v3`
- `nvdec`
- `d3d11_native_render`
- `display_mode_control_v1`

Required rows:

| Profile | Gate |
| --- | --- |
| 1080p60 | cross FPS >= local selected-profile baseline * 0.8 |
| 2k60 | cross FPS >= local selected-profile baseline * 0.8 |
| 1080p144 | cross FPS >= local selected-profile baseline * 0.8 |
| 1080p180 | compare only if both active display modes are >= 180 Hz |
| 1080p249 | compare only if both active display modes are >= 249 Hz |

Failure classes must be one of:

- `unsupported`
- `peer_version_mismatch`
- `display_refresh_limited`
- `profile_downgraded`
- `capture_error`
- `encode_error`
- `transport_loss`
- `decode_error`
- `render_error`
- `threshold_miss`
- `service_crash`

## WAN And Product Readiness Gates

The archived review is clear that LAN success does not prove WAN remote desktop readiness. WAN/product acceptance requires a separate branch with these gates:

| Area | Gate |
| --- | --- |
| WebRTC/TURN | configurable STUN+TURN, relay-only diagnostic mode, and TURN auth in reports |
| Relay fallback | explicit direct/relay selected route and fallback reason |
| Control plane | reliable ordered control channel for keyboard, mouse, clipboard, auth, and audit |
| Device identity | persistent device ID, pairing approval, certificate fingerprint, revoke, reconnect |
| NAT matrix | same LAN, home NAT, enterprise NAT, CGNAT, UDP-blocked, relay-only |
| Long run | 30 min, 2 h, and 8 h sessions with stall, reconnect, and memory metrics |
| Security review | no unauthenticated remote control, no silent trust-on-first-use without UI state |

## Required Verification Before Merge

Run these before this branch is merged or used as a cross-device baseline:

```powershell
cargo fmt --all -- --check
cargo test -p mrd-ipc
cargo test -p mrd-service lan_discovery
cargo test -p mrd-decode-nvdec
cargo test -p mrd-render-d3d11
cargo build -p app -p mrd-service
```

```powershell
cd apps/Rdesk
pnpm test -- --run src/app/services/lanE2eAutomationService.test.ts src/app/components/TestWorkbench/E2ETestPage.test.tsx src/app/components/TestWorkbench/MatrixTestPage.test.tsx
pnpm type-check
```

```powershell
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/test_paired_lan_canary_common.ps1
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p144 -DurationSecs 10 -NoBuild
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_local_dual_process_lan_canary.ps1 -ProfileId 1080p144 -DurationSecs 10 -NoBuild -LossPct 1 -BaseDelayMs 2 -JitterMs 3 -MtuBytes 1200
```

## Review Checklist

- Reports include selected profile, active display mode, FPS, stage p95, queue depth, dropped frames, and impairment counters.
- High-FPS rows are not accepted unless the active display mode can actually produce the requested refresh.
- Receiver path remains native shared texture; WebView/PNG preview remains diagnostic only.
- Synthetic network impairment does not block the sender loop.
- Legacy peers fail fast or skip with `peer_version_mismatch`/`unsupported`.
- Cross-device comparison never compares mismatched selected profiles.
- WAN readiness is not claimed from LAN-only evidence.
