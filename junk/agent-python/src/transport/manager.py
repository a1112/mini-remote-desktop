"""
Transport manager for multi-protocol support.

Handles protocol selection, automatic fallback, and performance monitoring.
"""

import asyncio
import logging
from dataclasses import dataclass, field
from enum import Enum
from typing import Optional, Dict, List, Callable, Any
from time import time

from .base import TransportAdapter, TransportError
from .stats import TransportStats, FrameInfo
from .quic_adapter import QUICAdapter, is_quic_available
from .webrtc_adapter import WebRTCAdapter, is_webrtc_available

logger = logging.getLogger(__name__)


class ProtocolType(Enum):
    """Supported transport protocols."""
    AUTO = "auto"
    QUIC = "quic"
    WEBRTC = "webrtc"


@dataclass
class TransportConfig:
    """Configuration for transport manager."""

    # Protocol selection
    preferred: ProtocolType = ProtocolType.AUTO
    fallback: ProtocolType = ProtocolType.QUIC
    allow_webrtc_fallback: bool = False

    # Auto-switching
    auto_switch: bool = True
    switch_cooldown: float = 10.0  # Seconds between switches
    max_switches: int = 5

    # Performance thresholds for switching
    rtt_threshold_ms: float = 100.0
    packet_loss_threshold: float = 5.0  # Percentage
    min_fps_threshold: float = 15.0

    # Connection timeout
    connection_timeout: float = 5.0

    # WebRTC config
    webrtc_ice_servers: Optional[List[Dict[str, Any]]] = None

    # QUIC config
    quic_host: str = "0.0.0.0"
    quic_port: int = 0


