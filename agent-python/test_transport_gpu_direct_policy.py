#!/usr/bin/env python3
"""
Tests for GPU-direct transport policy.

These tests lock the expected routing behavior for the short pipeline:
QUIC-only transport when WebRTC fallback is explicitly disabled.
"""

import sys
from pathlib import Path

import pytest

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))

from src.transport.manager import create_transport_manager


def test_create_transport_manager_can_disable_webrtc_fallback():
    """Manager should support explicit no-WebRTC mode for GPU-direct routing."""
    manager = create_transport_manager(
        preferred="quic",
        fallback="quic",
        auto_switch=False,
        allow_webrtc_fallback=False,
    )
    assert "webrtc" not in manager.available_protocols


def test_auto_mode_stays_quic_only_when_webrtc_fallback_disabled(monkeypatch: pytest.MonkeyPatch):
    """Even if WebRTC is available, strict mode should keep protocol list QUIC-only."""
    import src.transport.manager as manager_module

    monkeypatch.setattr(manager_module, "is_quic_available", lambda: True)
    monkeypatch.setattr(manager_module, "is_webrtc_available", lambda: True)

    manager = create_transport_manager(
        preferred="auto",
        fallback="quic",
        auto_switch=False,
        allow_webrtc_fallback=False,
    )

    assert manager.available_protocols == ["quic"]
