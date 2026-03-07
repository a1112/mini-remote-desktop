from pydantic import BaseModel


class SessionRequestIn(BaseModel):
    requester_user_id: str
    target_device_id: str


class SessionRequestOut(BaseModel):
    request_id: str
    signaling_url: str
    room: str
    status: str
