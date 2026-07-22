#!/usr/bin/env python3
"""Compare PIL vs mss capture performance."""
import sys
import time
import statistics
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent / "src"))

import asyncio
import numpy as np

async def test_mss():
    """Test mss capture performance."""
    print("\nTesting mss capture...")
    try:
        import mss
        import mss.screen_monitor

        monitor = mss.mss()
        monitor_width = monitor.monitors[1]["width"]
        monitor_height = monitor.monitors[1]["height"]

        print(f"  Resolution: {monitor_width}x{monitor_height}")

        # Warmup
        for _ in range(5):
            monitor.grab(monitor.monitors[1])

        # Benchmark
        times = []
        start = time.time()
        duration = 3.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            screenshot = monitor.grab(monitor.monitors[1])
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        fps = len(times) / duration
        avg = statistics.mean(times)
        print(f"  FPS: {fps:.1f}")
        print(f"  Avg: {avg:.2f} ms")
        return fps, avg

    except Exception as e:
        print(f"  Error: {e}")
        return None, None


async def test_pil():
    """Test PIL capture performance."""
    print("\nTesting PIL capture...")
    try:
        from PIL import ImageGrab
        import ctypes

        user32 = ctypes.windll.user32
        width = user32.GetSystemMetrics(0)
        height = user32.GetSystemMetrics(1)

        print(f"  Resolution: {width}x{height}")

        # Warmup
        for _ in range(5):
            ImageGrab.grab()

        # Benchmark
        times = []
        start = time.time()
        duration = 3.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            screenshot = ImageGrab.grab()
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        fps = len(times) / duration
        avg = statistics.mean(times)
        print(f"  FPS: {fps:.1f}")
        print(f"  Avg: {avg:.2f} ms")
        return fps, avg

    except Exception as e:
        print(f"  Error: {e}")
        return None, None


async def main():
    print("="*50)
    print("Screen Capture Performance Comparison")
    print("="*50)

    pil_fps, pil_avg = await test_pil()
    mss_fps, mss_avg = await test_mss()

    print("\n" + "="*50)
    print("Summary")
    print("="*50)

    if pil_fps and mss_fps:
        speedup = mss_fps / pil_fps if pil_fps > 0 else 0
        print(f"PIL:  {pil_fps:.1f} FPS, {pil_avg:.1f} ms")
        print(f"mss:  {mss_fps:.1f} FPS, {mss_avg:.1f} ms")
        print(f"Speedup: {speedup:.1f}x")

        if speedup > 1.5:
            print("\n✅ mss is significantly faster!")
        elif speedup > 1.1:
            print("\n⚠️ mss is moderately faster")
        else:
            print("\n❌ Similar performance")


if __name__ == "__main__":
    asyncio.run(main())
