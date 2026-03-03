#!/usr/bin/env python3
"""
NVENC WebRTC Agent - Main entry point

Usage:
    python run_nvenc_agent.py                    # Use default config
    python run_nvenc_agent.py --quality 30       # Set quality (QP)
    python run_nvenc_agent.py --monitor 1        # Select monitor
    python run_nvenc_agent.py --config path/to/config.json
    python run_nvenc_agent.py --transport quic   # Use QUIC protocol

Quality levels (QP):
    18 - Fidelity (~200 Mbps)
    24 - High quality (~80 Mbps) - default
    30 - Medium-high (~50 Mbps)
    36 - Medium (~35 Mbps)
    42 - Low (~30 Mbps)
    48 - Very low (~18 Mbps)

Transport protocols:
    auto - Automatically select best protocol (default)
    quic - Use QUIC protocol
    webrtc - Use WebRTC protocol
"""

import argparse
import asyncio
import logging
import sys
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.config import AgentConfig
from src.nvenc_agent import run_agent, NVENCAgent


def setup_logging(level: str = "INFO") -> None:
    """Setup logging configuration."""
    log_level = getattr(logging, level.upper(), logging.INFO)
    logging.basicConfig(
        level=log_level,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        datefmt="%H:%M:%S"
    )


def parse_args():
    """Parse command line arguments."""
    parser = argparse.ArgumentParser(
        description="NVENC WebRTC Agent - Hardware accelerated remote desktop",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__
    )

    parser.add_argument(
        "--config", "-c",
        type=str,
        help="Path to config.json file"
    )

    parser.add_argument(
        "--quality", "-q",
        type=int,
        choices=[18, 24, 30, 36, 42, 48],
        default=24,
        help="Quality level (QP value, lower is better)"
    )

    parser.add_argument(
        "--monitor", "-m",
        type=int,
        default=0,
        help="Monitor index to capture (default: 0)"
    )

    parser.add_argument(
        "--fps",
        type=int,
        choices=[15, 30, 60, 120, 144],
        help="Target frame rate"
    )

    parser.add_argument(
        "--device-name",
        type=str,
        help="Device name for registration"
    )

    parser.add_argument(
        "--signaling-url",
        type=str,
        help="Signaling server URL"
    )

    parser.add_argument(
        "--transport",
        type=str,
        choices=["auto", "quic", "webrtc"],
        default="auto",
        help="Transport protocol to use (default: auto)"
    )

    parser.add_argument(
        "--no-transport-switch",
        action="store_true",
        help="Disable automatic transport protocol switching"
    )

    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable verbose logging"
    )

    return parser.parse_args()


async def main():
    """Main entry point."""
    args = parse_args()
    setup_logging("DEBUG" if args.verbose else "INFO")

    logger = logging.getLogger("NVENC-Agent")

    # Load config
    if args.config:
        config_path = Path(args.config)
        config = AgentConfig.from_file(config_path)
    else:
        config = AgentConfig.from_file()

    # Override with command line args
    config.quality = args.quality
    config.monitor_index = args.monitor

    if args.fps:
        config.capture.fps = args.fps

    if args.device_name:
        config.device_name = args.device_name

    if args.signaling_url:
        config.ws_url = args.signaling_url

    # Transport configuration
    config.transport.preferred = args.transport
    config.transport.auto_switch = not args.no_transport_switch

    # Print configuration
    logger.info("=" * 60)
    logger.info("NVENC WebRTC Agent")
    logger.info("=" * 60)
    logger.info(f"  Signaling: {config.ws_url}")
    logger.info(f"  Device: {config.device_name}")
    logger.info(f"  Monitor: {config.monitor_index}")
    logger.info(f"  Quality: QP={config.quality}")
    logger.info(f"  FPS: {config.capture.fps}")
    logger.info(f"  Transport: {config.transport.preferred} (auto-switch: {config.transport.auto_switch})")
    logger.info("=" * 60)

    # Quality description
    quality_names = {
        18: "Fidelity",
        24: "High",
        30: "Medium-High",
        36: "Medium",
        42: "Low",
        48: "Very Low"
    }
    logger.info(f"  Quality Level: {quality_names.get(config.quality, 'Unknown')}")
    logger.info("=" * 60)

    # Run agent
    try:
        await run_agent(config)
    except KeyboardInterrupt:
        logger.info("Interrupted by user")
    except Exception as e:
        logger.exception(f"Agent error: {e}")
        sys.exit(1)


if __name__ == "__main__":
    asyncio.run(main())
