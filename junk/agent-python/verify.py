#!/usr/bin/env python3
"""
Verification script for agent-python.

Checks that all dependencies are available and basic functionality works.
"""

import sys
import asyncio
from pathlib import Path


def check_dependencies():
    """Check if all required dependencies are available."""
    print("Checking dependencies...")

    dependencies = [
        ("websockets", "WebSocket client library"),
        ("aiortc", "WebRTC implementation"),
        ("av", "PyAV for video encoding"),
        ("numpy", "Numerical computing"),
        ("PIL", "Pillow for image handling"),
        ("d3dshot", "Windows screen capture"),
    ]

    missing = []
    for module, description in dependencies:
        try:
            __import__(module)
            print(f"  [OK] {module} - {description}")
        except ImportError:
            print(f"  [MISSING] {module} - {description}")
            missing.append(module)

    if missing:
        print(f"\nMissing dependencies: {', '.join(missing)}")
        print("Install with: pip install -r requirements.txt")
        return False

    print("\nAll dependencies found!")
    return True


def check_project_structure():
    """Check if all project files exist."""
    print("\nChecking project structure...")

    required_files = [
        "src/__init__.py",
        "src/main.py",
        "src/config.py",
        "src/signaling/__init__.py",
        "src/signaling/protocol.py",
        "src/signaling/client.py",
        "src/capture/__init__.py",
        "src/capture/d3dshot_backend.py",
        "src/encoder/__init__.py",
        "src/encoder/pyav_encoder.py",
        "src/webrtc/__init__.py",
        "src/webrtc/peer.py",
        "src/webrtc/track.py",
        "src/utils/__init__.py",
        "src/utils/stats.py",
        "config.json",
        "requirements.txt",
        "pyproject.toml",
    ]

    missing = []
    for file in required_files:
        path = Path(file)
        if path.exists():
            print(f"  [OK] {file}")
        else:
            print(f"  [MISSING] {file}")
            missing.append(file)

    if missing:
        print(f"\nMissing files: {', '.join(missing)}")
        return False

    print("\nAll project files found!")
    return True


async def test_imports():
    """Test that all modules can be imported."""
    print("\nTesting module imports...")

    try:
        # Add src to path
        src_path = Path(__file__).parent / "src"
        sys.path.insert(0, str(src_path))

        # Test imports
        from config import AgentConfig
        print("  [OK] config module")

        from signaling.protocol import create_register_message
        print("  [OK] signaling.protocol module")

        from signaling.client import SignalingClient, SignalingConfig
        print("  [OK] signaling.client module")

        from capture.d3dshot_backend import D3DShotCapturer
        print("  [OK] capture.d3dshot_backend module")

        from encoder.pyav_encoder import PyAVEncoder
        print("  [OK] encoder.pyav_encoder module")

        from webrtc.peer import WebRTCPeerManager
        print("  [OK] webrtc.peer module")

        from webrtc.track import H264TrackProxy
        print("  [OK] webrtc.track module")

        from utils.stats import PerformanceStats
        print("  [OK] utils.stats module")

        print("\nAll modules imported successfully!")
        return True

    except Exception as e:
        print(f"\nImport error: {e}")
        import traceback
        traceback.print_exc()
        return False


async def test_config():
    """Test configuration loading."""
    print("\nTesting configuration...")

    try:
        src_path = Path(__file__).parent / "src"
        sys.path.insert(0, str(src_path))

        from config import AgentConfig

        # Load config
        config = AgentConfig.from_file(Path("config.json"))
        print(f"  ws_url: {config.ws_url}")
        print(f"  device_name: {config.device_name}")
        print(f"  capture.fps: {config.capture.fps}")
        print(f"  capture.bitrate_kbps: {config.capture.bitrate_kbps}")

        print("\nConfiguration loaded successfully!")
        return True

    except Exception as e:
        print(f"\nConfiguration error: {e}")
        import traceback
        traceback.print_exc()
        return False


async def main():
    """Run all verification checks."""
    print("=" * 50)
    print("agent-python Verification Script")
    print("=" * 50)

    checks = [
        ("Dependencies", check_dependencies),
        ("Project Structure", check_project_structure),
        ("Module Imports", test_imports),
        ("Configuration", test_config),
    ]

    results = []
    for name, check in checks:
        if asyncio.iscoroutinefunction(check):
            result = await check()
        else:
            result = check()
        results.append((name, result))

    print("\n" + "=" * 50)
    print("Summary")
    print("=" * 50)

    for name, result in results:
        status = "[PASS]" if result else "[FAIL]"
        print(f"  {status} {name}")

    all_passed = all(result for _, result in results)
    if all_passed:
        print("\nAll checks passed! The agent is ready to run.")
        print("Start with: python src/main.py")
        return 0
    else:
        print("\nSome checks failed. Please fix the issues above.")
        return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
