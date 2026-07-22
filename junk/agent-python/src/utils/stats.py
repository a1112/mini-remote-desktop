"""
Performance statistics tracking.

Tracks FPS, bitrate, latency, and other metrics.
"""

import logging
import time
from collections import deque
from dataclasses import dataclass, field
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class PerformanceStats:
    """
    Performance statistics tracker.

    Tracks frame rate, bitrate, encoding time, and other metrics.
    """

    # Frame tracking
    frames_captured: int = 0
    frames_encoded: int = 0
    frames_sent: int = 0
    frames_dropped: int = 0

    # Timing
    capture_time_total: float = 0.0
    encode_time_total: float = 0.0
    last_frame_time: float = 0.0

    # Bitrate tracking
    bytes_sent: int = 0
    bitrate_window: deque = field(default_factory=lambda: deque(maxlen=60))

    # Latency tracking
    latency_samples: deque = field(default_factory=lambda: deque(maxlen=100))

    # Timing for FPS calculation
    fps_window: deque = field(default_factory=lambda: deque(maxlen=60))

    def __post_init__(self):
        self._start_time = time.time()
        self._last_stats_update = self._start_time

    def record_captured_frame(self, capture_time: float = 0.0) -> None:
        """
        Record a captured frame.

        Args:
            capture_time: Time taken to capture (seconds)
        """
        self.frames_captured += 1
        if capture_time > 0:
            self.capture_time_total += capture_time

        now = time.time()
        if self.last_frame_time > 0:
            self.fps_window.append(now - self.last_frame_time)
        self.last_frame_time = now

    def record_encoded_frame(self, encode_time: float = 0.0, size: int = 0) -> None:
        """
        Record an encoded frame.

        Args:
            encode_time: Time taken to encode (seconds)
            size: Encoded frame size in bytes
        """
        self.frames_encoded += 1
        if encode_time > 0:
            self.encode_time_total += encode_time

        if size > 0:
            self.bytes_sent += size
            self.bitrate_window.append((time.time(), size))

    def record_sent_frame(self) -> None:
        """Record a frame sent via WebRTC."""
        self.frames_sent += 1

    def record_dropped_frame(self) -> None:
        """Record a dropped frame."""
        self.frames_dropped += 1

    def record_latency(self, latency_ms: float) -> None:
        """
        Record a latency measurement.

        Args:
            latency_ms: Latency in milliseconds
        """
        self.latency_samples.append(latency_ms)

    def get_fps(self) -> float:
        """
        Get current frames per second.

        Returns:
            FPS based on recent frame intervals
        """
        if not self.fps_window:
            return 0.0

        total_time = sum(self.fps_window)
        if total_time == 0:
            return 0.0

        return len(self.fps_window) / total_time

    def get_target_fps(self) -> float:
        """Get target FPS (for comparison)."""
        return 30.0  # Default target

    def get_bitrate_kbps(self) -> float:
        """
        Get current bitrate in kbps.

        Returns:
            Bitrate based on recent bytes sent
        """
        if not self.bitrate_window:
            return 0.0

        now = time.time()
        # Sum bytes from last second
        recent_bytes = sum(
            size for ts, size in self.bitrate_window if now - ts < 1.0
        )

        return (recent_bytes * 8) / 1000.0  # Convert to kbps

    def get_avg_capture_time_ms(self) -> float:
        """Get average capture time in milliseconds."""
        if self.frames_captured == 0:
            return 0.0
        return (self.capture_time_total / self.frames_captured) * 1000.0

    def get_avg_encode_time_ms(self) -> float:
        """Get average encode time in milliseconds."""
        if self.frames_encoded == 0:
            return 0.0
        return (self.encode_time_total / self.frames_encoded) * 1000.0

    def get_avg_latency_ms(self) -> float:
        """Get average latency in milliseconds."""
        if not self.latency_samples:
            return 0.0
        return sum(self.latency_samples) / len(self.latency_samples)

    def get_p95_latency_ms(self) -> float:
        """Get 95th percentile latency in milliseconds."""
        if not self.latency_samples:
            return 0.0
        sorted_latencies = sorted(self.latency_samples)
        index = int(len(sorted_latencies) * 0.95)
        return sorted_latencies[min(index, len(sorted_latencies) - 1)]

    def get_drop_rate(self) -> float:
        """
        Get frame drop rate.

        Returns:
            Drop rate as percentage (0-100)
        """
        total = self.frames_sent + self.frames_dropped
        if total == 0:
            return 0.0
        return (self.frames_dropped / total) * 100.0

    def get_uptime_seconds(self) -> float:
        """Get uptime in seconds."""
        return time.time() - self._start_time

    def reset(self) -> None:
        """Reset all statistics."""
        self.frames_captured = 0
        self.frames_encoded = 0
        self.frames_sent = 0
        self.frames_dropped = 0
        self.capture_time_total = 0.0
        self.encode_time_total = 0.0
        self.bytes_sent = 0
        self.bitrate_window.clear()
        self.latency_samples.clear()
        self.fps_window.clear()
        self.last_frame_time = 0.0
        self._start_time = time.time()

    def get_summary(self) -> dict:
        """
        Get a summary of all statistics.

        Returns:
            Dictionary with all stats
        """
        return {
            "fps": self.get_fps(),
            "target_fps": self.get_target_fps(),
            "bitrate_kbps": self.get_bitrate_kbps(),
            "avg_capture_time_ms": self.get_avg_capture_time_ms(),
            "avg_encode_time_ms": self.get_avg_encode_time_ms(),
            "avg_latency_ms": self.get_avg_latency_ms(),
            "p95_latency_ms": self.get_p95_latency_ms(),
            "drop_rate": self.get_drop_rate(),
            "frames_captured": self.frames_captured,
            "frames_encoded": self.frames_encoded,
            "frames_sent": self.frames_sent,
            "frames_dropped": self.frames_dropped,
            "uptime_seconds": self.get_uptime_seconds(),
        }

    def log_summary(self) -> None:
        """Log a summary of statistics."""
        summary = self.get_summary()
        logger.info(
            f"Stats: {summary['fps']:.1f} fps, "
            f"{summary['bitrate_kbps']:.0f} kbps, "
            f"{summary['avg_encode_time_ms']:.1f} ms encode, "
            f"{summary['drop_rate']:.1f}% drop"
        )


class LatencyTimer:
    """Context manager for measuring latency."""

    def __init__(self, stats: Optional[PerformanceStats] = None):
        """
        Initialize timer.

        Args:
            stats: PerformanceStats to record latency to
        """
        self.stats = stats
        self._start_time: Optional[float] = None

    def __enter__(self):
        self._start_time = time.perf_counter()
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        if self._start_time is not None and self.stats is not None:
            elapsed_ms = (time.perf_counter() - self._start_time) * 1000.0
            self.stats.record_latency(elapsed_ms)
        return False

    @property
    def elapsed_ms(self) -> float:
        """Get elapsed time in milliseconds."""
        if self._start_time is None:
            return 0.0
        return (time.perf_counter() - self._start_time) * 1000.0
