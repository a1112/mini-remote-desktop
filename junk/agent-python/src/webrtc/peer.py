"""
WebRTC peer connection manager.

Handles PeerConnection, SDP negotiation, and ICE candidates.
"""

import asyncio
import json
import logging
from typing import Any, Callable, Dict, Optional

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
    RTCConfiguration = None
    RTCSessionDescription = None
    RTCIceServer = None
    MediaStreamTrack = None

logger = logging.getLogger(__name__)


class WebRTCPeerManager:
    """
    Manager for WebRTC peer connections.

    Handles creating peer connections, processing offers,
    and managing ICE candidates.
    """

    def __init__(
        self,
        signaling,
        video_width: int = 1920,
        video_height: int = 1080,
        video_fps: int = 30,
    ):
        """
        Initialize the peer manager.

        Args:
            signaling: Signaling client for message exchange
            video_width: Video width
            video_height: Video height
            video_fps: Target frame rate
        """
        self.signaling = signaling
        self.video_width = video_width
        self.video_height = video_height
        self.video_fps = video_fps

        self._pc = None
        self._video_track = None
        self._controller_id = None
        self._connected = False
        self._pending_ice_candidates = []

    async def create_answer(
        self,
        offer_sdp: str,
        controller_id: str,
    ) -> Optional[str]:
        """
        Create an SDP answer from an offer.

        Args:
            offer_sdp: SDP offer string
            controller_id: Controller's device ID

        Returns:
            SDP answer string or None if failed
        """
        if not HAS_AIORTC:
            logger.error("aiortc not available")
            return None

        try:
            # Create new peer connection
            await self._create_peer_connection()

            if self._pc is None:
                logger.error("Failed to create peer connection")
                return None

            self._controller_id = controller_id

            # Set remote description (offer)
            offer = RTCSessionDescription(offer_sdp, type="offer")
            await self._pc.setRemoteDescription(offer)

            # Create answer
            answer = await self._pc.createAnswer()

            # Set local description
            await self._pc.setLocalDescription(answer)

            logger.info(f"Created SDP answer for controller {controller_id}")
            return answer.sdp

        except Exception as e:
            logger.error(f"Failed to create answer: {e}")
            return None

    async def _create_peer_connection(self) -> None:
        """Create a new RTCPeerConnection."""
        # Close existing connection
        if self._pc:
            await self.close()

        # Create configuration with STUN server
        config = RTCConfiguration(
            iceServers=[
                RTCIceServer(urls=["stun:stun.l.google.com:19302"]),
            ]
        )

        # Create peer connection
        self._pc = RTCPeerConnection(configuration=config)

        # Setup event handlers
        self._setup_pc_events()

        # Create and add video track
        from .track import H264TrackProxy

        self._video_track = H264TrackProxy(
            width=self.video_width,
            height=self.video_height,
            fps=self.video_fps,
        )

        self._pc.addTrack(self._video_track)

        logger.debug("Peer connection created")

    def _setup_pc_events(self) -> None:
        """Setup peer connection event handlers."""
        if self._pc is None:
            return

        @self._pc.on("icecandidate")
        def on_ice_candidate(candidate):
            """Handle local ICE candidate."""
            if candidate and self._controller_id:
                # Send candidate to signaling server
                candidate_dict = {
                    "candidate": candidate.candidate,
                    "sdpMid": candidate.sdpMid,
                    "sdpMLineIndex": candidate.sdpMLineIndex,
                }
                asyncio.create_task(
                    self.signaling.send_ice_candidate(candidate_dict, self._controller_id)
                )
            else:
                # All candidates gathered
                logger.debug("ICE candidate gathering complete")

        @self._pc.on("iceconnectionstatechange")
        def on_ice_connection_state_change():
            """Handle ICE connection state change."""
            state = self._pc.iceConnectionState if self._pc else "unknown"
            logger.info(f"ICE connection state: {state}")

            if state == "connected" or state == "completed":
                self._connected = True
                logger.info("WebRTC connection established")
            elif state == "failed" or state == "disconnected" or state == "closed":
                self._connected = False
                logger.warning(f"WebRTC connection {state}")

        @self._pc.on("connectionstatechange")
        def on_connection_state_change():
            """Handle connection state change."""
            state = self._pc.connectionState if self._pc else "unknown"
            logger.info(f"Peer connection state: {state}")

        @self._pc.on("track")
        def on_track(track):
            """Handle incoming track (from controller)."""
            logger.info(f"Received track: {track.kind}")

    async def add_remote_candidate(self, candidate_dict: Dict[str, Any]) -> bool:
        """
        Add a remote ICE candidate.

        Args:
            candidate_dict: Candidate dictionary from signaling

        Returns:
            True if successful
        """
        if self._pc is None:
            # Queue candidate for later
            self._pending_ice_candidates.append(candidate_dict)
            return True

        try:
            from aiortc import RTCIceCandidate

            candidate = RTCIceCandidate(
                candidate=candidate_dict.get("candidate", ""),
                sdpMid=candidate_dict.get("sdpMid"),
                sdpMLineIndex=candidate_dict.get("sdpMLineIndex"),
            )

            await self._pc.addIceCandidate(candidate)
            logger.debug("Added remote ICE candidate")
            return True

        except Exception as e:
            logger.error(f"Failed to add ICE candidate: {e}")
            return False

    async def send_video_frame(self, encoded_data: bytes) -> None:
        """
        Send an encoded video frame.

        Args:
            encoded_data: H.264 encoded frame data
        """
        if self._video_track:
            await self._video_track.send_encoded(encoded_data)

    async def request_keyframe(self) -> None:
        """Request a keyframe from the encoder (via PLI)."""
        logger.debug("Requesting keyframe")

    async def close(self) -> None:
        """Close the peer connection."""
        self._connected = False

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

        logger.debug("Peer connection closed")

    @property
    def is_connected(self) -> bool:
        """Check if WebRTC connection is established."""
        return self._connected and self._pc is not None

    @property
    def video_track(self):
        """Get the video track."""
        return self._video_track
