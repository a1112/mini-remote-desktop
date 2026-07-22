"""
Hardware-accelerated H.264 decoder using DXVA2/D3D11VA.

Supports Windows hardware acceleration via:
- DXVA2 (Windows 7+)
- D3D11VA (Windows 8+)
- NVIDIA CUVID/NVDEC (via PyAV)
- Intel Quick Sync (via PyAV)
"""

import asyncio
import logging
import threading
import time
from dataclasses import dataclass
from typing import Optional, List, Callable

import numpy as np
import numpy.typing as npt

logger = logging.getLogger(__name__)


@dataclass
class HWDecoderConfig:
    """Hardware decoder configuration."""
    width: int = 1920
    height: int = 1080
    codec: str = "h264"
    prefer_nvidia: bool = True
    prefer_intel: bool = False
    prefer_d3d11va: bool = True
    low_delay: bool = True
    thread_count: int = 1
    decoder_priority: Optional[List[str]] = None


@dataclass
class DecodedFrame:
    """Decoded video frame."""
    data: npt.NDArray[np.uint8]
    width: int
    height: int
    timestamp: float
    keyframe: bool = False
    decoder_type: str = ""


class HWDecoder:
    """
    Hardware-accelerated video decoder.

    Automatically selects the best available hardware decoder:
    1. NVIDIA NVDEC (via PyAV with h264_nvdec)
    2. Intel QSV (via PyAV with h264_qsv)
    3. D3D11VA (via PyAV with h264_d3d11va)
    4. DXVA2 (via PyAV with h264_dxva2)
    5. Software fallback (h264)
    """

    # Available decoder names in priority order
    DECODER_PRIORITY = [
        "h264_nvdec",    # NVIDIA NVDEC
        "h264_qsv",      # Intel Quick Sync
        "h264_d3d11va",  # D3D11 Video Acceleration
        "h264_dxva2",    # DXVA2 (Windows 7+)
        "h264",          # Software fallback
    ]

    def __init__(self, config: HWDecoderConfig = None):
        """Initialize hardware decoder."""
        self.config = config or HWDecoderConfig()
        self._codec_context = None
        self._decoder_name = ""
        self._width = 0
        self._height = 0
        self._frame_count = 0
        self._decode_errors = 0
        self._available_decoders = []
        self._initialized = False

    @classmethod
    def get_available_decoders(cls) -> List[str]:
        """Get list of available hardware decoders."""
        available = []
        try:
            import av
            for name in cls.DECODER_PRIORITY:
                try:
                    codec = av.CodecContext.create(name, 'r')
                    codec.close()
                    available.append(name)
                except Exception:
                    pass
        except ImportError:
            pass
        return available

    async def initialize(self) -> bool:
        """
        Initialize the hardware decoder.

        Returns:
            True if successful
        """
        try:
            import av

            self._available_decoders = self.get_available_decoders()

            if not self._available_decoders:
                logger.error("No decoders available (PyAV not installed or no codecs found)")
                return False

            self._width = self.config.width
            self._height = self.config.height

            ordered = self._resolve_decoder_order(self._available_decoders)

            # Try decoders in priority order
            for decoder_name in ordered:
                try:
                    self._codec_context = self._create_codec_context(av, decoder_name)
                    if self._codec_context is None:
                        continue

                    # Try to use hardware decoder
                    self._codec_context.thread_count = self.config.thread_count

                    # Enable low delay mode
                    if self.config.low_delay:
                        try:
                            self._codec_context.options = {
                                'flags': 'low_delay',
                                'flags2': 'fast',
                            }
                        except Exception:
                            pass

                    self._decoder_name = decoder_name
                    self._initialized = True

                    logger.info(
                        f"HW decoder initialized: {decoder_name} @ "
                        f"{self._width}x{self._height}"
                    )
                    return True

                except Exception as e:
                    logger.debug(f"Failed to init {decoder_name}: {e}")
                    if self._codec_context:
                        try:
                            self._codec_context.close()
                        except Exception:
                            pass
                    self._codec_context = None

            logger.error("Failed to initialize any decoder")
            return False

        except ImportError:
            logger.error("PyAV not available")
            return False
        except Exception as e:
            logger.error(f"Decoder initialization failed: {e}")
            return False

    def _resolve_decoder_order(self, available: List[str]) -> List[str]:
        custom = self.config.decoder_priority or []
        if not custom:
            return list(available)
        custom_norm = [v.strip().lower() for v in custom if v and v.strip()]
        seen = set()
        ordered: List[str] = []
        for name in custom_norm:
            if name in available and name not in seen:
                ordered.append(name)
                seen.add(name)
        for name in available:
            if name not in seen:
                ordered.append(name)
        return ordered

    def _create_codec_context(self, av_module, decoder_name: str):
        # Prefer explicit decoder names when available (e.g. h264_d3d11va/h264_dxva2).
        try:
            return av_module.CodecContext.create(decoder_name, "r")
        except Exception:
            pass

        # Fallback: regular h264 decoder with hardware hint.
        hwaccel = None
        if decoder_name.endswith("_d3d11va"):
            hwaccel = "d3d11va"
        elif decoder_name.endswith("_dxva2"):
            hwaccel = "dxva2"
        elif decoder_name.endswith("_qsv"):
            hwaccel = "qsv"
        elif decoder_name.endswith("_nvdec"):
            hwaccel = "cuda"

        options = {"hwaccel": hwaccel} if hwaccel else {}
        try:
            return av_module.CodecContext.create(self.config.codec, "r", options=options)
        except Exception:
            return None

    async def decode(
        self,
        encoded_data: bytes,
        timestamp: float = 0.0
    ) -> Optional[DecodedFrame]:
        """
        Decode H.264 encoded data.

        Args:
            encoded_data: H.264 encoded bytes
            timestamp: Frame timestamp

        Returns:
            DecodedFrame or None if decode failed
        """
        if not self._initialized or self._codec_context is None:
            return None

        try:
            # Decode in thread pool to avoid blocking
            loop = asyncio.get_event_loop()
            frame = await loop.run_in_executor(
                None,
                self._decode_sync,
                encoded_data,
                timestamp
            )

            if frame:
                self._frame_count += 1

            return frame

        except Exception as e:
            logger.debug(f"Decode error: {e}")
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

            # Create packet from encoded data
            packet = av.Packet(encoded_data)

            # Decode
            frames = self._codec_context.decode(packet)

            # Get first decoded frame
            for frame in frames:
                # Get frame data as numpy array
                img = frame.to_ndarray(format='rgb24')

                return DecodedFrame(
                    data=img,
                    width=frame.width,
                    height=frame.height,
                    timestamp=timestamp,
                    keyframe=frame.key_frame,
                    decoder_type=self._decoder_name
                )

            return None

        except Exception as e:
            logger.debug(f"Sync decode error: {e}")
            return None

    def decode_packet(self, packet) -> List[DecodedFrame]:
        """
        Decode an av.Packet directly.

        Args:
            packet: PyAV Packet object

        Returns:
            List of decoded frames
        """
        frames = []
        try:
            import av

            for frame in self._codec_context.decode(packet):
                img = frame.to_ndarray(format='rgb24')
                frames.append(DecodedFrame(
                    data=img,
                    width=frame.width,
                    height=frame.height,
                    timestamp=time.time(),
                    keyframe=frame.key_frame,
                    decoder_type=self._decoder_name
                ))
        except Exception as e:
            logger.debug(f"Packet decode error: {e}")

        return frames

    def feed_packet(self, packet) -> List[DecodedFrame]:
        """Feed a packet and get decoded frames."""
        return self.decode_packet(packet)

    async def flush(self) -> List[DecodedFrame]:
        """
        Flush the decoder buffer.

        Returns:
            Any remaining frames in the decoder buffer
        """
        if self._codec_context is None:
            return []

        try:
            loop = asyncio.get_event_loop()
            frames = await loop.run_in_executor(None, self._flush_sync)
            return frames
        except Exception as e:
            logger.debug(f"Flush error: {e}")
            return []

    def _flush_sync(self) -> List[DecodedFrame]:
        """Synchronous flush operation."""
        frames = []
        try:
            import av

            for frame in self._codec_context.decode():
                img = frame.to_ndarray(format='rgb24')
                frames.append(DecodedFrame(
                    data=img,
                    width=frame.width,
                    height=frame.height,
                    timestamp=time.time(),
                    keyframe=frame.key_frame,
                    decoder_type=self._decoder_name
                ))
        except Exception:
            pass

        return frames

    async def close(self) -> None:
        """Close the decoder and release resources."""
        if self._codec_context:
            try:
                self._codec_context.close()
            except Exception:
                pass
            self._codec_context = None

        self._initialized = False
        logger.info("HW decoder closed")

    def get_stats(self) -> dict:
        """Get decoder statistics."""
        return {
            'decoder_name': self._decoder_name,
            'frame_count': self._frame_count,
            'decode_errors': self._decode_errors,
            'width': self._width,
            'height': self._height,
            'available_decoders': self._available_decoders,
        }

    @property
    def width(self) -> int:
        """Get decoder width."""
        return self._width

    @property
    def height(self) -> int:
        """Get decoder height."""
        return self._height

    @property
    def decoder_name(self) -> str:
        """Get current decoder name."""
        return self._decoder_name

    @property
    def is_initialized(self) -> bool:
        """Check if decoder is initialized."""
        return self._initialized


