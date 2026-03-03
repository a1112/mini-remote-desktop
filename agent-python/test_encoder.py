#!/usr/bin/env python3
"""Test encoder with detailed debugging."""
import sys
import asyncio
from pathlib import Path
sys.path.insert(0, str(Path(__file__).parent / "src"))


async def test_encoder_debug():
    """Test encoder with debug output."""
    import av
    import logging

    logging.basicConfig(level=logging.DEBUG)

    print("Creating encoder...")
    encoder = av.CodecContext.create("libx264", "w")
    print(f"Encoder created: {encoder}")

    # Set basic properties
    encoder.width = 640
    encoder.height = 480
    print(f"Set dimensions: {encoder.width}x{encoder.height}")

    # Try setting framerate
    try:
        encoder.framerate = (30, 1)
        print(f"Set framerate: {encoder.framerate}")
    except Exception as e:
        print(f"Error setting framerate: {e}")

    # Try setting time_base
    try:
        encoder.time_base = (1, 90000)
        print(f"Set time_base: {encoder.time_base}")
    except Exception as e:
        print(f"Error setting time_base: {e}")

    # Set bitrate
    encoder.bit_rate = 1000000
    print(f"Set bitrate: {encoder.bit_rate}")

    # Set GOP size
    encoder.gop_size = 30
    print(f"Set GOP size: {encoder.gop_size}")

    # Try opening
    try:
        encoder.open()
        print("Encoder opened successfully!")
    except Exception as e:
        print(f"Error opening encoder: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    asyncio.run(test_encoder_debug())
