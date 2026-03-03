"""
Protocol manager for handling multiple streaming protocols.

Manages protocol selection, negotiation, and fallback.
"""

import asyncio
import logging
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Type

from .base import ProtocolHandler, FrameCallback, StatsCallback, StateCallback
from ..core.stats import ConnectionState

logger = logging.getLogger(__name__)


@dataclass
class ProtocolConfig:
    """Configuration for protocol manager."""
    # Preferred protocol order
    priority: List[str] = field(default_factory=lambda: ["webrtc", "quic", "jpeg"])

    # Protocol-specific settings
    webrtc_enabled: bool = True
    quic_enabled: bool = True
    jpeg_enabled: bool = True

    # Auto-fallback settings
    enable_fallback: bool = True
    fallback_timeout: float = 10.0  # seconds

    # Retry settings
    max_retries: int = 2
    retry_delay: float = 2.0

    # Decoder preferences for WebRTC handler
    webrtc_use_hw_decoder: bool = True
    webrtc_decoder_priority: List[str] = field(default_factory=list)
    webrtc_decoder_low_delay: bool = True


class ProtocolManager:
    """
    Manages multiple protocol handlers for remote desktop streaming.

    Features:
    - Protocol negotiation with agent
    - Automatic fallback on failure
    - Unified interface for all protocols
    - Connection state tracking
    """

    def __init__(self, config: ProtocolConfig = None):
        """Initialize protocol manager."""
        self.config = config or ProtocolConfig()
        self._handlers: Dict[str, Type[ProtocolHandler]] = {}
        self._current_handler: Optional[ProtocolHandler] = None
        self._current_protocol: str = ""
        self._connected_device_id: Optional[str] = None

        # Callbacks
        self._frame_callback: Optional[FrameCallback] = None
        self._stats_callback: Optional[StatsCallback] = None
        self._state_callback: Optional[StateCallback] = None

        # Register protocol handlers
        self._register_protocols()

    def _register_protocols(self) -> None:
        """Register available protocol handlers."""
        try:
            from .webrtc.handler import WebRTCProtocolHandler
            if self.config.webrtc_enabled:
                self._handlers["webrtc"] = WebRTCProtocolHandler
                logger.info("Registered WebRTC protocol")
        except ImportError as e:
            logger.warning(f"WebRTC protocol not available: {e}")

        try:
            from .quic.handler import QuicProtocolHandler
            if self.config.quic_enabled:
                self._handlers["quic"] = QuicProtocolHandler
                logger.info("Registered QUIC protocol")
        except ImportError as e:
            logger.warning(f"QUIC protocol not available: {e}")

        try:
            from .jpeg.handler import JPEGProtocolHandler
            if self.config.jpeg_enabled:
                self._handlers["jpeg"] = JPEGProtocolHandler
                logger.info("Registered JPEG protocol")
        except ImportError as e:
            logger.warning(f"JPEG protocol not available: {e}")

        if not self._handlers:
            logger.error("No protocol handlers available!")

    def get_available_protocols(self) -> List[str]:
        """Get list of available protocols."""
        return list(self._handlers.keys())

    def get_protocols_in_priority_order(self) -> List[str]:
        """Get protocols sorted by priority."""
        available = set(self._handlers.keys())
        return [p for p in self.config.priority if p in available]

    async def negotiate_protocol(
        self,
        device_capabilities: dict
    ) -> str:
        """
        Negotiate the best protocol with the agent.

        Args:
            device_capabilities: Agent's capabilities dict

        Returns:
            Selected protocol name
        """
        agent_protocols = device_capabilities.get("protocols", [])

        # Find first matching protocol in priority order
        for protocol in self.get_protocols_in_priority_order():
            if protocol in agent_protocols:
                logger.info(f"Negotiated protocol: {protocol}")
                return protocol

        # Default to first available
        available = self.get_available_protocols()
        if available:
            logger.warning(f"No matching protocol, using: {available[0]}")
            return available[0]

        raise RuntimeError("No protocols available")

    async def connect(
        self,
        target_device_id: str,
        offer_sdp: str,
        signaling_client,
        preferred_protocol: Optional[str] = None
    ) -> bool:
        """
        Connect to target device using best available protocol.

        Args:
            target_device_id: Target agent device ID
            offer_sdp: SDP offer string
            signaling_client: Signaling client for message exchange
            preferred_protocol: Force specific protocol (optional)

        Returns:
            True if connection successful
        """
        if preferred_protocol and preferred_protocol in self._handlers:
            protocol = preferred_protocol
        else:
            # Try each protocol in priority order
            protocol = None

        # Determine protocol to use
        protocols_to_try = (
            [preferred_protocol] if preferred_protocol and preferred_protocol in self._handlers
            else self.get_protocols_in_priority_order()
        )

        last_error = None

        for protocol in protocols_to_try:
            if not self.config.enable_fallback and protocol != protocols_to_try[0]:
                break

            logger.info(f"Attempting connection with protocol: {protocol}")

            try:
                handler_class = self._handlers[protocol]
                if protocol == "webrtc":
                    handler = handler_class(
                        use_hw_decoder=self.config.webrtc_use_hw_decoder,
                        decoder_priority=self.config.webrtc_decoder_priority,
                        decoder_low_delay=self.config.webrtc_decoder_low_delay,
                    )
                else:
                    handler = handler_class()

                # Set up callbacks
                if self._frame_callback:
                    handler.on_frame_received(self._frame_callback)
                if self._stats_callback:
                    handler.on_stats_update(self._stats_callback)
                if self._state_callback:
                    handler.on_state_change(self._state_callback)

                # Attempt connection with timeout
                answer = await asyncio.wait_for(
                    handler.connect(target_device_id, offer_sdp, signaling_client),
                    timeout=self.config.fallback_timeout
                )

                if answer:
                    self._current_handler = handler
                    self._current_protocol = protocol
                    self._connected_device_id = target_device_id
                    logger.info(f"Connected using protocol: {protocol}")
                    return True

            except asyncio.TimeoutError:
                logger.warning(f"Connection timeout with protocol: {protocol}")
                last_error = f"Timeout connecting with {protocol}"
                await handler.disconnect()

            except Exception as e:
                logger.warning(f"Connection failed with protocol {protocol}: {e}")
                last_error = str(e)
                try:
                    await handler.disconnect()
                except Exception:
                    pass

        # All protocols failed
        logger.error(f"All connection attempts failed. Last error: {last_error}")
        if self._state_callback:
            self._state_callback(ConnectionState.FAILED)
        return False

    async def add_ice_candidate(self, candidate: dict) -> bool:
        """Add ICE candidate to current handler."""
        if self._current_handler:
            return await self._current_handler.add_ice_candidate(candidate)
        return False

    def on_frame_received(self, callback: FrameCallback) -> None:
        """Register frame callback."""
        self._frame_callback = callback
        if self._current_handler:
            self._current_handler.on_frame_received(callback)

    def on_stats_update(self, callback: StatsCallback) -> None:
        """Register stats callback."""
        self._stats_callback = callback
        if self._current_handler:
            self._current_handler.on_stats_update(callback)

    def on_state_change(self, callback: StateCallback) -> None:
        """Register state change callback."""
        self._state_callback = callback
        if self._current_handler:
            self._current_handler.on_state_change(callback)

    async def disconnect(self) -> None:
        """Disconnect from current device."""
        if self._current_handler:
            await self._current_handler.disconnect()
            self._current_handler = None

        self._current_protocol = ""
        self._connected_device_id = None

    @property
    def current_protocol(self) -> str:
        """Get currently active protocol name."""
        return self._current_protocol

    @property
    def is_connected(self) -> bool:
        """Check if currently connected."""
        return (
            self._current_handler is not None and
            self._current_handler.is_connected
        )

    @property
    def connected_device_id(self) -> Optional[str]:
        """Get connected device ID."""
        return self._connected_device_id

    @property
    def current_handler(self) -> Optional[ProtocolHandler]:
        """Get current protocol handler."""
        return self._current_handler
