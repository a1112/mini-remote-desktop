#!/usr/bin/env python3
"""
Test NVENC hardware encoding with proper configuration.
"""
import av
import numpy as np
import time

print("="*60)
print("Hardware Encoder Performance Test")
print("="*60)

# Test frame
frame = av.VideoFrame.from_ndarray(
    np.random.randint(0, 255, (1080, 1920, 3), dtype=np.uint8),
    format='rgb24'
)

encoders = [
    ('libx264', {'preset': 'ultrafast', 'tune': 'zerolatency'}),
    ('h264_nvenc', {
        'preset': 'fast',
        'tune': 'll',  # low latency
        'rc': 'cbr',   # constant bitrate
        'b': '5000k'
    }),
    ('h264_mf', {}),
]

results = []

for codec_name, options in encoders:
    print(f"\nTesting {codec_name}...")
    try:
        # Create encoder
        enc = av.CodecContext.create(codec_name, 'w')
        enc.width = 1920
        enc.height = 1080
        enc.framerate = (30, 1)
        enc.bit_rate = 5_000_000
        # Don't set time_base - let encoder use default

        # Apply options
        for key, value in options.items():
            enc.options[key] = str(value)

        enc.open()

        # Encode 30 frames
        times = []
        total_bytes = 0

        for i in range(30):
            t0 = time.perf_counter()
            packets = list(enc.encode(frame))
            t1 = time.perf_counter()

            for pkt in packets:
                total_bytes += pkt.size

            times.append((t1 - t0) * 1000)

        enc.close()

        avg_time = sum(times) / len(times)
        fps = 1000 / avg_time if avg_time > 0 else 0
        avg_size = total_bytes / 30

        print(f"  ✅ Success!")
        print(f"     Avg encode time: {avg_time:.2f} ms")
        print(f"     Max FPS: {fps:.1f}")
        print(f"     Avg frame size: {avg_size:.0f} bytes")
        print(f"     Bitrate: {avg_size * 8 * 30 / 1000:.0f} kbps")

        results.append((codec_name, fps, avg_time))

    except Exception as e:
        print(f"  ❌ Failed: {e}")

print("\n" + "="*60)
print("SUMMARY")
print("="*60)

if results:
    print(f"{'Encoder':<15} {'Max FPS':<10} {'Avg Time':<10}")
    print("-"*40)
    for name, fps, avg in sorted(results, key=lambda x: x[1], reverse=True):
        marker = "🚀" if fps > 100 else "⚡" if fps > 60 else "💻"
        print(f"{marker} {name:<15} {fps:<10.1f} {avg:<10.2f} ms")
