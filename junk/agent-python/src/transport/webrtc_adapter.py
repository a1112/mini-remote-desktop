"""
WebRTC transport adapter implementation.

Wraps the existing aiortc WebRTC implementation to conform
to the TransportAdapter interface.
"""

import asyncio
import json
import logging
import time
from typing import Optional, Dict, Any

from .base import TransportAdapter, TransportError, ConnectionError, SendError
from .stats import TransportStats, FrameInfo

try:
    from aiortc import (
        RTCPeerConnection,
        RTCConfiguration,
        RTCSessionDescription,
        RTCIceServer,
        MediaStreamTrack,
    )
    from aiortc.sdp import SessionDescription
    HAS_AIORTC = True
except ImportError:
    HAS_AIORTC = False
    RTCPeerConnection = None

try:
    try:
        # Package import path: src.transport.webrtc_adapter
        from ..webrtc.track import H264TrackProxy
        from ..webrtc.rtp import H264Packetizer
    except ImportError:
        # Script/top-level import path: transport.webrtc_adapter with src on sys.path
        from webrtc.track import H264TrackProxy
        from webrtc.rtp import H264Packetizer
    HAS_TRACK_IMPL = True
except ImportError:
    HAS_TRACK_IMPL = False
    H264TrackProxy = None

logger = logging.getLogger(__name__)


