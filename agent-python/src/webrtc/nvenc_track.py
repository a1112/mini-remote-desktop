"""
NVENC video track for WebRTC.

Implements MediaStreamTrack that uses NVENC hardware encoding.
"""

import asyncio
import logging
import time
from typing import Optional

try:
    from aiortc import MediaStreamTrack
    from av import VideoFrame
    import av
    import numpy as np
    HAS_AIORTC = True
except ImportError:
    MediaStreamTrack = object
    VideoFrame = None
    HAS_AIORTC = False

from ..encoder.nvenc_encoder import NVENCEncoder, NVENCEncodedFrame, create_nvenc_encoder

logger = logging.getLogger(__name__)


class NVENCVideoTrack(MediaStreamTrack):
    """
    WebRTC video track using NVENC hardware encoding.

    This track receives raw BGRA frames, encodes them with NVENC,
    and passes the encoded H.264 data to WebRTC.
    """

    kind = "video"

    def __init__(
        self,
        d3d11_device,
        d3d11_context,
        width: int,
        height: int,
        fps: int = 60,
        quality: int = NVENCEncoder.QUALITY_HIGH
    ):
        """
        Initialize the NVENC video track.

        Args:
            d3d11_device: D3D11 device pointer
            d3d11_context: D3D11 context pointer
            width: Video width
            height: Video height
            fps: Target frame rate
            quality: QP value (18-51, lower is better)
        """
        super().__init__()
        self.width = width
        self.height = height
        self.fps = fps
        self.quality = quality

        # Create encoder
        self._encoder = create_nvenc_encoder(
            d3d11_device,
            d3d11_context,
            width,
            height,
            quality,
            fps
        )

        if not self._encoder:
            raise RuntimeError("Failed to create NVENC encoder")

        # Frame queue (raw BGRA frames)
        self._frame_queue: asyncio.Queue[Optional[bytes]] = asyncio.Queue(maxsize=10)

        # Encoded frame output (for direct RTP sending)
        self._encoded_queue: asyncio.Queue[Optional[NVENCEncodedFrame]] = asyncio.Queue(maxsize=30)

        # Timing
        self._pts = 0
        self._timestamp_base = int(time.time() * 90000)
        self._frame_duration = 90000 // max(1, fps)

        # Stats
        self._frames_encoded = 0
        self._frames_dropped = 0

        logger.info(f"NVENCVideoTrack created: {width}x{height} @ {fps}fps, QP={quality}")

    async def recv(self):
        """
        Receive the next frame for WebRTC transmission.

        Called by aiortc's RTP sender.

        Returns:
            VideoFrame for transmission
        """
        if not HAS_AIORTC:
            raise RuntimeError("aiortc not available")

        # Get raw frame from queue
        try:
            raw_frame = await asyncio.wait_for(
                self._frame_queue.get(),
                timeout=0.5
            )
        except asyncio.TimeoutError:
            # No frame available, return blank frame
            return self._create_blank_frame()

        if raw_frame is None:
            return self._create_blank_frame()

        # Encode with NVENC
        encoded = self._encoder.encode(raw_frame)
        if encoded:
            # Store for direct RTP access
            try:
                self._encoded_queue.put_nowait(encoded)
            except asyncio.QueueFull:
                self._frames_dropped += 1

            self._frames_encoded += 1

            # Create VideoFrame for aiortc
            # Decode the H.264 to get a VideoFrame
            return self._decode_to_videoframe(encoded.data)
        else:
            self._frames_dropped += 1
            return self._create_blank_frame()

    def _decode_to_videoframe(self, encoded_data: bytes) -> Optional[VideoFrame]:
        """Decode H.264 data to VideoFrame for aiortc."""
        try:
            codec = av.CodecContext.create("h264", "r")
            packet = av.Packet(encoded_data)
            frames = codec.decode(packet)

            if frames:
                frame = frames[0]
                frame.pts = self._pts
                frame.time_base = "1/90000"
                self._pts += self._frame_duration
                return frame

        except Exception as e:
            logger.debug(f"Decode error: {e}")

        return self._create_blank_frame()

    def _create_blank_frame(self) -> VideoFrame:
        """Create a blank frame."""
        if not HAS_AIORTC:
            return None

        arr = np.zeros((self.height, self.width, 3), dtype=np.uint8)
        frame = VideoFrame.from_ndarray(arr, format="rgb24")
        frame.pts = self._pts
        frame.time_base = "1/90000"
        self._pts += self._frame_duration
        return frame

    async def send_frame(self, frame_bgra: bytes) -> None:
        """
        Send a raw BGRA frame to the track.

        Args:
            frame_bgra: BGRA frame data (width * height * 4 bytes)
        """
        try:
            self._frame_queue.put_nowait(frame_bgra)
        except asyncio.QueueFull:
            # Drop oldest frame
            try:
                self._frame_queue.get_nowait()
                self._frame_queue.put_nowait(frame_bgra)
                self._frames_dropped += 1
            except asyncio.QueueEmpty:
                pass

    async def send_frame_numpy(self, frame_array) -> None:
        """
        Send a numpy array frame to the track.

        Args:
            frame_array: numpy array with shape (height, width, 4)
        """
        frame_contiguous = np.ascontiguousarray(frame_array)
        await self.send_frame(frame_contiguous.tobytes())

    async def get_encoded_frame(self) -> Optional[NVENCEncodedFrame]:
        """
        Get the most recent encoded frame for direct RTP sending.

        Returns:
            Encoded frame or None
        """
        try:
            return self._encoded_queue.get_nowait()
        except asyncio.QueueEmpty:
            return None

    def request_keyframe(self) -> None:
        """Request the next frame to be a keyframe."""
        if self._encoder:
            self._encoder.request_keyframe()

    def stop(self) -> None:
        """Stop the track and release resources."""
        if self._encoder:
            self._encoder.close()

        # Clear queues
        while not self._frame_queue.empty():
            try:
                self._frame_queue.get_nowait()
            except asyncio.QueueEmpty:
                break

        while not self._encoded_queue.empty():
            try:
                self._encoded_queue.get_nowait()
            except asyncio.QueueEmpty:
                break

        logger.info(f"NVENCVideoTrack stopped: {self._frames_encoded} frames encoded, "
                   f"{self._frames_dropped} dropped")

    @property
    def stats(self) -> dict:
        """Get encoding statistics."""
        return {
            "frames_encoded": self._frames_encoded,
            "frames_dropped": self._frames_dropped,
            "width": self.width,
            "height": self.height,
            "fps": self.fps,
            "quality": self.quality,
        }
