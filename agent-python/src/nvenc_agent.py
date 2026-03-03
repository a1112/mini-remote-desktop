"""
NVENC WebRTC Agent - Complete remote desktop agent with hardware encoding.

Integrates:
- DXGI screen capture
- NVENC hardware encoding
- Multi-protocol transport (QUIC/WebRTC) via TransportManager
- Signaling server communication
"""

import asyncio
import ctypes
import json
import logging
import numpy as np
import time
from pathlib import Path
from typing import Optional

from .config import AgentConfig
from .signaling.client import SignalingClient, SignalingConfig
from .webrtc.nvenc_track import NVENCVideoTrack

# 导入 WGC 捕获
from .capture.wgc_capture import WGCCapture

try:
    from .transport.manager import TransportManager, create_transport_manager, ProtocolType
    from .transport.stats import FrameInfo
    HAS_TRANSPORT = True
except ImportError:
    HAS_TRANSPORT = False
    ProtocolType = None  # Fallback

try:
    from aiortc import RTCPeerConnection, RTCSessionDescription, RTCConfiguration, RTCIceServer
    from aiortc.contrib.signaling import object_from_name, object_to_name
    HAS_AIORTC = True
except ImportError:
    HAS_AIORTC = False

logger = logging.getLogger(__name__)


class NVENCAgent:
    """
    NVENC-powered WebRTC agent.

    Captures screen with DXGI, encodes with NVENC, streams via multi-protocol transport.
    """

    def __init__(self, config: AgentConfig):
        """
        Initialize the NVENC agent.

        Args:
            config: Agent configuration
        """
        self.config = config

        # WGC Capture (Windows Graphics Capture)
        self._wgc_capture: Optional[WGCCapture] = None
        self._d3d11_device = None
        self._d3d11_context = None
        self._width = 0
        self._height = 0

        # Encoder
        self._video_track: Optional[NVENCVideoTrack] = None
        self._encoder = None

        # Transport Manager (multi-protocol)
        self._transport_manager: Optional[TransportManager] = None

        # Legacy WebRTC (fallback)
        self._peer_connection: Optional[RTCPeerConnection] = None
        self._controller_id: Optional[str] = None
        self._use_legacy_webrtc = False

        # Signaling
        signaling_cfg = SignalingConfig(
            ws_url=config.ws_url,
            device_name=config.device_name
        )
        self._signaling = SignalingClient(signaling_cfg)

        # State
        self._running = False
        self._capture_task: Optional[asyncio.Task] = None

        # Frame tracking
        self._frame_number = 0
        self._last_keyframe_time = 0

        # Capture mode: 'monitor' or 'window'
        self._capture_mode = getattr(config, 'capture_mode', 'monitor')
        self._capture_target = getattr(config, 'capture_target', None)  # HWND for window mode

        # Determine if we should use TransportManager
        self._use_transport_manager = HAS_TRANSPORT and hasattr(config, 'transport')

    async def initialize(self) -> bool:
        """
        Initialize the agent.

        Returns:
            True if successful
        """
        logger.info("Initializing NVENC Agent...")

        # Initialize capture
        if not await self._init_capture():
            logger.error("Failed to initialize capture")
            return False

        # Initialize encoder
        if not self._init_encoder():
            logger.error("Failed to initialize encoder")
            return False

        # Setup signaling event handlers
        self._setup_signaling_handlers()

        # Initialize transport manager if available
        if self._use_transport_manager:
            await self._init_transport_manager()

        logger.info(f"NVENC Agent initialized: {self._width}x{self._height}")
        return True

    async def _init_capture(self) -> bool:
        """Initialize WGC capture (monitor or window mode)."""
        try:
            self._wgc_capture = WGCCapture()

            if self._capture_mode == 'window' and self._capture_target:
                # 窗口捕获模式
                logger.info(f"Initializing WGC window capture: HWND=0x{self._capture_target:X}")
                if not self._wgc_capture.start_window(self._capture_target):
                    logger.error("Failed to start window capture")
                    return False
                logger.info("WGC window capture started")
            else:
                # 监视器捕获模式
                monitor_index = getattr(self.config, 'monitor_index', 0)
                logger.info(f"Initializing WGC monitor capture: index={monitor_index}")
                if not self._wgc_capture.start_monitor(monitor_index):
                    logger.error("Failed to start monitor capture")
                    logger.error("  Hint: Close Game Bar, NVIDIA Share, or other screen recorders")
                    return False
                logger.info("WGC monitor capture started")

            # Get D3D11 device/context for NVENC
            self._d3d11_device = self._wgc_capture.d3d11_device
            self._d3d11_context = self._wgc_capture.d3d11_context

            if not self._d3d11_device or not self._d3d11_context:
                logger.error("Failed to get D3D11 device/context from WGC")
                return False

            # Get dimensions
            self._width = self._wgc_capture.width
            self._height = self._wgc_capture.height

            # 等待首帧获取实际分辨率
            import time
            for attempt in range(10):
                frame = self._wgc_capture.capture_frame()
                if frame:
                    self._width = frame.width
                    self._height = frame.height
                    break
                await asyncio.sleep(0.1)

            logger.info(f"WGC Capture initialized: {self._width}x{self._height}")
            return True

        except Exception as e:
            logger.error(f"Failed to initialize WGC capture: {e}")
            import traceback
            traceback.print_exc()
            return False

    def _init_encoder(self) -> bool:
        """Initialize NVENC encoder track."""
        if not self._d3d11_device or not self._d3d11_context:
            logger.error("D3D11 device/context not available")
            return False

        quality = getattr(self.config, 'quality', 24)  # Default high quality

        try:
            self._video_track = NVENCVideoTrack(
                self._d3d11_device,
                self._d3d11_context,
                self._width,
                self._height,
                fps=self.config.framerate,
                quality=quality
            )
            self._encoder = self._video_track._encoder if hasattr(self._video_track, '_encoder') else None
            logger.info(f"NVENC track initialized: QP={quality}")
            return True
        except Exception as e:
            logger.error(f"Failed to create NVENC track: {e}")
            return False

    async def _init_transport_manager(self) -> bool:
        """Initialize transport manager for multi-protocol support."""
        if not HAS_TRANSPORT:
            return False

        try:
            from .transport.manager import TransportConfig, ProtocolType

            # Create transport config from agent config
            transport_config = TransportConfig(
                preferred=self._map_protocol(self.config.transport.preferred),
                fallback=self._map_protocol(self.config.transport.fallback),
                auto_switch=self.config.transport.auto_switch,
                connection_timeout=self.config.transport.connection_timeout,
                rtt_threshold_ms=self.config.transport.rtt_threshold_ms,
                packet_loss_threshold=self.config.transport.packet_loss_threshold,
                min_fps_threshold=self.config.transport.min_fps_threshold,
            )

            self._transport_manager = create_transport_manager(
                preferred=self.config.transport.preferred,
                auto_switch=self.config.transport.auto_switch,
            )

            # Setup protocol switch event handler
            self._transport_manager.on("protocol_switched", self._on_protocol_switched)

            logger.info(f"Transport manager initialized: {self._transport_manager.available_protocols}")
            return True

        except Exception as e:
            logger.warning(f"Failed to initialize transport manager: {e}, falling back to legacy WebRTC")
            self._use_legacy_webrtc = True
            return False

    def _map_protocol(self, proto_str: str):
        """Map protocol string to ProtocolType enum."""
        if ProtocolType is None:
            return None

        proto_map = {
            "auto": ProtocolType.AUTO,
            "quic": ProtocolType.QUIC,
            "webrtc": ProtocolType.WEBRTC,
        }
        return proto_map.get(proto_str.lower(), ProtocolType.AUTO)

    async def _on_protocol_switched(self, info: dict) -> None:
        """Handle protocol switched event."""
        from_proto = info.get("from", "unknown")
        to_proto = info.get("to", "unknown")
        reason = info.get("reason", "unknown")

        logger.info(f"Protocol switched: {from_proto} -> {to_proto} (reason: {reason})")

        # Request keyframe after switch
        if self._encoder:
            self._encoder.request_keyframe()

    def _setup_signaling_handlers(self) -> None:
        """Setup signaling event handlers."""
        self._signaling.on("offer", self._on_offer)
        self._signaling.on("ice_candidate", self._on_ice_candidate)

    async def start(self) -> None:
        """Start the agent."""
        logger.info("Starting NVENC Agent...")

        # Connect to signaling server
        if not await self._signaling.connect():
            logger.error("Failed to connect to signaling server")
            return

        # Register
        await self._signaling.register()

        # Start signaling receive loop
        asyncio.create_task(self._signaling.receive_loop())

        # Start capture loop
        self._running = True
        self._capture_task = asyncio.create_task(self._capture_loop())

        logger.info("NVENC Agent started")

    async def stop(self) -> None:
        """Stop the agent."""
        logger.info("Stopping NVENC Agent...")
        self._running = False

        # Stop capture loop
        if self._capture_task:
            self._capture_task.cancel()
            try:
                await self._capture_task
            except asyncio.CancelledError:
                pass

        # Disconnect transport manager
        if self._transport_manager:
            await self._transport_manager.disconnect()
            self._transport_manager = None

        # Close peer connection (legacy)
        if self._peer_connection:
            await self._peer_connection.close()
            self._peer_connection = None

        # Stop video track
        if self._video_track:
            self._video_track.stop()

        # Disconnect signaling
        await self._signaling.disconnect()

        # Stop WGC capture
        if self._wgc_capture:
            self._wgc_capture.stop()
            self._wgc_capture = None

        logger.info("NVENC Agent stopped")

    async def _capture_loop(self) -> None:
        """Main capture loop using WGC."""
        if not self._wgc_capture:
            return

        buffer_size = self._width * self._height * 4
        buffer = (ctypes.c_ubyte * buffer_size)()

        frame_time = 1.0 / self.config.framerate

        use_transport_manager = self._transport_manager and not self._use_legacy_webrtc

        logger.info(f"Capture loop started: {self.config.framerate} fps "
                   f"(mode: {'TransportManager' if use_transport_manager else 'Legacy'})")

        while self._running:
            start_time = asyncio.get_event_loop().time()

            # Capture frame using WGC
            frame = self._wgc_capture.capture_frame()

            if frame:
                sent_gpu_direct = False

                # Prefer GPU-direct texture path for transport manager pipeline.
                if use_transport_manager and self._encoder:
                    texture_ptr = int(frame.d3d11_texture) if frame.d3d11_texture else 0
                    if texture_ptr:
                        sent_gpu_direct = await self._encode_and_send_texture(texture_ptr, frame.timestamp)

                # Fallback to CPU path (legacy and/or GPU-direct unavailable).
                if not sent_gpu_direct and self._wgc_capture.copy_to_cpu(buffer):
                    frame_bytes = bytes(buffer)
                    if use_transport_manager and self._encoder:
                        await self._encode_and_send_frame(frame_bytes, frame.timestamp)
                    elif self._video_track:
                        await self._video_track.send_frame(frame_bytes)

            # Frame rate control
            elapsed = asyncio.get_event_loop().time() - start_time
            sleep_time = max(0, frame_time - elapsed)
            await asyncio.sleep(sleep_time)

    async def _encode_and_send_frame(self, frame_bytes: bytes, timestamp: int) -> None:
        """
        Encode frame with NVENC and send via TransportManager.

        Args:
            frame_bytes: Raw BGRA frame data
            timestamp: Frame timestamp
        """
        if not self._encoder or not self._transport_manager:
            return

        try:
            # Encode with NVENC
            encoded = self._encoder.encode(frame_bytes)
            if encoded:
                await self._send_encoded_frame(encoded, timestamp)

        except Exception as e:
            logger.debug(f"Frame encode/send error: {e}")

    async def _encode_and_send_texture(self, d3d11_texture_ptr: int, timestamp: int) -> bool:
        """
        Encode GPU texture directly and send via TransportManager.

        Returns:
            True if direct path succeeded and frame was sent.
        """
        if not self._encoder or not self._transport_manager:
            return False

        try:
            encoded = self._encoder.encode_d3d11(d3d11_texture_ptr)
            if not encoded:
                return False

            await self._send_encoded_frame(encoded, timestamp)
            return True
        except Exception as e:
            logger.debug(f"GPU-direct encode/send error: {e}")
            return False

    async def _send_encoded_frame(self, encoded, timestamp: int) -> None:
        """Send already encoded frame via transport manager."""
        # Detect keyframe (H.264 IDR frame)
        is_keyframe = self._is_keyframe(encoded.data)

        frame_info = FrameInfo(
            data=encoded.data,
            timestamp=timestamp or int(time.time() * 1000000),
            is_keyframe=is_keyframe,
            width=self._width,
            height=self._height,
            frame_number=self._frame_number,
        )

        await self._transport_manager.send_media(frame_info)

        self._frame_number += 1

        if is_keyframe:
            self._last_keyframe_time = time.time()

    def _is_keyframe(self, data: bytes) -> bool:
        """Check if H.264 data is a keyframe (IDR)."""
        # H.264 NAL units start with 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01
        # IDR frame NAL type is 5 (lower 5 bits of nal_header)

        # Find NAL unit start codes
        i = 0
        while i < len(data) - 4:
            if data[i:i+3] == b'\x00\x00\x01' or data[i:i+4] == b'\x00\x00\x00\x01':
                # Skip start code
                if data[i:i+4] == b'\x00\x00\x00\x01':
                    i += 4
                else:
                    i += 3

                if i >= len(data):
                    break

                # NAL header: first byte
                # nal_type = nal_header & 0x1F
                nal_type = data[i] & 0x1F

                # NAL type 5 is IDR (keyframe)
                if nal_type == 5:
                    return True
            i += 1

        return False

    async def _on_offer(self, controller_id: str, offer_sdp: str) -> None:
        """Handle WebRTC/QUIC offer from controller."""
        logger.info(f"Received offer from {controller_id}")

        self._controller_id = controller_id

        # Check if offer contains QUIC protocol hint
        try:
            offer_data = json.loads(offer_sdp) if offer_sdp.startswith('{') else {}
            is_quic_offer = offer_data.get("protocol") == "quic"
        except (json.JSONDecodeError, TypeError):
            is_quic_offer = False

        # Use TransportManager if available and appropriate
        if self._transport_manager and not self._use_legacy_webrtc:
            await self._handle_offer_transport_manager(controller_id, offer_sdp, offer_data if is_quic_offer else {})
        elif HAS_AIORTC:
            await self._handle_offer_legacy_webrtc(controller_id, offer_sdp)
        else:
            logger.error("No transport available")

    async def _handle_offer_transport_manager(
        self,
        controller_id: str,
        offer: str,
        metadata: dict
    ) -> None:
        """Handle offer via TransportManager."""
        try:
            metadata["controller_id"] = controller_id

            answer = await self._transport_manager.connect(offer, metadata)

            # Send answer via signaling
            await self._signaling.send_answer(answer, controller_id)
            logger.info(f"Sent answer via {self._transport_manager.active_protocol.upper()}")

        except Exception as e:
            logger.error(f"TransportManager connection failed: {e}")
            # Fallback to legacy WebRTC
            self._use_legacy_webrtc = True
            if HAS_AIORTC:
                await self._handle_offer_legacy_webrtc(controller_id, offer)

    async def _handle_offer_legacy_webrtc(self, controller_id: str, offer_sdp: str) -> None:
        """Handle offer via legacy WebRTC path."""
        if not HAS_AIORTC:
            logger.error("aiortc not available")
            return

        # Create peer connection
        if self._peer_connection:
            await self._peer_connection.close()

        self._peer_connection = RTCPeerConnection()
        self._peer_connection.on("icecandidate", self._on_ice_candidate)

        # Add video track
        if self._video_track:
            self._peer_connection.addTrack(self._video_track)

        # Set remote description (offer)
        await self._peer_connection.setRemoteDescription(
            RTCSessionDescription(sdp=offer_sdp, type="offer")
        )

        # Create answer
        answer = await self._peer_connection.createAnswer()
        await self._peer_connection.setLocalDescription(answer)

        # Send answer via signaling
        await self._signaling.send_answer(answer.sdp, controller_id)
        logger.info("Sent answer to controller (legacy WebRTC)")

    async def _on_ice_candidate(self, candidate) -> None:
        """Handle local ICE candidate."""
        if not self._controller_id:
            return

        candidate_dict = {
            "candidate": candidate.candidate,
            "sdpMid": candidate.sdpMid,
            "sdpMLineIndex": candidate.sdpMLineIndex,
        }
        await self._signaling.send_ice_candidate(candidate_dict, self._controller_id)

    async def _on_ice_candidate(self, candidate_dict: dict) -> None:
        """Handle remote ICE candidate."""
        if not self._peer_connection:
            return

        from aiortc import RTCIceCandidate

        candidate = RTCIceCandidate(
            sdpMid=candidate_dict.get("sdpMid"),
            sdpMLineIndex=candidate_dict.get("sdpMLineIndex"),
            candidate=candidate_dict.get("candidate")
        )
        await self._peer_connection.addIceCandidate(candidate)

    @property
    def is_running(self) -> bool:
        """Check if agent is running."""
        return self._running

    @property
    def stats(self) -> dict:
        """Get agent statistics."""
        stats = {
            "width": self._width,
            "height": self._height,
            "framerate": self.config.framerate,
            "frame_number": self._frame_number,
        }

        if self._video_track:
            stats["encoder"] = self._video_track.stats

        # Add transport statistics
        if self._transport_manager:
            stats["transport"] = self._transport_manager.get_stats_dict()

        return stats

    @property
    def transport_manager(self) -> Optional[TransportManager]:
        """Get the transport manager."""
        return self._transport_manager

    @property
    def active_protocol(self) -> Optional[str]:
        """Get the active transport protocol."""
        if self._transport_manager:
            return self._transport_manager.active_protocol
        return "webrtc" if self._peer_connection else None


async def run_agent(config: AgentConfig) -> None:
    """
    Run the NVENC agent.

    Args:
        config: Agent configuration
    """
    agent = NVENCAgent(config)

    if not await agent.initialize():
        logger.error("Failed to initialize agent")
        return

    try:
        await agent.start()

        # Keep running
        while agent.is_running:
            await asyncio.sleep(1)

    except KeyboardInterrupt:
        logger.info("Interrupted by user")
    finally:
        await agent.stop()


if __name__ == "__main__":
    import sys
    from .config import AgentConfig

    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s"
    )

    config = AgentConfig.from_file_or_default()

    asyncio.run(run_agent(config))
