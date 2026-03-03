"""
Base protocol handler interface.

All protocol implementations (WebRTC, QUIC, JPEG) must inherit from this.
"""

from abc import ABC, abstractmethod
from typing import Callable, Optional
import numpy as np
import numpy.typing as npt

from ..core.stats import ConnectionState


# Type aliases for callbacks
FrameCallback = Callable[[npt.NDArray[np.uint8]], None]
StatsCallback = Callable[[dict], None]
StateCallback = Callable[[ConnectionState], None]


class ProtocolHandler(ABC):
    """
    Abstract base class for protocol handlers.

    Each protocol (WebRTC, QUIC, JPEG) must implement this interface
    to provide unified video streaming and control capabilities.
    """

    def __init__(self):
        """Initialize protocol handler."""
        self._frame_callback: Optional[FrameCallback] = None
        self._stats_callback: Optional[StatsCallback] = None
        self._state_callback: Optional[StateCallback] = None

    @abstractmethod
    async def connect(
        self,
        target_device_id: str,
        offer_sdp: str,
        signaling_client
    ) -> str:
        """
        Initiate connection to target device.

        Args:
            target_device_id: Target agent device ID
            offer_sdp: SDP offer string (for WebRTC)
            signaling_client: Signaling client for message exchange

        Returns:
            SDP answer string (or equivalent protocol response)
        """
        pass

    @abstractmethod
    async def add_ice_candidate(self, candidate: dict) -> bool:
        """
        Add ICE candidate (for WebRTC).

        Args:
            candidate: ICE candidate dictionary

        Returns:
            True if successful
        """
        pass

    @abstractmethod
    def on_frame_received(self, callback: FrameCallback) -> None:
        """
        Register callback for received video frames.

        Args:
            callback: Function to call with each frame (numpy array)
        """
        pass

    @abstractmethod
    def on_stats_update(self, callback: StatsCallback) -> None:
        """
        Register callback for statistics updates.

        Args:
            callback: Function to call with stats dict
        """
        pass

    @abstractmethod
    def on_state_change(self, callback: StateCallback) -> None:
        """
        Register callback for connection state changes.

        Args:
            callback: Function to call with new ConnectionState
        """
        pass

    @abstractmethod
    async def disconnect(self) -> None:
        """Disconnect from the remote device."""
        pass

    @property
    @abstractmethod
    def name(self) -> str:
        """Get protocol name (e.g., 'webrtc', 'quic', 'jpeg')."""
        pass

    @property
    @abstractmethod
    def is_connected(self) -> bool:
        """Check if currently connected."""
        pass

    @property
    def connected_device_id(self) -> Optional[str]:
        """Get connected device ID."""
        return getattr(self, "_connected_device_id", None)


class DecoderProtocolHandler(ProtocolHandler):
    """
    Base class for protocol handlers that use video decoding.

    Provides common frame callback handling and decoder management.
    """

    def __init__(self):
        """Initialize decoder protocol handler."""
        super().__init__()
        self._decoder = None
        self._decode_queue = None

    def on_frame_received(self, callback: FrameCallback) -> None:
        """Register frame callback."""
        self._frame_callback = callback

    def on_stats_update(self, callback: StatsCallback) -> None:
        """Register stats callback."""
        self._stats_callback = callback

    def on_state_change(self, callback: StateCallback) -> None:
        """Register state change callback."""
        self._state_callback = callback

    def _emit_frame(self, frame: npt.NDArray[np.uint8]) -> None:
        """Emit frame to registered callback."""
        if self._frame_callback:
            self._frame_callback(frame)

    def _emit_stats(self, stats: dict) -> None:
        """Emit stats to registered callback."""
        if self._stats_callback:
            self._stats_callback(stats)

    def _emit_state(self, state: ConnectionState) -> None:
        """Emit state change to registered callback."""
        if self._state_callback:
            self._state_callback(state)

    async def _initialize_decoder(
        self,
        width: int = 1920,
        height: int = 1080,
        codec: str = "h264"
    ) -> bool:
        """
        Initialize video decoder.

        Args:
            width: Frame width
            height: Frame height
            codec: Codec name

        Returns:
            True if successful
        """
        try:
            from av import CodecContext

            self._decoder = CodecContext.create(codec, 'r')
            self._decoder.thread_count = 1  # Low latency

            try:
                self._decoder.options = {
                    'flags': 'low_delay',
                    'flags2': 'fast',
                }
            except Exception:
                pass

            return True

        except ImportError:
            return False
        except Exception as e:
            print(f"Decoder init error: {e}")
            return False

    async def _decode_frame(self, encoded_data: bytes) -> Optional[npt.NDArray[np.uint8]]:
        """
        Decode an encoded video frame.

        Args:
            encoded_data: Encoded frame data

        Returns:
            Decoded frame as numpy array or None
        """
        if self._decoder is None:
            return None

        try:
            import av
            from av import Packet

            packet = Packet(encoded_data)
            frames = self._decoder.decode(packet)

            for frame in frames:
                # Convert to RGB
                img = frame.to_ndarray(format='rgb24')
                return img

            return None

        except Exception as e:
            print(f"Decode error: {e}")
            return None

    async def _close_decoder(self) -> None:
        """Close the decoder and release resources."""
        if self._decoder:
            try:
                self._decoder.close()
            except Exception:
                pass
            self._decoder = None
