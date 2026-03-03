"""
H.264 decoder using PyAV.

Decodes H.264 encoded frames back to RGB/RGBA for:
- Testing and verification
- Performance benchmarking
- Debugging encoding issues
"""

import asyncio
import logging
import io
import time
from dataclasses import dataclass
from typing import Optional, List

import numpy as np

logger = logging.getLogger(__name__)


@dataclass
class DecodedFrame:
    """A decoded video frame."""
    data: np.ndarray  # RGB or RGBA image data (height, width, channels)
    width: int
    height: int
    format: str  # 'rgb24', 'rgba', 'yuv420p', etc.
    timestamp: float
    keyframe: bool = False

    @property
    def shape(self) -> tuple:
        """Get frame shape."""
        return self.data.shape

    def to_bytes(self) -> bytes:
        """Convert frame to raw bytes."""
        return self.data.tobytes()

    def save(self, path: str) -> None:
        """Save frame as image file."""
        from PIL import Image
        Image.fromarray(self.data).save(path)


class PyAVDecoder:
    """
    H.264 decoder using PyAV.

    Can decode from:
    - H.264 byte stream (Annex B format)
    - PyAV packets
    - Container input (MP4, MKV, etc.)
    """

    def __init__(self, thread_count: int = 1):
        """
        Initialize the decoder.

        Args:
            thread_count: Number of decoder threads (default 1 for low latency)
        """
        self.thread_count = thread_count
        self._codec_context = None
        self._width = 0
        self._height = 0
        self._frame_count = 0
        self._keyframe_count = 0
        self._decode_errors = 0
        self._last_keyframe_data = None

    async def initialize(
        self,
        width: int = 1920,
        height: int = 1080,
        codec: str = "h264"
    ) -> bool:
        """
        Initialize the decoder.

        Args:
            width: Expected frame width
            height: Expected frame height
            codec: Codec name (default: h264)

        Returns:
            True if initialization succeeded
        """
        try:
            import av

            self._width = width
            self._height = height

            # Create decoder
            self._codec_context = av.CodecContext.create(codec, 'r')
            self._codec_context.thread_count = self.thread_count

            # For low latency decoding
            try:
                self._codec_context.options = {
                    'flags': 'low_delay',
                    'flags2': 'fast',
                }
            except Exception:
                pass  # Options might not be supported

            logger.info(f"PyAV decoder initialized: {width}x{height}")
            return True

        except ImportError:
            logger.error("PyAV not available")
            return False
        except Exception as e:
            logger.error(f"Failed to initialize decoder: {e}")
            return False

    async def decode(
        self,
        encoded_data: bytes,
        timestamp: float = 0.0
    ) -> Optional[DecodedFrame]:
        """
        Decode H.264 encoded data.

        Args:
            encoded_data: H.264 encoded bytes (Annex B format with start codes)
            timestamp: Frame timestamp

        Returns:
            DecodedFrame or None if decode failed
        """
        if self._codec_context is None:
            return None

        try:
            # Decode in thread pool
            loop = asyncio.get_event_loop()
            frame = await loop.run_in_executor(
                None,
                self._decode_sync,
                encoded_data,
                timestamp
            )

            if frame:
                self._frame_count += 1
                if frame.keyframe:
                    self._keyframe_count += 1

            return frame

        except Exception as e:
            logger.error(f"Decode error: {e}")
            self._decode_errors += 1
            return None

    def _decode_sync(
        self,
        encoded_data: bytes,
        timestamp: float
    ) -> Optional[DecodedFrame]:
        """Synchronous decode operation."""
        try:
            import av

            # Parse H.264 NALUs from byte stream
            packet = av.Packet(encoded_data)

            # Decode
            frames = self._codec_context.decode(packet)

            # Get first decoded frame
            for frame in frames:
                # Convert to RGB
                img = frame.to_ndarray(format='rgb24')

                return DecodedFrame(
                    data=img,
                    width=frame.width,
                    height=frame.height,
                    format='rgb24',
                    timestamp=timestamp,
                    keyframe=frame.key_frame
                )

            return None

        except Exception as e:
            logger.debug(f"Sync decode error: {e}")
            return None

    async def decode_all(self, encoded_data: bytes) -> List[DecodedFrame]:
        """
        Decode all frames in encoded data.

        Useful for testing with pre-encoded files.

        Args:
            encoded_data: H.264 encoded bytes

        Returns:
            List of decoded frames
        """
        if self._codec_context is None:
            return []

        try:
            loop = asyncio.get_event_loop()
            frames = await loop.run_in_executor(
                None,
                self._decode_all_sync,
                encoded_data
            )
            return frames

        except Exception as e:
            logger.error(f"Decode all error: {e}")
            return []

    def _decode_all_sync(self, encoded_data: bytes) -> List[DecodedFrame]:
        """Decode all frames synchronously."""
        try:
            import av

            packet = av.Packet(encoded_data)
            frames = []

            for frame in self._codec_context.decode(packet):
                img = frame.to_ndarray(format='rgb24')
                frames.append(DecodedFrame(
                    data=img,
                    width=frame.width,
                    height=frame.height,
                    format='rgb24',
                    timestamp=time.time(),
                    keyframe=frame.key_frame
                ))

            return frames

        except Exception as e:
            logger.debug(f"Decode all sync error: {e}")
            return []

    async def decode_file(
        self,
        file_path: str,
        max_frames: int = 0
    ) -> List[DecodedFrame]:
        """
        Decode all frames from a file.

        Args:
            file_path: Path to H.264 file (raw or container)
            max_frames: Maximum frames to decode (0 = all)

        Returns:
            List of decoded frames
        """
        try:
            import av

            frames = []

            with av.open(file_path) as container:
                # Find video stream
                video_stream = None
                for stream in container.streams:
                    if stream.type == 'video':
                        video_stream = stream
                        break

                if video_stream is None:
                    logger.error(f"No video stream in {file_path}")
                    return []

                self._width = video_stream.width
                self._height = video_stream.height

                # Decode frames
                for packet in container.demux(video_stream):
                    for frame in video_stream.decode(packet):
                        img = frame.to_ndarray(format='rgb24')
                        frames.append(DecodedFrame(
                            data=img,
                            width=frame.width,
                            height=frame.height,
                            format='rgb24',
                            timestamp=frame.time,
                            keyframe=frame.key_frame
                        ))

                        if max_frames > 0 and len(frames) >= max_frames:
                            break

                    if max_frames > 0 and len(frames) >= max_frames:
                        break

            logger.info(f"Decoded {len(frames)} frames from {file_path}")
            return frames

        except Exception as e:
            logger.error(f"Decode file error: {e}")
            return []

    def get_stats(self) -> dict:
        """Get decoder statistics."""
        return {
            'frame_count': self._frame_count,
            'keyframe_count': self._keyframe_count,
            'decode_errors': self._decode_errors,
            'width': self._width,
            'height': self._height,
        }

    def reset_stats(self) -> None:
        """Reset decoder statistics."""
        self._frame_count = 0
        self._keyframe_count = 0
        self._decode_errors = 0

    async def close(self) -> None:
        """Close the decoder and release resources."""
        if self._codec_context:
            try:
                self._codec_context.close()
            except Exception:
                pass
            self._codec_context = None

        logger.info("PyAV decoder closed")

    @property
    def width(self) -> int:
        """Get decoder width."""
        return self._width

    @property
    def height(self) -> int:
        """Get decoder height."""
        return self._height


