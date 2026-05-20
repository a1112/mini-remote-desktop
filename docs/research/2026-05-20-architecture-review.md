# Architecture Review - Service Kernel Boundary

Date: 2026-05-20

## Scope

This review focuses on the Windows/service mainline after the service-kernel
contract work. It checks whether the workspace can be inspected on Windows,
where warnings concentrate, and which modules are likely to force large
architecture edits if left as-is.

## Check Baseline

Commands run during this review:

- `cargo check --workspace --message-format=json`
- `cargo check -p mrd-service -p mrd-ipc -p mrd-session -p mrd-application -p heartbeat-server --message-format=json`

Current result:

- Full workspace check exits 0 on Windows.
- Focus packages exit 0 with 0 warnings.
- Full workspace still has 33 warnings outside the service-kernel focus:
  `apps/Rdesk/src-tauri` 19, `apps/rdesk-legacy-harness` 11,
  `tests/capture-render-demo` 2, `tests/integration` 1.

## Main Blockers Removed

- `heartbeat-server` had duplicated heartbeat DTOs, invalid serde attributes on
  non-serialized config, async bind misuse in sync constructors, moved values,
  and a spawned receive buffer lifetime issue. The heartbeat protocol is now
  shared through `heartbeat-rs/src/protocol.rs`.
- `mrd-capture-macos` pulled macOS-only crates on Windows. `core-graphics` and
  `screencapturekit` now live under macOS target dependencies, and the crate is
  cfg-gated on non-macOS.
- Workspace interface drift was fixed for `CapturedFrame::d3d11_shared_bgra`
  and `FramePixelFormat::Nv12` in legacy/demo targets.

## Large Files And Risk

Largest service files:

- `apps/mrd-service/src/lan_discovery.rs`: about 9.1k lines.
- `apps/mrd-service/src/app_state.rs`: about 1.9k lines.
- `apps/mrd-service/src/media_adaptation.rs`: about 1.8k lines.
- `apps/mrd-service/src/ipc_server.rs`: about 1.1k lines.
- `apps/mrd-service/src/capabilities.rs`: about 1.0k lines.

Largest shared crates:

- `crates/mrd-decode-nvdec/src/lib.rs`: about 3.2k lines.
- `crates/mrd-encode-nvenc/src/lib.rs`: about 2.0k lines.
- `crates/mrd-render-opengl/src/lib.rs`: about 1.6k lines.
- `crates/mrd-ipc/src/lib.rs`: about 1.3k lines.
- `crates/mrd-decode/src/lib.rs`: about 1.2k lines.

The main architecture risk is not only file size; it is ownership ambiguity.
`lan_discovery.rs` owns discovery, peer registry, protocol envelope parsing,
media sender, media receiver, display-mode capability, test impairment, and
matrix support. Any behavioral change there can accidentally affect discovery,
media transport, profile negotiation, and test reporting at the same time.

`app_state.rs` owns multiple registries and runtime queues. It is currently the
right owner for service state, but the registries need smaller modules so
future session/media/identity changes do not require editing the whole state
object.

`ipc_server.rs` still mixes IPC connection loop, request dispatch, LAN control,
capability response building, identity, shell, telemetry and session bootstrap.
The next split should make it an accept loop plus a thin dispatcher.

## Boundary Recommendation

The service remains the kernel owner. Rdesk should only manage windows, native
surface lifecycle, local preferences and IPC calls.

Recommended service modules:

- `lan/protocol`: announcement DTOs, protocol version, media envelope constants.
- `lan/discovery`: UDP bind, announce, probe, peer TTL.
- `lan/peer_registry`: peer map, freshness, capability indexing.
- `lan/media_sender`: capture, encode, packetize, sender metrics.
- `lan/media_receiver`: reassemble, decode, render enqueue, receiver metrics.
- `lan/media_profile`: source/profile negotiation, codec and chroma defaults.
- `lan/display_control`: remote display mode list/set/restore.
- `runtime/session`: session registry and lifecycle snapshots.
- `runtime/media`: pipeline registry, render queues, stage metrics.
- `runtime/identity`: pairing and device identity snapshots.
- `runtime/audit`: append/query audit events.
- `runtime/shell_state`: UI/tray/autostart status.
- `handlers/*`: one handler group per capability, identity, control, telemetry,
  session and transport domain.

## Warning Policy

The focus packages are now quiet. Remaining warning cleanup should not be done
as broad mechanical churn during media work. Each future feature branch should
keep its touched package warning-neutral.

`mrd-ipc` now keeps legacy wire DTOs in a scoped `wire` module with a local
`missing_docs` allow. This preserves serde names and public re-exports while
making the remaining warning budget meaningful. New IPC DTOs should be placed
in domain modules with explicit docs instead of extending the legacy blob.

`mrd-service` has local `dead_code` allows only in staged kernel modules where
the APIs are contract scaffolding or platform-dependent runtime hooks. These
allows should be removed when the planned module split makes the code reachable
through focused tests.