class WebRTCAdapter(TransportAdapter):
    """
    WebRTC transport adapter using aiortc.

    Features:
    - Browser-compatible peer-to-peer transport
    - ICE/STUN/TURN support
    - SRTP for media encryption
    - RTCP feedback for congestion control
    """

    def __init__(
        self,
        signaling=None,
        video_width: int = 1920,
        video_height: int = 1080,
        video_fps: int = 30,
        ice_servers: Optional[list] = None,
    ):
        """
        Initialize WebRTC adapter.

        Args:
            signaling: Signaling client for SDP/ICE exchange
            video_width: Video width
            video_height: Video height
            video_fps: Target frame rate
            ice_servers: Optional ICE servers (STUN/TURN)
        """
        if not HAS_AIORTC:
            raise ImportError("aiortc is required for WebRTC transport")

        super().__init__("webrtc")

        self.signaling = signaling
        self.video_width = video_width
        self.video_height = video_height
        self.video_fps = video_fps

        # ICE servers configuration
        self.ice_servers = ice_servers or [
            {"urls": ["stun:stun.l.google.com:19302"]},
            {"urls": ["stun:stun1.l.google.com:19302"]},
        ]

        # WebRTC state
        self._pc: Optional[RTCPeerConnection] = None
        self._video_track: Optional[MediaStreamTrack] = None
        self._controller_id: Optional[str] = None
        self._pending_ice_candidates: list = []

        # Frame handling
        self._frame_queue: asyncio.Queue[bytes] = asyncio.Queue(maxsize=30)
        self._frame_number = 0

        # RTT tracking via getStats
        self._stats_task: Optional[asyncio.Task] = None

        logger.info("WebRTC adapter initialized")

    async def connect(self, offer: str, metadata: Optional[dict] = None) -> str:
        """
        Establish WebRTC peer connection.

        Args:
            offer: SDP offer string from controller
            metadata: Optional metadata (may contain controller_id)

        Returns:
            SDP answer string

        Raises:
            ConnectionError: If connection fails
        """
        try:
            # Extract controller_id from metadata
            if metadata:
                self._controller_id = metadata.get("controller_id")

            # Create peer connection
            await self._create_peer_connection()

            if self._pc is None:
                raise ConnectionError("Failed to create peer connection", protocol="webrtc")

            # Set remote description (offer)
            offer_desc = RTCSessionDescription(sdp=offer, type="offer")
            await self._pc.setRemoteDescription(offer_desc)

            # Create answer
            answer = await self._pc.createAnswer()
            await self._pc.setLocalDescription(answer)

            # Send any pending ICE candidates
            for candidate in self._pending_ice_candidates:
                await self._add_local_ice_candidate(candidate)

            logger.info(f"WebRTC SDP answer created for controller {self._controller_id}")
            return answer.sdp

        except Exception as e:
            logger.error(f"WebRTC connection failed: {e}")
            raise ConnectionError(f"WebRTC connection failed: {e}", protocol="webrtc")

    async def _create_peer_connection(self) -> None:
        """Create a new RTCPeerConnection."""
        # Close existing connection
        if self._pc:
            await self.close()

        # Create configuration with ICE servers
        ice_server_list = []
        for server in self.ice_servers:
            urls = server.get("urls", [])
            if isinstance(urls, str):
                urls = [urls]
            ice_server_list.append(RTCIceServer(urls=urls))

        config = RTCConfiguration(iceServers=ice_server_list)
        self._pc = RTCPeerConnection(configuration=config)

        # Setup event handlers
        self._setup_pc_events()

        # Create and add video track
        if HAS_TRACK_IMPL:
            self._video_track = H264TrackProxy(
                width=self.video_width,
                height=self.video_height,
                fps=self.video_fps,
            )
            self._pc.addTrack(self._video_track)
        else:
            # Create a basic track
            from aiortc import MediaStreamTrack
            from av import VideoFrame
            import numpy as np

            class BasicVideoTrack(MediaStreamTrack):
                kind = "video"

                def __init__(self, width, height, fps):
                    super().__init__()
                    self.width = width
                    self.height = height
                    self.fps = fps
                    self._queue = asyncio.Queue(maxsize=30)
                    self._pts = 0

                async def recv(self):
                    try:
                        data = await asyncio.wait_for(self._queue.get(), timeout=0.5)
                        if data is None:
                            raise asyncio.TimeoutError
                    except asyncio.TimeoutError:
                        pass

                    arr = np.zeros((self.height, self.width, 3), dtype=np.uint8)
                    frame = VideoFrame.from_ndarray(arr, format="rgb24")
                    frame.pts = self._pts
                    frame.time_base = "1/90000"
                    self._pts += 90000 // max(1, self.fps)
                    return frame

                def send_encoded(self, data):
                    try:
                        self._queue.put_nowait(data)
                    except asyncio.QueueFull:
                        try:
                            self._queue.get_nowait()
                            self._queue.put_nowait(data)
                        except asyncio.QueueEmpty:
                            pass

                def stop(self):
                    while not self._queue.empty():
                        try:
                            self._queue.get_nowait()
                        except asyncio.QueueEmpty:
                            break

            self._video_track = BasicVideoTrack(self.video_width, self.video_height, self.video_fps)
            self._pc.addTrack(self._video_track)

        logger.debug("Peer connection created")

    def _setup_pc_events(self) -> None:
        """Setup peer connection event handlers."""
        if self._pc is None:
            return

        @self._pc.on("icecandidate")
        def on_ice_candidate(candidate):
            """Handle local ICE candidate."""
            if candidate and self._controller_id and self.signaling:
                candidate_dict = {
                    "candidate": candidate.candidate,
                    "sdpMid": candidate.sdpMid,
                    "sdpMLineIndex": candidate.sdpMLineIndex,
                }
                asyncio.create_task(
                    self._send_ice_via_signaling(candidate_dict)
                )
            elif candidate:
                # Store for later
                self._pending_ice_candidates.append(candidate)

        @self._pc.on("iceconnectionstatechange")
        def on_ice_connection_state_change():
            """Handle ICE connection state change."""
            state = self._pc.iceConnectionState if self._pc else "unknown"
            logger.info(f"ICE connection state: {state}")

            if state == "connected" or state == "completed":
                self._update_connection_state(True)
                logger.info("WebRTC connection established")

                # Start stats collection
                if self._stats_task is None or self._stats_task.done():
                    self._stats_task = asyncio.create_task(self._stats_loop())

            elif state == "failed" or state == "disconnected" or state == "closed":
                self._update_connection_state(False)
                logger.warning(f"WebRTC connection {state}")

                if self._stats_task:
                    self._stats_task.cancel()

        @self._pc.on("connectionstatechange")
        def on_connection_state_change():
            """Handle connection state change."""
            state = self._pc.connectionState if self._pc else "unknown"
            logger.info(f"Peer connection state: {state}")

        @self._pc.on("track")
        def on_track(track):
            """Handle incoming track (from controller)."""
            logger.info(f"Received track: {track.kind}")

    async def _send_ice_via_signaling(self, candidate_dict: dict) -> None:
        """Send ICE candidate via signaling server."""
        if self.signaling:
            try:
                await self.signaling.send_ice_candidate(candidate_dict, self._controller_id)
            except Exception as e:
                logger.error(f"Failed to send ICE candidate: {e}")

    async def _add_local_ice_candidate(self, candidate) -> None:
        """Add a stored local ICE candidate."""
        if self._controller_id and self.signaling:
            candidate_dict = {
                "candidate": candidate.candidate,
                "sdpMid": candidate.sdpMid,
                "sdpMLineIndex": candidate.sdpMLineIndex,
            }
            await self._send_ice_via_signaling(candidate_dict)

    async def add_remote_candidate(self, candidate_dict: Dict[str, Any]) -> bool:
        """
        Add a remote ICE candidate.

        Args:
            candidate_dict: Candidate dictionary from signaling

        Returns:
            True if successful
        """
        if self._pc is None:
            self._pending_ice_candidates.append(candidate_dict)
            return True

        try:
            from aiortc import RTCIceCandidate

            candidate = RTCIceCandidate(
                sdpMid=candidate_dict.get("sdpMid"),
                sdpMLineIndex=candidate_dict.get("sdpMLineIndex"),
                candidate=candidate_dict.get("candidate")
            )

            await self._pc.addIceCandidate(candidate)
            logger.debug("Added remote ICE candidate")
            return True

        except Exception as e:
            logger.error(f"Failed to add ICE candidate: {e}")
            return False

    async def send_media(self, frame: FrameInfo) -> None:
        """
        Send an encoded media frame via WebRTC.

        Args:
            frame: Frame information including encoded data

        Raises:
            SendError: If send fails
        """
        if not self.is_connected or not self._video_track:
            raise SendError("Not connected", protocol="webrtc")

        try:
            # Send encoded data to track
            if hasattr(self._video_track, 'send_encoded'):
                await self._video_track.send_encoded(frame.data)
            else:
                # Fallback: put in queue
                await self._frame_queue.put(frame.data)

            # Update stats
            self._stats.bytes_sent += frame.size
            self._stats.packets_sent += 1
            self._stats.frames_sent += 1
            self._frame_number += 1

            # Update bandwidth periodically
            if self._frame_number % 30 == 0:
                self._stats.update_bandwidth()
                self._stats.update_fps(self._frame_number)

        except asyncio.QueueFull:
            self._stats.frames_dropped += 1
            self._stats.packets_lost += 1
            raise SendError("Frame queue full", protocol="webrtc")
        except Exception as e:
            self._stats.connection_errors += 1
            raise SendError(f"Failed to send frame: {e}", protocol="webrtc")

    async def request_keyframe(self) -> None:
        """
        Request a keyframe from the encoder.

        For WebRTC, this sends a PLI (Picture Loss Indication).
        """
        logger.debug("Requesting keyframe via PLI")
        # The track handles PLI internally
        if hasattr(self._video_track, 'request_keyframe'):
            self._video_track.request_keyframe()

    async def disconnect(self) -> None:
        """Close WebRTC connection and cleanup."""
        logger.info("Disconnecting WebRTC adapter...")

        self._update_connection_state(False)

        if self._stats_task:
            self._stats_task.cancel()
            self._stats_task = None

        if self._video_track:
            self._video_track.stop()
            self._video_track = None

        if self._pc:
            try:
                await self._pc.close()
            except Exception:
                pass
            self._pc = None

        self._controller_id = None
        self._pending_ice_candidates.clear()

        # Clear frame queue
        while not self._frame_queue.empty():
            try:
                self._frame_queue.get_nowait()
            except asyncio.QueueEmpty:
                break

        logger.info("WebRTC adapter disconnected")

    async def _stats_loop(self) -> None:
        """Periodic statistics collection via WebRTC getStats."""
        while self.is_connected and self._pc:
            try:
                await asyncio.sleep(1.0)

                # Get stats from peer connection
                stats = await self._pc.getStats()

                # Extract RTT from candidate pair stats
                for report in stats.values():
                    if report.type == "candidate-pair" and report.state == "succeeded":
                        if hasattr(report, "currentRoundTripTime"):
                            rtt_ms = report.currentRoundTripTime * 1000
                            self._stats.update_rtt(rtt_ms)

            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.debug(f"Stats collection error: {e}")

    @property
    def peer_connection(self):
        """Get the underlying RTCPeerConnection."""
        return self._pc

    @property
    def video_track(self):
        """Get the video track."""
        return self._video_track


def create_webrtc_offer(
    controller_id: str,
    sdp: str
) -> tuple[str, str]:
    """
    Parse WebRTC offer from signaling format.

    Args:
        controller_id: Controller device ID
        sdp: SDP offer string

    Returns:
        Tuple of (controller_id, offer_sdp)
    """
    return controller_id, sdp


def is_webrtc_available() -> bool:
    """Check if WebRTC support is available."""
    return HAS_AIORTC
