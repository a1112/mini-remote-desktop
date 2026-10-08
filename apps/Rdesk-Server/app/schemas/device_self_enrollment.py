from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from app.schemas.device import DeviceRegisterRequest


class DeviceSelfRegistrationPayload(DeviceRegisterRequest):
    """Machine metadata and an optional recovery target covered by the signature."""

    expected_device_id: str | None = Field(default=None, min_length=1, max_length=64, strict=True)


class DeviceSelfEnrollmentChallengeRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    protocol_version: Literal[1]
    key_id: str = Field(min_length=64, max_length=64, pattern=r"^[0-9a-f]{64}$")
    public_key: str = Field(min_length=64, max_length=64, pattern=r"^[0-9a-f]{64}$")

    @field_validator("protocol_version", mode="before")
    @classmethod
    def strict_protocol_version(cls, value: object) -> object:
        if type(value) is not int:
            raise ValueError("protocol version must be an integer")
        return value


class DeviceSelfEnrollmentChallengeResponse(BaseModel):
    protocol_version: Literal[1] = 1
    challenge_id: str
    nonce: str = Field(repr=False)
    api_url: str
    expires_at_ms: int


class DeviceSelfRegisterRequest(DeviceSelfEnrollmentChallengeRequest):
    challenge_id: str = Field(min_length=32, max_length=32, pattern=r"^[0-9a-f]{32}$")
    nonce: str = Field(min_length=64, max_length=64, pattern=r"^[0-9a-f]{64}$", repr=False)
    registration_json: str = Field(min_length=1, max_length=8192, repr=False)
    signature: str = Field(min_length=128, max_length=128, pattern=r"^[0-9a-f]{128}$", repr=False)

    @field_validator("registration_json")
    @classmethod
    def bound_registration_bytes(cls, value: str) -> str:
        if len(value.encode("utf-8")) > 8192:
            raise ValueError("registration exceeds byte limit")
        return value
