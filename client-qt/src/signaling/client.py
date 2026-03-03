"""
WebSocket signaling client for controller.

Compatible with mini-remote-desktop signaling server.
"""

import asyncio
import json
import logging
from dataclasses import dataclass
from typing import Any, Callable, Dict, List, Optional

import websockets.client

from .protocol import (
    SignalingMessage,
    DeviceInfo,
    create_register_message,
    parse_device_list,
    parse_answer,
    parse_ice_candidate,
)

logger = logging.getLogger(__name__)


@dataclass
class SignalingConfig:
    """Signaling client configuration."""
    ws_url: str = "ws://127.0.0.1:9527"
    reconnect_interval: float = 5.0
    ping_interval: float = 30.0


class SignalingClient:
    """
    WebSocket signaling client for controller.

    Handles communication with the signaling server, including
    registration, device list management, SDP exchange, and
    ICE candidate forwarding.
    """

    def __init__(self, config: SignalingConfig):
        """Initialize signaling client."""
        self.config = config
        self._ws: Optional[websockets.client.WebSocketClientProtocol] = None
        self._device_id: Optional[str] = None
        self._running = False
        self._event_handlers: Dict[str, List[Callable]] = {}
        self._receive_task: Optional[asyncio.Task] = None
        self._loop = asyncio.get_event_loop()

    def on(self, event: str, callback: Callable) -> None:
        """Register an event handler."""
        if event not in self._event_handlers:
            self._event_handlers[event] = []
        self._event_handlers[event].append(callback)

    def off(self, event: str, callback: Callable) -> None:
        """Unregister an event handler."""
        if event in self._event_handlers and callback in self._event_handlers[event]:
            self._event_handlers[event].remove(callback)

    def _emit(self, event: str, *args, **kwargs) -> None:
        """Call all event handlers for an event."""
        if event in self._event_handlers:
            for callback in self._event_handlers[event]:
                try:
                    callback(*args, **kwargs)
                except Exception as e:
                    logger.error(f"Error in {event} handler: {e}")

    async def connect(self) -> bool:
        """
        Connect to the signaling server.

        Returns:
            True if connection successful
        """
        try:
            logger.info(f"Connecting to signaling server at {self.config.ws_url}")
            self._ws = await websockets.client.connect(
                self.config.ws_url,
                close_timeout=10,
                ping_interval=self.config.ping_interval,
            )
            self._running = True

            # Start receive loop
            self._receive_task = asyncio.create_task(self._receive_loop())

            logger.info("Connected to signaling server")
            return True

        except Exception as e:
            logger.error(f"Failed to connect to signaling server: {e}")
            return False

    async def disconnect(self) -> None:
        """Disconnect from the signaling server."""
        self._running = False

        if self._receive_task:
            self._receive_task.cancel()
            try:
                await self._receive_task
            except asyncio.CancelledError:
                pass
            self._receive_task = None

        if self._ws:
            try:
                await self._ws.close()
            except Exception:
                pass
            self._ws = None

        logger.info("Disconnected from signaling server")

    async def register(self, name: str, capabilities: Optional[Dict] = None) -> None:
        """
        Register this controller with the signaling server.

        Args:
            name: Controller name
            capabilities: Optional capabilities dict
        """
        if not self._ws:
            raise RuntimeError("Not connected to signaling server")

        # Import here to avoid circular import
        from .protocol import Capabilities

        caps = None
        if capabilities:
            caps = Capabilities(**capabilities)

        msg = create_register_message(name, caps)
        await self._send(msg)
        logger.info(f"Registered as {name}")

    async def send_offer(
        self,
        target_device_id: str,
        offer_sdp: str,
        transport: str = "webrtc",
        capabilities: Optional[Dict] = None
    ) -> None:
        """
        Send SDP offer to agent via signaling server.

        Args:
            target_device_id: Target agent device ID
            offer_sdp: SDP offer string
            transport: Transport protocol
            capabilities: Optional capabilities dict
        """
        from .protocol import create_offer_message, Capabilities

        caps = None
        if capabilities:
            caps = Capabilities(**capabilities)

        msg = create_offer_message(target_device_id, offer_sdp, transport, caps)
        await self._send(msg)
        logger.info(f"Sent offer to {target_device_id}")

    async def send_ice_candidate(
        self,
        candidate: Dict[str, Any],
        target_device_id: str
    ) -> None:
        """
        Send ICE candidate to agent via signaling server.

        Args:
            candidate: ICE candidate dictionary
            target_device_id: Target device ID
        """
        from .protocol import create_ice_candidate_message

        msg = create_ice_candidate_message(candidate, target_device_id)
        await self._send(msg)
        logger.debug(f"Sent ICE candidate to {target_device_id}")

    async def send_capture_update(
        self,
        target_device_id: str,
        capture_patch: Dict[str, Any],
    ) -> None:
        """
        Send runtime capture/encode control update to target agent.

        Args:
            target_device_id: Target agent device ID
            capture_patch: Partial capture config patch
        """
        msg = {
            "type": "control",
            "action": "updateCapture",
            "payload": {
                "targetDeviceId": target_device_id,
                "capture": capture_patch,
            },
        }
        await self._send(json.dumps(msg))
        logger.info("Sent capture update to %s: %s", target_device_id, capture_patch)

    async def get_device_list(self) -> None:
        """Request device list from signaling server."""
        msg = {
            "type": "device",
            "action": "getDeviceList",
            "payload": {}
        }
        await self._send(json.dumps(msg))

    async def _receive_loop(self) -> None:
        """Main receive loop for incoming messages."""
        if not self._ws:
            return

        try:
            async for raw_message in self._ws:
                if not self._running:
                    break

                try:
                    data = json.loads(raw_message)
                    await self._handle_message(data)
                except json.JSONDecodeError as e:
                    logger.warning(f"Failed to parse message as JSON: {e}")
                except Exception as e:
                    logger.error(f"Error handling message: {e}")

        except websockets.exceptions.ConnectionClosed:
            logger.info("Signaling server connection closed")
        except Exception as e:
            logger.error(f"Error in receive loop: {e}")
        finally:
            self._running = False
            self._emit("disconnected")

    async def _handle_message(self, data: Dict[str, Any]) -> None:
        """Handle an incoming signaling message."""
        msg_type = data.get("type", "")
        action = data.get("action", "")
        payload = data.get("payload")

        logger.debug(f"Received message: type={msg_type}, action={action}")

        if msg_type == "system" and action == "connected":
            device_id = payload.get("deviceId") if payload else None
            self._device_id = device_id
            self._emit("connected", device_id)

        elif msg_type == "device" and action == "registered":
            device_id = payload.get("deviceId") if payload else None
            device_list = parse_device_list(payload)
            self._device_id = device_id
            self._emit("registered", device_id, device_list)

        elif msg_type == "device" and action == "deviceList":
            device_list = parse_device_list(payload)
            self._emit("device_list", device_list)

        elif msg_type == "device" and action == "offline":
            device_id = payload.get("deviceId") if payload else None
            self._emit("device_offline", device_id)

        elif msg_type == "webrtc":
            if action == "answer":
                answer_sdp = parse_answer(payload)
                if answer_sdp:
                    self._emit("answer", answer_sdp)
            elif action == "iceCandidate":
                candidate = parse_ice_candidate(payload)
                if candidate:
                    self._emit("ice_candidate", candidate)
            elif action == "offer":
                # Controller shouldn't receive offers
                pass
            elif action == "error":
                error_msg = payload.get("message", "Unknown error") if payload else "Unknown error"
                self._emit("error", error_msg)

        elif msg_type == "stream" and action == "frame":
            # JPEG frame stream (for protocol fallback)
            self._emit("frame", payload)

        elif msg_type == "error":
            error_msg = payload.get("message", "Unknown error") if payload else "Unknown error"
            self._emit("error", error_msg)

        else:
            logger.debug(f"Unhandled message type/action: {msg_type}/{action}")

    async def _send(self, message: str) -> None:
        """Send a message to the signaling server."""
        if not self._ws:
            raise RuntimeError("Not connected to signaling server")

        try:
            await self._ws.send(message)
        except Exception as e:
            logger.error(f"Failed to send message: {e}")
            raise

    @property
    def device_id(self) -> Optional[str]:
        """Get the device ID assigned by the signaling server."""
        return self._device_id

    @property
    def is_connected(self) -> bool:
        """Check if connected to the signaling server."""
        return (
            self._ws is not None and
            not self._ws.closed and
            self._running
        )
