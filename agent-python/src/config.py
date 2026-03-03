"""
Configuration management for agent-python.

Compatible with agent-rust configuration format.
"""

import json
import platform
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


@dataclass
class CaptureConfig:
    """Capture configuration matching agent-rust CaptureConfig."""

    fps: int = 30
    backend: str = "d3dshot"
    encoder: str = "software"
    target_width: int = 1920
    target_height: int = 1080
    queue_depth: int = 8
    gop: int = 60
    bitrate_kbps: int = 5000
    max_bitrate_kbps: int = 8000
    min_fps: int = 15
    max_fps: int = 60
    frame_pacing_enable: bool = True
    frame_pacing_batch_packets: int = 6
    force_idr_on_pli: bool = True
    idr_interval_sec: int = 2
    rtp_use_manual_packetizer: bool = True
    rtp_mtu: int = 1200
    network_adapt_enable: bool = True
    network_adapt_floor_bitrate_kbps: int = 2000
    network_adapt_ceiling_bitrate_kbps: int = 10000
    stats_interval_ms: int = 1000


@dataclass
class AgentConfig:
    """Main agent configuration matching agent-rust AgentConfig."""

    ws_url: str = "ws://127.0.0.1:9527"
    device_name: str = "Python Agent"
    capture: CaptureConfig = field(default_factory=CaptureConfig)

    @classmethod
    def from_file(cls, path: Optional[Path] = None) -> "AgentConfig":
        """
        Load configuration from JSON file.

        Returns default config if file doesn't exist or parsing fails.
        """
        if path is None:
            # Default to config.json in current directory
            path = Path("config.json")

        # Try to read and parse the config file
        try:
            with open(path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except (FileNotFoundError, json.JSONDecodeError):
            # Return default config
            config = cls()
            # Auto-detect hostname for device name
            if config.device_name == "Python Agent":
                hostname = platform.node()
                if hostname and hostname.strip():
                    config.device_name = f"{hostname} - Python Agent"
            return config

        # Merge with defaults
        config = cls()

        if "ws_url" in data:
            config.ws_url = data["ws_url"]
        if "device_name" in data:
            config.device_name = data["device_name"]

        # Auto-detect hostname if still default
        if config.device_name == "Python Agent":
            hostname = platform.node()
            if hostname and hostname.strip():
                config.device_name = f"{hostname} - Python Agent"

        # Merge capture config
        if "capture" in data and isinstance(data["capture"], dict):
            capture_data = data["capture"]
            for key, value in capture_data.items():
                if hasattr(config.capture, key):
                    setattr(config.capture, key, value)

        # Normalize and validate
        _normalize_config(config)

        return config


def _normalize_config(config: AgentConfig) -> None:
    """Normalize and validate configuration values."""

    # Normalize string values to lowercase
    config.capture.backend = config.capture.backend.lower()
    config.capture.encoder = config.capture.encoder.lower()

    # Clamp numeric values to valid ranges
    config.capture.fps = max(1, min(240, config.capture.fps))
    config.capture.target_width = max(0, min(7680, config.capture.target_width))
    config.capture.target_height = max(0, min(4320, config.capture.target_height))
    config.capture.queue_depth = max(1, min(64, config.capture.queue_depth))
    config.capture.gop = max(1, min(600, config.capture.gop))
    config.capture.bitrate_kbps = max(100, min(200_000, config.capture.bitrate_kbps))
    config.capture.max_bitrate_kbps = max(100, min(300_000, config.capture.max_bitrate_kbps))
    config.capture.min_fps = max(1, min(240, config.capture.min_fps))
    config.capture.max_fps = max(1, min(240, config.capture.max_fps))
    config.capture.frame_pacing_batch_packets = max(1, min(64, config.capture.frame_pacing_batch_packets))
    config.capture.idr_interval_sec = max(1, min(30, config.capture.idr_interval_sec))
    config.capture.rtp_mtu = max(576, min(1460, config.capture.rtp_mtu))
    config.capture.network_adapt_floor_bitrate_kbps = max(
        100, min(200_000, config.capture.network_adapt_floor_bitrate_kbps)
    )
    config.capture.network_adapt_ceiling_bitrate_kbps = max(
        100, min(300_000, config.capture.network_adapt_ceiling_bitrate_kbps)
    )
    config.capture.stats_interval_ms = max(200, min(10_000, config.capture.stats_interval_ms))

    # Ensure min <= max for fps and bitrate
    config.capture.min_fps = min(config.capture.min_fps, config.capture.max_fps)
    config.capture.network_adapt_floor_bitrate_kbps = min(
        config.capture.network_adapt_floor_bitrate_kbps,
        config.capture.network_adapt_ceiling_bitrate_kbps,
    )


def get_default_config() -> AgentConfig:
    """Get default configuration."""
    return AgentConfig.from_file()
