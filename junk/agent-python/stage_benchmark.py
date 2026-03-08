#!/usr/bin/env python3
"""Stage-by-stage GPU pipeline benchmark with capture session retry."""

import argparse
import ctypes
import json
import statistics
import time
from pathlib import Path
from typing import Dict, List, Optional, Sequence

import sys

sys.path.insert(0, str(Path(__file__).parent / "src"))

from src.capture.wgc_capture import WGCCapture
from src.encoder.nvenc_encoder import create_nvenc_encoder, NVENCEncoder
from src.transport.stats import FrameInfo


def _series_stats(samples: Sequence[float]) -> Optional[Dict[str, float]]:
    if not samples:
        return None
    result = {
        "avg": statistics.mean(samples),
        "min": min(samples),
        "max": max(samples),
    }
    result["p95"] = statistics.quantiles(samples, n=20)[18] if len(samples) >= 20 else result["max"]
    return result


def summarize_stage_metrics(rows: List[Dict[str, float]]) -> Dict[str, object]:
    capture = [r["capture_ms"] for r in rows]
    encode = [r["encode_ms"] for r in rows]
    send = [r["send_ms"] for r in rows]
    total = [r["total_ms"] for r in rows]
    sizes = [r["encoded_size"] for r in rows if r["encoded_size"] > 0]

    summary: Dict[str, object] = {
        "frames": len(rows),
        "capture_ms": _series_stats(capture),
        "encode_ms": _series_stats(encode),
        "send_ms": _series_stats(send),
        "total_ms": _series_stats(total),
        "avg_encoded_size": int(statistics.mean(sizes)) if sizes else 0,
    }

    total_avg = summary["total_ms"]["avg"] if summary["total_ms"] else 0.0  # type: ignore[index]
    summary["estimated_fps"] = (1000.0 / total_avg) if total_avg > 0 else 0.0
    return summary


def _start_capture_with_retry(monitor_index: int, retries: int, retry_delay_ms: int) -> Optional[WGCCapture]:
    for attempt in range(1, retries + 1):
        capture = WGCCapture()
        if capture.start_monitor(monitor_index):
            return capture
        capture.stop()
        if attempt < retries:
            time.sleep(retry_delay_ms / 1000.0)
    return None


def run_stage_benchmark(
    monitor_index: int = 0,
    retries: int = 5,
    retry_delay_ms: int = 300,
    target_frames: int = 240,
    quality: int = NVENCEncoder.QUALITY_HIGH,
    framerate: int = 144,
) -> Dict[str, object]:
    capture = _start_capture_with_retry(monitor_index, retries, retry_delay_ms)
    if capture is None:
        return {
            "ok": False,
            "error": "failed_to_start_capture_session",
            "monitor_index": monitor_index,
            "retries": retries,
        }

    try:
        warmup = None
        for _ in range(30):
            warmup = capture.capture_frame()
            if warmup and warmup.d3d11_texture:
                break
            time.sleep(0.01)

        if not warmup or not warmup.d3d11_texture:
            return {"ok": False, "error": "no_initial_frame"}

        encoder = create_nvenc_encoder(
            capture.d3d11_device,
            capture.d3d11_context,
            warmup.width,
            warmup.height,
            quality=quality,
            framerate=framerate,
        )
        if encoder is None:
            return {"ok": False, "error": "failed_to_init_nvenc"}

        rows: List[Dict[str, float]] = []
        sent = 0

        for i in range(target_frames):
            t0 = time.perf_counter()
            frame = capture.capture_frame()
            t1 = time.perf_counter()
            if not frame or not frame.d3d11_texture:
                continue

            encoded = encoder.encode_d3d11(int(frame.d3d11_texture))
            t2 = time.perf_counter()
            if not encoded:
                continue

            _ = FrameInfo(
                data=encoded.data,
                timestamp=int(frame.timestamp),
                is_keyframe=bool(encoded.key_frame),
                width=frame.width,
                height=frame.height,
                frame_number=i,
            )
            t3 = time.perf_counter()

            rows.append(
                {
                    "capture_ms": (t1 - t0) * 1000.0,
                    "encode_ms": (t2 - t1) * 1000.0,
                    "send_ms": (t3 - t2) * 1000.0,
                    "total_ms": (t3 - t0) * 1000.0,
                    "encoded_size": float(encoded.size),
                }
            )
            sent += 1

        encoder.close()

        summary = summarize_stage_metrics(rows)
        return {
            "ok": True,
            "monitor_index": monitor_index,
            "resolution": f"{warmup.width}x{warmup.height}",
            "frames_target": target_frames,
            "frames_processed": sent,
            "summary": summary,
        }
    finally:
        capture.stop()


def main() -> None:
    parser = argparse.ArgumentParser(description="Stage benchmark for agent-python GPU pipeline")
    parser.add_argument("--monitor", type=int, default=0, help="monitor index")
    parser.add_argument("--frames", type=int, default=240, help="target frame count")
    parser.add_argument("--retries", type=int, default=5, help="capture start retries")
    parser.add_argument("--retry-delay-ms", type=int, default=300, help="retry delay")
    parser.add_argument("--fps", type=int, default=144, help="target encoder fps")
    parser.add_argument("--quality", type=int, default=NVENCEncoder.QUALITY_HIGH, help="nvenc qp")
    parser.add_argument("--out", type=str, default="", help="optional output json path")
    args = parser.parse_args()

    result = run_stage_benchmark(
        monitor_index=args.monitor,
        retries=args.retries,
        retry_delay_ms=args.retry_delay_ms,
        target_frames=args.frames,
        quality=args.quality,
        framerate=args.fps,
    )

    text = json.dumps(result, ensure_ascii=False, indent=2)
    print(text)
    if args.out:
        Path(args.out).write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
