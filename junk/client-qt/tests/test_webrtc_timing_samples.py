import asyncio
import unittest
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from src.protocols.webrtc.handler import WebRTCProtocolHandler


class _TwoFrameTrack:
    kind = 'video'

    def __init__(self):
        self.recv_calls = 0

    async def recv(self):
        self.recv_calls += 1
        if self.recv_calls <= 2:
            class _Frame:
                def to_ndarray(self, format='rgb24'):
                    return np.zeros((2, 2, 3), dtype=np.uint8)

            await asyncio.sleep(0.001)
            return _Frame()
        raise RuntimeError('eof')


class TestWebRTCTimingSamples(unittest.IsolatedAsyncioTestCase):
    async def test_timing_sample_emitted_for_frames(self):
        handler = WebRTCProtocolHandler(use_hw_decoder=False)
        handler._pc = object()
        handler._connected = True

        samples = []
        handler.on_timing_sample(lambda s: samples.append(s))

        track = _TwoFrameTrack()
        await handler._receive_video(track)

        self.assertGreaterEqual(len(samples), 2)
        self.assertIsNone(samples[0]['transport_gap_ms'])
        self.assertIsNone(samples[0]['output_gap_ms'])
        self.assertIsNotNone(samples[1]['transport_gap_ms'])
        self.assertIsNotNone(samples[1]['output_gap_ms'])


if __name__ == '__main__':
    unittest.main()
