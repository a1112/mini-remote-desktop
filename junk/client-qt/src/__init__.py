"""
Multi-Protocol Remote Desktop Client (Qt)

A Qt-based remote desktop viewer supporting multiple protocols:
- WebRTC (via aiortc)
- QUIC (via aioquic)
- JPEG Streaming (native)

Compatible with mini-remote-desktop agent-rust and agent-python.
"""

__version__ = "0.1.0"
__author__ = "Claude"

from .protocols.manager import ProtocolManager
from .signaling.client import SignalingClient
from .core.stats import Stats

__all__ = [
    "ProtocolManager",
    "SignalingClient",
    "Stats",
]
