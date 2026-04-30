# Capture To Render Demo

Standalone visual demo for the native capture-to-render path. It captures frames from the platform capture backend and uploads them directly to the platform renderer, without encode/decode, transport, Tauri pages, or matrix orchestration.

## macOS

```bash
cargo run --manifest-path tests/capture-render-demo/Cargo.toml -- \
  --width 1280 \
  --height 720 \
  --fps 60 \
  --duration-ms 15000
```

Run continuously until the window is closed or the process is stopped:

```bash
cargo run --manifest-path tests/capture-render-demo/Cargo.toml -- \
  --width 1280 \
  --height 720 \
  --fps 60 \
  --continuous
```

List capturable windows:

```bash
cargo run --manifest-path tests/capture-render-demo/Cargo.toml -- --list-windows
```

Capture one window by ScreenCaptureKit window ID:

```bash
cargo run --manifest-path tests/capture-render-demo/Cargo.toml -- \
  --window-id 12345 \
  --width 1280 \
  --height 720
```

The demo intentionally captures the real screen/window. If the render window is on the captured display, recursive “screen inside screen” feedback is expected and useful for checking continuous present.
