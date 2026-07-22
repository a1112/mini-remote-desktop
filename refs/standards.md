# Standards References

Standards and protocol specifications are not vendored as Git submodules.

Track these externally when designing implementation details:

- QUIC and DATAGRAM extension behavior for reliable control plus lossy media split.
- RTP/WebRTC only for browser and NAT compatibility paths, not the current LAN QUIC canary.
- H.264 low-latency encoding constraints and decoder access-unit boundaries.
- AV1 low-latency mode as a later capability profile, not the first LAN acceptance gate.
- TLS, certificate fingerprints, and device identity verification for the control/security plane.
