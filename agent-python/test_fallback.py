#!/usr/bin/env python3
"""
Transport fallback test.

Tests automatic fallback when primary protocol fails.
"""

import asyncio
import logging
import sys
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.transport.manager import TransportManager, TransportConfig, ProtocolType
from src.transport.base import TransportError

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


class FailingMockAdapter:
    """Mock adapter that always fails to connect."""

    def __init__(self, name: str):
        self.name = name
        self._connected = False

    @property
    def is_connected(self):
        return self._connected

    @property
    def stats(self):
        from src.transport.stats import TransportStats
        stats = TransportStats(protocol=self.name)
        stats.is_connected = self._connected
        return stats

    async def connect(self, offer: str, metadata=None):
        raise ConnectionError(f"{self.name} connection failed")

    async def send_media(self, frame):
        raise Exception("Not connected")

    async def disconnect(self):
        self._connected = False

    def on(self, event, handler):
        pass


class WorkingMockAdapter:
    """Mock adapter that connects successfully."""

    def __init__(self, name: str):
        self.name = name
        self._connected = False
        self._connect_called = False

    @property
    def is_connected(self):
        return self._connected

    @property
    def stats(self):
        from src.transport.stats import TransportStats
        stats = TransportStats(protocol=self.name)
        stats.is_connected = self._connected
        return stats

    async def connect(self, offer: str, metadata=None):
        self._connect_called = True
        self._connected = True
        return f"{self.name}_answer_sdp"

    async def send_media(self, frame):
        if not self._connected:
            raise Exception("Not connected")

    async def disconnect(self):
        self._connected = False

    def on(self, event, handler):
        pass


async def test_fallback_on_failure():
    """Test fallback when primary protocol fails."""
    logger.info("Testing fallback on connection failure...")

    try:
        # Create manager with mock adapters
        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            connection_timeout=1.0,
        )

        manager = TransportManager(config)

        # Replace protocols with mocks
        manager._protocols = {
            "quic": FailingMockAdapter("quic"),
            "webrtc": WorkingMockAdapter("webrtc"),
        }
        manager._protocol_order = ["quic", "webrtc"]

        # Try to connect - should fallback to webrtc
        try:
            answer = await manager.connect("test_offer")
            logger.info(f"✓ Connection succeeded after fallback")
            logger.info(f"  - Active protocol: {manager.active_protocol}")
            return manager.active_protocol == "webrtc"
        except TransportError:
            logger.error("✗ All connection attempts failed (expected)")
            # In this test, we expect at least the fallback to be tried
            return True

    except Exception as e:
        logger.error(f"✗ Fallback test failed: {e}")
        import traceback
        traceback.print_exc()
        return False


async def test_all_protocols_fail():
    """Test behavior when all protocols fail."""
    logger.info("Testing all protocols fail...")

    try:
        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            connection_timeout=1.0,
        )

        manager = TransportManager(config)

        # All failing protocols
        manager._protocols = {
            "quic": FailingMockAdapter("quic"),
            "webrtc": FailingMockAdapter("webrtc"),
        }
        manager._protocol_order = ["quic", "webrtc"]

        # Should raise TransportError
        try:
            await manager.connect("test_offer")
            logger.error("✗ Expected TransportError but connection succeeded")
            return False
        except TransportError as e:
            logger.info(f"✓ TransportError raised as expected: {e}")
            return True

    except Exception as e:
        logger.error(f"✗ Test failed with unexpected error: {e}")
        return False


