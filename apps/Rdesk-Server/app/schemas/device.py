from typing import Optional

from pydantic import BaseModel, Field


class DeviceOut(BaseModel):
    id: str
    name: str
    device_id: str
    os: str
    icon: str
    status: str
    location: str
    ping: int | None
    last_seen: str
    cpu: int | None
    ram: int | None
    disk: int | None
    ip: str
    group: str
    favorite: bool
    is_bound: bool | None = None


class DeviceRegisterRequest(BaseModel):
    """Device registration request."""

    motherboard_serial: str = Field(..., min_length=1, max_length=128)
    hostname: str = Field(..., min_length=1, max_length=128)
    os_version: str = Field(..., min_length=1, max_length=256)
    device_name: Optional[str] = Field(None, min_length=1, max_length=128)
    cpu_info: Optional[str] = None
    total_memory_mb: Optional[int] = None
    gpu_info: Optional[str] = None


class DeviceRegisterResponse(BaseModel):
    device_id: str
    device_name: str
    access_token: str


class DeviceBindRequest(BaseModel):
    device_id: str
    # Kept temporarily for client compatibility. The API never uses it as the
    # ownership principal and rejects it when it differs from the bearer user.
    user_id: str | None = None


class DeviceAutoBindRequest(BaseModel):
    device_id: str
    user_id: str | None = None


class DeviceUnbindRequest(BaseModel):
    device_id: str
    user_id: str | None = None


class DeviceBindingStatus(BaseModel):
    is_bound: bool
    bound_user_id: str | None = None
    bound_username: str | None = None
    bound_at: str | None = None


class DeviceAutoBindResponse(BaseModel):
    success: bool
    message: str
    kicked_user: dict | None = None
    is_new_binding: bool = False


class DeviceRenameRequest(BaseModel):
    name: str = Field(..., min_length=1, max_length=128)


class DeviceRenameResponse(BaseModel):
    success: bool
    message: str
    device_id: str
    new_name: str
