"""Measure the actual loopback desktop stream. No input injection."""
import argparse
import json
import statistics
import time
from pathlib import Path
from smoke import Ws

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=30)
    parser.add_argument("--legacy", action="store_true")
    parser.add_argument("--output", default="")
    args = parser.parse_args()
    client = Ws("/mobile/desktop/ws" if args.legacy else "/mobile/desktop/video/ws")
    client.ready()
    received, latencies, sequences, sizes, captures, encodes = [], [], [], [], [], []
    start = time.perf_counter()
    dimensions = None
    try:
        while time.perf_counter() - start < args.seconds:
            kind, data = client.read()
            if kind == 1:
                message = json.loads(data)
                if message.get("type") == "error":
                    raise RuntimeError(message)
                continue
            if kind != 2:
                continue
            now = time.perf_counter()
            if not args.legacy:
                assert data[:8] == b"MRDWC01\0", "expected H.264 packet"
                length = int.from_bytes(data[8:12], "little")
                header = json.loads(data[12:12+length])
                assert len(data) > 12 + length and header["codec_format"] == "annexb"
                dimensions = [header["width"], header["height"]]
                latencies.append(time.time_ns() / 1000 - header["capture_unix_us"])
                sequences.append(header["sequence"])
                captures.append(header["capture_call_us"])
                encodes.append(header["encode_call_us"])
            received.append(now)
            sizes.append(len(data))
    finally:
        client.close()
    duration = received[-1] - received[0] if len(received) > 1 else 0
    report = {
        "codec": "JPEG" if args.legacy else "H264",
        "duration_s": round(duration, 3),
        "frames": len(received),
        "receive_fps": round((len(received) - 1) / duration, 2) if duration else 0,
        "resolution": dimensions,
        "mbps": round(sum(sizes[1:]) * 8 / duration / 1e6, 3) if duration else 0,
        "sequence_gaps": sum(max(0, b-a-1) for a,b in zip(sequences, sequences[1:])),
    }
    for key, values in [("capture_to_receive_ms", latencies), ("capture_call_ms", captures), ("encode_call_ms", encodes)]:
        if values:
            ordered = sorted(values)
            report[key] = {"p50": round(statistics.median(ordered)/1000, 2),
                           "p95": round(ordered[int((len(ordered)-1)*.95)]/1000, 2)}
    encoded = json.dumps(report, indent=2)
    if args.output:
        Path(args.output).write_text(encoded + "\n", encoding="utf-8")
    print(encoded)
    if not args.legacy:
        assert report["receive_fps"] >= 57, "60 FPS receive gate failed"

if __name__ == "__main__":
    main()
