import asyncio
import unittest
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from src.protocols.webrtc.handler import WebRTCProtocolHandler


class _SingleFrameTrack:
    kind = "video"

    def __init__(self):
        self.recv_calls = 0

    async def recv(self):
        self.recv_calls += 1
        if self.recv_calls == 1:
            class _Frame:
                def to_ndarray(self, format="rgb24"):
                    return np.zeros((2, 2, 3), dtype=np.uint8)

            return _Frame()
        await asyncio.sleep(0)
        raise RuntimeError("eof")


class TestWebRTCFirstFrame(unittest.IsolatedAsyncioTestCase):
    async def test_receive_video_should_not_exit_before_connected_flag_flips(self):
        handler = WebRTCProtocolHandler(use_hw_decoder=False)
        handler._pc = object()
        frames = []
        handler.on_frame_received(lambda f: frames.append(f))

        # Simulate on_track callback firing before ICE connected.
        track = _SingleFrameTrack()
        task = asyncio.create_task(handler._receive_video(track))

        # Flip connected shortly after task starts.
        await asyncio.sleep(0.01)
        handler._connected = True

        await asyncio.wait_for(task, timeout=0.2)

        self.assertGreaterEqual(track.recv_calls, 1)
        self.assertGreaterEqual(len(frames), 1)


if __name__ == "__main__":
    unittest.main()
