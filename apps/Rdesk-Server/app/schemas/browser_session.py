from typing import Literal
from pydantic import BaseModel, ConfigDict, Field, field_validator

from app.schemas.realtime import SignalingCredentialResponse
from app.schemas.session import DeviceSessionCreateIn, DeviceSessionOut, WanMediaProfileV3


class BrowserSessionCreateIn(DeviceSessionCreateIn):
    access_mode: Literal["attended"] = "attended"
    route_policy: Literal["direct_first", "relay_only"] = "direct_first"
    controller_public_key: list[int] = Field(min_length=32, max_length=32, repr=False)
    requested_profile: WanMediaProfileV3 | None = Field(default_factory=lambda: WanMediaProfileV3(
        width=1920, height=1080, fps=30, bitrate_mbps=10, codec="h264"))

    @field_validator("controller_public_key")
    @classmethod
    def public_key(cls, value):
        if any(type(byte) is not int or not 0 <= byte <= 255 for byte in value) or not any(value):
            raise ValueError("invalid browser public key")
        return value

    @field_validator("requested_scopes")
    @classmethod
    def browser_scopes(cls, value):
        if "screen.view" not in value or any(scope not in {
            "screen.view", "input.keyboard", "input.pointer"} for scope in value):
            raise ValueError("unsupported browser permissions")
        return value

    @field_validator("requested_profile")
    @classmethod
    def h264_only(cls, value):
        if value is not None and (value.codec != "h264" or value.bit_depth not in {None, 8}
                                  or value.hdr_enabled is True):
            raise ValueError("unsupported browser media profile")
        return value


class BrowserSessionOut(BaseModel):
    model_config = ConfigDict(extra="forbid")
    controller_device_id: str
    controller_key_id: str
    expires_at_ms: int
    session: DeviceSessionOut
    credential: SignalingCredentialResponse
    signaling_url: str
    signaling_server_device_id: str
    signaling_server_key_id: str
    target_key_id: str
    relay_directory_key_id: str
    relay_directory_public_key: list[int] = Field(min_length=32, max_length=32)


class BrowserRelayAccessIn(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    generation: int | None = Field(default=None, ge=0)
    refresh: bool = False
