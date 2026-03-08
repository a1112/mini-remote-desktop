"""
Transport statistics and monitoring data classes.
"""

from dataclasses import dataclass, field
from time import time
from typing import Optional


@dataclass
class TransportStats:
    """
    Statistics for transport protocol performance monitoring.

    Tracks latency, packet loss, bandwidth, and frame rate metrics.
    """

    # Protocol identification
    protocol: str = "unknown"

    # Connection state
    is_connected: bool = False
    connected_at: Optional[float] = None

    # Latency metrics (milliseconds)
    rtt_ms: float = 0.0
    rtt_avg_ms: float = 0.0
    rtt_max_ms: float = 0.0
    rtt_samples: int = 0

    # Packet loss
    packets_sent: int = 0
    packets_lost: int = 0
    packet_loss_percent: float = 0.0

    # Bandwidth (bytes per second)
    bytes_sent: int = 0
    bytes_received: int = 0
    bandwidth_bps: float = 0.0
    bandwidth_kbps: float = 0.0

    # Frame metrics
    frames_sent: int = 0
    frames_dropped: int = 0
    fps: float = 0.0

    # Error tracking
    connection_errors: int = 0
    reconnect_count: int = 0

    # Timestamps
    last_update: float = field(default_factory=time)
    stats_start: float = field(default_factory=time)

    def update_rtt(self, rtt_ms: float) -> None:
        """
        Update RTT metrics with a new sample.

        Args:
            rtt_ms: New RTT measurement in milliseconds
        """
        self.rtt_ms = rtt_ms
        self.rtt_samples += 1

        # Update average (exponential moving average)
        if self.rtt_avg_ms == 0:
            self.rtt_avg_ms = rtt_ms
        else:
            alpha = 0.1  # Smoothing factor
            self.rtt_avg_ms = (alpha * rtt_ms) + ((1 - alpha) * self.rtt_avg_ms)

        # Update max
        if rtt_ms > self.rtt_max_ms:
            self.rtt_max_ms = rtt_ms

        self.last_update = time()

    def update_packet_loss(self) -> None:
        """Recalculate packet loss percentage."""
        if self.packets_sent > 0:
            self.packet_loss_percent = (self.packets_lost / self.packets_sent) * 100

    def update_bandwidth(self, window_seconds: float = 1.0) -> None:
        """
        Calculate current bandwidth based on bytes sent.

        Args:
            window_seconds: Time window for calculation
        """
        elapsed = time() - self.stats_start
        if elapsed > 0:
            self.bandwidth_bps = self.bytes_sent / elapsed
            self.bandwidth_kbps = self.bandwidth_bps / 1000

    def update_fps(self, frames_sent: int, window_seconds: float = 1.0) -> None:
        """
        Calculate current frame rate.

        Args:
            frames_sent: Current total frames sent
            window_seconds: Time window for calculation
        """
        elapsed = time() - self.stats_start
        if elapsed > 0:
            self.fps = frames_sent / elapsed

    def reset(self) -> None:
        """Reset statistics (but keep protocol name)."""
        protocol = self.protocol
        current_time = time()

        self.__dict__.update({
            "protocol": protocol,
            "is_connected": False,
            "connected_at": None,
            "rtt_ms": 0.0,
            "rtt_avg_ms": 0.0,
            "rtt_max_ms": 0.0,
            "rtt_samples": 0,
            "packets_sent": 0,
            "packets_lost": 0,
            "packet_loss_percent": 0.0,
            "bytes_sent": 0,
            "bytes_received": 0,
            "bandwidth_bps": 0.0,
            "bandwidth_kbps": 0.0,
            "frames_sent": 0,
            "frames_dropped": 0,
            "fps": 0.0,
            "connection_errors": 0,
            "reconnect_count": 0,
            "last_update": current_time,
            "stats_start": current_time,
        })

    def to_dict(self) -> dict:
        """Convert statistics to dictionary for logging/monitoring."""
        return {
            "protocol": self.protocol,
            "is_connected": self.is_connected,
            "rtt_ms": round(self.rtt_ms, 2),
            "rtt_avg_ms": round(self.rtt_avg_ms, 2),
            "rtt_max_ms": round(self.rtt_max_ms, 2),
            "packet_loss_percent": round(self.packet_loss_percent, 2),
            "bandwidth_kbps": round(self.bandwidth_kbps, 2),
            "fps": round(self.fps, 2),
            "frames_sent": self.frames_sent,
            "frames_dropped": self.frames_dropped,
            "connection_errors": self.connection_errors,
        }

    def __repr__(self) -> str:
        return (f"TransportStats(protocol={self.protocol}, "
                f"connected={self.is_connected}, "
                f"rtt_avg={self.rtt_avg_ms:.1f}ms, "
                f"loss={self.packet_loss_percent:.1f}%, "
                f"fps={self.fps:.1f})")


@dataclass
class FrameInfo:
    """
    Information about an encoded frame for transport.
    """

    data: bytes
    timestamp: int
    is_keyframe: bool = False
    width: int = 1920
    height: int = 1080
    frame_number: int = 0

    @property
    def size(self) -> int:
        """Get frame size in bytes."""
        return len(self.data)
