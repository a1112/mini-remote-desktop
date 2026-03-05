# Python Core Transport Suite

Automated core-layer comparison for `agent-rust` transports:
- `webrtc`
- `quic`
- `webtransport`

This suite focuses on:
- control-plane negotiation success (`offer/answer/selectedTransport`)
- media/stream receiving on transport core path
- jitter / stall statistics
- agent-side error and drop signals from logs

## Multi-terminal stress (5 clients)

Run 5 concurrent controller terminals against one `agent-rust` session:

```powershell
cd J:\ProjectTest\remote-desktop\mini-remote-desktop
python .\tests\python-core-transport\run_multi_terminal_stress.py --transport quic --clients 5 --duration 20
```

Supported transports:
- `webrtc`
- `quic`
- `webtransport`

## Run

```powershell
cd J:\ProjectTest\remote-desktop\mini-remote-desktop
python .\tests\python-core-transport\run_core_transport_suite.py
```

## HEVC/AV1 + ROI matrix

Run codec/ROI matrix on core transports (includes WebRTC and WebTransport):

```powershell
cd J:\ProjectTest\remote-desktop\mini-remote-desktop
python .\tests\python-core-transport\run_codec_roi_matrix.py
```

The matrix report now includes `roi_mode` comparison (`quality` vs `performance(requireNative)`),
with delta columns to compare FPS / jitter / CPU / GPU against baseline mode.

FPS cap tier matrix (72/120/144/180/240) across resolutions:

```powershell
cd J:\ProjectTest\remote-desktop\mini-remote-desktop
python .\tests\python-core-transport\run_fps_tier_matrix.py --codec av1 --transport webtransport --tiers 72,120,144,180,240 --resolutions 1280x720,1920x1080
```

Direct single-suite run with forced codec/ROI and native ROI probe switch:

```powershell
cd J:\ProjectTest\remote-desktop\mini-remote-desktop
python .\tests\python-core-transport\run_core_transport_suite.py --transports webtransport --codec av1 --roi-enable --native-roi-probe
```

## Output

A timestamped folder is created under `mini-remote-desktop/logs/`, containing:
- `suite_result.json`
- `suite_report.md`
- per-transport logs (`signaling.*.log`, `agent.*.log`)

The report also includes `Agent Pipeline` jitter metrics parsed from agent-side
`[PIPELINE-STATS]` logs (capture / encode-output / send-interval / queue-wait / send std).
Thresholds are configured under `thresholds.*.json -> thresholds.agent_pipeline_jitter`.

## Notes

- `agent-python` is intentionally excluded.
- This suite only targets core transport paths for `agent-rust`.
- `webtransport` probe uses HTTP/3 CONNECT WebTransport session with `aioquic`.
