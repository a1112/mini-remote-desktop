"""Multi-protocol support for remote desktop connections."""

from .base import ProtocolHandler, FrameCallback, StatsCallback
from .manager import ProtocolManager, ProtocolConfig

__all__ = [
    "ProtocolHandler",
    "FrameCallback",
    "StatsCallback",
    "ProtocolManager",
    "ProtocolConfig",
]
