# Mini Remote Desktop Rebuild Design

**Date:** 2026-03-07

**Goal:** Rebuild `mini-remote-desktop` into a clean product-oriented workspace centered on `Rdesk` and `Rdesk-Server`, using recovered files only as reference material and not as a direct source of truth.

## Context

The current repository contains multiple generations of exploration:

- product-facing code: `Rdesk`, `Rdesk-Server`
- historical Rust binaries: `agent-rust`, `controller-rust`, `signaling-rs`
- older web/server experiments: `web`, `web-client`, `server`
- experimental validation: `GPUTest`
- partially recovered advanced work from `worktrees/layered-core-migration`

The recovered tree is useful as a reference, but it is not trustworthy enough to restore wholesale. The rebuild should therefore establish a new clean mainline and selectively re-implement the validated architecture.

## Design Principles

1. `G:\Project\mini-remote-desktop` is the only mainline source tree.
2. Recovered directories are reference material only.
3. `Rdesk` and `Rdesk-Server` are the long-term product entry points.
4. Shared runtime/media/signaling capabilities live in `crates/`.
5. `GPUTest` is a lab/verification consumer only and must not become a product entry point.
6. Historical implementations are moved out of the mainline path so they stop defining architecture by accident.

## Target Top-Level Structure

The rebuilt repository should converge to:

- `apps/Rdesk`
- `apps/Rdesk-Server`
- `apps/realtime-server`
- `crates/`
- `labs/GPUTest`
- `junk/`
- `docs/`

## Ownership Rules

### Product Mainline

- `apps/Rdesk`
  - frontend UI
  - `src-tauri` desktop host
  - `RemoteController`
  - `RemoteAgent`
- `apps/Rdesk-Server`
  - FastAPI management API
  - authentication
  - sidecar management
- `apps/realtime-server`
  - Rust realtime sidecar
  - websocket signaling
  - session routing
  - relay/transport coordination entry point

### Shared Core

`crates/` hosts reusable core layers:

- `mrd-proto`
- `mrd-session`
- `mrd-signal-proto`
- `mrd-signal-client`
- `mrd-signal-server`
- `mrd-pipeline-core`
- `mrd-render`
- `mrd-render-d3d11`
- `mrd-decode`
- `mrd-encode`
- `mrd-transport-quic`
- `mrd-transport-webrtc`
- `mrd-capture-d3d11dup`

### Verification

- `labs/GPUTest`
  - validates shared crates
  - hosts experiments and benchmarks
  - does not define product topology

### Junk Drawer

`junk/` temporarily stores projects that are not current product mainline:

- `agent-rust`
- `controller-rust`
- `signaling-rs`
- `web`
- `web-client`
- `server`
- `agent-python`
- `client-qt`

These are retained for reference and incremental extraction only.

## Runtime Architecture

### Client Side

`apps/Rdesk/src-tauri` owns:

- session lifecycle
- realtime management client
- signaling/realtime client
- multi-window render shell
- runtime host
- pipeline host

It does not own low-level codec or transport implementations; those come from `crates/`.

### Server Side

`apps/Rdesk-Server` owns:

- login/auth
- device/session management
- sidecar lifecycle APIs
- configuration and status APIs

`apps/realtime-server` owns:

- websocket signaling
- session-based message routing
- realtime health endpoint
- sidecar runtime process behavior

### Data Flow

1. `Rdesk` authenticates against `Rdesk-Server`
2. `Rdesk` queries sidecar status through FastAPI
3. `Rdesk` connects to `realtime-server`
4. signaling negotiates session and transport
5. transport ingress produces encoded frames
6. shared decode/render pipeline presents into render windows

## Rebuild Strategy

The rebuild is not a direct file restore. It follows four milestones:

1. Restore clean workspace and shared protocol/signaling crates.
2. Restore `Rdesk` host structure and multi-window runtime shell.
3. Restore minimal real media path: render, decode, encode, frame routing.
4. Restore service-side realtime loop: `Rdesk-Server` + `realtime-server` + WebRTC/QUIC ingress.

`GPUTest` is attached after each milestone as a verification consumer.

## Error Handling and Risk Control

- Never trust recovered files without comparing them against baseline structure and current design intent.
- Prefer re-implementation from validated behavior over blind copy.
- Keep top-level moves explicit and grouped.
- Preserve runnable checkpoints after each milestone.

## Verification Standard

The rebuild is only considered successful when:

- the new top-level layout is in place
- `apps/Rdesk` and `apps/Rdesk-Server` are the only product entry points
- `apps/realtime-server` runs as a separate sidecar
- `crates/` hosts the shared runtime core
- `labs/GPUTest` validates shared capabilities without owning product flow
- `junk/` contains non-mainline historical code

