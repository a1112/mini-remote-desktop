"""
Simple H.264 encoder using PyAV CodecContext directly.

This is simpler than the container-based approach and works better
for real-time streaming.
"""

import logging
import time
import numpy as np
from dataclasses import dataclass
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class SimpleEncodedFrame:
    """An encoded H.264 frame."""
    data: bytes
    is_keyframe: bool
    timestamp: float
    pts: int


class SimpleH264Encoder:
    """
    Simple H.264 encoder using PyAV CodecContext.

    Direct encoding without container overhead.
    """

    def __init__(
        self,
        width: int = 1920,
        height: int = 1080,
        fps: int = 30,
        bitrate_kbps: int = 5000,
    ):
        self.width = width
        self.height = height
        self.fps = fps
        self.bitrate = bitrate_kbps * 1000
        self._codec = None
        self._pts = 0

    async def initialize(self) -> bool:
        """Initialize the encoder."""
        try:
            import av

            # Create codec context
            self._codec = av.CodecContext.create('libx264', 'w')

            # Just open with minimal config
            self._codec.open()

            # Now set the properties
            self._codec.width = self.width
            self._codec.height = self.height

            logger.info(
                f"Simple H.264 encoder: {width}x{height} @ {fps}fps, {bitrate_kbps}kbps"
            )
            return True

        except Exception as e:
            logger.error(f"Encoder init failed: {e}")
            return False

    async def encode(
        self,
        frame_data: bytes,
        width: int,
        height: int,
        format: str = "RGB",
    ) -> Optional[SimpleEncodedFrame]:
        """Encode a frame."""
        if self._codec is None:
            return None

        try:
            import av
            import asyncio

            # Convert to numpy
            arr = np.frombuffer(frame_data, dtype=np.uint8)
            arr = arr.reshape((height, width, 3))

            # Create video frame
            frame = av.VideoFrame.from_ndarray(arr, format='rgb24')
            frame.pts = self._pts
            # time_base will be set by codec
            self._pts += 1

            # Encode in thread pool
            loop = asyncio.get_event_loop()
            result = await loop.run_in_executor(None, self._encode_sync, frame)

            return result

        except Exception as e:
            logger.error(f"Encode error: {e}")
            return None

    def _encode_sync(self, frame) -> Optional[SimpleEncodedFrame]:
        """Synchronous encode."""
        try:
            for packet in self._codec.encode(frame):
                # Check if keyframe
                is_keyframe = packet.is_keyframe

                return SimpleEncodedFrame(
                    data=packet.to_bytes(),
                    is_keyframe=is_keyframe,
                    timestamp=time.time(),
                    pts=frame.pts,
                )

            # No packet output (buffering)
            return None

        except Exception as e:
            logger.error(f"Sync encode error: {e}")
            return None

    async def flush(self) -> Optional[SimpleEncodedFrame]:
        """Flush encoder."""
        if self._codec is None:
            return None

        try:
            import asyncio

            loop = asyncio.get_event_loop()
            return await loop.run_in_executor(None, self._flush_sync)

        except Exception as e:
            logger.error(f"Flush error: {e}")
            return None

    def _flush_sync(self) -> Optional[SimpleEncodedFrame]:
        """Synchronous flush."""
        try:
            for packet in self._codec.encode():
                return SimpleEncodedFrame(
                    data=packet.to_bytes(),
                    is_keyframe=packet.is_keyframe,
                    timestamp=time.time(),
                    pts=self._pts - 1,
                )
            return None
        except Exception as e:
            logger.error(f"Sync flush error: {e}")
            return None

    async def close(self) -> None:
        """Close encoder."""
        if self._codec:
            try:
                self._codec.close()
            except Exception:
                pass
            self._codec = None
        logger.info("Simple H.264 encoder closed")
