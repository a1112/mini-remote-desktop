"""
Main entry point for agent-python.

High-performance Python agent for mini-remote-desktop.
Compatible with signaling-rs and controller-rust.
"""

import asyncio
import logging
import signal
import sys
from pathlib import Path

# Add src directory to path for imports
sys.path.insert(0, str(Path(__file__).parent))

from config import AgentConfig
from signaling.client import SignalingClient, SignalingConfig
from capture.d3dshot_backend import ScreenCapturer
from encoder.pyav_encoder import PyAVEncoder
from webrtc.peer import WebRTCPeerManager
from utils.stats import PerformanceStats

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
    datefmt="%H:%M:%S",
)
logger = logging.getLogger("agent-python")


class PythonAgent:
    """
    Main agent class coordinating all components.

    Manages the complete pipeline:
    Screen Capture -> H.264 Encode -> WebRTC Send
    """

    def __init__(self, config: AgentConfig):
        """
        Initialize the agent.

        Args:
            config: Agent configuration
        """
        self.config = config

        # Components
        self.signaling = None
        self.capturer = None
        self.encoder = None
        self.webrtc = None
        self.stats = PerformanceStats()

        # State
        self._running = False
        self._capture_task = None
        self._signaling_task = None

    async def initialize(self) -> bool:
        """
        Initialize all components.

        Returns:
            True if successful
        """
        logger.info("Initializing Python Agent...")

        # Initialize signaling client
        sig_config = SignalingConfig(
            ws_url=self.config.ws_url,
            device_name=self.config.device_name,
        )
        self.signaling = SignalingClient(sig_config)
        self._setup_signaling_handlers()

        # Connect to signaling server
        if not await self.signaling.connect():
            logger.error("Failed to connect to signaling server")
            return False

        # Register with signaling server
        await self.signaling.register()
        logger.info(f"Registered as {self.config.device_name}")

        # Initialize capturer
        self.capturer = ScreenCapturer(
            target_fps=self.config.capture.fps,
        )
        if not await self.capturer.initialize():
            logger.error("Failed to initialize capturer")
            return False

        # Initialize encoder
        self.encoder = PyAVEncoder(
            width=self.capturer.screen_width,
            height=self.capturer.screen_height,
            fps=self.config.capture.fps,
            bitrate_kbps=self.config.capture.bitrate_kbps,
            gop_size=self.config.capture.gop,
            hardware_accel="nvenc" in self.config.capture.encoder,
            preset="ultrafast",
            tune="zerolatency",
        )
        if not await self.encoder.initialize():
            logger.error("Failed to initialize encoder")
            return False

        # Initialize WebRTC peer manager
        self.webrtc = WebRTCPeerManager(
            signaling=self.signaling,
            video_width=self.capturer.screen_width,
            video_height=self.capturer.screen_height,
            video_fps=self.config.capture.fps,
        )

        logger.info("Python Agent initialized successfully")
        logger.info(
            f"Configuration: {self.capturer.screen_width}x{self.capturer.screen_height} "
            f"@ {self.config.capture.fps}fps, {self.config.capture.bitrate_kbps}kbps"
        )
        return True

    def _setup_signaling_handlers(self) -> None:
        """Setup event handlers for signaling client."""
        if self.signaling is None:
            return

        def on_connected(device_id: str):
            """Handle connection to signaling server."""
            logger.info(f"Connected to signaling server, device ID: {device_id}")

        def on_registered(device_id: str, device_list: list):
            """Handle registration confirmation."""
            logger.info(f"Registered with device ID: {device_id}")
            if device_list:
                logger.debug(f"Available devices: {len(device_list)}")

        async def on_offer(controller_id: str, offer_sdp: str):
            """Handle WebRTC offer from controller."""
            logger.info(f"Received offer from controller: {controller_id}")

            # Create answer
            answer_sdp = await self.webrtc.create_answer(offer_sdp, controller_id)
            if answer_sdp:
                await self.signaling.send_answer(answer_sdp, controller_id)
                logger.info("Sent SDP answer")

                # Start capture loop if not already running
                if not self._running:
                    self._running = True
                    self._capture_task = asyncio.create_task(self._capture_loop())

        async def on_ice_candidate(candidate: dict):
            """Handle ICE candidate from controller."""
            await self.webrtc.add_remote_candidate(candidate)

        def on_disconnected():
            """Handle disconnection from signaling server."""
            logger.warning("Disconnected from signaling server")
            self._running = False

        # Register handlers
        self.signaling.on("connected", on_connected)
        self.signaling.on("registered", on_registered)
        self.signaling.on("offer", on_offer)
        self.signaling.on("ice_candidate", on_ice_candidate)
        self.signaling.on("disconnected", on_disconnected)

    async def _capture_loop(self) -> None:
        """Main capture loop."""
        logger.info("Starting capture loop")

        while self._running:
            try:
                # Check if WebRTC is connected
                if not self.webrtc.is_connected:
                    await asyncio.sleep(0.1)
                    continue

                # Capture frame
                frame = await self.capturer.capture_frame()
                if frame is None:
                    await asyncio.sleep(0.01)
                    continue

                self.stats.record_captured_frame()

                # Encode frame
                encoded = await self.encoder.encode(
                    frame.data,
                    frame.width,
                    frame.height,
                    frame.format,
                )

                if encoded is None:
                    continue

                self.stats.record_encoded_frame(size=len(encoded.data))

                # Send via WebRTC
                await self.webrtc.send_video_frame(encoded.data)
                self.stats.record_sent_frame()

                # Periodic stats logging
                if self.stats.frames_sent % (self.config.capture.stats_interval_ms // 10) == 0:
                    self.stats.log_summary()

            except Exception as e:
                logger.error(f"Error in capture loop: {e}")
                await asyncio.sleep(0.1)

        logger.info("Capture loop stopped")

    async def run(self) -> None:
        """Run the agent main loop."""
        self._running = True

        # Start signaling receive loop
        self._signaling_task = asyncio.create_task(self.signaling.receive_loop())

        # Wait for shutdown signal
        try:
            while self._running:
                await asyncio.sleep(1)
        except asyncio.CancelledError:
            pass

        # Cleanup
        if self._signaling_task:
            self._signaling_task.cancel()
            try:
                await self._signaling_task
            except asyncio.CancelledError:
                pass

    async def shutdown(self) -> None:
        """Shutdown the agent."""
        logger.info("Shutting down...")
        self._running = False

        if self._capture_task:
            self._capture_task.cancel()
            try:
                await self._capture_task
            except asyncio.CancelledError:
                pass

        if self.webrtc:
            await self.webrtc.close()

        if self.encoder:
            await self.encoder.close()

        if self.capturer:
            await self.capturer.close()

        if self.signaling:
            await self.signaling.disconnect()

        logger.info("Shutdown complete")


async def main_async() -> int:
    """Main async entry point."""
    # Load configuration
    config = AgentConfig.from_file()

    # Create agent
    agent = PythonAgent(config)

    # Setup signal handlers
    loop = asyncio.get_event_loop()

    def signal_handler():
        logger.info("Received shutdown signal")
        asyncio.create_task(agent.shutdown())

    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, signal_handler)
        except NotImplementedError:
            # Windows may not support all signals
            pass

    # Initialize and run
    if not await agent.initialize():
        logger.error("Failed to initialize agent")
        return 1

    try:
        await agent.run()
    except Exception as e:
        logger.error(f"Agent error: {e}", exc_info=True)
        return 1
    finally:
        await agent.shutdown()

    return 0


def main() -> int:
    """Main entry point."""
    try:
        return asyncio.run(main_async())
    except KeyboardInterrupt:
        return 0
    except Exception as e:
        logger.error(f"Fatal error: {e}", exc_info=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
