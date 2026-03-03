#!/usr/bin/env python3
"""
Test complete encode-decode pipeline.

Verifies that:
1. Screen capture works
2. H.264 encoding produces valid output
3. Decoding can reconstruct frames
4. End-to-end latency is acceptable
"""

import asyncio
import sys
import time
import numpy as np
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from capture.d3dshot_backend import ScreenCapturer, CapturedFrame
from encoder.simple_encoder import SimpleH264Encoder, SimpleEncodedFrame
from decoder.pyav_decoder import PyAVDecoder, DecodedFrame
import logging

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)

# Enable debug for encoder
logging.getLogger('encoder.pyav_encoder').setLevel(logging.DEBUG)
logger = logging.getLogger(__name__)


async def test_encode_decode_pipeline():
    """Test the complete encode-decode pipeline."""

    print("=" * 60)
    print("Encode-Decode Pipeline Test")
    print("=" * 60)

    # Initialize components
    capturer = ScreenCapturer(
        target_fps=30,
        preferred_backend="pil"  # Use PIL for testing
    )

    encoder = SimpleH264Encoder(
        width=1920,
        height=1080,
        fps=30,
        bitrate_kbps=5000
    )

    decoder = PyAVDecoder()

    # Initialize
    print("\n1. Initializing components...")
    if not await capturer.initialize():
        print("   ❌ Capturer init failed")
        return

    if not await encoder.initialize():
        print("   ❌ Encoder init failed")
        return

    if not await decoder.initialize():
        print("   ❌ Decoder init failed")
        return

    print("   ✅ All components initialized")

    # Capture and encode a few frames
    print("\n2. Testing encode-decode pipeline (10 frames)...")

    successful = 0
    failed = 0
    encode_times = []
    decode_times = []
    total_times = []

    for i in range(15):  # Try more frames to get encoder output
        print(f"   Frame {i+1}/15... ", end="", flush=True)

        t0 = time.time()

        # Capture
        captured_frame = await capturer.capture_frame()
        if captured_frame is None:
            print("❌ Capture failed")
            failed += 1
            continue

        t1 = time.time()

        # Encode
        encoded_frame = await encoder.encode(
            captured_frame.data,
            captured_frame.width,
            captured_frame.height
        )
        if encoded_frame is None:
            # No output yet, but not necessarily a failure
            print("⏳ No output (encoder buffering)")
            continue

        t2 = time.time()

        # Decode
        decoded_frame = await decoder.decode(encoded_frame.data, t0)
        if decoded_frame is None:
            print("❌ Decode failed")
            failed += 1
            continue

        t3 = time.time()

        # Success
        encode_time = (t2 - t1) * 1000
        decode_time = (t3 - t2) * 1000
        total_time = (t3 - t0) * 1000

        encode_times.append(encode_time)
        decode_times.append(decode_time)
        total_times.append(total_time)

        keyframe = "🔑" if encoded_frame.is_keyframe else "  "
        size_kb = len(encoded_frame.data) / 1024

        print(f"✅ {keyframe} {encode_time:5.1f}ms enc, {decode_time:5.1f}ms dec, {size_kb:5.1f}KB")
        successful += 1

    # Statistics
    print("\n3. Statistics:")
    print(f"   Successful: {successful}")
    print(f"   Failed: {failed}")

    if encode_times:
        avg_encode = sum(encode_times) / len(encode_times)
        avg_decode = sum(decode_times) / len(decode_times)
        avg_total = sum(total_times) / len(total_times)

        print(f"\n   Latency:")
        print(f"     Avg Encode: {avg_encode:.1f} ms")
        print(f"     Avg Decode: {avg_decode:.1f} ms")
        print(f"     Avg Total:  {avg_total:.1f} ms")

        # Estimate max FPS
        max_fps = 1000 / avg_total if avg_total > 0 else 0
        print(f"\n   Estimated Max FPS: {max_fps:.1f}")

        # Rating
        if avg_total < 30:
            rating = "⭐⭐⭐ Excellent (<30ms)"
        elif avg_total < 50:
            rating = "⭐⭐ Good (<50ms)"
        elif avg_total < 100:
            rating = "⭐ Acceptable (<100ms)"
        else:
            rating = "❌ Poor (>100ms)"
        print(f"   Rating: {rating}")

    # Decoder stats
    stats = decoder.get_stats()
    print(f"\n   Decoder:")
    print(f"     Frames decoded: {stats['frame_count']}")
    print(f"     Keyframes: {stats['keyframe_count']}")
    print(f"     Errors: {stats['decode_errors']}")

    # Cleanup
    print("\n4. Cleanup...")
    await capturer.close()
    await encoder.close()
    await decoder.close()
    print("   ✅ Done")

    print("\n" + "=" * 60)
    print("Pipeline Test Complete")
    print("=" * 60)


async def test_live_decode():
    """Test live decode with streaming."""

    print("\n" + "=" * 60)
    print("Live Decode Test (5 seconds)")
    print("=" * 60)

    capturer = ScreenCapturer(target_fps=30, preferred_backend="pil")
    encoder = SimpleH264Encoder(fps=30, width=1920, height=1080)
    decoder = PyAVDecoder()

    if not await capturer.initialize():
        print("❌ Capturer init failed")
        return

    if not await encoder.initialize():
        print("❌ Encoder init failed")
        return

    if not await decoder.initialize():
        print("❌ Decoder init failed")
        return

    print("\nCapturing, encoding, and decoding for 5 seconds...\n")

    start_time = time.time()
    frame_count = 0
    decode_count = 0

    while time.time() - start_time < 5.0:
        # Capture
        captured = await capturer.capture_frame()
        if not captured:
            continue

        # Encode
        encoded = await encoder.encode(captured.data, captured.width, captured.height)
        if not encoded:
            continue

        frame_count += 1

        # Decode every 5th frame to save CPU
        if frame_count % 5 == 0:
            decoded = await decoder.decode(encoded.data)
            if decoded:
                decode_count += 1

        # Progress
        if frame_count % 30 == 0:
            elapsed = time.time() - start_time
            fps = frame_count / elapsed
            print(f"   {frame_count} frames captured, {decode_count} decoded, {fps:.1f} FPS")

    elapsed = time.time() - start_time
    avg_fps = frame_count / elapsed

    print(f"\nResults:")
    print(f"   Frames captured: {frame_count}")
    print(f"   Frames decoded: {decode_count}")
    print(f"   Average FPS: {avg_fps:.1f}")

    # Cleanup
    await capturer.close()
    await encoder.close()
    await decoder.close()

    print("\n" + "=" * 60)


async def main():
    """Run all tests."""
    try:
        await test_encode_decode_pipeline()
        await test_live_decode()
    except KeyboardInterrupt:
        print("\n\nTest interrupted")
    except Exception as e:
        logger.exception(f"Test failed: {e}")


if __name__ == "__main__":
    asyncio.run(main())
