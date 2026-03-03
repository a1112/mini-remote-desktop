"""
Statistics tracking for remote desktop connections.
"""

import time
from dataclasses import dataclass, field
from typing import Optional
from enum import Enum


class ConnectionState(Enum):
    """Connection state enumeration."""
    DISCONNECTED = "disconnected"
    CONNECTING = "connecting"
    CONNECTED = "connected"
    FAILED = "failed"


@dataclass
class ConnectionStats:
    """Real-time connection statistics."""
    # Connection info
    state: ConnectionState = ConnectionState.DISCONNECTED
    protocol: str = ""
    connected_at: Optional[float] = None
    device_id: Optional[str] = None
    device_name: Optional[str] = None

    # Network stats
    latency_ms: float = 0.0
    bitrate_mbps: float = 0.0
    packet_loss: float = 0.0
    jitter_ms: float = 0.0

    # Video stats
    fps: float = 0.0
    width: int = 0
    height: int = 0
    codec: str = ""
    keyframe_count: int = 0

    # Internal counters
    _bytes_received: int = 0
    _frames_received: int = 0
    _last_bytes: int = 0
    _last_time: float = 0.0
    _last_frame_time: float = 0.0
    _frame_times: list = field(default_factory=list)

    def update_connection_state(self, state: ConnectionState) -> None:
        """Update connection state."""
        self.state = state
        if state == ConnectionState.CONNECTED:
            self.connected_at = time.time()

    def update_network_stats(
        self,
        latency_ms: float = 0.0,
        packet_loss: float = 0.0,
        jitter_ms: float = 0.0
    ) -> None:
        """Update network statistics."""
        self.latency_ms = latency_ms
        self.packet_loss = packet_loss
        self.jitter_ms = jitter_ms

        # Calculate bitrate
        current_time = time.time()
        if self._last_time > 0:
            elapsed = current_time - self._last_time
            bytes_diff = self._bytes_received - self._last_bytes
            self.bitrate_mbps = (bytes_diff * 8) / (elapsed * 1_000_000)

        self._last_time = current_time
        self._last_bytes = self._bytes_received

    def update_video_stats(self, width: int, height: int, codec: str = "") -> None:
        """Update video statistics."""
        self.width = width
        self.height = height
        self.codec = codec

    def add_received_bytes(self, bytes_count: int) -> None:
        """Add received bytes to counter."""
        self._bytes_received += bytes_count

    def add_received_frame(self, is_keyframe: bool = False) -> None:
        """Add received frame to counter."""
        self._frames_received += 1
        if is_keyframe:
            self.keyframe_count += 1

        # Calculate FPS
        current_time = time.time()
        if self._last_frame_time > 0:
            elapsed = current_time - self._last_frame_time
            self._frame_times.append(elapsed)

            # Keep only last 30 frame intervals
            if len(self._frame_times) > 30:
                self._frame_times.pop(0)

            if self._frame_times:
                avg_interval = sum(self._frame_times) / len(self._frame_times)
                self.fps = 1.0 / avg_interval if avg_interval > 0 else 0.0

        self._last_frame_time = current_time

    def reset(self) -> None:
        """Reset all statistics."""
        self.state = ConnectionState.DISCONNECTED
        self.protocol = ""
        self.connected_at = None
        self.device_id = None
        self.device_name = None

        self.latency_ms = 0.0
        self.bitrate_mbps = 0.0
        self.packet_loss = 0.0
        self.jitter_ms = 0.0

        self.fps = 0.0
        self.width = 0
        self.height = 0
        self.codec = ""
        self.keyframe_count = 0

        self._bytes_received = 0
        self._frames_received = 0
        self._last_bytes = 0
        self._last_time = 0.0
        self._last_frame_time = 0.0
        self._frame_times.clear()

    @property
    def uptime_seconds(self) -> float:
        """Get connection uptime in seconds."""
        if self.connected_at:
            return time.time() - self.connected_at
        return 0.0

    @property
    def total_bytes_received(self) -> int:
        """Get total bytes received."""
        return self._bytes_received

    @property
    def total_frames_received(self) -> int:
        """Get total frames received."""
        return self._frames_received


class Stats:
    """
    Statistics manager for remote desktop connections.

    Tracks connection state, network performance, and video statistics.
    Emits Qt signals when stats update.
    """

    def __init__(self):
        """Initialize statistics manager."""
        self._connection = ConnectionStats()
        self._callbacks = []

    def register_callback(self, callback) -> None:
        """Register a callback for stats updates."""
        if callback not in self._callbacks:
            self._callbacks.append(callback)

    def unregister_callback(self, callback) -> None:
        """Unregister a stats update callback."""
        if callback in self._callbacks:
            self._callbacks.remove(callback)

    def _notify(self) -> None:
        """Notify all registered callbacks."""
        for callback in self._callbacks:
            try:
                callback(self._connection)
            except Exception as e:
                print(f"Stats callback error: {e}")

    @property
    def connection(self) -> ConnectionStats:
        """Get connection stats."""
        return self._connection

    def update_connection_state(self, state: ConnectionState) -> None:
        """Update connection state."""
        self._connection.update_connection_state(state)
        self._notify()

    def update_network_stats(
        self,
        latency_ms: float = 0.0,
        packet_loss: float = 0.0,
        jitter_ms: float = 0.0
    ) -> None:
        """Update network statistics."""
        self._connection.update_network_stats(latency_ms, packet_loss, jitter_ms)
        self._notify()

    def update_video_stats(self, width: int, height: int, codec: str = "") -> None:
        """Update video statistics."""
        self._connection.update_video_stats(width, height, codec)
        self._notify()

    def add_received_bytes(self, bytes_count: int) -> None:
        """Add received bytes to counter."""
        self._connection.add_received_bytes(bytes_count)
        self._notify()

    def add_received_frame(self, is_keyframe: bool = False) -> None:
        """Add received frame to counter."""
        self._connection.add_received_frame(is_keyframe)
        self._notify()

    def set_protocol(self, protocol: str) -> None:
        """Set current protocol."""
        self._connection.protocol = protocol
        self._notify()

    def set_device_info(self, device_id: str, device_name: str) -> None:
        """Set connected device info."""
        self._connection.device_id = device_id
        self._connection.device_name = device_name
        self._notify()

    def reset(self) -> None:
        """Reset all statistics."""
        self._connection.reset()
        self._notify()
