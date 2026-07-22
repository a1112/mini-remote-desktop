"""
H.264 video encoder using PyAV.

Supports hardware acceleration when available.
"""

import asyncio
import io
import logging
import time
from dataclasses import dataclass
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class EncodedFrame:
    """An encoded H.264 frame."""

    data: bytes  # H.264 NAL units (Annex B format)
    is_keyframe: bool
    timestamp: float
    pts: int
    dts: int

    def __post_init__(self):
        if self.timestamp == 0.0:
            self.timestamp = time.time()


class PyAVEncoder:
    """
    H.264 encoder using PyAV (ffmpeg bindings).

    Uses container-based encoding for better compatibility.
    """

    def __init__(
        self,
        width: int,
        height: int,
        fps: int = 30,
        bitrate_kbps: int = 5000,
        gop_size: int = 60,
        hardware_accel: bool = False,
        preset: str = "ultrafast",
        tune: str = "zerolatency",
    ):
        """
        Initialize the encoder.

        Args:
            width: Frame width
            height: Frame height
            fps: Target frame rate
            bitrate_kbps: Target bitrate in kbps
            gop_size: GOP size (keyframe interval)
            hardware_accel: Enable hardware acceleration
            preset: Encoding preset
            tune: Encoding tuning
        """
        self.width = width
        self.height = height
        self.fps = fps
        self.bitrate = bitrate_kbps * 1000  # Convert to bps
        self.gop_size = gop_size
        self.hardware_accel = hardware_accel
        self.preset = preset
        self.tune = tune

        self._output = None
        self._container = None
        self._stream = None
        self._pts_counter = 0
        self._frame_duration = 1.0 / max(1, fps)
        self._last_keyframe_pts = -gop_size

    async def initialize(self) -> bool:
        """
        Initialize the encoder.

        Returns:
            True if successful, False otherwise
        """
        try:
            import av

            # Create in-memory output container
            self._output = io.BytesIO()
            self._container = av.open(self._output, 'w', format='h264')

            # Add video stream
            codec_name = "libx264"
            if self.hardware_accel:
                # Try hardware encoders in order of preference
                # h264_mf is fastest on Windows (126 FPS), then NVENC, then QSV
                for hw_name in ["h264_mf", "h264_nvenc", "h264_qsv", "h264_amf"]:
                    try:
                        self._stream = self._container.add_stream(hw_name, rate=self.fps)
                        codec_name = hw_name
                        logger.info(f"✅ Using hardware encoder: {hw_name}")
                        break
                    except Exception as e:
                        logger.debug(f"Hardware encoder {hw_name} not available: {e}")
                        continue

            if self._stream is None:
                self._stream = self._container.add_stream(codec_name, rate=self.fps)
                logger.info(f"Using software encoder: {codec_name}")

            # Configure stream
            self._stream.width = self.width
            self._stream.height = self.height
            self._stream.bit_rate = self.bitrate

            logger.info(
                f"PyAV encoder initialized: {self.width}x{self.height} "
                f"@ {self.fps}fps, {self.bitrate_kbps}kbps"
            )
            return True

        except ImportError:
            logger.error("PyAV not available. Install with: pip install av")
            return False
        except Exception as e:
            logger.error(f"Failed to initialize encoder: {e}")
            return False

    async def encode(
        self,
        frame_data: bytes,
        width: int,
        height: int,
        format: str = "RGB",
    ) -> Optional[EncodedFrame]:
        """
        Encode a frame.

        Args:
            frame_data: Raw frame data
            width: Frame width
            height: Frame height
            format: Pixel format (RGB, BGR, etc.)

        Returns:
            EncodedFrame or None if encoding failed
        """
        if self._stream is None:
            logger.warning("Encoder not initialized")
            return None

        try:
            import av
            import numpy as np

            # Convert frame data to numpy array
            arr = np.frombuffer(frame_data, dtype=np.uint8)

            # Reshape based on format
            if format in ("BGR", "RGB"):
                arr = arr.reshape((height, width, 3))
            elif format in ("BGRA", "RGBA"):
                arr = arr.reshape((height, width, 4))
            else:
                logger.error(f"Unsupported pixel format: {format}")
                return None

            # Convert to RGB if needed
            if format == "BGR":
                arr = arr[:, :, [2, 1, 0]]

            # Create av.VideoFrame
            frame = av.VideoFrame.from_ndarray(arr, format="rgb24")
            frame.pts = self._pts_counter

            # Encode frame
            loop = asyncio.get_event_loop()
            result = await loop.run_in_executor(
                None, self._encode_sync, frame
            )

            self._pts_counter += 1
            return result

        except Exception as e:
            logger.error(f"Encoding error: {e}")
            return None

    def _encode_sync(self, frame) -> Optional[EncodedFrame]:
        """
        Synchronous encoding (run in thread pool).
        """
        try:
            # Get current output position
            start_pos = self._output.tell()

            # Encode and mux the frame
            for packet in self._stream.encode(frame):
                self._container.mux(packet)

            # Check if new data was written
            end_pos = self._output.tell()
            if end_pos > start_pos:
                # Read the newly written data
                self._output.seek(start_pos)
                data = self._output.read(end_pos - start_pos)
                self._output.seek(end_pos)  # Reset for next write

                # Check if it's a keyframe (simple heuristic: starts with SPS)
                is_keyframe = data.startswith(b'\x00\x00\x00\x01\x67') or data.startswith(b'\x00\x00\x01\x67')

                return EncodedFrame(
                    data=data,
                    is_keyframe=is_keyframe,
                    timestamp=time.time(),
                    pts=self._pts_counter,
                    dts=self._pts_counter,
                )

            return None

        except Exception as e:
            logger.error(f"Synchronous encoding error: {e}")
            return None

    async def request_keyframe(self) -> bool:
        """
        Request a keyframe (IDR frame).

        Returns:
            True if successful
        """
        # Force flush and get new data which should contain a keyframe
        if self._stream:
            try:
                start_pos = self._output.tell()
                for packet in self._stream.encode():
                    self._container.mux(packet)
                end_pos = self._output.tell()
                return True
            except Exception as e:
                logger.debug(f"Request keyframe error: {e}")
        return False

    async def flush(self) -> Optional[EncodedFrame]:
        """
        Flush encoder to get remaining frames.

        Returns:
            EncodedFrame or None
        """
        if self._stream is None:
            return None

        try:
            start_pos = self._output.tell()
            for packet in self._stream.encode():
                self._container.mux(packet)

            end_pos = self._output.tell()
            if end_pos > start_pos:
                self._output.seek(start_pos)
                data = self._output.read(end_pos - start_pos)
                self._output.seek(end_pos)

                return EncodedFrame(
                    data=data,
                    is_keyframe=True,  # Flushed data is usually keyframe
                    timestamp=time.time(),
                    pts=self._pts_counter,
                    dts=self._pts_counter,
                )
            return None

        except Exception as e:
            logger.error(f"Flush error: {e}")
            return None

    async def close(self) -> None:
        """Clean up encoder resources."""
        if self._container:
            try:
                # Flush any remaining data
                for packet in self._stream.encode():
                    self._container.mux(packet)
                self._container.close()
            except Exception:
                pass
            self._container = None
            self._stream = None
            self._output = None

        logger.info("PyAV encoder closed")

    @property
    def bitrate_kbps(self) -> int:
        """Get current bitrate in kbps."""
        return self.bitrate // 1000
