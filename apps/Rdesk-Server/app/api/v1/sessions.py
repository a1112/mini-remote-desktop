from datetime import datetime

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.config import settings
from app.db.session import get_db
from app.models.device import Device
from app.models.session_request import SessionRequest
from app.models.user import User
from app.schemas.session import SessionRequestIn, SessionRequestOut

router = APIRouter(prefix="/sessions", tags=["sessions"])


@router.post("/request", response_model=SessionRequestOut)
async def request_session(
    payload: SessionRequestIn, db: AsyncSession = Depends(get_db)
) -> SessionRequestOut:
    user = await db.scalar(select(User).where(User.id == payload.requester_user_id))
    if not user:
        raise HTTPException(status_code=404, detail="Requester not found")

    device = await db.scalar(select(Device).where(Device.id == payload.target_device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Target device not found")

    room = f"{payload.requester_user_id}:{payload.target_device_id}:{int(datetime.utcnow().timestamp())}"
    req = SessionRequest(
        requester_user_id=payload.requester_user_id,
        target_device_id=payload.target_device_id,
        signaling_room=room,
        status="requested",
    )
    db.add(req)
    await db.commit()
    await db.refresh(req)
    return SessionRequestOut(
        request_id=req.id,
        signaling_url=settings.signaling_ws_url,
        room=room,
        status=req.status,
    )
