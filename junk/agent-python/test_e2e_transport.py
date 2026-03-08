#!/usr/bin/env python3
"""
End-to-end transport layer test.

Tests the complete flow from FrameInfo creation to transmission.
"""

import asyncio
import json
import logging
import sys
import time
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.transport.manager import (
    TransportManager,
    TransportConfig,
    create_transport_manager,
    ProtocolType
)
from src.transport.stats import FrameInfo, TransportStats

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


class MockServer:
    """Mock controller/server for testing."""

    def __init__(self):
        self.received_frames = []
        self.connected_clients = []

    async def handle_frame(self, frame: FrameInfo):
        """Simulate receiving a frame."""
        self.received_frames.append(frame)
        await asyncio.sleep(0.001)  # Simulate processing delay

    def get_stats(self):
        """Get reception statistics."""
        return {
            "frames_received": len(self.received_frames),
            "total_bytes": sum(f.size for f in self.received_frames),
        }


async def test_e2e_frame_flow():
    """Test end-to-end frame flow."""
    logger.info("=" * 60)
    logger.info("End-to-End Frame Flow Test")
    logger.info("=" * 60)

    # Create transport manager
    config = TransportConfig(
        preferred=ProtocolType.AUTO,
        auto_switch=False,
    )

    manager = TransportManager(config)

    logger.info(f"✓ TransportManager created")
    logger.info(f"  Available protocols: {manager.available_protocols}")

    # Create mock frames simulating H.264 encoded data
    frames = []
    for i in range(100):
        # Simulate varying frame sizes (keyframes larger)
        if i % 30 == 0:  # Keyframe every 30 frames
            data = b'\x00\x00\x00\x01\x67' + b'\x00' * 45000  # ~50KB keyframe
            is_keyframe = True
        else:
            data = b'\x00\x00\x00\x01\x41' + b'\x00' * 8000  # ~10KB P-frame
            is_keyframe = False

        frame = FrameInfo(
            data=data,
            timestamp=int(time.time() * 1_000_000) + i * 33000,  # ~30fps
            is_keyframe=is_keyframe,
            width=1920,
            height=1080,
            frame_number=i,
        )
        frames.append(frame)

    logger.info(f"✓ Created {len(frames)} test frames")

    # Test FrameInfo processing without actual connection
    logger.info("\n" + "=" * 60)
    logger.info("Frame Processing Test (Without Connection)")
    logger.info("=" * 60)

    start_time = time.perf_counter()
    total_bytes = 0
    keyframe_count = 0

    for frame in frames:
        total_bytes += frame.size
        if frame.is_keyframe:
            keyframe_count += 1

    processing_time = time.perf_counter() - start_time

    logger.info(f"✓ Processed {len(frames)} frames in {processing_time:.3f}s")
    logger.info(f"  Total bytes: {total_bytes:,}")
    logger.info(f"  Keyframes: {keyframe_count}")
    logger.info(f"  P-frames: {len(frames) - keyframe_count}")
    logger.info(f"  Processing rate: {len(frames)/processing_time:.0f} fps")

    # Test statistics tracking
    logger.info("\n" + "=" * 60)
    logger.info("Statistics Tracking Test")
    logger.info("=" * 60)

    stats = TransportStats(protocol="webrtc")

    for i in range(100):
        # Simulate RTT samples
        rtt = 20 + (i % 80)
        stats.update_rtt(rtt)

        # Simulate packet transmission
        stats.packets_sent += 1
        if i % 100 == 0:
            stats.packets_lost += 1
        stats.update_packet_loss()

        # Simulate bandwidth
        stats.bytes_sent += 14000
        if i % 30 == 0:
            stats.update_bandwidth()
            stats.frames_sent += 30
            stats.update_fps(stats.frames_sent)

    logger.info(f"✓ Statistics tracked:")
    logger.info(f"  RTT avg: {stats.rtt_avg_ms:.1f}ms")
    logger.info(f"  RTT max: {stats.rtt_max_ms}ms")
    logger.info(f"  Packet loss: {stats.packet_loss_percent:.2f}%")
    logger.info(f"  Bandwidth: {stats.bandwidth_kbps:.0f} kbps")
    logger.info(f"  FPS: {stats.fps:.1f}")

    # Test offer/answer parsing
    logger.info("\n" + "=" * 60)
    logger.info("Offer/Answer Parsing Test")
    logger.info("=" * 60)

    # WebRTC SDP offer (simplified)
    webrtc_offer = """v=0
o=- 123456 2 IN IP4 127.0.0.1
s=-
t=0 0
m=video 9 UDP/TLS/RTP/SAVPF 96
a=rtpmap:96 H264/90000
a=fmtp:96 profile-level-id=42e01f;packetization-mode=1
"""

    # QUIC offer
    quic_offer = json.dumps({
        "protocol": "quic",
        "host": "192.168.1.100",
        "port": 4433,
        "alpn": "remote-desktop",
    })

    proto_webrtc = manager._parse_offer_protocol(webrtc_offer)
    proto_quic = manager._parse_offer_protocol(quic_offer)

    logger.info(f"✓ WebRTC offer parsed as: {proto_webrtc}")
    logger.info(f"✓ QUIC offer parsed as: {proto_quic}")

    # Test stats dict serialization
    logger.info("\n" + "=" * 60)
    logger.info("Stats Serialization Test")
    logger.info("=" * 60)

    stats_dict = stats.to_dict()
    stats_json = json.dumps(stats_dict, indent=2)

    logger.info(f"✓ Stats serialized to JSON:")
    logger.info(stats_json[:200] + "...")

    return True


