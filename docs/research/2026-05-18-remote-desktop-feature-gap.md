# Remote Desktop Feature Gap Research Archive

## Source

- Local source document: `C:\Users\10428\Downloads\deep-research-report.md`
- Archived on: 2026-05-18
- Scope: Compare `mini-remote-desktop` with ToDesk, Sunlogin, AnyDesk, TeamViewer, RustDesk, Chrome Remote Desktop, and Splashtop.

## Core Finding

`mini-remote-desktop` has a credible low-latency native media direction, especially around DXGI/D3D11 capture, NVENC/NVDEC, QUIC media transport, native render, capability matrices, and benchmark telemetry. The largest gap is not another isolated media optimization. The missing product surface is the control, security, deployment, and operations plane that mature remote desktop products use to move from demos to repeatable production use.

## High-Impact Gaps

| Gap | Why It Matters | Existing Project Surface | Implementation Direction |
| --- | --- | --- | --- |
| Device identity and pairing trust | Prevents unknown peers from silently becoming controllable devices. | LAN device id, LAN discovery, service build/media capability handshake. | Add trusted-device records, approval state, certificate fingerprint, revoke flow. |
| Authorization and policy | Commercial tools gate actions by user, device, time, and capability. | Session lifecycle and IPC request boundaries. | Add explicit consent/policy checks before control/media/session operations. |
| Audit logging | Enterprise and support workflows need session traceability and incident review. | Session registry, telemetry store, runtime snapshots. | Add service-owned audit event stream, then persist and expose in UI. |
| Public/weak-network connectivity | Real deployments need NAT traversal, relay, diagnostics, and fallback modes. | LAN QUIC and early WebRTC concepts. | Keep LAN native path strict; add separate relay/WebRTC/TURN acceptance later. |
| Deployment and fleet management | Mature products support unattended install, updates, policy, and diagnostics at scale. | Service manager, autostart, shell state. | Add deployment status and update readiness after trust/audit base exists. |
| Peripheral features | Remote print, USB, serial, camera, audio, clipboard, and file transfer complete daily workflows. | Control-plane plans and test workbench. | Implement after reliable control channel and audit semantics are stable. |

## Priority Decision

The next low-risk product gap to implement is service-owned audit logging. It is smaller than full identity/pairing, but it creates the traceability foundation needed for pairing, authorization, relay diagnostics, and enterprise reporting.

## This Branch Implementation Target

Add an IPC-visible audit log:

- service records device registration and session lifecycle decisions;
- each event includes timestamp, action, outcome, optional session id, actor device id, peer device id, transport, reason, and details;
- callers can query by session, action, and limit;
- tests cover IPC contract and in-process service behavior.

This is intentionally an in-memory v1. Persistence, retention policy, export, and UI pages should follow after the event schema stabilizes.

## Roadmap Alignment

| Priority | Area | Task |
| --- | --- | --- |
| P0 | `area/security type/implementation` | Add audit event schema and service-owned event registry. |
| P0 | `area/security type/design` | Use audit events as input to device trust and pairing approval design. |
| P1 | `area/control type/implementation` | Emit audit events from reliable control-plane operations. |
| P1 | `area/product type/research` | Compare audit/recording/export behavior with RustDesk, TeamViewer, Splashtop, ToDesk, and Sunlogin. |
