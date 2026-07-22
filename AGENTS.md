# AGENTS.md

This file provides guidance to Codex (Codex.ai/code) when working with code in this repository.

## Repository Structure

This is a high-performance remote desktop system in mid-rebuild. The active mainline is:

```
apps/
  Rdesk/              # Tauri desktop client (UI shell)
  mrd-service/        # Local session orchestrator service (IPC server)
  Rdesk-Server/       # FastAPI backend (auth, device registry, session API)
  realtime-server/    # Rust realtime sidecar (WebSocket signaling)

crates/               # Shared Rust crates (domain logic, infrastructure adapters)
  mrd-session/        # Session domain model (SessionLifecycleState, QuicSessionCoordinator)
  mrd-application/    # Application use cases (orchestrates session lifecycle via abstract ports)
  mrd-proto/          # Shared protocol types (SessionId, DeviceId, BackendRole)
  mrd-ipc/            # Local IPC protocol (Rdesk ↔ mrd-service communication)
  mrd-signal-*/       # Signaling client/server/protocol
  mrd-transport-*/    # QUIC/WebRTC transport adapters
  mrd-capture-dxgi/   # DXGI screen capture
  mrd-encode-*/       # H.264 encoders (NVENC, OpenH264)
  mrd-decode*/        # H.264 decoders (NVDEC, software)
  mrd-render*/        # D3D11 rendering

common-control-proto/ # Legacy shared control protocol (still in use)
heartbeat-rs/         # UDP heartbeat/discovery service
tools/                # External dependencies (NVIDIA Codec SDK headers)
tests/                # Integration and component tests
junk/                 # Historical implementations (reference only, NOT architecture-defining)
```

## Architecture Migration Target

The repository is migrating to "thin shell + local service" architecture:

- `Rdesk` → UI shell only (window management, local settings)
- `mrd-service` → Session orchestrator (single entry point via IPC)
- `mrd-application` → Use case layer (start/accept/sync session)
- `mrd-session` → Session domain model (lifecycle state, roles)
- `crates/mrd-transport-*/` → Infrastructure adapters (QUIC, WebRTC)

See: `docs/plans/2026-03-20-mrd-service-architecture-migration.md`

## Common Commands

### Rust Workspace

```bash
# Build entire workspace
cargo build

# Build specific package
cargo build -p mrd-service
cargo build -p realtime-server

# Run tests (including ignored component tests)
cargo test
cargo test -- --ignored

# Run specific package tests
cargo test -p mrd-session
```

### Frontend (apps/Rdesk)

```bash
cd apps/Rdesk
pnpm install          # Install dependencies
pnpm dev              # Start Vite dev server
pnpm build            # Build for production
pnpm tauri:dev        # Run Tauri dev mode (frontend + Rust backend)
pnpm tauri:build      # Build Tauri application
pnpm test             # Run Vitest unit tests
pnpm type-check       # TypeScript type checking
```

### Backend (apps/Rdesk-Server)

```bash
cd apps/Rdesk-Server
pip install -r requirements.txt
python -m uvicorn app.main:app --reload  # Run development server
```

### Realtime Server

```bash
cargo run -p realtime-server
```

### Component Matrix Tests

```powershell
# Run single component test
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_case.ps1 `
  -CasePath tests/component-matrix/cases/capture.dxgi.json

# Run full component matrix
powershell -ExecutionPolicy Bypass -File tests/component-matrix/scripts/run_component_matrix.ps1
```

### Transport Benchmarks

```powershell
# Quick transport benchmark
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.json

# NVENC transport benchmark
powershell -ExecutionPolicy Bypass -File tests/benchmarks/scripts/run_transport_matrix.ps1 `
  -ScenarioPath tests/benchmarks/scenarios/quick.transport.nvenc.json
```

### Python Core Transport Tests

```powershell
python tests/python-core-transport/run_core_transport_suite.py
```

## Key Architecture Concepts

### Session Domain Model (mrd-session)

- `SessionLifecycleState`: Explicit state machine (Created → Listening/Connecting → Connected → Streaming → Failed/Closed)
- `QuicSessionSnapshot`: Domain state independent of Quinn implementation
- `QuicSessionCoordinator`: Domain-level session state management

### Application Layer (mrd-application)

Uses abstract ports for infrastructure independence:
- `SignalingPort`: Signaling client interface
- `SessionCoordinatorPort`: Session state management interface
- `QuicHostPort`: QUIC transport host interface

Use cases:
- `apply_realtime_events()`: Drain signaling events and apply to session coordinators
- `sync_quic_host_from_session_snapshot()`: Sync QUIC host with domain state

### Transport Layer

Two transport implementations:
- `mrd-transport-quic-quinn`: QUIC via Quinn
- `mrd-transport-webrtc`: WebRTC via webrtc-rs

### Pipeline Components

- `mrd-capture-dxgi`: DXGI screen capture
- `mrd-encode-nvenc`: NVIDIA H.264 hardware encoder
- `mrd-encode-openh264`: OpenH264 software encoder
- `mrd-decode-nvdec`: NVIDIA H.264 hardware decoder
- `mrd-decode`: Software H.264 decoder
- `mrd-render-d3d11`: D3D11 rendering

## Important Notes

- **DO NOT** reference `junk/` for architecture decisions — it contains historical implementations only
- When adding reusable Rust logic, place it under `crates/`
- When adding product features, place entrypoints under `apps/`
- Hardware encoder tests (`mrd-encode-nvenc`, `mrd-decode-nvdec`) require NVIDIA GPUs and will show zero throughput on unsupported hosts
- The workspace uses resolver = "2" for consistent dependency resolution

## Testing Architecture

Frontend tests use three-layer pyramid (see `apps/Rdesk/docs/test-architecture.md`):
1. Service Tests: Pure logic mocks (Vitest)
2. Component Tests: DOM + interactions (@testing-library/react, happy-dom)
3. Contract Tests: Tauri command verification

## Design Documents

See `docs/plans/` for:
- Migration plans (mrd-service architecture migration)
- Component designs (NVDEC, QUIC, encoder enhancement)
- Rebuild plans (mini-remote-desktop rebuild)
