#!/usr/bin/env python3
"""
Protocol switching test.

Tests TransportManager's ability to switch between protocols.
"""

import asyncio
import json
import logging
import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.transport.manager import (
    TransportManager,
    TransportConfig,
    ProtocolType,
    create_transport_manager
)
from src.transport.stats import FrameInfo

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


class MockTransportAdapter:
    """Mock transport adapter for testing."""

    def __init__(self, name: str, fail_connect: bool = False):
        self.name = name
        self._fail_connect = fail_connect
        self._connected = False
        self._stats_sent = 0

    @property
    def is_connected(self):
        return self._connected

    @property
    def stats(self):
        from src.transport.stats import TransportStats
        stats = TransportStats(protocol=self.name)
        stats.is_connected = self._connected
        stats.packets_sent = self._stats_sent
        return stats

    async def connect(self, offer: str, metadata=None):
        if self._fail_connect:
            raise Exception(f"{self.name} connection failed")
        self._connected = True
        return f"{self.name}_answer"

    async def send_media(self, frame: FrameInfo):
        if not self._connected:
            raise Exception("Not connected")
        self._stats_sent += 1

    async def disconnect(self):
        self._connected = False

    def on(self, event, handler):
        pass


async def test_transport_config():
    """Test TransportConfig creation."""
    logger.info("Testing TransportConfig...")

    try:
        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            fallback=ProtocolType.WEBRTC,
            auto_switch=True,
            rtt_threshold_ms=100.0,
        )

        logger.info(f"✓ TransportConfig created:")
        logger.info(f"  - Preferred: {config.preferred}")
        logger.info(f"  - Fallback: {config.fallback}")
        logger.info(f"  - Auto-switch: {config.auto_switch}")
        logger.info(f"  - RTT threshold: {config.rtt_threshold_ms}ms")

        return True

    except Exception as e:
        logger.error(f"✗ TransportConfig test failed: {e}")
        return False


async def test_manager_creation():
    """Test TransportManager creation."""
    logger.info("Testing TransportManager creation...")

    try:
        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            fallback=ProtocolType.WEBRTC,
            auto_switch=True,
        )

        manager = TransportManager(config)

        logger.info(f"✓ TransportManager created:")
        logger.info(f"  - Available protocols: {manager.available_protocols}")

        return True

    except Exception as e:
        logger.error(f"✗ TransportManager creation failed: {e}")
        return False


async def test_create_helper():
    """Test create_transport_manager helper function."""
    logger.info("Testing create_transport_manager helper...")

    try:
        manager = create_transport_manager(
            preferred="auto",
            auto_switch=True,
        )

        logger.info(f"✓ Manager created via helper:")
        logger.info(f"  - Available: {manager.available_protocols}")

        return True

    except Exception as e:
        logger.error(f"✗ Helper function failed: {e}")
        return False


async def test_protocol_selection():
    """Test protocol preference handling."""
    logger.info("Testing protocol selection...")

    try:
        # Test QUIC preference
        config_quic = TransportConfig(preferred=ProtocolType.QUIC)
        manager_quic = TransportManager(config_quic)
        logger.info(f"✓ QUIC preferred: {manager_quic.available_protocols}")

        # Test WebRTC preference
        config_webrtc = TransportConfig(preferred=ProtocolType.WEBRTC)
        manager_webrtc = TransportManager(config_webrtc)
        logger.info(f"✓ WebRTC preferred: {manager_webrtc.available_protocols}")

        return True

    except Exception as e:
        logger.error(f"✗ Protocol selection test failed: {e}")
        return False


async def test_stats_dict():
    """Test stats dictionary output."""
    logger.info("Testing stats dictionary...")

    try:
        from src.transport.stats import TransportStats

        stats = TransportStats(protocol="test")
        stats.packets_sent = 1000
        stats.packets_lost = 5
        stats.update_packet_loss()
        stats.update_rtt(42.5)

        stats_dict = stats.to_dict()

        logger.info(f"✓ Stats dict created:")
        for key, value in stats_dict.items():
            logger.info(f"  - {key}: {value}")

        return True

    except Exception as e:
        logger.error(f"✗ Stats dict test failed: {e}")
        return False