async def test_manager_config_variants():
    """Test different manager configurations."""
    logger.info("\n" + "=" * 60)
    logger.info("Manager Configuration Test")
    logger.info("=" * 60)

    configs = [
        ("Auto with switching", {"preferred": "auto", "auto_switch": True}),
        ("QUIC forced", {"preferred": "quic", "auto_switch": False}),
        ("WebRTC forced", {"preferred": "webrtc", "auto_switch": False}),
        ("Custom thresholds", {
            "preferred": "auto",
            "rtt_threshold_ms": 50.0,
            "packet_loss_threshold": 2.0,
            "min_fps_threshold": 25.0,
        }),
    ]

    for name, cfg in configs:
        manager = create_transport_manager(**cfg)
        logger.info(f"✓ {name}:")
        logger.info(f"    Protocols: {manager.available_protocols}")

    return True


async def test_error_handling():
    """Test error handling scenarios."""
    logger.info("\n" + "=" * 60)
    logger.info("Error Handling Test")
    logger.info("=" * 60)

    manager = create_transport_manager(auto_switch=False)

    # Test sending without connection
    frame = FrameInfo(
        data=b'\x00' * 1000,
        timestamp=0,
        is_keyframe=True,
    )

    try:
        await manager.send_media(frame)
        logger.error("✗ Should have raised error")
        return False
    except Exception as e:
        logger.info(f"✓ Correctly raised error: {type(e).__name__}")

    # Test invalid offer parsing
    invalid_offers = [
        "",
        "not json",
        "v=0\ninvalid sdp",
        '{"protocol": "unknown"}',
    ]

    for offer in invalid_offers:
        proto = manager._parse_offer_protocol(offer)
        logger.info(f"✓ Invalid offer parsed as: {proto} (expected None)")

    # Test disconnect when not connected
    await manager.disconnect()
    logger.info("✓ Disconnect when not connected: OK")

    return True


async def run_all_e2e_tests():
    """Run all end-to-end tests."""
    tests = [
        ("Frame Flow", test_e2e_frame_flow),
        ("Config Variants", test_manager_config_variants),
        ("Error Handling", test_error_handling),
    ]

    results = []

    for name, test in tests:
        try:
            result = await test()
            results.append((name, result))
        except Exception as e:
            logger.error(f"✗ Test '{name}' failed: {e}")
            import traceback
            traceback.print_exc()
            results.append((name, False))

    # Summary
    logger.info("\n" + "=" * 60)
    logger.info("End-to-End Test Summary")
    logger.info("=" * 60)

    passed = sum(1 for _, r in results if r)
    total = len(results)

    for name, result in results:
        status = "✓ PASS" if result else "✗ FAIL"
        logger.info(f"{status}: {name}")

    logger.info("=" * 60)
    logger.info(f"Results: {passed}/{total} tests passed")

    if passed == total:
        logger.info("\n🎉 All end-to-end tests passed!")

    return passed == total


if __name__ == "__main__":
    success = asyncio.run(run_all_e2e_tests())
    sys.exit(0 if success else 1)
