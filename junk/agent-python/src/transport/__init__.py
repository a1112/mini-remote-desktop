"""
Transport layer for multi-protocol support.

Provides abstract interface and implementations for different transport protocols:
- QUIC: High-performance UDP-based protocol
- WebRTC: Browser-compatible peer-to-peer protocol
"""

from .base import TransportAdapter, TransportStats
from .manager import TransportManager, TransportConfig

__all__ = [
    "TransportAdapter",
    "TransportStats",
    "TransportManager",
    "TransportConfig",
]
