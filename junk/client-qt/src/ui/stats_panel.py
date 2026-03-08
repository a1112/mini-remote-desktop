"""
Statistics panel for displaying connection metrics.

Shows real-time statistics like latency, bitrate, FPS, packet loss.
"""

import logging
from typing import Optional

from PySide6.QtCore import Qt, QTimer
from PySide6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel,
    QFrame, QGridLayout, QSizePolicy
)
from PySide6.QtGui import QFont

from ..core.stats import ConnectionStats, ConnectionState

logger = logging.getLogger(__name__)


class StatsPanel(QWidget):
    """
    Panel displaying real-time connection statistics.

    Shows:
    - Connection state
    - Protocol used
    - Latency (RTT)
    - Bitrate (Mbps)
    - Frame rate (FPS)
    - Packet loss (%)
    - Resolution
    - Connection uptime
    """

    def __init__(self, parent=None):
        """Initialize stats panel."""
        super().__init__(parent)

        self._stats: Optional[ConnectionStats] = None
        self._update_interval = 1000  # ms

        self._setup_ui()

        # Update timer
        self._update_timer = QTimer()
        self._update_timer.timeout.connect(self._update_display)

    def _setup_ui(self) -> None:
        """Setup UI components."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(8, 8, 8, 8)
        layout.setSpacing(4)

        # Header
        header = self._create_header()
        layout.addWidget(header)

        # Stats grid
        self._stats_grid = self._create_stats_grid()
        layout.addWidget(self._stats_grid)

        # Initial state
        self._set_disconnected_state()

    def _create_header(self) -> QLabel:
        """Create header label."""
        label = QLabel("Connection Statistics")
        label.setStyleSheet("""
            QLabel {
                font-size: 14px;
                font-weight: 600;
                color: #e0e0e0;
                padding: 4px 0;
            }
        """)
        return label

    def _create_stats_grid(self) -> QFrame:
        """Create statistics grid layout."""
        frame = QFrame()
        frame.setStyleSheet("""
            QFrame {
                background-color: #2a2a2a;
                border: 1px solid #3a3a3a;
                border-radius: 4px;
            }
        """)

        layout = QGridLayout(frame)
        layout.setContentsMargins(12, 8, 12, 8)
        layout.setSpacing(8)

        # Create stat items
        self._state_label = self._create_stat_item("State", "Disconnected", "icon")
        self._protocol_label = self._create_stat_item("Protocol", "-", "")
        self._latency_label = self._create_stat_item("Latency", "-", "ms")
        self._bitrate_label = self._create_stat_item("Bitrate", "-", "Mbps")
        self._fps_label = self._create_stat_item("FPS", "-", "")
        self._packet_loss_label = self._create_stat_item("Loss", "-", "%")
        self._resolution_label = self._create_stat_item("Resolution", "-", "")
        self._uptime_label = self._create_stat_item("Uptime", "-", "s")

        # Add to grid (2 columns)
        layout.addWidget(self._state_label, 0, 0)
        layout.addWidget(self._protocol_label, 0, 1)
        layout.addWidget(self._latency_label, 1, 0)
        layout.addWidget(self._bitrate_label, 1, 1)
        layout.addWidget(self._fps_label, 2, 0)
        layout.addWidget(self._packet_loss_label, 2, 1)
        layout.addWidget(self._resolution_label, 3, 0)
        layout.addWidget(self._uptime_label, 3, 1)

        return frame

    def _create_stat_item(self, title: str, value: str, unit: str) -> QLabel:
        """Create a stat item label."""
        text = f"{title}: {value} {unit}".strip()
        label = QLabel(text)
        label.setStyleSheet("""
            QLabel {
                font-size: 11px;
                color: #b0b0b0;
                padding: 2px;
            }
        """)
        return label

    def _set_disconnected_state(self) -> None:
        """Set all stats to disconnected state."""
        self._update_label(self._state_label, "State", "Disconnected", "")
        self._update_label(self._protocol_label, "Protocol", "-", "")
        self._update_label(self._latency_label, "Latency", "-", "ms")
        self._update_label(self._bitrate_label, "Bitrate", "-", "Mbps")
        self._update_label(self._fps_label, "FPS", "-", "")
        self._update_label(self._packet_loss_label, "Loss", "-", "%")
        self._update_label(self._resolution_label, "Resolution", "-", "")
        self._update_label(self._uptime_label, "Uptime", "-", "s")

    def _update_label(self, label: QLabel, title: str, value: str, unit: str) -> None:
        """Update a stat label."""
        text = f"{title}: {value} {unit}".strip()
        label.setText(text)

        # Color code based on value
        if title == "State":
            if value == "Connected":
                label.setStyleSheet("color: #4ade80;")  # Green
            elif value == "Connecting":
                label.setStyleSheet("color: #fbbf24;")  # Yellow
            elif value == "Failed":
                label.setStyleSheet("color: #f87171;")  # Red
            else:
                label.setStyleSheet("color: #9ca3af;")  # Gray
        elif title == "Loss" and value != "-":
            try:
                loss_val = float(value)
                if loss_val > 5:
                    label.setStyleSheet("color: #f87171;")  # Red
                elif loss_val > 1:
                    label.setStyleSheet("color: #fbbf24;")  # Yellow
                else:
                    label.setStyleSheet("color: #4ade80;")  # Green
            except ValueError:
                label.setStyleSheet("color: #b0b0b0;")
        elif title == "Latency" and value != "-":
            try:
                lat_val = float(value)
                if lat_val > 200:
                    label.setStyleSheet("color: #f87171;")  # Red
                elif lat_val > 100:
                    label.setStyleSheet("color: #fbbf24;")  # Yellow
                else:
                    label.setStyleSheet("color: #4ade80;")  # Green
            except ValueError:
                label.setStyleSheet("color: #b0b0b0;")
        else:
            label.setStyleSheet("color: #b0b0b0;")

    def set_stats(self, stats: ConnectionStats) -> None:
        """
        Set the connection statistics to display.

        Args:
            stats: ConnectionStats object
        """
        self._stats = stats
        self._update_timer.start(self._update_interval)
        self._update_display()

    def _update_display(self) -> None:
        """Update the display with current stats."""
        if self._stats is None:
            self._set_disconnected_state()
            return

        # State
        state_text = self._stats.state.value.title()
        self._update_label(self._state_label, "State", state_text, "")

        # Protocol
        protocol = self._stats.protocol if self._stats.protocol else "-"
        self._update_label(self._protocol_label, "Protocol", protocol.upper(), "")

        # Latency
        latency = f"{self._stats.latency_ms:.0f}" if self._stats.latency_ms > 0 else "-"
        self._update_label(self._latency_label, "Latency", latency, "ms")

        # Bitrate
        bitrate = f"{self._stats.bitrate_mbps:.1f}" if self._stats.bitrate_mbps > 0 else "-"
        self._update_label(self._bitrate_label, "Bitrate", bitrate, "Mbps")

        # FPS
        fps = f"{self._stats.fps:.0f}" if self._stats.fps > 0 else "-"
        self._update_label(self._fps_label, "FPS", fps, "")

        # Packet loss
        loss = f"{self._stats.packet_loss:.1f}" if self._stats.packet_loss > 0 else "0.0"
        self._update_label(self._packet_loss_label, "Loss", loss, "%")

        # Resolution
        if self._stats.width > 0 and self._stats.height > 0:
            resolution = f"{self._stats.width}x{self._stats.height}"
        else:
            resolution = "-"
        self._update_label(self._resolution_label, "Resolution", resolution, "")

        # Uptime
        uptime = self._stats.uptime_seconds
        if uptime > 0:
            if uptime >= 3600:
                uptime_text = f"{uptime / 3600:.1f}h"
            elif uptime >= 60:
                uptime_text = f"{uptime / 60:.0f}m"
            else:
                uptime_text = f"{uptime:.0f}s"
        else:
            uptime_text = "-"
        self._update_label(self._uptime_label, "Uptime", uptime_text, "")

    def clear_stats(self) -> None:
        """Clear statistics and stop updates."""
        self._stats = None
        self._update_timer.stop()
        self._set_disconnected_state()