def create_decoder(
    width: int = 1920,
    height: int = 1080,
    thread_count: int = 1
) -> PyAVDecoder:
    """
    Create a configured H.264 decoder.

    Args:
        width: Expected frame width
        height: Expected frame height
        thread_count: Number of decoder threads

    Returns:
        Configured PyAVDecoder
    """
    decoder = PyAVDecoder(thread_count=thread_count)
    # Note: needs async initialize
    return decoder


class StreamDecoder:
    """
    Streaming decoder for continuous decoding.

    Maintains decoder state across multiple frames.
    """

    def __init__(
        self,
        width: int = 1920,
        height: int = 1080,
        output_callback=None
    ):
        """
        Initialize stream decoder.

        Args:
            width: Frame width
            height: Frame height
            output_callback: Optional callback for decoded frames
        """
        self.width = width
        self.height = height
        self.output_callback = output_callback
        self._decoder = None
        self._running = False
        self._frame_queue = asyncio.Queue(maxsize=30)

    async def initialize(self) -> bool:
        """Initialize the decoder."""
        self._decoder = PyAVDecoder()
        return await self._decoder.initialize(self.width, self.height)

    async def start(self) -> None:
        """Start the decoder loop."""
        if self._running:
            return

        self._running = True
        asyncio.create_task(self._decode_loop())

    async def _decode_loop(self) -> None:
        """Main decode loop."""
        while self._running:
            try:
                # Get encoded data with timeout
                encoded_data, timestamp = await asyncio.wait_for(
                    self._frame_queue.get(),
                    timeout=1.0
                )

                # Decode
                frame = await self._decoder.decode(encoded_data, timestamp)

                if frame and self.output_callback:
                    await self.output_callback(frame)

            except asyncio.TimeoutError:
                continue
            except Exception as e:
                logger.error(f"Decode loop error: {e}")

    async def feed(self, encoded_data: bytes, timestamp: float = 0.0) -> None:
        """
        Feed encoded data to the decoder.

        Args:
            encoded_data: H.264 encoded bytes
            timestamp: Frame timestamp
        """
        try:
            await asyncio.wait_for(
                self._frame_queue.put((encoded_data, timestamp)),
                timeout=0.1
            )
        except asyncio.TimeoutError:
            logger.warning("Decoder frame queue full, dropping packet")

    async def stop(self) -> None:
        """Stop the decoder."""
        self._running = False
        if self._decoder:
            await self._decoder.close()

    def get_stats(self) -> dict:
        """Get decoder stats."""
        if self._decoder:
            return self._decoder.get_stats()
        return {}
