#!/usr/bin/env python3
"""
Simple test script to verify agent-python basic functionality.
"""
import sys
import asyncio
from pathlib import Path

# Add src to path
sys.path.insert(0, str(Path(__file__).parent / "src"))


async def test_config():
    """Test configuration loading."""
    print("Testing configuration...")
    from config import AgentConfig
    config = AgentConfig.from_file()
    print(f"  ws_url: {config.ws_url}")
    print(f"  device_name: {config.device_name}")
    print(f"  capture.fps: {config.capture.fps}")
    print(f"  capture.bitrate_kbps: {config.capture.bitrate_kbps}")
    print("  [OK] Config loaded")


async def test_signaling_protocol():
    """Test signaling protocol functions."""
    print("\nTesting signaling protocol...")
    from signaling.protocol import (
        create_register_message,
        create_answer_message,
        create_ice_candidate_message,
        SignalingMessage,
    )

    # Test register message
    reg = create_register_message("Test Agent")
    assert '"type"' in reg and '"device"' in reg
    assert '"action"' in reg and '"register"' in reg
    assert "Test Agent" in reg
    print("  [OK] Register message")

    # Test answer message
    ans = create_answer_message("test-sdp", "controller-123")
    assert '"webrtc"' in ans and '"answer"' in ans
    assert "controller-123" in ans
    print("  [OK] Answer message")

    # Test ICE candidate message
    ice = create_ice_candidate_message({"candidate": "test"}, "controller-123")
    assert '"webrtc"' in ice and '"iceCandidate"' in ice
    print("  [OK] ICE candidate message")

    # Test SignalingMessage
    msg = SignalingMessage.from_dict({
        "type": "system",
        "action": "connected",
        "payload": {"deviceId": "test-123"}
    })
    assert msg.type == "system"
    assert msg.action == "connected"
    print("  [OK] SignalingMessage parsing")


async def test_capturer():
    """Test screen capturer."""
    print("\nTesting screen capturer...")
    from capture.d3dshot_backend import ScreenCapturer

    capturer = ScreenCapturer(target_fps=30)

    # Test initialization
    if await capturer.initialize():
        print(f"  [OK] Capturer initialized: {capturer.screen_width}x{capturer.screen_height}")

        # Test frame capture
        frame = await capturer.capture_frame()
        if frame:
            print(f"  [OK] Frame captured: {frame.width}x{frame.height}, {len(frame.data)} bytes, format={frame.format}")
        else:
            print("  [WARN] No frame captured")

        await capturer.close()
        print("  [OK] Capturer closed")
    else:
        print("  [WARN] Capturer initialization failed")


async def test_encoder():
    """Test H.264 encoder."""
    print("\nTesting H.264 encoder...")
    from encoder.pyav_encoder import PyAVEncoder

    encoder = PyAVEncoder(
        width=640,
        height=480,
        fps=30,
        bitrate_kbps=1000,
        gop_size=30,
    )

    # Test initialization
    if await encoder.initialize():
        print("  [OK] Encoder initialized")

        # Test encoding (create a simple test frame)
        import numpy as np
        test_frame = np.zeros((480, 640, 3), dtype=np.uint8)
        test_frame[:] = [128, 128, 128]  # Gray frame

        encoded = await encoder.encode(test_frame.tobytes(), 640, 480, "RGB")
        if encoded:
            print(f"  [OK] Frame encoded: {len(encoded.data)} bytes, keyframe={encoded.is_keyframe}")
        else:
            print("  [WARN] No encoded output")

        await encoder.close()
        print("  [OK] Encoder closed")
    else:
        print("  [WARN] Encoder initialization failed")


async def test_webrtc_peer():
    """Test WebRTC peer manager."""
    print("\nTesting WebRTC peer manager...")

    # Create a mock signaling client
    from signaling.client import SignalingConfig
    config = SignalingConfig(ws_url="ws://127.0.0.1:9527")

    try:
        from webrtc.peer import WebRTCPeerManager
        print("  [OK] WebRTCPeerManager imported")
    except Exception as e:
        print(f"  [WARN] WebRTCPeerManager import failed: {e}")


async def main():
    """Run all tests."""
    print("=" * 50)
    print("agent-python Functional Tests")
    print("=" * 50)

    try:
        await test_config()
        await test_signaling_protocol()
        await test_capturer()
        await test_encoder()
        await test_webrtc_peer()

        print("\n" + "=" * 50)
        print("All tests completed!")
        print("=" * 50)
        return 0
    except Exception as e:
        print(f"\nTest failed with error: {e}")
        import traceback
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
