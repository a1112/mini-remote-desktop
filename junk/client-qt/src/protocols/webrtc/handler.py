"""
WebRTC protocol handler using aiortc.

Receives H.264 video stream via WebRTC and decodes for display.
Supports hardware-accelerated decoding (DXVA2/D3D11VA/NVDEC/QSV).
"""

import asyncio
import logging
import time
from typing import Callable, Optional, Dict, Any

import numpy as np
import numpy.typing as npt

from ..base import DecoderProtocolHandler
from ...core.stats import ConnectionState

logger = logging.getLogger(__name__)


class WebRTCProtocolHandler(DecoderProtocolHandler):
    """
    WebRTC protocol handler for receiving video streams.

    Uses aiortc for WebRTC peer connection and receives
    H.264 encoded video frames.
    """

    def __init__(
        self,
        use_hw_decoder: bool = True,
        decoder_priority: Optional[list[str]] = None,
        decoder_low_delay: bool = True,
    ):
        """Initialize WebRTC handler."""
        super().__init__()
        self._pc = None
        self._video_track = None
        self._connected = False
        self._connected_device_id = None
        self._signaling = None
        self._use_hw_decoder = use_hw_decoder
        self._decoder_priority = list(decoder_priority or [])
        self._decoder_low_delay = decoder_low_delay
        self._answer_future: Optional[asyncio.Future] = None
        self._signaling_answer_cb = None
        self._signaling_ice_cb = None
        self._timing_callback: Optional[Callable[[dict], None]] = None
        self._last_recv_ts: Optional[float] = None
        self._last_emit_ts: Optional[float] = None

        # Hardware decoder (optional)
        self._hw_decoder = None
        self._hw_decoder_initialized = False

        # Stats tracking
        self._stats = {
            "bytes_received": 0,
            "frames_received": 0,
            "last_update": time.time()
        }

    @property
    def name(self) -> str:
        """Get protocol name."""
        return "webrtc"

    @property
    def is_connected(self) -> bool:
        """Check if connected."""
        return self._connected and self._pc is not None

    async def connect(
        self,
        target_device_id: str,
        offer_sdp: str,
        signaling_client
    ) -> str:
        """
        Connect to target device via WebRTC.

        Args:
            target_device_id: Target agent device ID
            offer_sdp: SDP offer string
            signaling_client: Signaling client

        Returns:
            SDP answer string
        """
        try:
            from aiortc import (
                RTCPeerConnection,
                RTCConfiguration,
                RTCSessionDescription,
                RTCIceServer,
            )

            self._connected_device_id = target_device_id
            self._signaling = signaling_client
            self._emit_state(ConnectionState.CONNECTING)

            # Create peer connection
            config = RTCConfiguration(
                iceServers=[
                    RTCIceServer(urls=["stun:stun.l.google.com:19302"]),
                    RTCIceServer(urls=["stun:stun1.l.google.com:19302"]),
                ]
            )
            self._pc = RTCPeerConnection(configuration=config)
            self._setup_pc_events()
            # Ensure SDP contains a video m-line and ICE attributes expected by agent-rust.
            self._pc.addTransceiver("video", direction="recvonly")

            loop = asyncio.get_running_loop()
            self._answer_future = loop.create_future()

            # Bind signaling callbacks so this handler can receive answer/candidates.
            self._bind_signaling_callbacks()

            # Controller side should create and send OFFER.
            local_offer = await self._pc.createOffer()
            await self._pc.setLocalDescription(local_offer)

            await signaling_client.send_offer(
                target_device_id=target_device_id,
                offer_sdp=local_offer.sdp,
                transport="webrtc",
            )

            # Wait for remote ANSWER.
            answer_sdp = await asyncio.wait_for(self._answer_future, timeout=12.0)
            answer = RTCSessionDescription(answer_sdp, type="answer")
            await self._pc.setRemoteDescription(answer)

            logger.info(f"WebRTC connection initiated to {target_device_id}")
            return answer_sdp

        except ImportError:
            logger.error("aiortc not available")
            raise RuntimeError("aiortc library is required for WebRTC")

        except Exception as e:
            logger.error(f"WebRTC connection failed: {e}")
            self._emit_state(ConnectionState.FAILED)
            raise

    def _setup_pc_events(self) -> None:
        """Setup peer connection event handlers."""
        if self._pc is None:
            return

        @self._pc.on("icecandidate")
        def on_ice_candidate(candidate):
            """Handle local ICE candidate."""
            if candidate and self._signaling:
                candidate_dict = {
                    "candidate": candidate.candidate,
                    "sdpMid": candidate.sdpMid,
                    "sdpMLineIndex": candidate.sdpMLineIndex,
                }
                asyncio.create_task(
                    self._signaling.send_ice_candidate(
                        candidate_dict,
                        self._connected_device_id
                    )
                )

        @self._pc.on("iceconnectionstatechange")
        def on_ice_connection_state_change():
            """Handle ICE connection state change."""
            state = self._pc.iceConnectionState if self._pc else "unknown"
            logger.info(f"ICE connection state: {state}")

            if state == "connected" or state == "completed":
                self._connected = True
                self._emit_state(ConnectionState.CONNECTED)
                # Start stats reporting
                asyncio.create_task(self._report_stats())
            elif state in ("failed", "disconnected", "closed"):
                self._connected = False
                self._emit_state(ConnectionState.DISCONNECTED)

        @self._pc.on("connectionstatechange")
        def on_connection_state_change():
            """Handle connection state change."""
            state = self._pc.connectionState if self._pc else "unknown"
            logger.info(f"Peer connection state: {state}")

        @self._pc.on("track")
        def on_track(track):
            """Handle incoming track from agent."""
            logger.info(f"Received track: {track.kind}")
            if track.kind == "video":
                self._video_track = track
                asyncio.create_task(self._receive_video(track))

    def _bind_signaling_callbacks(self) -> None:
        if self._signaling is None:
            return
        if self._signaling_answer_cb is None:
            def _answer_cb(answer_sdp: str):
                asyncio.create_task(self._on_remote_answer(answer_sdp))
            self._signaling_answer_cb = _answer_cb
            self._signaling.on("answer", _answer_cb)
        if self._signaling_ice_cb is None:
            def _ice_cb(candidate: dict):
                asyncio.create_task(self._on_remote_ice(candidate))
            self._signaling_ice_cb = _ice_cb
            self._signaling.on("ice_candidate", _ice_cb)

    def on_timing_sample(self, callback: Callable[[dict], None]) -> None:
        """Register callback for per-frame timing samples."""
        self._timing_callback = callback

    async def _on_remote_answer(self, answer_sdp: str) -> None:
        if self._answer_future and not self._answer_future.done():
            self._answer_future.set_result(answer_sdp)

    async def _on_remote_ice(self, candidate: dict) -> None:
        if not self._pc:
            return
        await self.add_ice_candidate(candidate)

    async def _receive_video(self, track) -> None:
        """Receive video frames from track."""
        try:
            # Initialize hardware decoder if enabled
            if self._use_hw_decoder:
                await self._init_hw_decoder()

            first_frame_logged = False
            while True:
                # on_track can fire before ICE reaches connected; keep task alive.
                if self._pc is None:
                    break
                if not self._connected:
                    await asyncio.sleep(0.01)
                    continue

                wait_start = time.perf_counter()
                frame = await track.recv()
                recv_ts = time.perf_counter()
                if frame is None:
                    break

                # Check if this is an encoded frame or decoded frame
                # aiortc typically returns decoded frames, but we can
                # access the encoded data if available

                if hasattr(frame, 'data') and frame.data is not None:
                    # Encoded frame - use hardware decoder
                    if self._hw_decoder and self._hw_decoder_initialized:
                        decoded = await self._hw_decoder.decode(
                            frame.data,
                            time.time()
                        )
                        if decoded:
                            img = decoded.data
                        else:
                            # Fallback to software decode
                            img = frame.to_ndarray(format='rgb24')
                    else:
                        img = frame.to_ndarray(format='rgb24')
                else:
                    # Decoded frame from aiortc
                    img = frame.to_ndarray(format='rgb24')

                decoded_ts = time.perf_counter()

                # Emit frame callback
                self._emit_frame(img)
                if not first_frame_logged:
                    logger.info("First video frame received")
                    first_frame_logged = True
                emit_ts = time.perf_counter()

                if self._timing_callback:
                    transport_gap_ms = None
                    output_gap_ms = None
                    if self._last_recv_ts is not None:
                        transport_gap_ms = (recv_ts - self._last_recv_ts) * 1000.0
                    if self._last_emit_ts is not None:
                        output_gap_ms = (emit_ts - self._last_emit_ts) * 1000.0
                    self._timing_callback({
                        "transport_gap_ms": transport_gap_ms,
                        "output_gap_ms": output_gap_ms,
                        "recv_wait_ms": (recv_ts - wait_start) * 1000.0,
                        "decode_ms": (decoded_ts - recv_ts) * 1000.0,
                        "emit_ms": (emit_ts - decoded_ts) * 1000.0,
                    })
                self._last_recv_ts = recv_ts
                self._last_emit_ts = emit_ts

                # Update stats
                self._stats["frames_received"] += 1

        except Exception as e:
            logger.error(f"Video receive error: {e}")

    async def _init_hw_decoder(self) -> None:
        """Initialize hardware decoder."""
        if self._hw_decoder is None and not self._hw_decoder_initialized:
            try:
                from ...decoder.hw_decoder import HWDecoder, HWDecoderConfig

                self._hw_decoder = HWDecoder(HWDecoderConfig(
                    width=1920,
                    height=1080,
                    codec="h264",
                    low_delay=self._decoder_low_delay,
                    decoder_priority=self._decoder_priority,
                ))

                self._hw_decoder_initialized = await self._hw_decoder.initialize()

                if self._hw_decoder_initialized:
                    logger.info(f"Hardware decoder initialized: {self._hw_decoder.decoder_name}")
                else:
                    logger.warning("Hardware decoder init failed, using software decode")

            except ImportError:
                logger.debug("Hardware decoder not available, using software decode")
                self._hw_decoder_initialized = False
            except Exception as e:
                logger.warning(f"Hardware decoder error: {e}, using software decode")
                self._hw_decoder_initialized = False

    async def _report_stats(self) -> None:
        """Periodically report connection statistics."""
        while self._connected and self._pc:
            try:
                # Get stats from peer connection
                stats = await self._pc.getStats()

                for report in stats.values():
                    if self._is_video_inbound_rtp_report(report):
                        bytes_received = (
                            getattr(report, "bytesReceived", None)
                            or getattr(report, "bytes_received", 0)
                            or 0
                        )
                        self._stats["bytes_received"] = bytes_received

                        # Calculate bitrate
                        now = time.time()
                        elapsed = now - self._stats["last_update"]
                        if elapsed > 0:
                            bitrate_mbps = (
                                bytes_received * 8 / elapsed / 1_000_000
                            )
                        else:
                            bitrate_mbps = 0.0

                        # Prepare stats dict
                        stats_dict = {
                            "protocol": "webrtc",
                            "bytes_received": self._stats["bytes_received"],
                            "frames_received": self._stats["frames_received"],
                            "bitrate_mbps": bitrate_mbps,
                            "timestamp": now,
                        }

                        # Add latency if available
                        if hasattr(report, "currentRoundTripTime"):
                            stats_dict["latency_ms"] = report.currentRoundTripTime * 1000
                        elif hasattr(report, "current_round_trip_time"):
                            stats_dict["latency_ms"] = report.current_round_trip_time * 1000

                        # Add packet loss if available
                        packets_lost = getattr(report, "packetsLost", None)
                        packets_recv = getattr(report, "packetsReceived", None)
                        if packets_lost is None:
                            packets_lost = getattr(report, "packets_lost", None)
                        if packets_recv is None:
                            packets_recv = getattr(report, "packets_received", None)
                        if packets_lost is not None and packets_recv is not None:
                            total = packets_lost + packets_recv
                            if total > 0:
                                stats_dict["packet_loss"] = packets_lost / total * 100

                        self._emit_stats(stats_dict)
                        self._stats["last_update"] = now

                await asyncio.sleep(1)

            except Exception as e:
                logger.error(f"Stats reporting error: {e}")
                break

    @staticmethod
    def _is_video_inbound_rtp_report(report: Any) -> bool:
        if getattr(report, "type", None) != "inbound-rtp":
            return False
        media_type = getattr(report, "mediaType", None)
        if media_type is None:
            media_type = getattr(report, "kind", None)
        if media_type is None:
            media_type = getattr(report, "media_type", None)
        return media_type == "video"

    async def add_ice_candidate(self, candidate: dict) -> bool:
        """Add remote ICE candidate."""
        if self._pc is None:
            return False

        try:
            from aiortc.sdp import candidate_from_sdp

            sdp_candidate = candidate.get("candidate", "")
            if not sdp_candidate:
                return False
            ice_candidate = candidate_from_sdp(sdp_candidate)
            ice_candidate.sdpMid = candidate.get("sdpMid")
            ice_candidate.sdpMLineIndex = candidate.get("sdpMLineIndex")

            await self._pc.addIceCandidate(ice_candidate)
            logger.debug("Added remote ICE candidate")
            return True

        except Exception as e:
            logger.error(f"Failed to add ICE candidate: {e}")
            return False

    async def disconnect(self) -> None:
        """Disconnect WebRTC connection."""
        self._connected = False

        # Close hardware decoder
        if self._hw_decoder:
            try:
                await self._hw_decoder.close()
            except Exception:
                pass
            self._hw_decoder = None
            self._hw_decoder_initialized = False

        if self._pc:
            try:
                await self._pc.close()
            except Exception:
                pass
            self._pc = None

        self._video_track = None
        self._connected_device_id = None
        if self._signaling and self._signaling_answer_cb:
            self._signaling.off("answer", self._signaling_answer_cb)
            self._signaling_answer_cb = None
        if self._signaling and self._signaling_ice_cb:
            self._signaling.off("ice_candidate", self._signaling_ice_cb)
            self._signaling_ice_cb = None
        self._answer_future = None
        self._emit_state(ConnectionState.DISCONNECTED)

    def get_decoder_stats(self) -> dict:
        """Get decoder statistics."""
        if self._hw_decoder:
            return self._hw_decoder.get_stats()
        return {}
