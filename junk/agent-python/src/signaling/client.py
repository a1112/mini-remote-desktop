"""
WebSocket signaling client.

Compatible with signaling-rs server.
"""

import asyncio
import json
import logging
from dataclasses import dataclass
from typing import Any, Callable, Dict, Optional

import websockets.client

from .protocol import (
    SignalingMessage,
    create_register_message,
    create_answer_message,
    create_ice_candidate_message,
    parse_offer,
    parse_ice_candidate,
)

logger = logging.getLogger(__name__)


@dataclass
class SignalingConfig:
    """Signaling client configuration."""

    ws_url: str = "ws://127.0.0.1:9527"
    device_name: str = "Python Agent"
    reconnect_interval: float = 5.0
    ping_interval: float = 30.0


class SignalingClient:
    """
    WebSocket signaling client for agent-python.

    Handles communication with the signaling server, including
    registration, SDP exchange, and ICE candidate forwarding.
    """

    def __init__(self, config: SignalingConfig):
        self.config = config
        self._ws: Optional[websockets.client.WebSocketClientProtocol] = None
        self._device_id: Optional[str] = None
        self._running = False
        self._message_queue: asyncio.Queue[Dict[str, Any]] = asyncio.Queue()
        self._event_handlers: Dict[str, Callable] = {}

    def on(self, event: str, callback: Callable) -> None:
        """Register an event handler."""
        self._event_handlers[event] = callback

    def _emit(self, event: str, *args, **kwargs) -> None:
        """Call an event handler if registered."""
        if event in self._event_handlers:
            try:
                self._event_handlers[event](*args, **kwargs)
            except Exception as e:
                logger.error(f"Error in {event} handler: {e}")

    async def connect(self) -> bool:
        """
        Connect to the signaling server.

        Returns:
            True if connection successful, False otherwise
        """
        try:
            logger.info(f"Connecting to signaling server at {self.config.ws_url}")
            self._ws = await websockets.client.connect(
                self.config.ws_url,
                close_timeout=10,
                ping_interval=self.config.ping_interval,
            )
            self._running = True
            logger.info("Connected to signaling server")
            return True
        except Exception as e:
            logger.error(f"Failed to connect to signaling server: {e}")
            return False

    async def disconnect(self) -> None:
        """Disconnect from the signaling server."""
        self._running = False
        if self._ws:
            try:
                await self._ws.close()
            except Exception:
                pass
            self._ws = None
        logger.info("Disconnected from signaling server")

    async def register(self) -> None:
        """Register this agent with the signaling server."""
        if not self._ws:
            raise RuntimeError("Not connected to signaling server")

        msg = create_register_message(self.config.device_name)
        await self._send(msg)
        logger.info(f"Registered as {self.config.device_name}")

    async def send_answer(self, sdp: str, controller_id: str) -> None:
        """
        Send SDP answer to controller via signaling server.

        Args:
            sdp: SDP answer string
            controller_id: Controller's device ID
        """
        msg = create_answer_message(sdp, controller_id)
        await self._send(msg)
        logger.debug("Sent SDP answer")

    async def send_ice_candidate(
        self, candidate_dict: Dict[str, Any], controller_id: str
    ) -> None:
        """
        Send ICE candidate to controller via signaling server.

        Args:
            candidate_dict: ICE candidate dictionary from aiortc
            controller_id: Controller's device ID
        """
        msg = create_ice_candidate_message(candidate_dict, controller_id)
        await self._send(msg)
        logger.debug(f"Sent ICE candidate: {candidate_dict.get('candidate', '')[:50]}...")

    async def receive_loop(self) -> None:
        """
        Main receive loop for incoming messages.

        This method runs continuously and emits events for
        received messages. Call it as a background task.
        """
        if not self._ws:
            raise RuntimeError("Not connected to signaling server")

        try:
            async for raw_message in self._ws:
                if not self._running:
                    break

                try:
                    data = json.loads(raw_message)
                    msg = SignalingMessage.from_dict(data)
                    await self._handle_message(msg)
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

    async def _handle_message(self, msg: SignalingMessage) -> None:
        """Handle an incoming signaling message."""
        msg_type = msg.type
        action = msg.action

        logger.debug(f"Received message: type={msg_type}, action={action}")

        if msg_type == "system" and action == "connected":
            device_id = msg.payload.get("deviceId") if msg.payload else None
            self._device_id = device_id
            self._emit("connected", device_id)

        elif msg_type == "device" and action == "registered":
            device_id = msg.payload.get("deviceId") if msg.payload else None
            device_list = msg.payload.get("deviceList", []) if msg.payload else []
            self._device_id = device_id
            self._emit("registered", device_id, device_list)

        elif msg_type == "device" and action == "deviceList":
            device_list = msg.payload.get("deviceList", []) if msg.payload else []
            self._emit("device_list", device_list)

        elif msg_type == "device" and action == "offline":
            device_id = msg.payload.get("deviceId") if msg.payload else None
            self._emit("device_offline", device_id)

        elif msg_type == "webrtc" and action == "offer":
            result = parse_offer(msg.payload) if msg.payload else None
            if result:
                controller_id, offer_sdp = result
                self._emit("offer", controller_id, offer_sdp)
            else:
                logger.warning("Received invalid offer payload")

        elif msg_type == "webrtc" and action == "iceCandidate":
            candidate = parse_ice_candidate(msg.payload) if msg.payload else None
            if candidate:
                self._emit("ice_candidate", candidate)
            else:
                logger.warning("Received invalid ICE candidate payload")

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
        return self._ws is not None and not self._ws.closed and self._running
