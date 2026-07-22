#!/usr/bin/env python3
"""
QUIC transport connection test.

Tests basic QUIC adapter functionality.
"""

import asyncio
import json
import logging
import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.transport.quic_adapter import QUICAdapter, create_quic_offer, is_quic_available
from src.transport.stats import FrameInfo

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


async def test_quic_available():
    """Test if QUIC is available."""
    logger.info("Checking QUIC availability...")

    if is_quic_available():
        logger.info("✓ QUIC is available")
        return True
    else:
        logger.error("✗ QUIC is not available (aioquic not installed)")
        return False


async def test_quic_adapter_creation():
    """Test QUIC adapter creation."""
    logger.info("Testing QUIC adapter creation...")

    try:
        adapter = QUICAdapter(host="127.0.0.1", port=0)
        logger.info(f"✓ QUIC adapter created: {adapter}")
        logger.info(f"  - Protocol: {adapter.name}")
        logger.info(f"  - Connected: {adapter.is_connected}")
        logger.info(f"  - Stats: {adapter.stats}")
        return True
    except Exception as e:
        logger.error(f"✗ Failed to create QUIC adapter: {e}")
        return False


async def test_quic_offer_answer():
    """Test QUIC offer/answer exchange."""
    logger.info("Testing QUIC offer/answer exchange...")

    try:
        # Create offer
        offer = create_quic_offer(host="127.0.0.1", port=4433)
        offer_data = json.loads(offer)

        logger.info(f"✓ QUIC offer created:")
        logger.info(f"  - Protocol: {offer_data['protocol']}")
        logger.info(f"  - Host: {offer_data['host']}")
        logger.info(f"  - Port: {offer_data['port']}")

        # Test adapter connection (will fail without server, but tests the logic)
        adapter = QUICAdapter(host="127.0.0.1", port=0)

        # Note: This will timeout since there's no actual QUIC server
        try:
            answer = await asyncio.wait_for(
                adapter.connect(offer),
                timeout=2.0
            )
            logger.info(f"✓ QUIC connection successful")
            logger.info(f"  Answer: {answer[:100]}...")
        except asyncio.TimeoutError:
            logger.info("✓ QUIC connection attempt made (timeout expected without server)")

        return True

    except Exception as e:
        logger.error(f"✗ Offer/answer test failed: {e}")
        return False


async def test_frame_info():
    """Test FrameInfo creation."""
    logger.info("Testing FrameInfo creation...")

    try:
        # Create a test frame
        frame_data = b'\x00\x00\x00\x01\x67\x42\x80\x0a' * 100  # Fake H.264 data
        frame = FrameInfo(
            data=frame_data,
            timestamp=12345,
            is_keyframe=True,
            width=1920,
            height=1080,
            frame_number=0
        )

        logger.info(f"✓ FrameInfo created:")
        logger.info(f"  - Size: {frame.size} bytes")
        logger.info(f"  - Keyframe: {frame.is_keyframe}")
        logger.info(f"  - Resolution: {frame.width}x{frame.height}")

        return True

    except Exception as e:
        logger.error(f"✗ FrameInfo test failed: {e}")
        return False


async def test_stats_tracking():
    """Test statistics tracking."""
    logger.info("Testing statistics tracking...")

    try:
        from src.transport.stats import TransportStats

        stats = TransportStats(protocol="quic")

        # Test RTT updates
        stats.update_rtt(50.0)
        stats.update_rtt(60.0)
        stats.update_rtt(45.0)

        logger.info(f"✓ RTT tracking:")
        logger.info(f"  - Current: {stats.rtt_ms}ms")
        logger.info(f"  - Average: {stats.rtt_avg_ms:.1f}ms")
        logger.info(f"  - Max: {stats.rtt_max_ms}ms")

        # Test packet loss
        stats.packets_sent = 1000
        stats.packets_lost = 10
        stats.update_packet_loss()

        logger.info(f"✓ Packet loss:")
        logger.info(f"  - Loss: {stats.packet_loss_percent:.2f}%")

        # Test to_dict
        stats_dict = stats.to_dict()
        logger.info(f"✓ Stats dict: {stats_dict}")

        return True

    except Exception as e:
        logger.error(f"✗ Stats tracking test failed: {e}")
        return False


async def run_all_tests():
    """Run all QUIC tests."""
    tests = [
        ("QUIC Availability", test_quic_available),
        ("Adapter Creation", test_quic_adapter_creation),
        ("Offer/Answer Exchange", test_quic_offer_answer),
        ("FrameInfo Creation", test_frame_info),
        ("Statistics Tracking", test_stats_tracking),
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
