#!/usr/bin/env python3
"""
Performance benchmark for agent-python.

Tests capture, encoding, and end-to-end latency.
"""
import sys
import asyncio
import time
import statistics
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "src"))


async def benchmark_capture():
    """Benchmark screen capture performance."""
    print("\n" + "="*50)
    print("Screen Capture Benchmark")
    print("="*50)

    from capture.d3dshot_backend import ScreenCapturer
    import numpy as np

    capturer = ScreenCapturer(target_fps=60)

    if not await capturer.initialize():
        print("[ERROR] Failed to initialize capturer")
        return

    print(f"Resolution: {capturer.screen_width}x{capturer.screen_height}")
    print(f"Target FPS: {capturer.target_fps}")

    # Warmup
    for _ in range(5):
        await capturer.capture_frame()

    # Benchmark
    frames = []
    times = []
    duration = 5.0  # 5 seconds
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        frame = await capturer.capture_frame()
        t1 = time.perf_counter()

        if frame:
            times.append((t1 - t0) * 1000)  # ms
            frames.append(frame)

    await capturer.close()

    # Calculate stats
    fps = len(frames) / duration
    avg_time = statistics.mean(times)
    p95_time = statistics.quantiles(times, n=20)[18] if len(times) > 20 else max(times)
    p99_time = statistics.quantiles(times, n=100)[98] if len(times) > 100 else max(times)
    max_time = max(times)

    print(f"\nResults ({duration}s):")
    print(f"  Actual FPS:     {fps:.1f}")
    print(f"  Frame time avg: {avg_time:.2f} ms")
    print(f"  Frame time P95: {p95_time:.2f} ms")
    print(f"  Frame time P99: {p99_time:.2f} ms")
    print(f"  Frame time max: {max_time:.2f} ms")

    # Rating
    if fps >= 55:
        rating = "优秀"
    elif fps >= 30:
        rating = "良好"
    elif fps >= 15:
        rating = "一般"
    else:
        rating = "较差"
    print(f"  Rating: {rating}")


async def benchmark_encoding():
    """Benchmark H.264 encoding performance."""
    print("\n" + "="*50)
    print("H.264 Encoding Benchmark")
    print("="*50)

    from encoder.pyav_encoder import PyAVEncoder
    import numpy as np

    # Test different resolutions
    test_cases = [
        (1920, 1080, "1080p"),
        (2560, 1440, "1440p"),
        (1280, 720, "720p"),
    ]

    for width, height, name in test_cases:
        print(f"\nTesting {name} ({width}x{height}):")

        encoder = PyAVEncoder(
            width=width,
            height=height,
            fps=30,
            bitrate_kbps=5000,
        )

        if not await encoder.initialize():
            print("  [ERROR] Failed to initialize encoder")
            continue

        # Create test frame
        frame_data = np.random.randint(0, 255, (height, width, 3), dtype=np.uint8)

        # Warmup
        for _ in range(3):
            await encoder.encode(frame_data.tobytes(), width, height, "RGB")

        # Benchmark
        times = []
        successful = 0

        for _ in range(10):
            t0 = time.perf_counter()
            result = await encoder.encode(frame_data.tobytes(), width, height, "RGB")
            t1 = time.perf_counter()

            if result:
                times.append((t1 - t0) * 1000)
                successful += 1

        await encoder.close()

        if times:
            avg = statistics.mean(times)
            p95 = statistics.quantiles(times, n=20)[18] if len(times) > 20 else max(times)
            print(f"  Encode time avg: {avg:.2f} ms")
            print(f"  Encode time P95: {p95:.2f} ms")
            print(f"  Max FPS:       {1000/avg:.0f}")
            print(f"  Success rate:  {successful}/10")

            # Rating
            if avg < 10:
                rating = "优秀"
            elif avg < 20:
                rating = "良好"
            elif avg < 40:
                rating = "一般"
            else:
                rating = "较差"
            print(f"  Rating: {rating}")


async def benchmark_end_to_end():
    """Benchmark capture + encode pipeline."""
    print("\n" + "="*50)
    print("End-to-End Pipeline Benchmark")
    print("="*50)

    from capture.d3dshot_backend import ScreenCapturer
    from encoder.pyav_encoder import PyAVEncoder
    import numpy as np

    capturer = ScreenCapturer(target_fps=30)
    if not await capturer.initialize():
        print("[ERROR] Failed to initialize capturer")
        return

    encoder = PyAVEncoder(
        width=capturer.screen_width,
        height=capturer.screen_height,
        fps=30,
        bitrate_kbps=5000,
    )
    if not await encoder.initialize():
        print("[ERROR] Failed to initialize encoder")
        await capturer.close()
        return

    print(f"Resolution: {capturer.screen_width}x{capturer.screen_height}")
    print(f"Target FPS: 30")

    # Warmup
    for _ in range(3):
        frame = await capturer.capture_frame()
        if frame:
            await encoder.encode(frame.data, frame.width, frame.height, frame.format)

    # Benchmark
    pipeline_times = []
    capture_times = []
    encode_times = []
    successful = 0

    start = time.time()
    duration = 5.0

    while time.time() - start < duration:
        # Capture
        t0 = time.perf_counter()
        frame = await capturer.capture_frame()
        t1 = time.perf_counter()

        if frame:
            capture_times.append((t1 - t0) * 1000)

            # Encode
            t2 = time.perf_counter()
            result = await encoder.encode(frame.data, frame.width, frame.height, frame.format)
            t3 = time.perf_counter()

            if result:
                encode_times.append((t3 - t2) * 1000)
                pipeline_times.append((t3 - t0) * 1000)
                successful += 1

    await encoder.close()
    await capturer.close()

    # Stats
    actual_fps = successful / duration

    print(f"\nResults ({duration}s):")
    print(f"  Successful frames: {successful}")
    print(f"  Actual pipeline FPS: {actual_fps:.1f}")

    if pipeline_times:
        print(f"  Pipeline latency:")
        print(f"    Avg: {statistics.mean(pipeline_times):.2f} ms")
        print(f"    P95: {statistics.quantiles(pipeline_times, n=20)[18]:.2f} ms")
        print(f"    Max: {max(pipeline_times):.2f} ms")

    if capture_times:
        print(f"  Capture latency:")
        print(f"    Avg: {statistics.mean(capture_times):.2f} ms")
        print(f"    Max: {max(capture_times):.2f} ms")

    if encode_times:
        print(f"  Encode latency:")
        print(f"    Avg: {statistics.mean(encode_times):.2f} ms")
        print(f"    Max: {max(encode_times):.2f} ms")


async def main():
    """Run all benchmarks."""
    print("="*50)
    print("agent-python Performance Benchmark")
    print("="*50)
    print(f"Python: {sys.version}")
    print(f"Platform: {sys.platform}")

    try:
        import numpy as np
        print(f"NumPy: {np.__version__}")
    except:
        pass

    try:
        import av
        print(f"PyAV: {av.__version__}")
    except:
        pass

    try:
        import aiortc
        print(f"aiortc: {aiortc.__version__}")
    except:
        pass

    await benchmark_capture()
    await benchmark_encoding()
    await benchmark_end_to_end()

    print("\n" + "="*50)
    print("Benchmark Complete!")
    print("="*50)


if __name__ == "__main__":
    asyncio.run(main())
