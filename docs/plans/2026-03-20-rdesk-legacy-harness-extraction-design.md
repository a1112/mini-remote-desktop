# Rdesk Legacy Harness Extraction Design

**Date:** 2026-03-20

## Goal

Close `P2-1` by removing the old direct-control runtime from the `app` binary while preserving the remaining validation and benchmark assets in a separate legacy harness package.

After this change:

- `apps/Rdesk/src-tauri` remains a shell-only desktop app.
- legacy modules are no longer compiled into the `app` non-test build.
- `#![allow(dead_code)]` is removed from `apps/Rdesk/src-tauri/src/main.rs`.
- the old validation/benchmark code continues to exist, but outside the shell package.

## Problem

The hard-cut migration has already reduced the active Tauri command surface to service/IPC/render commands, but the shell binary still compiles the old mainline modules:

- `quic_host`
- `quic_session`
- `realtime_client`
- `realtime_runtime`
- `session_lifecycle`
- `session_runtime`
- `webrtc_host`
- `webrtc_media`
- `webrtc_session`
- `benchmark`

That means the cutover is only complete at the command-entry layer. The binary and code ownership are still mixed.

## Decision

Use a dedicated legacy harness package rather than keeping the old code under `#[cfg(test)]` inside `apps/Rdesk/src-tauri`.

### Why this approach

This gives the cleanest architecture boundary:

- `app` becomes truly shell-only.
- legacy runtime code remains available for migration validation.
- tests and benchmark helpers stop polluting the shell crate.
- future deletion becomes a package-level removal instead of an invasive file-by-file cleanup inside the production shell.

## Non-Goals

This design does not:

- redesign the legacy runtime itself
- rewrite legacy tests into the new IPC architecture
- merge the harness into `mrd-service`
- remove existing render-shell behavior from `Rdesk`

The purpose is extraction, not modernization.

## Target Layout

```text
apps/
├── Rdesk/
│   └── src-tauri/
│       ├── src/
│       │   ├── main.rs
│       │   ├── app_settings.rs
│       │   ├── ipc_client.rs
│       │   ├── service_manager.rs
│       │   ├── device_info.rs
│       │   ├── frame_sink.rs
│       │   ├── render_host.rs
│       │   ├── render_surface_catalog.rs
│       │   └── render_window_registry.rs
│       └── tests/
│           └── ipc_shell_smoke.rs
│
└── rdesk-legacy-harness/
    ├── Cargo.toml
    ├── src/
    │   ├── lib.rs
    │   ├── benchmark.rs
    │   ├── quic_host.rs
    │   ├── quic_session.rs
    │   ├── realtime_client.rs
    │   ├── realtime_runtime.rs
    │   ├── session_lifecycle.rs
    │   ├── session_runtime.rs
    │   ├── webrtc_host.rs
    │   ├── webrtc_media.rs
    │   ├── webrtc_session.rs
    │   └── quic_transport_harness.rs
    └── tests/
        └── legacy_runtime.rs
```

## Ownership Rules

### `app` may keep

- shell boot and Tauri command registration
- service lifecycle commands
- IPC-backed session control commands
- render-shell state
- local settings
- local hardware inspection

### `app` may not keep

- old session coordinators
- old realtime runtime
- old QUIC/WebRTC host orchestration
- benchmark helper runtime
- shell-local session lifecycle state

### `rdesk-legacy-harness` owns

- old direct runtime code kept only for validation/reference
- legacy runtime tests
- benchmark helpers still tied to the old direct mainline

## File Mapping

### Remove from `app` non-test build

From `apps/Rdesk/src-tauri/src/main.rs`:

- `mod quic_host;`
- `mod quic_session;`
- `mod realtime_client;`
- `mod realtime_runtime;`
- `mod session_lifecycle;`
- `mod session_runtime;`
- `mod webrtc_host;`
- `mod webrtc_media;`
- `mod webrtc_session;`
- `mod benchmark;`
- `mod quic_transport_harness;`

Also remove the corresponding `use` items and any helper functions that only exist for the old runtime.

### Keep in `app`

- `app_settings`
- `ipc_client`
- `service_manager`
- `device_info`
- `frame_sink`
- render-related shell modules

## Test Strategy

### `app`

`app` should retain only shell-level tests such as:

- IPC shell smoke tests
- service lifecycle tests if any remain package-local
- render-shell tests that do not need the legacy direct runtime

### `rdesk-legacy-harness`

The following should move out of `main.rs` test blocks:

- legacy realtime/session flow tests
- QUIC/WebRTC direct runtime integration tests
- legacy benchmark-oriented tests
- transport harness helpers

## Migration Constraints

To avoid another partial cleanup, the extraction must be validated by explicit closure conditions.

`P2-1` is closed only when all are true:

1. `apps/Rdesk/src-tauri/src/main.rs` no longer has `#![allow(dead_code)]`.
2. `cargo check -p app` succeeds without compiling legacy runtime ownership into the shell.
3. `app` no longer declares the legacy runtime modules listed above.
4. legacy validation assets still run from `rdesk-legacy-harness`.

## Risks

### Risk 1: hidden shell dependencies

Some render-shell tests or helpers may still indirectly depend on legacy modules.

**Mitigation:** move tests together with the modules they exercise instead of trying to keep mixed dependencies inside `app`.

### Risk 2: over-cleanup

Deleting legacy runtime code too early would destroy useful regression assets.

**Mitigation:** extract first, delete later.

### Risk 3: package split churn

Moving modules may require path/import cleanup and shared helper exposure.

**Mitigation:** keep the extraction mechanical; do not refactor internals during the move.

## Success Criteria

The extraction is successful when:

- `app` compiles as a shell-only crate
- `main.rs` no longer suppresses dead-code cleanup globally
- the legacy runtime no longer contributes to the `app` binary
- the old validation/benchmark assets still exist in a separate package
- the reviewer finding about “旧主线代码仍然整块留在 Rdesk 二进制里” is no longer true
