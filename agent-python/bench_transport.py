#!/usr/bin/env python3
"""
Transport performance benchmark.

Compares performance metrics between different transport protocols.
"""

import asyncio
import logging
import sys
import time
from pathlib import Path
from typing import List

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.transport.stats import TransportStats, FrameInfo

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
)

logger = logging.getLogger(__name__)


class BenchmarkResults:
    """Benchmark results container."""

    def __init__(self):
        self.protocol = "unknown"
        self.frames_sent = 0
        self.bytes_sent = 0
        self.duration = 0.0
        self.fps = 0.0
        self.bandwidth_mbps = 0.0
        self.avg_latency_ms = 0.0
        self.errors = 0


async def benchmark_frame_creation(num_frames: int = 1000) -> BenchmarkResults:
    """Benchmark FrameInfo creation overhead."""
    logger.info(f"Benchmarking FrameInfo creation ({num_frames} frames)...")

    results = BenchmarkResults()
    results.protocol = "FrameInfo Creation"

    # Generate test frame data (simulating H.264 encoded frame)
    frame_sizes = [
        50_000,   # Keyframe
        10_000,   # P-frame
        8_000,    # P-frame
        12_000,   # P-frame
    ]

    start_time = time.perf_counter()

    for i in range(num_frames):
        size = frame_sizes[i % len(frame_sizes)]
        data = b'\x00' * size

        frame = FrameInfo(
            data=data,
            timestamp=int(time.time() * 1_000_000),
            is_keyframe=(i % 30 == 0),
            width=1920,
            height=1080,
            frame_number=i,
        )

        results.frames_sent += 1
        results.bytes_sent += frame.size

    end_time = time.perf_counter()
    results.duration = end_time - start_time
    results.fps = num_frames / results.duration
    results.bandwidth_mbps = (results.bytes_sent * 8) / (results.duration * 1_000_000)

    logger.info(f"✓ FrameInfo creation benchmark complete:")
    logger.info(f"  - Frames: {results.frames_sent}")
    logger.info(f"  - Duration: {results.duration:.3f}s")
    logger.info(f"  - Throughput: {results.fps:.0f} fps")
    logger.info(f"  - Bandwidth: {results.bandwidth_mbps:.1f} Mbps")

    return results


async def benchmark_stats_updates(num_updates: int = 10000) -> BenchmarkResults:
    """Benchmark TransportStats updates."""
    logger.info(f"Benchmarking TransportStats updates ({num_updates} updates)...")

    results = BenchmarkResults()
    results.protocol = "Stats Updates"

    stats = TransportStats(protocol="benchmark")

    start_time = time.perf_counter()

    for i in range(num_updates):
        # Simulate RTT samples
        rtt = 20 + (i % 80)  # 20-100ms RTT
        stats.update_rtt(rtt)

        # Simulate packet tracking
        stats.packets_sent += 1
        if i % 100 == 0:  # 1% packet loss
            stats.packets_lost += 1
        stats.update_packet_loss()

        # Periodic updates
        if i % 100 == 0:
            stats.bytes_sent += 1_400_000  # Simulated
            stats.update_bandwidth()
            stats.frames_sent += 30
            stats.update_fps(stats.frames_sent)

    end_time = time.perf_counter()
    results.duration = end_time - start_time

    logger.info(f"✓ Stats updates benchmark complete:")
    logger.info(f"  - Updates: {num_updates}")
    logger.info(f"  - Duration: {results.duration:.3f}s")
    logger.info(f"  - Rate: {num_updates/results.duration:.0f} updates/sec")
    logger.info(f"  - Final RTT avg: {stats.rtt_avg_ms:.1f}ms")
    logger.info(f"  - Final packet loss: {stats.packet_loss_percent:.2f}%")

    return results


async def benchmark_json_encoding(num_frames: int = 1000) -> BenchmarkResults:
    """Benchmark JSON encoding for offer/answer."""
    logger.info(f"Benchmarking JSON encoding ({num_frames} encodings)...")

    results = BenchmarkResults()
    results.protocol = "JSON Encoding"

    import json

    # Create sample offer/answer data
    offer_data = {
        "protocol": "quic",
        "host": "192.168.1.100",
        "port": 4433,
        "alpn": "remote-desktop",
        "supports_migration": True,
        "stream_ids": list(range(10)),
    }

    start_time = time.perf_counter()

    for _ in range(num_frames):
        # Encode
        json_str = json.dumps(offer_data)
        results.bytes_sent += len(json_str)

        # Decode
        parsed = json.loads(json_str)

        results.frames_sent += 1

    end_time = time.perf_counter()
    results.duration = end_time - start_time

    logger.info(f"✓ JSON encoding benchmark complete:")
    logger.info(f"  - Encodings: {results.frames_sent}")
    logger.info(f"  - Duration: {results.duration:.3f}s")
    logger.info(f"  - Rate: {results.frames_sent/results.duration:.0f} encodings/sec")

    return results


