# Remote Streaming Frontier Review

Source: `C:\Users\10428\Downloads\deep-research-report2.md`

## Executive Summary

The research report confirms the current direction of `mini-remote-desktop`: keep the product centered on a low-latency media engine, with the high-performance path implemented as a native capture, encode, transport, decode, and render pipeline. The browser path should remain a gateway or diagnostics surface rather than the primary high-frame-rate rendering target.

The strongest near-term architecture is:

- Windows control and peer path: DXGI shared capture, NVENC H.264 first, QUIC datagram media transport, NVDEC decode, and D3D11 native/shared render.
- Linux peer path: PipeWire capture, explicit encoder capability probing, and profile downgrade when hardware encode is not available.
- Web compatibility path: browser gateway with bounded frame rate and explicit degradation labels.
- Test discipline: fixed scene profiles, fixed thresholds, exported JSON/Markdown reports, and per-stage p50/p95 metrics.

## Findings

Remote display quality is gated less by raw network throughput and more by copies, format conversions, queueing, and mismatched runtime profiles. The previous failures around `invalid magic`, WebRTC unsupported peers, and `1920x1080 -> 1728x1080` negotiation mismatches should be treated as product observability bugs, not only media bugs.

The report recommends measuring the media path as separate stages:

- capture
- scale or convert
- encode
- fragment and send
- receive and reassemble
- decode
- render upload
- present
- queue depth
- dropped frames

These metrics map directly to the matrix work already added in the LAN media parity branch.

## Architecture Implications

The service remains the correct home for the heavy media runtime. Rdesk should create windows, own native surface lifecycle, and show state, but it should not carry high-bandwidth frame payloads through React or WebView. This aligns with the current thin-shell plus local-service migration.

QUIC datagrams are still the preferred LAN media transport for the mainline. WebRTC is useful for NAT/browser compatibility, but it should not block LAN acceptance. A peer that only lacks WebRTC should be marked `skipped` for WebRTC rows, not failed for the whole matrix.

High-frame modes need explicit proof that they are not silently limited by display refresh rate, capture pacing, encoder completion, or renderer present scheduling. The paired canary should therefore include 144, 180, and 249 FPS rows and report selected FPS, current FPS, dropped frames, and stage p95.

## KPI Targets

The practical acceptance targets for the next phase are:

- 1080p144 LAN QUIC: cross-device FPS at least 80 percent of local baseline.
- 1080p180 and 1080p249: runtime selected FPS must match requested FPS or be reported as profile downgraded.
- 2K60: decoded FPS remains at or above 45 FPS on capable Windows peers.
- No `invalid magic`, legacy probe fallback, profile mismatch, or service IPC crash in accepted rows.

## Risks

The main risks are hidden fallback paths and unclear peer versioning. A peer can advertise `quic` while still running an older media payload format or a CPU-backed sender path. The product should fail fast on media protocol mismatch and require explicit capabilities such as `dxgi_capture`, `nvenc_h264`, `quic_datagram_media_v2`, `nvdec`, and `d3d11_native_render`.

License risk is also material when using open-source references. GPL and AGPL projects can guide architecture and tests, but implementation code must stay original unless the project explicitly accepts the license consequences.
