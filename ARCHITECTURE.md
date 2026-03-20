# Mini Remote Desktop - Current Architecture

## Overview

`mini-remote-desktop` is being rebuilt into a clean product-oriented workspace.

The active mainline is now organized around:

```text
mini-remote-desktop/
├── apps/
│   ├── Rdesk/             # Desktop client product
│   ├── Rdesk-Server/      # Backend/API product
│   └── realtime-server/   # Realtime sidecar service
├── crates/                # Shared Rust mainline crates
├── common-control-proto/  # Shared control protocol still in use
├── heartbeat-rs/          # Heartbeat/discovery service still retained in mainline
├── docs/                  # Design and rebuild documentation
├── tests/                 # Integration and regression coverage
├── tools/                 # External tools and local helper dependencies
├── labs/                  # Validation-only projects, to be restored selectively
└── junk/                  # Historical implementations, temp scripts, and reference material
```

Anything under `junk/` is reference-only. It is not architecture-defining.

## Mainline Components

### `apps/Rdesk`

Primary desktop client product, based on Tauri.

Responsibilities:

- session lifecycle and desktop UX
- runtime host integration
- realtime signaling/client orchestration
- render shell and local device management

Typical substructure:

```text
apps/Rdesk/
├── src/          # frontend app
└── src-tauri/    # Tauri host and Rust runtime
```

### `apps/Rdesk-Server`

Primary backend/API service, based on FastAPI.

Responsibilities:

- auth and user management
- device registration and binding
- session request management
- management-plane integration with realtime services

Typical substructure:

```text
apps/Rdesk-Server/
└── app/
    ├── api/
    ├── core/
    ├── db/
    ├── models/
    └── schemas/
```

### `apps/realtime-server`

Rust realtime sidecar service.

Responsibilities:

- realtime signaling/session routing
- websocket-facing realtime endpoints
- coordination with shared signaling/protocol crates

### `crates/*`

Shared Rust mainline crates that hold product-level reusable logic.

Current examples include:

- `mrd-proto`
- `mrd-session`
- `mrd-signal-proto`
- `mrd-signal-client`
- `mrd-signal-server`
- `mrd-pipeline-core`
- `mrd-render`
- `mrd-render-d3d11`
- `mrd-decode`

These crates are the preferred destination for reusable runtime logic during rebuild.

### `common-control-proto`

Legacy shared control protocol crate that is still retained in the active workspace.

Current role:

- shared control event framing
- controller/agent message compatibility

Long-term expectation:

- either remain as a dedicated shared crate
- or be absorbed into the rebuilt `crates/*` layout once responsibilities are fully migrated

### `heartbeat-rs`

Heartbeat and discovery service still retained in the active workspace.

Current role:

- UDP heartbeat/online presence
- lightweight discovery support

It remains in mainline because the workspace still references it directly.

## Data Flow

### Product-Oriented Flow

```text
Rdesk client
  -> Rdesk-Server for management/auth/device APIs
  -> realtime-server for realtime session coordination
  -> shared crates for protocol/session/render/decode building blocks
  -> heartbeat-rs for optional presence/discovery path
```

### Rebuild Rule

When adding or restoring behavior:

1. Put product entrypoints under `apps/`
2. Put reusable Rust logic under `crates/`
3. Keep validation-only work under `labs/`
4. Move old implementations and one-off material under `junk/`

## Historical Components

The following categories have been intentionally moved out of the mainline path into `junk/`:

- older controller and agent implementations
- older signaling/web entrypoints
- temporary benchmark scripts
- generated artifacts and captured outputs
- partially recovered reference trees

These historical trees may still contain useful implementation ideas, but they must not define current architecture.

## Current Runtime Baseline

At this stage of the rebuild, the practical baseline is:

1. `apps/Rdesk` as the client host
2. `apps/Rdesk-Server` as the backend/API
3. `apps/realtime-server` as the realtime sidecar
4. `crates/*` as shared runtime building blocks
5. `common-control-proto` and `heartbeat-rs` temporarily retained while migration continues

## Migration Target: mrd-service Architecture

The repository is migrating to a "thin shell + local service" architecture where `Rdesk` becomes a desktop UI shell and a new `mrd-service` handles all session orchestration.

### Target Layering

```text
┌─────────────────────────────────────────────────────────────────┐
│ Product Shell Layer                                             │
│ apps/Rdesk - UI, window management, local settings              │
│   ↓ Local IPC (Named Pipe / Unix Socket)                        │
├─────────────────────────────────────────────────────────────────┤
│ Local Service Layer                                             │
│ apps/mrd-service - Session orchestrator, IPC server             │
│   ↓                                                              │
├─────────────────────────────────────────────────────────────────┤
│ Application Layer                                               │
│ crates/mrd-application - Use cases (start/accept/sync session)  │
│   ↓                                                              │
├─────────────────────────────────────────────────────────────────┤
│ Session Domain Layer                                            │
│ crates/mrd-session - Session aggregate, role, state             │
│   ↓                                                              │
├─────────────────────────────────────────────────────────────────┤
│ Infrastructure Adapter Layer                                    │
│ mrd-transport-quic-quinn, mrd-transport-webrtc                  │
│ mrd-capture-dxgi, mrd-encode-nvenc, mrd-decode, mrd-render      │
│ mrd-signal-client, mrd-signal-server                             │
└─────────────────────────────────────────────────────────────────┘
```

### Key Changes

- `Rdesk` will **not** directly own QUIC/WebRTC hosts or session coordinators
- `mrd-service` becomes the **only** session orchestration entry point
- Media/transport crates remain as **infrastructure capabilities**
- UI crashes will not automatically terminate session orchestration

### Migration Path

See `docs/plans/2026-03-20-mrd-service-architecture-migration.md` for detailed design and `docs/plans/2026-03-20-mrd-service-architecture-migration-design.md` for implementation plan.

## Notes

- This file describes the current mainline structure, not the older mixed-layout repository.
- Historical path names such as `agent-rust/`, `controller-rust/`, `web/`, and `server/` are no longer part of the active root layout.
- For rebuild sequencing and decisions, see the documents under `docs/plans/`.