async def benchmark_protocol_switch(num_switches: int = 100) -> BenchmarkResults:
    """Benchmark protocol switching overhead."""
    logger.info(f"Benchmarking protocol switching ({num_switches} switches)...")

    results = BenchmarkResults()
    results.protocol = "Protocol Switch"

    from src.transport.manager import TransportConfig, ProtocolType

    config = TransportConfig(
        preferred=ProtocolType.AUTO,
        switch_cooldown=0.0,  # No cooldown for benchmark
    )

    start_time = time.perf_counter()

    for i in range(num_switches):
        # Simulate switch decision logic
        current = "quic" if i % 2 == 0 else "webrtc"
        fallback = "webrtc" if current == "quic" else "quic"

        # Simulate stats check
        stats = {
            "rtt_avg_ms": 50 + (i % 100),
            "packet_loss_percent": (i % 10) / 2,
            "fps": 30 - (i % 15),
        }

        # Decision logic
        should_switch = (
            stats["rtt_avg_ms"] > 100 or
            stats["packet_loss_percent"] > 5 or
            stats["fps"] < 15
        )

        if should_switch:
            results.frames_sent += 1  # Count as a switch

    end_time = time.perf_counter()
    results.duration = end_time - start_time

    logger.info(f"✓ Protocol switch benchmark complete:")
    logger.info(f"  - Decisions: {num_switches}")
    logger.info(f"  - Switches: {results.frames_sent}")
    logger.info(f"  - Duration: {results.duration:.3f}s")
    logger.info(f"  - Rate: {num_switches/results.duration:.0f} decisions/sec")

    return results


async def run_all_benchmarks():
    """Run all transport benchmarks."""
    benchmarks = [
        ("FrameInfo Creation", 1000, benchmark_frame_creation),
        ("Stats Updates", 10000, benchmark_stats_updates),
        ("JSON Encoding", 1000, benchmark_json_encoding),
        ("Protocol Switch", 10000, benchmark_protocol_switch),
    ]

    results = []

    for name, iterations, bench_fn in benchmarks:
        print()
        logger.info(f"{'='*60}")
        logger.info(f"Benchmark: {name}")
        logger.info(f"{'='*60}")

        try:
            result = await bench_fn(iterations)
            results.append((name, result))
        except Exception as e:
            logger.error(f"Benchmark '{name}' failed: {e}")
            import traceback
            traceback.print_exc()

    # Summary
    print()
    logger.info(f"{'='*60}")
    logger.info("Benchmark Summary")
    logger.info(f"{'='*60}")

    for name, result in results:
        logger.info(f"\n{name}:")
        logger.info(f"  Duration: {result.duration:.3f}s")
        if result.fps > 0:
            logger.info(f"  FPS: {result.fps:.0f}")
        if result.bandwidth_mbps > 0:
            logger.info(f"  Bandwidth: {result.bandwidth_mbps:.1f} Mbps")
        logger.info(f"  Operations: {result.frames_sent}")

    logger.info(f"{'='*60}")


async def run_comparison_test():
    """Compare mock transport performance."""
    print()
    logger.info(f"{'='*60}")
    logger.info("Transport Comparison Test")
    logger.info(f"{'='*60}")

    class MockTransport:
        def __init__(self, name: str, latency_ms: float, loss_percent: float):
            self.name = name
            self._latency = latency_ms / 1000
            self._loss = loss_percent / 100
            self._stats = TransportStats(protocol=name)
            self._sent = 0
            self._lost = 0

        @property
        def stats(self):
            return self._stats

        async def send_frame(self, frame: FrameInfo):
            # Simulate latency
            await asyncio.sleep(self._latency)

            # Simulate packet loss
            import random
            if random.random() < self._loss:
                self._lost += 1
                self._stats.packets_lost += 1
            else:
                self._sent += 1
                self._stats.bytes_sent += frame.size

            self._stats.packets_sent += 1
            self._stats.update_rtt(self._latency * 1000)

    # Create mock transports
    transports = [
        ("QUIC (ideal)", MockTransport("quic", 20, 0.1)),
        ("QUIC (poor)", MockTransport("quic", 150, 5.0)),
        ("WebRTC (good)", MockTransport("webrtc", 40, 1.0)),
        ("WebRTC (bad)", MockTransport("webrtc", 200, 10.0)),
    ]

    test_frames = 100

    for name, transport in transports:
        logger.info(f"\nTesting: {name}")

        start = time.perf_counter()

        for i in range(test_frames):
            frame = FrameInfo(
                data=b'\x00' * 10_000,
                timestamp=i * 33_000,  # ~30fps
                is_keyframe=(i % 30 == 0),
            )
            await transport.send_frame(frame)

        duration = time.perf_counter() - start

        transport._stats.update_packet_loss()
        transport._stats.update_bandwidth()

        logger.info(f"  Duration: {duration:.2f}s")
        logger.info(f"  Sent: {transport._sent}, Lost: {transport._lost}")
        logger.info(f"  Loss rate: {transport._stats.packet_loss_percent:.2f}%")
        logger.info(f"  Avg RTT: {transport._stats.rtt_avg_ms:.1f}ms")
        logger.info(f"  Bandwidth: {transport._stats.bandwidth_kbps:.0f} kbps")


if __name__ == "__main__":
    asyncio.run(run_all_benchmarks())
    asyncio.run(run_comparison_test())
