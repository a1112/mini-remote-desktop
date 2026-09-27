# Mobile gateway local verification

## Start

Run the gateway in the logged-in Windows desktop session. An NVIDIA GPU with
NVENC support is required for the H.264 endpoint; the old JPEG endpoint remains
available for the existing Android client. The performance page reports a clear
error when NVENC is unavailable, rather than pretending to deliver 60 FPS.

```powershell
cargo build -p mrd-mobile-gateway-app
./apps/mrd-mobile-gateway/start.ps1
```

The launcher uses `CARGO_TARGET_DIR` when set and resolves installed NVIDIA DCH
driver DLLs in the process PATH. `-ListenAddress '0.0.0.0:9534'` enables the already
supported private-LAN discovery and connections. Default is loopback only.

## Actual dynamic desktop → browser display

Open `http://127.0.0.1:9534/mobile/desktop` in a WebCodecs-capable browser on this
computer. Click the 30-second test. Keep an animated source visible on the captured
**primary** screen; an offscreen browser canvas is not evidence of moving capture.
This temporary fixture provides a changing source and closes after 45 seconds:

```powershell
powershell -NoProfile -STA -File tests/mobile-gateway/moving-source.ps1
```

The page counts received packets, actual decoder outputs, and canvas draws
separately. It samples displayed pixels to count visual changes and reports
dropped draws, sequence gaps, decode errors, and queue depth. A static source can
produce 60 encoded frames/sec while having no meaningful visual changes.

`capture_to_draw_ms` is the timestamp before DXGI acquisition to browser canvas
submission, measured with the same machine clock. It does not measure physical
display scanout, Android latency, Wi-Fi performance, or remote input response.

## Transport and control recovery

```powershell
python tests/mobile-gateway/benchmark.py --seconds 30
python tests/mobile-gateway/benchmark.py --legacy --seconds 10
python tests/mobile-gateway/video_control.py
cargo test -p mrd-mobile-gateway --lib
cargo test -p nvenc --lib
```

`smoke.py` additionally exercises discovery and the legacy phone relay; run it
when no real phone is publishing, with the gateway bound to the LAN address.
None of these checks injects Windows input.

The H.264 binary envelope is `MRDWC01\0`, followed by a little-endian u32 JSON
length, JSON metadata, and one Annex B access unit. Metadata includes sequence,
capture timestamp, dimensions, actual SPS codec profile, and keyframe flag.
`capture_call_us` and `encode_call_us` measure API calls, not complete hardware
encode latency; NVENC output is asynchronous. Control messages include
`request_keyframe` and `ping`/`pong`, in addition to the existing input protocol.

The capture queue holds two frames. Overflow invalidates delta references and
requests an IDR; SPS/PPS are repeated on keyframes. A blocked socket write times
out. Browser decode backlog is bounded and discarded with keyframe recovery.

The gateway allows one active desktop capture at a time. Quick reconnects wait
for the previous worker to release its capture resources; another still-active
viewer receives a busy error. DXGI permits one duplication per output in a
process ([Microsoft API documentation](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutput1-duplicateoutput)).
`video_control.py` includes ten immediate disconnect/reconnect cycles.

## Measured 2026-09-27

RTX 5060 Ti, 2560×1440, local browser, 30 seconds of moving primary-screen content:

| Measurement | Result |
| --- | ---: |
| Received / decoded | 60.03 / 60.03 FPS |
| Canvas drawn / visual changes | 59.57 / 59.53 per second |
| Capture to canvas P50 / P95 | 21.59 / 24.93 ms |
| Decoder P50 / P95 | 0.5 / 0.9 ms |
| Decode errors / sequence gaps | 0 / 0 |

Before the fix, the debug JPEG path measured 0.88 FPS. The first H.264 version
received 60 FPS but accumulated about 200 ms in the hardware decoder. Explicit
zero-reordering, VUI bitstream restrictions, and one reference frame removed that
decoder backlog. The final build's raw dynamic-run report is
`results/2026-09-27-final-dynamic-display.json`; earlier measurements are retained
with their scenario and limitations.

These results cover the Windows-to-local-browser path. The current Android
JPEG sender and receiver have **not** been upgraded or verified at 60 FPS.
