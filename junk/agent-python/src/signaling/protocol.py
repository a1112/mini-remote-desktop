"""
Signaling protocol definitions.

Compatible with signaling-rs and controller-rust protocol.
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

try:
    from aiortc import RTCSessionDescription
    HAS_AIORTC = True
except ImportError:
    RTCSessionDescription = None
    HAS_AIORTC = False


@dataclass
class DeviceInfo:
    """Device information."""

    id: str
    name: str
    online: bool


@dataclass
class DevicePayload:
    """Device message payload."""

    kind: Optional[str] = None
    device_type: Optional[str] = None
    name: Optional[str] = None
    device_id: Optional[str] = None
    device_list: Optional[List[DeviceInfo]] = None

    # Aliases for JSON compatibility
    type: Optional[str] = None

    def __post_init__(self):
        # Handle 'type' field mapping to device_type
        if self.type is not None and self.device_type is None:
            self.device_type = self.type


@dataclass
class SessionDescriptionJson:
    """Session description in JSON format."""

    sdp_type: str
    sdp: str

    # Alias for JSON compatibility
    type: Optional[str] = None

    def __post_init__(self):
        # Handle 'type' field mapping to sdp_type
        if self.type is not None and self.sdp_type == "answer":
            self.sdp_type = self.type

    def to_rtc_session_description(self):
        """Convert to aiortc RTCSessionDescription."""
        if not HAS_AIORTC:
            return None
        return RTCSessionDescription(self.sdp, self.sdp_type)


@dataclass
class IceCandidateJson:
    """ICE candidate in JSON format."""

    candidate: str
    sdp_mid: Optional[str] = None
    sdp_mline_index: Optional[int] = None

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for aiortc."""
        return {
            "candidate": self.candidate,
            "sdpMid": self.sdp_mid,
            "sdpMLineIndex": self.sdp_mline_index,
        }


@dataclass
class WebRTCPayload:
    """WebRTC message payload."""

    target_device_id: Optional[str] = None
    controller_id: Optional[str] = None
    session_id: Optional[str] = None
    offer: Optional[SessionDescriptionJson] = None
    answer: Optional[SessionDescriptionJson] = None
    candidate: Optional[IceCandidateJson] = None


@dataclass
class SystemPayload:
    """System message payload."""

    device_id: Optional[str] = None


@dataclass
class DeviceMessage:
    """Device message."""

    action: str
    payload: Optional[DevicePayload] = None


@dataclass
class WebRTCMessage:
    """WebRTC message."""

    action: str
    payload: Optional[WebRTCPayload] = None


@dataclass
class SystemMessage:
    """System message."""

    action: str
    payload: Optional[SystemPayload] = None


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
            payload=data.get("payload"),
        )

    def to_dict(self) -> Dict[str, Any]:
        """Convert to dictionary for JSON serialization."""
        result = {"type": self.type, "action": self.action}
        if self.payload is not None:
            result["payload"] = self.payload
        return result

    def to_json(self) -> str:
        """Convert to JSON string."""
        import json
        return json.dumps(self.to_dict())


def create_register_message(name: str) -> str:
    """
    Create a registration message for an agent.

    Args:
        name: Device name to register

    Returns:
        JSON string of the registration message
    """
    import json

    payload = {
        "type": "agent-python",
        "name": name,
    }
    message = {
        "type": "device",
        "action": "register",
        "payload": payload,
    }
    return json.dumps(message)


def create_answer_message(
    sdp: str, controller_id: str
) -> str:
    """
    Create an answer message.

    Args:
        sdp: SDP answer string
        controller_id: Controller's device ID

    Returns:
        JSON string of the answer message
    """
    import json

    payload = {
        "answer": {
            "type": "answer",
            "sdp": sdp,
        },
        "controllerId": controller_id,
    }
    message = {
        "type": "webrtc",
        "action": "answer",
        "payload": payload,
    }
    return json.dumps(message)


def create_ice_candidate_message(
    candidate_dict: Dict[str, Any], controller_id: str
) -> str:
    """
    Create an ICE candidate message.

    Args:
        candidate_dict: ICE candidate dictionary from aiortc
        controller_id: Controller's device ID

    Returns:
        JSON string of the ICE candidate message
    """
    import json

    payload = {
        "candidate": candidate_dict,
        "controllerId": controller_id,
    }
    message = {
        "type": "webrtc",
        "action": "iceCandidate",
        "payload": payload,
    }
    return json.dumps(message)


def parse_offer(payload: Dict[str, Any]) -> Optional[tuple[str, str]]:
    """
    Parse offer from payload.

    Args:
        payload: WebRTC payload dictionary

    Returns:
        Tuple of (controller_id, offer_sdp) or None if invalid
    """
    if not payload:
        return None

    controller_id = payload.get("controllerId", "")
    offer_data = payload.get("offer", {})

    if not controller_id or not offer_data:
        return None

    offer_sdp = offer_data.get("sdp", "")
    if not offer_sdp:
        return None

    return controller_id, offer_sdp


def parse_ice_candidate(payload: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """
    Parse ICE candidate from payload.

    Args:
        payload: WebRTC payload dictionary

    Returns:
        Candidate dictionary or None if invalid
    """
    if not payload:
        return None

    candidate = payload.get("candidate")
    if not candidate:
        return None

    return candidate if isinstance(candidate, dict) else None
