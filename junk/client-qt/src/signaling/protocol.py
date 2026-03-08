"""
Signaling protocol definitions for controller client.

Compatible with mini-remote-desktop signaling server.
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional
import json


@dataclass
class DeviceInfo:
    """Device information from signaling server."""
    id: str
    name: str
    online: bool = True

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary."""
        return {
            "id": self.id,
            "name": self.name,
            "online": self.online
        }

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "DeviceInfo":
        """Create from dictionary."""
        return cls(
            id=data.get("id", ""),
            name=data.get("name", ""),
            online=data.get("online", True)
        )


@dataclass
class Capabilities:
    """Client capabilities for protocol negotiation."""
    protocols: List[str] = field(default_factory=lambda: ["webrtc"])
    platforms: List[str] = field(default_factory=lambda: ["qt"])
    codecs: List[str] = field(default_factory=lambda: ["h264"])
    features: List[str] = field(default_factory=list)

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary."""
        return {
            "protocols": self.protocols,
            "platforms": self.platforms,
            "codecs": self.codecs,
            "features": self.features
        }


@dataclass
class SignalingMessage:
    """Complete signaling message with type tag."""
    type: str
    action: str
    payload: Optional[Dict[str, Any]] = None

    @classmethod
    def from_dict(cls, data: Dict[str, Any]) -> "SignalingMessage":
        """Create from dictionary (JSON parsed)."""
        return cls(
            type=data.get("type", ""),
            action=data.get("action", ""),
            payload=data.get("payload")
        )

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result = {"type": self.type, "action": self.action}
        if self.payload is not None:
            result["payload"] = self.payload
        return result

    def to_json(self) -> str:
        """Convert to JSON string."""
        return json.dumps(self.to_dict())


def create_register_message(
    name: str,
    capabilities: Optional[Capabilities] = None
) -> str:
    """
    Create a registration message for a controller.

    Args:
        name: Controller name
        capabilities: Optional capabilities object

    Returns:
        JSON string of the registration message
    """
    if capabilities is None:
        capabilities = Capabilities()

    payload = {
        "type": "controller",
        "name": name,
        "protocolVersion": 2,
        "transports": capabilities.protocols,
        "capabilities": capabilities.to_dict()
    }

    message = {
        "type": "device",
        "action": "register",
        "payload": payload
    }
    return json.dumps(message)


def create_offer_message(
    target_device_id: str,
    offer_sdp: str,
    transport: str = "webrtc",
    capabilities: Optional[Capabilities] = None
) -> str:
    """
    Create an offer message for connecting to an agent.

    Args:
        target_device_id: Target agent device ID
        offer_sdp: SDP offer string
        transport: Transport protocol (webrtc, quic, etc.)
        capabilities: Optional capabilities for negotiation

    Returns:
        JSON string of the offer message
    """
    if capabilities is None:
        capabilities = Capabilities()

    payload = {
        "targetDeviceId": target_device_id,
        "offer": {
            "type": "offer",
            "sdp": offer_sdp
        },
        "transport": transport,
        "capabilities": capabilities.to_dict()
    }

    message = {
        "type": "webrtc",
        "action": "offer",
        "payload": payload
    }
    return json.dumps(message)


def create_ice_candidate_message(
    candidate: Dict[str, Any],
    target_device_id: str
) -> str:
    """
    Create an ICE candidate message.

    Args:
        candidate: ICE candidate dictionary
        target_device_id: Target device ID

    Returns:
        JSON string of the ICE candidate message
    """
    payload = {
        "targetDeviceId": target_device_id,
        "candidate": candidate
    }

    message = {
        "type": "webrtc",
        "action": "iceCandidate",
        "payload": payload
    }
    return json.dumps(message)


def parse_device_list(payload: Optional[Dict[str, Any]]) -> List[DeviceInfo]:
    """
    Parse device list from payload.

    Args:
        payload: Message payload

    Returns:
        List of DeviceInfo objects
    """
    if not payload:
        return []

    device_list_data = payload.get("deviceList", [])
    return [DeviceInfo.from_dict(d) for d in device_list_data]


def parse_answer(payload: Optional[Dict[str, Any]]) -> Optional[str]:
    """
    Parse SDP answer from payload.

    Args:
        payload: WebRTC message payload

    Returns:
        SDP answer string or None if invalid
    """
    if not payload:
        return None

    answer_data = payload.get("answer")
    if not answer_data:
        return None

    return answer_data.get("sdp")


def parse_ice_candidate(payload: Optional[Dict[str, Any]]) -> Optional[Dict[str, Any]]:
    """
    Parse ICE candidate from payload.

    Args:
        payload: WebRTC message payload

    Returns:
        Candidate dictionary or None if invalid
    """
    if not payload:
        return None

    candidate = payload.get("candidate")
    if candidate and isinstance(candidate, dict):
        return candidate

    return None
