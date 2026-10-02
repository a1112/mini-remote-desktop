from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class SignalingCredentialRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    device_key_id: str = Field(pattern=r"^[a-f0-9]{64}$")
    role: Literal["Controller", "Agent"]


class SignalingCredentialResponse(BaseModel):
    token: str = Field(repr=False)
    expires_at_ms: int
    device_id: str
    device_key_id: str
    role: Literal["Controller", "Agent"]
