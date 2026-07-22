# Open Source Reference Map

Source: `C:\Users\10428\Downloads\deep-research-report.md`

## Executive Summary

The research report positions third-party projects as reference material, not implementation sources. The selected projects cover the product surface that `mini-remote-desktop` needs to mature: media capture and encoding, device identity, pairing, relay/gateway design, browser compatibility, Linux session handling, and degraded-network streaming.

The resulting reference split is:

- Media engine: OBS Studio, PipeWire, scrcpy, SRT.
- Device identity and pairing: Syncthing, KDE Connect.
- Remote desktop product and relay shape: RustDesk.
- Browser and compatibility gateway: Apache Guacamole client/server and xrdp.

## Reference Boundaries

The repository should treat `/refs` as a read-only research shelf. It is useful for understanding APIs, module boundaries, state machines, test fixtures, and performance tactics. It must not become the source of architecture truth, and code must not be copied directly into `apps/` or `crates/`.

Architecture decisions continue to live in `docs/plans/`, reusable Rust code remains under `crates/`, and product entrypoints remain under `apps/`.

## Project Guidance

RustDesk is useful for product packaging, self-hosting, relay, and operational shape. It is not the model for the LAN media engine internals, because the current project is already moving toward a service-owned native pipeline.

OBS Studio and PipeWire are the highest-value references for capture graph design and platform-specific source handling. They should inform source enumeration, capability probing, and explicit format negotiation.

scrcpy is a focused reference for low-latency end-to-end display and input closure. It is especially useful for queue sizing, backpressure, and keeping the UI shell thin.

Syncthing and KDE Connect provide stronger patterns for device identity, approval, fingerprints, revocation, and device capability plugins than ad hoc LAN discovery strings.

Guacamole and xrdp should define the compatibility perimeter: browser gateway, legacy protocol bridging, and Linux/RDP session behavior. They should not be allowed to pull the high-performance LAN path back into a lowest-common-denominator protocol.

SRT is the degraded-network reference. Its value is in latency, loss, and retransmission tradeoffs rather than direct protocol reuse for the LAN QUIC path.

## Implementation Direction

The reference map supports a three-track roadmap:

- Keep LAN QUIC media as the performance core.
- Add a credible control/security plane for pairing, identity, permissions, and audit.
- Define gateway and relay products as explicit degraded or compatibility modes.

Each reference project is pinned under `/refs/projects/*`, and the lock file records the upstream tag, commit, category, license, priority, and intended usage.