class TransportManager:
    """
    Multi-protocol transport manager.

    Features:
    - Automatic protocol selection based on availability and performance
    - Automatic fallback on connection failure or performance degradation
    - Real-time performance monitoring
    - Protocol switching with cooldown to prevent flapping
    """

    def __init__(self, config: Optional[TransportConfig] = None):
        """
        Initialize transport manager.

        Args:
            config: Transport configuration
        """
        self.config = config or TransportConfig()

        # Available protocols
        self._protocols: Dict[str, TransportAdapter] = {}
        self._protocol_order: List[str] = []

        # Active connection
        self._active: Optional[TransportAdapter] = None
        self._active_protocol: Optional[str] = None

        # State tracking
        self._switch_count = 0
        self._last_switch_time: float = 0
        self._connected: bool = False
        self._monitoring_task: Optional[asyncio.Task] = None

        # Setup available protocols
        self._setup_protocols()

        logger.info(f"TransportManager initialized with protocols: {self._protocol_order}")

    def _setup_protocols(self) -> None:
        """Setup available transport protocols."""
        # Determine protocol order based on preference
        preferred = self.config.preferred

        if preferred == ProtocolType.QUIC:
            order = ["quic", "webrtc"]
        elif preferred == ProtocolType.WEBRTC:
            order = ["webrtc", "quic"]
        else:  # AUTO - default to QUIC first
            order = ["quic", "webrtc"]

        # Initialize available protocols
        for proto_name in order:
            try:
                if proto_name == "quic" and is_quic_available():
                    self._protocols["quic"] = QUICAdapter(
                        host=self.config.quic_host,
                        port=self.config.quic_port,
                    )
                    self._protocol_order.append("quic")
                    logger.info("QUIC protocol available")

                elif (
                    proto_name == "webrtc"
                    and self.config.allow_webrtc_fallback
                    and is_webrtc_available()
                ):
                    self._protocols["webrtc"] = WebRTCAdapter(
                        video_width=1920,
                        video_height=1080,
                        video_fps=30,
                        ice_servers=self.config.webrtc_ice_servers,
                    )
                    self._protocol_order.append("webrtc")
                    logger.info("WebRTC protocol available")

            except Exception as e:
                logger.warning(f"Failed to initialize {proto_name}: {e}")

        if not self._protocol_order:
            logger.warning("No transport protocols available!")

    async def connect(self, offer: str, metadata: Optional[dict] = None) -> str:
        """
        Establish connection using the best available protocol.

        Tries protocols in order until one succeeds.

        Args:
            offer: Connection offer (may contain protocol hints)
            metadata: Optional connection metadata

        Returns:
            Connection answer

        Raises:
            TransportError: If all protocols fail
        """
        # Try to determine protocol from offer
        offer_proto = self._parse_offer_protocol(offer)
        if offer_proto and offer_proto in self._protocol_order:
            # Prioritize the requested protocol
            order = [offer_proto] + [p for p in self._protocol_order if p != offer_proto]
        else:
            order = self._protocol_order

        last_error = None

        for proto_name in order:
            protocol = self._protocols.get(proto_name)
            if not protocol:
                continue

            logger.info(f"Attempting connection with {proto_name.upper()}...")

            try:
                # Connect with timeout
                answer = await asyncio.wait_for(
                    protocol.connect(offer, metadata),
                    timeout=self.config.connection_timeout
                )

                # Success!
                self._active = protocol
                self._active_protocol = proto_name
                self._connected = True

                # Start monitoring
                if self.config.auto_switch:
                    self._monitoring_task = asyncio.create_task(self._monitor_loop())

                logger.info(f"Connected using {proto_name.upper()}")
                return answer

            except asyncio.TimeoutError:
                logger.warning(f"{proto_name.upper()} connection timed out")
                last_error = f"{proto_name} connection timed out"

            except Exception as e:
                logger.warning(f"{proto_name.upper()} connection failed: {e}")
                last_error = str(e)

        # All protocols failed
        self._connected = False
        raise TransportError(f"All connection attempts failed. Last error: {last_error}")

    def _parse_offer_protocol(self, offer: str) -> Optional[str]:
        """
        Parse protocol hint from offer.

        Args:
            offer: Offer JSON string

        Returns:
            Protocol name or None
        """
        try:
            import json
            data = json.loads(offer)
            proto = data.get("protocol", "").lower()
            if proto in ["quic", "webrtc"]:
                return proto
        except (json.JSONDecodeError, TypeError):
            pass
        return None

    async def send_media(self, frame: FrameInfo) -> None:
        """
        Send media frame using active transport.

        Args:
            frame: Frame information

        Raises:
            TransportError: If no active connection or send fails
        """
        if not self._active or not self._connected:
            raise TransportError("Not connected")

        try:
            await self._active.send_media(frame)
        except Exception as e:
            logger.error(f"Send failed: {e}")
            # Try to switch protocols if auto-switch is enabled
            if self.config.auto_switch and self._can_switch():
                await self._switch_protocol("send_error")
            raise

    async def request_keyframe(self) -> None:
        """Request a keyframe from the encoder."""
        if self._active:
            await self._active.request_keyframe()

    async def disconnect(self) -> None:
        """Disconnect and cleanup."""
        logger.info("Disconnecting transport manager...")

        self._connected = False

        if self._monitoring_task:
            self._monitoring_task.cancel()
            self._monitoring_task = None

        if self._active:
            await self._active.disconnect()
            self._active = None
            self._active_protocol = None

        # Cleanup all protocols
        for protocol in self._protocols.values():
            try:
                await protocol.disconnect()
            except Exception:
                pass

        logger.info("Transport manager disconnected")

    async def _monitor_loop(self) -> None:
        """Monitor performance and trigger protocol switches if needed."""
        while self._connected and self._active:
            try:
                await asyncio.sleep(1.0)

                # Check if we should switch
                if await self._should_switch():
                    await self._switch_protocol("performance")

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error(f"Monitor loop error: {e}")

    async def _should_switch(self) -> bool:
        """
        Check if protocol should be switched based on performance.

        Returns:
            True if switching is recommended
        """
        if not self._active:
            return False

        stats = self._active.stats

        # Check RTT threshold
        if stats.rtt_avg_ms > self.config.rtt_threshold_ms:
            logger.info(f"RTT threshold exceeded: {stats.rtt_avg_ms:.1f}ms > {self.config.rtt_threshold_ms}ms")
            return True

        # Check packet loss threshold
        stats.update_packet_loss()
        if stats.packet_loss_percent > self.config.packet_loss_threshold:
            logger.info(f"Packet loss threshold exceeded: {stats.packet_loss_percent:.1f}% > {self.config.packet_loss_threshold}%")
            return True

        # Check FPS threshold
        if stats.fps > 0 and stats.fps < self.config.min_fps_threshold:
            logger.info(f"FPS threshold exceeded: {stats.fps:.1f} < {self.config.min_fps_threshold}")
            return True

        return False

    def _can_switch(self) -> bool:
        """Check if switching is allowed (cooldown and max switches)."""
        # Check cooldown
        if time() - self._last_switch_time < self.config.switch_cooldown:
            return False

        # Check max switches
        if self._switch_count >= self.config.max_switches:
            logger.warning("Max protocol switches reached")
            return False

        return True

    async def _switch_protocol(self, reason: str) -> bool:
        """
        Switch to the fallback protocol.

        Args:
            reason: Reason for switching (for logging)

        Returns:
            True if switch successful
        """
        if not self._can_switch():
            return False

        # Determine fallback protocol from config
        current = self._active_protocol
        fallback = self.config.fallback.value

        if fallback not in self._protocols:
            logger.warning(f"Fallback protocol {fallback} not available")
            return False

        if fallback == current:
            return False

        logger.info(f"Switching from {current} to {fallback} (reason: {reason})")

        try:
            # Disconnect current
            old_stats = self._active.stats.to_dict() if self._active else {}
            if self._active:
                await self._active.disconnect()

            # Switch to fallback
            self._active = self._protocols[fallback]
            self._active_protocol = fallback
            self._switch_count += 1
            self._last_switch_time = time()

            # Emit event
            self._emit("protocol_switched", {
                "from": current,
                "to": fallback,
                "reason": reason,
                "old_stats": old_stats,
            })

            logger.info(f"Switched to {fallback.upper()}")
            return True

        except Exception as e:
            logger.error(f"Protocol switch failed: {e}")
            return False

    def on(self, event: str, handler: Callable) -> None:
        """
        Register an event handler.

        Events:
        - "connected": Transport connected
        - "disconnected": Transport disconnected
        - "protocol_switched": Protocol was switched (arg: dict with from/to/reason)
        - "stats": Statistics updated

        Args:
            event: Event name
            handler: Callback function
        """
        # Forward to all protocols
        for protocol in self._protocols.values():
            protocol.on(event, handler)

    def _emit(self, event: str, *args, **kwargs) -> None:
        """Emit event to registered handlers."""
        if self._active:
            self._active._emit(event, *args, **kwargs)

    @property
    def is_connected(self) -> bool:
        """Check if connected."""
        return self._connected and self._active is not None and self._active.is_connected

    @property
    def active_protocol(self) -> Optional[str]:
        """Get active protocol name."""
        return self._active_protocol

    @property
    def stats(self) -> Optional[TransportStats]:
        """Get active transport statistics."""
        if self._active:
            return self._active.stats
        return None

    @property
    def available_protocols(self) -> List[str]:
        """Get list of available protocols."""
        return self._protocol_order.copy()

    def get_stats_dict(self) -> dict:
        """Get all statistics as a dictionary."""
        stats = {
            "connected": self._connected,
            "active_protocol": self._active_protocol,
            "switch_count": self._switch_count,
            "available_protocols": self._protocol_order,
        }

        if self._active:
            stats["active_stats"] = self._active.stats.to_dict()

        # Get stats from all protocols
        stats["all_stats"] = {}
        for name, protocol in self._protocols.items():
            stats["all_stats"][name] = protocol.stats.to_dict()

        return stats


def create_transport_manager(
    preferred: str = "auto",
    fallback: str = "quic",
    allow_webrtc_fallback: bool = False,
    auto_switch: bool = True,
    **kwargs
) -> TransportManager:
    """
    Create a transport manager with the specified configuration.

    Args:
        preferred: Preferred protocol ("auto", "quic", "webrtc")
        fallback: Fallback protocol ("quic" or "webrtc")
        allow_webrtc_fallback: Whether WebRTC path can be used as fallback
        auto_switch: Enable automatic protocol switching
        **kwargs: Additional configuration options

    Returns:
        Configured TransportManager
    """
    # Map string to ProtocolType
    proto_map = {
        "auto": ProtocolType.AUTO,
        "quic": ProtocolType.QUIC,
        "webrtc": ProtocolType.WEBRTC,
    }

    config = TransportConfig(
        preferred=proto_map.get(preferred.lower(), ProtocolType.AUTO),
        fallback=proto_map.get(fallback.lower(), ProtocolType.QUIC),
        allow_webrtc_fallback=allow_webrtc_fallback,
        auto_switch=auto_switch,
        **kwargs
    )

    return TransportManager(config)
