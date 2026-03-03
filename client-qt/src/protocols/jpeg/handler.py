"""
JPEG streaming protocol handler.

Receives JPEG frames via WebSocket signaling (fallback protocol).
"""

import asyncio
import base64
import logging
import time
from typing import Callable, Optional

import numpy as np
import numpy.typing as npt
from PIL import Image
import io

from ..base import DecoderProtocolHandler
from ...core.stats import ConnectionState

logger = logging.getLogger(__name__)


class JPEGProtocolHandler(DecoderProtocolHandler):
    """
    JPEG streaming protocol handler.

    Receives JPEG frames via WebSocket signaling channel.
    This is a simple fallback protocol for low-bandwidth scenarios.
    """

    def __init__(self):
        """Initialize JPEG handler."""
        super().__init__()
        self._connected = False
        self._connected_device_id = None
        self._signaling = None
        self._frame_count = 0
        self._last_stats_time = time.time()

    @property
    def name(self) -> str:
        """Get protocol name."""
        return "jpeg"

    @property
    def is_connected(self) -> bool:
        """Check if connected."""
        return self._connected

    async def connect(
        self,
        target_device_id: str,
        offer_sdp: str,
        signaling_client
    ) -> str:
        """
        Connect to target device via JPEG streaming.

        Args:
            target_device_id: Target agent device ID
            offer_sdp: Not used for JPEG protocol
            signaling_client: Signaling client

        Returns:
            Response string
        """
        self._connected_device_id = target_device_id
        self._signaling = signaling_client
        self._emit_state(ConnectionState.CONNECTING)

        # Subscribe to frame events from signaling
        self._signaling.on("frame", self._handle_frame)

        self._connected = True
        self._emit_state(ConnectionState.CONNECTED)
        logger.info(f"JPEG streaming connected to {target_device_id}")

        return "jpeg-connected"

    async def add_ice_candidate(self, candidate: dict) -> bool:
        """JPEG doesn't use ICE candidates."""
        return True  # Ignore

    async def _handle_frame(self, payload: dict) -> None:
        """Handle incoming JPEG frame from signaling."""
        if not self._connected:
            return

        try:
            # Extract JPEG data from payload
            jpeg_data = payload.get("data")
            device_id = payload.get("deviceId")

            if device_id != self._connected_device_id:
                return

            if jpeg_data:
                # Decode base64 if needed
                if isinstance(jpeg_data, str):
                    jpeg_bytes = base64.b64decode(jpeg_data)
                else:
                    jpeg_bytes = jpeg_data

                # Decode JPEG to numpy array
                img = Image.open(io.BytesIO(jpeg_bytes))
                img_array = np.array(img)

                # Convert to RGB if needed
                if len(img_array.shape) == 3 and img_array.shape[2] == 4:
                    img_array = img_array[:, :, :3]  # Remove alpha
                elif len(img_array.shape) == 2:
                    # Grayscale to RGB
                    img_array = np.stack([img_array] * 3, axis=-1)

                # Emit frame
                self._emit_frame(img_array)
                self._frame_count += 1

                # Report stats periodically
                now = time.time()
                if now - self._last_stats_time >= 1.0:
                    stats = {
                        "protocol": "jpeg",
                        "frames_received": self._frame_count,
                        "timestamp": now,
                    }
                    self._emit_stats(stats)
                    self._last_stats_time = now

        except Exception as e:
            logger.error(f"JPEG frame handling error: {e}")

    async def disconnect(self) -> None:
        """Disconnect JPEG streaming."""
        self._connected = False

        if self._signaling:
            self._signaling.off("frame", self._handle_frame)

        self._connected_device_id = None
        self._emit_state(ConnectionState.DISCONNECTED)
        logger.info("JPEG streaming disconnected")