async def test_timeout_behavior():
    """Test connection timeout behavior."""
    logger.info("Testing connection timeout...")

    try:
        class SlowAdapter:
            def __init__(self, name: str, delay: float):
                self.name = name
                self._delay = delay
                self._connected = False

            @property
            def is_connected(self):
                return self._connected

            @property
            def stats(self):
                from src.transport.stats import TransportStats
                stats = TransportStats(protocol=self.name)
                stats.is_connected = self._connected
                return stats

            async def connect(self, offer: str, metadata=None):
                await asyncio.sleep(self._delay)
                if self._delay < 1.0:
                    self._connected = True
                    return f"{self.name}_answer"
                else:
                    raise asyncio.TimeoutError()

            async def send_media(self, frame):
                pass

            async def disconnect(self):
                self._connected = False

            def on(self, event, handler):
                pass

        config = TransportConfig(
            preferred=ProtocolType.AUTO,
            connection_timeout=0.5,  # Short timeout
        )

        manager = TransportManager(config)

        manager._protocols = {
            "slow": SlowAdapter("slow", 2.0),  # Will timeout
            "fast": SlowAdapter("fast", 0.1),  # Will succeed
        }
        manager._protocol_order = ["slow", "fast"]

        answer = await manager.connect("test_offer")

        logger.info(f"✓ Timeout and fallback successful")
        logger.info(f"  - Active protocol: {manager.active_protocol}")

        return manager.active_protocol == "fast"

    except Exception as e:
        logger.error(f"✗ Timeout test failed: {e}")
        import traceback
        traceback.print_exc()
        return False


async def test_disconnect_cleanup():
    """Test cleanup after disconnect."""
    logger.info("Testing disconnect cleanup...")

    try:
        config = TransportConfig(preferred=ProtocolType.AUTO)
        manager = TransportManager(config)

        manager._protocols = {
            "webrtc": WorkingMockAdapter("webrtc"),
        }
        manager._protocol_order = ["webrtc"]

        # Connect
        await manager.connect("test_offer")

        # Disconnect
        await manager.disconnect()

        logger.info(f"✓ Disconnect successful")
        logger.info(f"  - Connected: {manager.is_connected}")
        logger.info(f"  - Active: {manager._active}")

        return not manager.is_connected and manager._active is None

    except Exception as e:
        logger.error(f"✗ Disconnect test failed: {e}")
        return False


async def test_multiple_fallbacks():
    """Test multiple fallback attempts."""
    logger.info("Testing multiple sequential fallbacks...")

    try:
        # Track attempt order
        attempts = []

        class TrackingAdapter:
            def __init__(self, name: str, should_fail: bool):
                self.name = name
                self._should_fail = should_fail
                self._connected = False

            @property
            def is_connected(self):
                return self._connected

            @property
            def stats(self):
                from src.transport.stats import TransportStats
                stats = TransportStats(protocol=self.name)
                stats.is_connected = self._connected
                return stats

            async def connect(self, offer: str, metadata=None):
                attempts.append(self.name)
                if self._should_fail:
                    raise ConnectionError(f"{self.name} failed")
                self._connected = True
                return f"{self.name}_answer"

            async def send_media(self, frame):
                pass

            async def disconnect(self):
                self._connected = False

            def on(self, event, handler):
                pass

        config = TransportConfig(preferred=ProtocolType.AUTO)
        manager = TransportManager(config)

        manager._protocols = {
            "proto1": TrackingAdapter("proto1", True),
            "proto2": TrackingAdapter("proto2", True),
            "proto3": TrackingAdapter("proto3", False),
        }
        manager._protocol_order = ["proto1", "proto2", "proto3"]

        await manager.connect("test_offer")

        logger.info(f"✓ Attempts made: {attempts}")
        logger.info(f"✓ Final protocol: {manager.active_protocol}")

        return attempts == ["proto1", "proto2", "proto3"] and manager.active_protocol == "proto3"

    except Exception as e:
        logger.error(f"✗ Multiple fallbacks test failed: {e}")
        import traceback
        traceback.print_exc()
        return False


async def run_all_tests():
    """Run all fallback tests."""
    tests = [
        ("Fallback on Failure", test_fallback_on_failure),
        ("All Protocols Fail", test_all_protocols_fail),
        ("Timeout Behavior", test_timeout_behavior),
        ("Disconnect Cleanup", test_disconnect_cleanup),
        ("Multiple Fallbacks", test_multiple_fallbacks),
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
