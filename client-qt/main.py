#!/usr/bin/env python3
"""
Multi-Protocol Remote Desktop Client (Qt)

A Qt-based remote desktop viewer supporting multiple protocols:
- WebRTC (via aiortc)
- QUIC (via aioquic)
- JPEG Streaming (native)

Compatible with mini-remote-desktop agent-rust and agent-python.

Usage:
    python main.py [--config CONFIG] [--signaling-url URL]
"""

import asyncio
import logging
import os
import sys
from pathlib import Path

from PySide6.QtWidgets import QApplication
from PySide6.QtCore import QSettings, Qt

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))

from src.ui.main_window import MainWindow


def setup_logging(level: str = "INFO") -> None:
    """Setup logging configuration."""
    log_level = getattr(logging, level.upper(), logging.INFO)

    # Console logging
    logging.basicConfig(
        level=log_level,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        datefmt="%H:%M:%S"
    )


def parse_args():
    """Parse command line arguments."""
    import argparse

    parser = argparse.ArgumentParser(
        description="Multi-Protocol Remote Desktop Client",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__
    )

    parser.add_argument(
        "--config", "-c",
        type=str,
        help="Path to config.yaml file"
    )

    parser.add_argument(
        "--signaling-url",
        type=str,
        help="Override signaling server URL"
    )

    parser.add_argument(
        "--verbose", "-v",
        action="store_true",
        help="Enable verbose logging"
    )

    parser.add_argument(
        "--debug",
        action="store_true",
        help="Enable debug logging"
    )

    return parser.parse_args()


def main():
    """Main entry point."""
    args = parse_args()

    # Setup logging
    log_level = "DEBUG" if args.debug else ("DEBUG" if args.verbose else "INFO")
    setup_logging(log_level)

    logger = logging.getLogger("client-qt")
    logger.info("=" * 60)
    logger.info("Multi-Protocol Remote Desktop Client")
    logger.info("=" * 60)

    # Prefer D3D11/ANGLE on Windows for low-latency composition.
    if sys.platform.startswith("win"):
        os.environ.setdefault("QT_OPENGL", "angle")
        os.environ.setdefault("QSG_RHI_BACKEND", "d3d11")
        logger.info(
            "Qt backend prefs: QT_OPENGL=%s QSG_RHI_BACKEND=%s",
            os.environ.get("QT_OPENGL"),
            os.environ.get("QSG_RHI_BACKEND"),
        )

    # Create Qt application
    app = QApplication(sys.argv)
    app.setApplicationName("Remote Desktop Viewer")
    app.setOrganizationName("MiniRemoteDesktop")

    # Enable high DPI scaling
    app.setAttribute(Qt.ApplicationAttribute.AA_EnableHighDpiScaling, True)
    app.setAttribute(Qt.ApplicationAttribute.AA_UseHighDpiPixmaps, True)

    # Create and show main window
    window = MainWindow()
    window.show()

    logger.info("Application started")

    # Run event loop
    exit_code = app.exec()

    logger.info(f"Application exited with code {exit_code}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