async def test_offer_parsing():
    """Test offer parsing logic."""
    logger.info("Testing offer parsing...")

    try:
        # Test QUIC offer
        quic_offer = json.dumps({
            "protocol": "quic",
            "host": "127.0.0.1",
            "port": 4433
        })

        config = TransportConfig()
        manager = TransportManager(config)

        proto = manager._parse_offer_protocol(quic_offer)
        logger.info(f"✓ QUIC offer parsed: {proto}")

        # Test WebRTC offer (SDP - not JSON)
        webrtc_offer = "v=0\r\no=- 123456 2 IN IP4 127.0.0.1\r\n..."
        proto = manager._parse_offer_protocol(webrtc_offer)
        logger.info(f"✓ WebRTC offer parsed: {proto}")

        return True

    except Exception as e:
        logger.error(f"✗ Offer parsing test failed: {e}")
        return False


async def test_switch_cooldown():
    """Test protocol switch cooldown logic."""
    logger.info("Testing switch cooldown...")

    try:
        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            switch_cooldown=1.0,  # 1 second cooldown
            max_switches=3,
        )

        manager = TransportManager(config)

        # Initially, switch should be allowed
        can_switch_1 = manager._can_switch()
        logger.info(f"✓ Initial can_switch: {can_switch_1}")

        # Simulate a switch
        manager._last_switch_time = asyncio.get_event_loop().time()
        manager._switch_count += 1

        # Immediately after, switch should not be allowed (cooldown)
        can_switch_2 = manager._can_switch()
        logger.info(f"✓ After switch can_switch: {can_switch_2}")

        # Wait for cooldown
        await asyncio.sleep(1.1)

        can_switch_3 = manager._can_switch()
        logger.info(f"✓ After cooldown can_switch: {can_switch_3}")

        return True

    except Exception as e:
        logger.error(f"✗ Switch cooldown test failed: {e}")
        return False


async def test_frame_creation():
    """Test FrameInfo with various data."""
    logger.info("Testing FrameInfo creation...")

    try:
        # Test keyframe
        keyframe = FrameInfo(
            data=b'\x00\x00\x00\x01\x65' * 50,  # IDR NAL
            timestamp=1000,
            is_keyframe=True,
            width=1920,
            height=1080,
            frame_number=0
        )
        logger.info(f"✓ Keyframe: {keyframe.size} bytes")

        # Test regular frame
        frame = FrameInfo(
            data=b'\x00\x00\x00\x01\x41' * 30,  # P-frame NAL
            timestamp=1033,
            is_keyframe=False,
            width=1920,
            height=1080,
            frame_number=1
        )
        logger.info(f"✓ P-frame: {frame.size} bytes")

        return True

    except Exception as e:
        logger.error(f"✗ Frame creation test failed: {e}")
        return False


async def run_all_tests():
    """Run all protocol switching tests."""
    tests = [
        ("TransportConfig", test_transport_config),
        ("TransportManager Creation", test_manager_creation),
        ("Create Helper", test_create_helper),
        ("Protocol Selection", test_protocol_selection),
        ("Stats Dictionary", test_stats_dict),
        ("Offer Parsing", test_offer_parsing),
        ("Switch Cooldown", test_switch_cooldown),
        ("Frame Creation", test_frame_creation),
    ]

    results = []

    for name, test in tests:
        print()
        logger.info(f"{'='*60}")
        logger.info(f"Test: {name}")
        logger.info(f"{'='*60}")

        try:
            result = await test()
            results.append((name, result))
        except Exception as e:
            logger.error(f"Test '{name}' crashed: {e}")
            import traceback
            traceback.print_exc()
            results.append((name, False))

    # Summary
    print()
    logger.info(f"{'='*60}")
    logger.info("Test Summary")
    logger.info(f"{'='*60}")

    passed = sum(1 for _, result in results if result)
    total = len(results)

    for name, result in results:
        status = "✓ PASS" if result else "✗ FAIL"
        logger.info(f"{status}: {name}")

    logger.info(f"{'='*60}")
    logger.info(f"Results: {passed}/{total} tests passed")
    logger.info(f"{'='*60}")

    return passed == total


if __name__ == "__main__":
    success = asyncio.run(run_all_tests())
    sys.exit(0 if success else 1)
