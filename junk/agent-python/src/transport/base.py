"""
Abstract base class for transport protocol adapters.

Defines the interface that all transport adapters must implement.
"""

from abc import ABC, abstractmethod
from typing import Optional, Callable, Awaitable
import logging

from .stats import TransportStats, FrameInfo

logger = logging.getLogger(__name__)


class TransportAdapter(ABC):
    """
    Abstract base class for transport protocol adapters.

    All transport implementations (QUIC, WebRTC, etc.) must inherit from
    this class and implement the required methods.
    """

    def __init__(self, name: str):
        """
        Initialize the transport adapter.

        Args:
            name: Protocol name (e.g., "quic", "webrtc")
        """
        self._name = name
        self._stats = TransportStats(protocol=name)
        self._connected = False
        self._event_handlers = {}

    @property
    def name(self) -> str:
        """Get protocol name."""
        return self._name

    @property
    def is_connected(self) -> bool:
        """Check if transport is connected."""
        return self._connected and self._stats.is_connected

    @property
    def stats(self) -> TransportStats:
        """Get transport statistics."""
        return self._stats

    @abstractmethod
    async def connect(self, offer: str, metadata: Optional[dict] = None) -> str:
        """
        Establish connection using the transport protocol.

        Args:
            offer: Connection offer/parameters from remote peer
            metadata: Optional connection metadata

        Returns:
            Connection answer/response to send back to peer

        Raises:
            TransportError: If connection fails
        """
        pass

    @abstractmethod
    async def send_media(self, frame: FrameInfo) -> None:
        """
        Send an encoded media frame through the transport.

        Args:
            frame: Frame information including encoded data

        Raises:
            TransportError: If send fails
        """
        pass

    @abstractmethod
    async def disconnect(self) -> None:
        """
        Close the transport connection and cleanup resources.
        """
        pass

    async def request_keyframe(self) -> None:
        """
        Request a keyframe from the encoder (via PLI/FIR).

        Default implementation does nothing - override if supported.
        """
        pass

    def on(self, event: str, handler: Callable) -> None:
        """
        Register an event handler.

        Common events:
        - "connected": Transport connected successfully
        - "disconnected": Transport disconnected
        - "error": Transport error occurred
        - "stats": Statistics updated (arg: TransportStats)

        Args:
            event: Event name
            handler: Callback function
        """
        self._event_handlers[event] = handler

    def _emit(self, event: str, *args, **kwargs) -> None:
        """
        Emit an event to registered handlers.

        Args:
            event: Event name
            *args: Positional arguments to pass to handler
            **kwargs: Keyword arguments to pass to handler
        """
        handler = self._event_handlers.get(event)
        if handler:
            try:
                if asyncio.iscoroutinefunction(handler):
                    asyncio.create_task(handler(*args, **kwargs))
                else:
                    handler(*args, **kwargs)
            except Exception as e:
                logger.error(f"Error in {event} handler: {e}")

    def _update_connection_state(self, connected: bool) -> None:
        """
        Update internal connection state.

        Args:
            connected: New connection state
        """
        self._connected = connected
        self._stats.is_connected = connected

        if connected and self._stats.connected_at is None:
            import time
            self._stats.connected_at = time()
            self._emit("connected")
        elif not connected:
            self._stats.connected_at = None
            self._emit("disconnected")

    def __repr__(self) -> str:
        return f"{self.__class__.__name__}(name={self._name}, connected={self._connected})"


class TransportError(Exception):
    """Base exception for transport errors."""

    def __init__(self, message: str, protocol: str = "unknown"):
        self.protocol = protocol
        super().__init__(f"[{protocol}] {message}")


class ConnectionError(TransportError):
    """Raised when connection establishment fails."""

    pass


class SendError(TransportError):
    """Raised when frame send fails."""

    pass


# Import asyncio here to avoid circular import
import asyncio
