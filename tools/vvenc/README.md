# Local VVenC Bootstrap

This directory keeps the optional H.266/VVC software encoder dependency local to the
repository. Generated source, build, install, and environment files are ignored.

```powershell
powershell -ExecutionPolicy Bypass -File tools\vvenc\setup_vvenc.ps1
```

The script installs VVenC under `tools/vvenc/install`, validates
`lib/pkgconfig/libvvenc.pc`, and writes `tools/vvenc/env.local.ps1`.

Use the generated environment when building the feature-gated Rust path:

```powershell
. .\tools\vvenc\env.local.ps1
cargo test -p mrd-encode-vvenc --features software-vvenc --no-run
```
