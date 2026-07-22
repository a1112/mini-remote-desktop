# Architecture Warning Cleanup Plan

Date: 2026-05-20

## Goal

Keep the project easy to evolve by enforcing service-owned runtime boundaries
and preventing warning drift in the Windows/service mainline.

## Current Gate

Required for service-kernel work:

```powershell
cargo check --workspace --message-format=json
cargo check -p mrd-service -p mrd-ipc -p mrd-session -p mrd-application -p heartbeat-server --message-format=json
cargo fmt --all -- --check
cargo build -p mrd-service
```

Current baseline:

- Full workspace check: passes.
- Focus packages: 0 warnings.
- Full workspace residual warnings: 33, all outside the focus packages.

## Non-Negotiable Boundaries

- `mrd-service` owns session runtime, media runtime, capability/profile
  evaluation, transport policy, identity/trust, audit and telemetry.
- Rdesk owns UI shell, native surface lifecycle and user interaction. It must
  not carry high-bandwidth frame data or become the source of truth for session
  state.
- IPC wire shape is stable unless a migration is explicitly planned. Refactors
  may move modules and re-export types, but must not rename serde variants or
  fields.
- Platform-only dependencies must be target-gated in `Cargo.toml`, not hidden
  behind runtime checks.

## Module Split Order

1. `lan/protocol`

   Move constants, announcement DTOs, media protocol version, codec ids, and
   envelope helpers out of `lan_discovery.rs`. Tests should prove old JSON and
   QUIC envelope wire shapes still round-trip.

2. `lan/peer_registry`

   Move peer freshness, TTL pruning, capability indexing and probe endpoint
   selection. This allows discovery tests to run without media sender setup.

3. `lan/media_profile`

   Move source/profile negotiation, codec/chroma defaults, aspect-ratio
   downgrade classification and canary profile generation. This is the highest
   leverage split for future adaptive profile work.

4. `lan/media_sender` and `lan/media_receiver`

   Move capture/encode/packetize and reassemble/decode/render paths only after
   protocol and profile code are isolated. Run `lan_discovery` tests after each
   move.

5. `runtime/*`

   Split `app_state.rs` registries into session, media, identity, audit and
   shell state modules. Keep `AppState` as the composition root.

6. `handlers/*`

   Move `ipc_server.rs` request handlers into focused modules. `ipc_server`
   should retain endpoint bind, accept loop and dispatch only.

## Warning Rules

- New public DTOs and trait methods need docs when they are in a warning-enabled
  crate.
- Prefer deleting unused fields/methods. Use local `#[allow(dead_code)]` only
  for platform hooks, staged contracts, or diagnostics code that has documented
  ownership.
- Do not add crate-wide warning allows to new crates.
- If a branch touches `mrd-ipc`, the post-change `mrd-ipc` warning count must
  not increase.
- If a branch touches `mrd-service`, the focus package check must remain at 0
  warnings unless the branch documents a temporary exception.

## Residual Cleanup Backlog

- `apps/Rdesk/src-tauri`: remove or localize 19 warnings after confirming which
  legacy harness paths still feed current test workbench flows.
- `apps/rdesk-legacy-harness`: either migrate remaining users to
  `mrd-service` tests or mark the harness as reference-only.
- `tests/capture-render-demo`: narrow warning scope after deciding whether the
  demo remains macOS-only or becomes a cross-platform visual smoke test.
- `tests/integration`: clean the last integration warning when the corresponding
  legacy pipeline case is updated.
