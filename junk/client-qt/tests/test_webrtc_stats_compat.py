import unittest
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from src.protocols.webrtc.handler import WebRTCProtocolHandler


class _ReportMediaType:
    type = 'inbound-rtp'
    mediaType = 'video'


class _ReportKind:
    type = 'inbound-rtp'
    kind = 'video'


class _ReportAudio:
    type = 'inbound-rtp'
    kind = 'audio'


class _ReportOtherType:
    type = 'candidate-pair'
    kind = 'video'


class TestWebRTCStatsCompat(unittest.TestCase):
    def test_video_inbound_rtp_accepts_mediaType(self):
        self.assertTrue(WebRTCProtocolHandler._is_video_inbound_rtp_report(_ReportMediaType()))

    def test_video_inbound_rtp_accepts_kind(self):
        self.assertTrue(WebRTCProtocolHandler._is_video_inbound_rtp_report(_ReportKind()))

    def test_video_inbound_rtp_rejects_audio(self):
        self.assertFalse(WebRTCProtocolHandler._is_video_inbound_rtp_report(_ReportAudio()))

    def test_video_inbound_rtp_rejects_other_type(self):
        self.assertFalse(WebRTCProtocolHandler._is_video_inbound_rtp_report(_ReportOtherType()))


if __name__ == '__main__':
    unittest.main()