def get_available_decoders() -> List[str]:
    """Get list of available hardware decoders."""
    return HWDecoder.get_available_decoders()


def create_decoder(
    width: int = 1920,
    height: int = 1080,
    codec: str = "h264"
) -> HWDecoder:
    """
    Create a configured hardware decoder.

    Args:
        width: Frame width
        height: Frame height
        codec: Codec name

    Returns:
        Configured HWDecoder (needs initialize() call)
    """
    return HWDecoder(HWDecoderConfig(width=width, height=height, codec=codec))


class StreamDecoder:
    """
    Streaming decoder with queue-based processing.

    Maintains decoder state and handles continuous decoding.
    """

    def __init__(
        self,
        width: int = 1920,
        height: int = 1080,
        output_callback: Optional[Callable] = None
    ):
        """Initialize stream decoder."""
        self.width = width
        self.height = height
        self.output_callback = output_callback
        self._decoder: Optional[HWDecoder] = None
        self._running = False
        self._decode_task = None
        self._frame_queue: asyncio.Queue = asyncio.Queue(maxsize=10)

    async def initialize(self) -> bool:
        """Initialize the decoder."""
        self._decoder = HWDecoder(HWDecoderConfig(
            width=self.width,
            height=self.height
        ))
        return await self._decoder.initialize()

    async def start(self) -> None:
        """Start the decode loop."""
        if self._running:
            return

        self._running = True
        self._decode_task = asyncio.create_task(self._decode_loop())

    async def _decode_loop(self) -> None:
        """Main decode loop."""
        decoder = self._decoder

        while self._running and decoder:
            try:
                # Get encoded data with timeout
                item = await asyncio.wait_for(
                    self._frame_queue.get(),
                    timeout=1.0
                )

                if item is None:  # Poison pill
                    break

                encoded_data, timestamp = item

                # Decode
                frame = await decoder.decode(encoded_data, timestamp)

                if frame and self.output_callback:
                    # Run callback in thread pool to avoid blocking
                    await asyncio.get_event_loop().run_in_executor(
                        None,
                        self.output_callback,
                        frame
                    )

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

        if self._decode_task:
            self._frame_queue.put(None)  # Poison pill
            try:
                await self._decode_task
            except asyncio.CancelledError:
                pass
            self._decode_task = None

        if self._decoder:
            await self._decoder.close()

    def get_stats(self) -> dict:
        """Get decoder stats."""
        if self._decoder:
            return self._decoder.get_stats()
        return {}
