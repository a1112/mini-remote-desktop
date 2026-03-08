# Component Matrix

This directory contains single-component performance validation for mainline crates.

Current first wave:
- `mrd-capture-dxgi`
- `mrd-encode-openh264`
- `mrd-decode`

Current second wave:
- `mrd-transport-webrtc` sender boundary
- `mrd-transport-webrtc` receiver boundary
- `mrd-transport-quic-quinn` sender boundary
- `mrd-transport-quic-quinn` receiver boundary
- `mrd-render-d3d11`

Each crate owns:
- normal functionality tests in `cargo test`
- ignored performance tests for latency/throughput distribution

Transport sender measures:
- `packetize + sender-boundary` latency distribution
- access-unit size, written bytes, packets-per-sample

Transport receiver measures:
- `RTP payload -> Annex-B access unit` latency distribution
- payload bytes, reassembled access-unit size, packets-per-sample

QUIC transport sender/receiver measure:
- datagram send boundary latency
- datagram receive boundary latency
- payload bytes and packets-per-sample

Render measures:
- `upload_frame()` boundary latency distribution
- frame bytes and throughput

Matrix artifacts are written to:
- `artifacts/component-matrix/`
