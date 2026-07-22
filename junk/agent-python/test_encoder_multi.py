#!/usr/bin/env python3
"""Test encoder with multiple frames."""
import sys
import asyncio
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent / "src"))


async def test_encoder_multiple_frames():
    """Test encoder with multiple frames."""
    import av
    import numpy as np
    from encoder.pyav_encoder import PyAVEncoder

    print("Creating encoder...")
    encoder = PyAVEncoder(
        width=640,
        height=480,
        fps=30,
        bitrate_kbps=1000,
        gop_size=30,
    )

    if not await encoder.initialize():
        print("Failed to initialize encoder")
        return

    print("Encoding frames...")

    # Encode several frames
    for i in range(10):
        # Create a test frame with a gradient
        frame_data = np.zeros((480, 640, 3), dtype=np.uint8)
        frame_data[:, :] = [i * 25, i * 25, i * 25]  # Gradient

        encoded = await encoder.encode(frame_data.tobytes(), 640, 480, "RGB")
        if encoded:
            print(f"  Frame {i}: {len(encoded.data)} bytes, keyframe={encoded.is_keyframe}")
        else:
            print(f"  Frame {i}: No output")

    await encoder.close()
    print("Test complete!")


if __name__ == "__main__":
    asyncio.run(test_encoder_multiple_frames())
