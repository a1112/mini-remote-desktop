"""
NVENC WebRTC Agent - Complete remote desktop agent with hardware encoding.

Integrates:
- DXGI screen capture
- NVENC hardware encoding
- WebRTC transport via aiortc
- Signaling server communication
"""

import asyncio
import ctypes
import logging
import numpy as np
from pathlib import Path
from typing import Optional

from .config import AgentConfig
from .signaling.client import SignalingClient, SignalingConfig
from .webrtc.nvenc_track import NVENCVideoTrack

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

    Captures screen with DXGI, encodes with NVENC, streams via WebRTC.
    """

    def __init__(self, config: AgentConfig):
        """
        Initialize the NVENC agent.

        Args:
            config: Agent configuration
        """
        self.config = config

        # Hybrid capture (DXGI + D3D11)
        self._capture_dll = None
        self._capture_handle = None
        self._d3d11_device = None
        self._d3d11_context = None
        self._width = 0
        self._height = 0

        # Encoder
        self._video_track: Optional[NVENCVideoTrack] = None

        # WebRTC
        self._peer_connection: Optional[RTCPeerConnection] = None
        self._controller_id: Optional[str] = None

        # Signaling
        signaling_cfg = SignalingConfig(
            ws_url=config.ws_url,
            device_name=config.device_name
        )
        self._signaling = SignalingClient(signaling_cfg)

        # State
        self._running = False
        self._capture_task: Optional[asyncio.Task] = None

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

        # Initialize encoder track
        if not self._init_encoder():
            logger.error("Failed to initialize encoder")
            return False

        # Setup signaling event handlers
        self._setup_signaling_handlers()

        logger.info(f"NVENC Agent initialized: {self._width}x{self._height}")
        return True

    async def _init_capture(self) -> bool:
        """Initialize DXGI hybrid capture."""
        dll_path = Path(__file__).parent.parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'
        if not dll_path.exists():
            logger.error(f"Capture DLL not found: {dll_path}")
            return False

        try:
            self._capture_dll = ctypes.CDLL(str(dll_path))

            # Setup function signatures
            class HybridFrame(ctypes.Structure):
                _fields_ = [
                    ("width", ctypes.c_int),
                    ("height", ctypes.c_int),
                    ("stride", ctypes.c_int),
                    ("format", ctypes.c_int),
                    ("timestamp", ctypes.c_longlong),
                    ("d3d11_resource", ctypes.c_void_p),
                    ("d3d12_resource", ctypes.c_void_p),
                ]

            self._capture_dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
            self._capture_dll.init_hybrid_capture.restype = ctypes.c_void_p

            self._capture_dll.capture_hybrid_frame.argtypes = [
                ctypes.c_void_p, ctypes.POINTER(HybridFrame)
            ]
            self._capture_dll.capture_hybrid_frame.restype = ctypes.c_int

            self._capture_dll.copy_hybrid_frame_to_cpu.argtypes = [
                ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.c_int
            ]
            self._capture_dll.copy_hybrid_frame_to_cpu.restype = ctypes.c_int

            self._capture_dll.get_hybrid_d3d11_device.argtypes = [ctypes.c_void_p]
            self._capture_dll.get_hybrid_d3d11_device.restype = ctypes.c_void_p

            self._capture_dll.get_hybrid_d3d11_context.argtypes = [ctypes.c_void_p]
            self._capture_dll.get_hybrid_d3d11_context.restype = ctypes.c_void_p

            self._capture_dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
            self._capture_dll.free_hybrid_capture.restype = None

            # Initialize capture
            self._capture_handle = self._capture_dll.init_hybrid_capture(
                self.config.monitor_index, 0
            )

            if not self._capture_handle:
                logger.error("Failed to initialize capture")
                return False

            # Get D3D11 device/context for NVENC
            self._d3d11_device = self._capture_dll.get_hybrid_d3d11_device(self._capture_handle)
            self._d3d11_context = self._capture_dll.get_hybrid_d3d11_context(self._capture_handle)

            # Get screen dimensions
            frame_info = HybridFrame()
            self._capture_dll.capture_hybrid_frame(
                self._capture_handle, ctypes.byref(frame_info)
            )
            self._width = frame_info.width
            self._height = frame_info.height

            logger.info(f"Capture initialized: {self._width}x{self._height}")
            return True

        except Exception as e:
            logger.error(f"Failed to initialize capture: {e}")
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
            logger.info(f"NVENC track initialized: QP={quality}")
            return True
        except Exception as e:
            logger.error(f"Failed to create NVENC track: {e}")
            return False

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

        # Close peer connection
        if self._peer_connection:
            await self._peer_connection.close()
            self._peer_connection = None

        # Stop video track
        if self._video_track:
            self._video_track.stop()

        # Disconnect signaling
        await self._signaling.disconnect()

        # Free capture
        if self._capture_handle and self._capture_dll:
            self._capture_dll.free_hybrid_capture(self._capture_handle)
            self._capture_handle = None

        logger.info("NVENC Agent stopped")

    async def _capture_loop(self) -> None:
        """Main capture loop."""
        if not self._capture_dll or not self._capture_handle:
            return

        class HybridFrame(ctypes.Structure):
            _fields_ = [
                ("width", ctypes.c_int),
                ("height", ctypes.c_int),
                ("stride", ctypes.c_int),
                ("format", ctypes.c_int),
                ("timestamp", ctypes.c_longlong),
                ("d3d11_resource", ctypes.c_void_p),
                ("d3d12_resource", ctypes.c_void_p),
            ]

        buffer_size = self._width * self._height * 4
        buffer = (ctypes.c_ubyte * buffer_size)()
        frame_info = HybridFrame()

        frame_time = 1.0 / self.config.framerate

        logger.info(f"Capture loop started: {self.config.framerate} fps")

        while self._running:
            start_time = asyncio.get_event_loop().time()

            # Capture frame
            result = self._capture_dll.capture_hybrid_frame(
                self._capture_handle, ctypes.byref(frame_info)
            )

            if result == 1:
                # Copy to CPU
                copy_result = self._capture_dll.copy_hybrid_frame_to_cpu(
                    self._capture_handle, buffer, buffer_size
                )

                if copy_result == 1 and self._video_track:
                    # Send to encoder track
                    frame_bytes = bytes(buffer)
                    await self._video_track.send_frame(frame_bytes)

            # Frame rate control
            elapsed = asyncio.get_event_loop().time() - start_time
            sleep_time = max(0, frame_time - elapsed)
            await asyncio.sleep(sleep_time)

    async def _on_offer(self, controller_id: str, offer_sdp: str) -> None:
        """Handle WebRTC offer from controller."""
        logger.info(f"Received offer from {controller_id}")

        self._controller_id = controller_id

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
        logger.info("Sent answer to controller")

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
        }

        if self._video_track:
            stats["encoder"] = self._video_track.stats

        return stats


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
