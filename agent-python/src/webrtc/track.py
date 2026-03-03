"""
H.264 video track for WebRTC.

Implements MediaStreamTrack for sending H.264 encoded video.
"""

import asyncio
import logging
import time
from typing import Optional

try:
    from aiortc import MediaStreamTrack
    from av import VideoFrame
    HAS_AIORTC = True
except ImportError:
    MediaStreamTrack = object
    VideoFrame = None
    HAS_AIORTC = False

logger = logging.getLogger(__name__)


class H264VideoTrack(MediaStreamTrack):
    """
    H.264 video track for aiortc.

    Receives encoded frames and converts them to VideoFrame
    for transmission via WebRTC.
    """

    kind = "video"

    def __init__(self, fps: int = 30):
        """
        Initialize the video track.

        Args:
            fps: Target frame rate
        """
        super().__init__()
        self.fps = fps
        self._frame_queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=30)
        self._running = False
        self._pts_counter = 0
        self._timestamp_base = int(time.time() * 90000)  # 90kHz clock
        self._frame_duration = 90000 // max(1, fps)  # in 90kHz units

        logger.debug(f"H264VideoTrack initialized: {fps}fps")

    async def recv(self):
        """
        Receive the next frame for transmission.

        This method is called by aiortc's RTP sender.

        Returns:
            VideoFrame for transmission
        """
        if VideoFrame is None:
            raise RuntimeError("aiortc not available")

        # Wait for next encoded frame
        try:
            encoded_data = await asyncio.wait_for(
                self._frame_queue.get(),
                timeout=1.0,
            )
        except asyncio.TimeoutError:
            # No frame available, return a blank frame
            logger.debug("No frame available, returning blank frame")
            return self._create_blank_frame()

        # Create VideoFrame from encoded H.264
        # For aiortc with manual packetizer, we can pass encoded data directly
        # But for MediaStreamTrack, we need to decode or wrap appropriately

        # For H.264 streaming, we can create a VideoFrame that wraps the encoded data
        frame = VideoFrame(width=1920, height=1080)
        frame.pts = self._pts_counter
        frame.time_base = "1/90000"

        # Store encoded data for the packetizer
        frame._encoded_data = encoded_data  # type: ignore

        self._pts_counter += self._frame_duration

        return frame

    def _create_blank_frame(self):
        """Create a blank/placeholder frame."""
        if VideoFrame is None:
            return None

        frame = VideoFrame(width=1920, height=1080)
        frame.pts = self._pts_counter
        frame.time_base = "1/90000"
        self._pts_counter += self._frame_duration
        return frame

    async def send_frame(self, encoded_data: bytes) -> None:
        """
        Send an encoded frame to the track.

        Args:
            encoded_data: H.264 encoded frame data (Annex B format)
        """
        try:
            # Non-blocking put - drop frame if queue is full
            self._frame_queue.put_nowait(encoded_data)
        except asyncio.QueueFull:
            # Drop oldest frame
            try:
                self._frame_queue.get_nowait()
                self._frame_queue.put_nowait(encoded_data)
            except asyncio.QueueEmpty:
                pass

    def stop(self) -> None:
        """Stop the track."""
        self._running = False
        while not self._frame_queue.empty():
            try:
                self._frame_queue.get_nowait()
            except asyncio.QueueEmpty:
                break
        logger.debug("H264VideoTrack stopped")


class H264TrackProxy(MediaStreamTrack):
    """
    Alternative H.264 track that uses aiortc's built-in H.264 support.

    This track passes encoded H.264 frames directly to the RTP sender,
    allowing aiortc's H.264 packetizer to handle the RTP packaging.
    """

    kind = "video"

    def __init__(self, width: int = 1920, height: int = 1080, fps: int = 30):
        """
        Initialize the video track.

        Args:
            width: Video width
            height: Video height
            fps: Target frame rate
        """
        super().__init__()
        self.width = width
        self.height = height
        self.fps = fps
        self._encoded_frames: asyncio.Queue[Optional[bytes]] = asyncio.Queue(maxsize=30)
        self._pts = 0
        self._started = False

        logger.debug(f"H264TrackProxy initialized: {width}x{height} @ {fps}fps")

    async def recv(self):
        """
        Receive the next frame.

        Returns:
            VideoFrame for transmission
        """
        if VideoFrame is None:
            raise RuntimeError("aiortc not available")

        if not self._started:
            self._started = True

        # Get encoded frame
        try:
            encoded_data = await asyncio.wait_for(
                self._encoded_frames.get(),
                timeout=1.0,
            )
        except asyncio.TimeoutError:
            # Return blank frame
            encoded_data = None

        if encoded_data is None:
            return self._create_blank_frame()

        # Create a VideoFrame from encoded data
        # We'll use a special approach - decode the frame and re-encode
        # This is inefficient but works with aiortc's MediaStreamTrack
        try:
            import av
            import numpy as np

            # Decode the H.264 frame
            codec = av.CodecContext.create("h264", "r")
            packet = av.Packet(encoded_data)
            frames = codec.decode(packet)

            if frames:
                frame = frames[0]
                frame.pts = self._pts
                frame.time_base = "1/90000"
                self._pts += 90000 // max(1, self.fps)
                return frame

        except Exception as e:
            logger.debug(f"Decode error: {e}")

        return self._create_blank_frame()

    def _create_blank_frame(self):
        """Create a blank frame."""
        if VideoFrame is None:
            return None

        import numpy as np

        # Create a black frame
        arr = np.zeros((self.height, self.width, 3), dtype=np.uint8)
        frame = VideoFrame.from_ndarray(arr, format="rgb24")
        frame.pts = self._pts
        frame.time_base = "1/90000"
        self._pts += 90000 // max(1, self.fps)
        return frame

    async def send_encoded(self, encoded_data: bytes) -> None:
        """
        Send an encoded H.264 frame.

        Args:
            encoded_data: H.264 encoded frame data
        """
        try:
            self._encoded_frames.put_nowait(encoded_data)
        except asyncio.QueueFull:
            # Drop oldest frame
            try:
                self._encoded_frames.get_nowait()
                self._encoded_frames.put_nowait(encoded_data)
            except asyncio.QueueEmpty:
                pass

    def stop(self) -> None:
        """Stop the track."""
        while not self._encoded_frames.empty():
            try:
                self._encoded_frames.get_nowait()
            except asyncio.QueueEmpty:
                break
        logger.debug("H264TrackProxy stopped")
