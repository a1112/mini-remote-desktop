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

## Output

A timestamped folder is created under `mini-remote-desktop/logs/`, containing:
- `suite_result.json`
- `suite_report.md`
- per-transport logs (`signaling.*.log`, `agent.*.log`)

## Notes

- `agent-python` is intentionally excluded.
- This suite only targets core transport paths for `agent-rust`.
- `webtransport` probe uses HTTP/3 CONNECT WebTransport session with `aioquic`.
