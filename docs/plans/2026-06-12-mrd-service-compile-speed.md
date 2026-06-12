# MRD Service Compile Speed Notes

Goal: keep the default `mrd-service` development build focused on the local
service path while preserving heavier preview features behind explicit flags.

## Default Fast Path

Use the default build for IPC, LAN discovery, Web bridge, WebCodecs preview,
file transfer, device actions, and service orchestration work:

```powershell
cargo check -p mrd-service
cargo build -p mrd-service
```

The default build intentionally excludes heavier optional stacks:

- Browser WebRTC preview: upstream `webrtc` plus `mrd-transport-webrtc`
- Software VVC encode: `mrd-encode-vvenc` and its VVenC integration

These are unnecessary for most service iteration and can be enabled explicitly
when the relevant route or codec work is under test.

## Full Browser WebRTC Preview Build

Use the feature build when working on `/browser/webrtc-preview/*` routes:

```powershell
cargo check -p mrd-service --features browser-webrtc-preview
cargo build -p mrd-service --features browser-webrtc-preview
```

When the feature is not enabled, the Web bridge still exposes the preview
routes, but they return `E_BROWSER_WEBRTC_PREVIEW_DISABLED` with instructions to
rebuild with `--features browser-webrtc-preview`.

## Full Software VVC Build

Use the feature build when working on the VVenC/VVdeC software H.266 path:

```powershell
cargo check -p mrd-service --features production-vvc-software-codec
cargo build -p mrd-service --features production-vvc-software-codec
```

The default capability report continues to list software VVC encode/decode as
unimplemented rather than silently advertising an unavailable codec path.

On Windows, this feature also requires the native VVC toolchain to be available
to Cargo: `libvvenc.pc` through `PKG_CONFIG_PATH` for `vvenc-sys`, and
`libclang.dll` through `LIBCLANG_PATH` for bindgen.

## Dev Profile

The workspace dev profile uses `debug = 1` to reduce debug artifact size and
link pressure. This keeps useful stack traces while avoiding very large local
`target/debug` growth during repeated Windows builds.
