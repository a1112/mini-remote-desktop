#!/usr/bin/env python3
"""Test encoder with flush."""
import sys
import asyncio
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent / "src"))


async def test_encoder_with_flush():
    """Test encoder with multiple frames and flush."""
    import numpy as np
    from encoder.pyav_encoder import PyAVEncoder

    print("Creating encoder...")
    encoder = PyAVEncoder(
        width=640,
        height=480,
        fps=30,
        bitrate_kbps=1000,
    )

    if not await encoder.initialize():
        print("Failed to initialize encoder")
        return

    print("Encoding 30 frames...")

    # Encode many frames
    for i in range(30):
        frame_data = np.ones((480, 640, 3), dtype=np.uint8) * (i * 8)
        encoded = await encoder.encode(frame_data.tobytes(), 640, 480, "RGB")
        if encoded:
            print(f"  Frame {i}: {len(encoded.data)} bytes, keyframe={encoded.is_keyframe}")

    # Flush to get remaining data
    print("Flushing...")
    flushed = await encoder.flush()
    if flushed:
        print(f"  Flush: {len(flushed.data)} bytes, keyframe={flushed.is_keyframe}")

    await encoder.close()
    print("Test complete!")


if __name__ == "__main__":
    asyncio.run(test_encoder_with_flush())
