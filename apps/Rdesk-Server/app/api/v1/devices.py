from datetime import datetime

from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy import Select, select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import selectinload

from app.core.security import create_access_token, get_current_user
from app.db.session import get_db
from app.models.device import Device, generate_device_id_from_serial
from app.models.user import User
from app.schemas.device import (
    DeviceAutoBindRequest,
    DeviceAutoBindResponse,
    DeviceBindRequest,
    DeviceBindingStatus,
    DeviceOut,
    DeviceRegisterRequest,
    DeviceRegisterResponse,
    DeviceRenameRequest,
    DeviceRenameResponse,
    DeviceUnbindRequest,
)

router = APIRouter(prefix="/devices", tags=["devices"])


def _is_admin(user: User) -> bool:
    return user.role == "admin"


def _can_access_device(user: User, device: Device) -> bool:
    return _is_admin(user) or (
        bool(device.is_bound) and device.bound_user_id == user.id
    )


def _require_device_access(user: User, device: Device, *, hide: bool) -> None:
    if _can_access_device(user, device):
        return
    if hide:
        raise HTTPException(status_code=404, detail="Device not found")
    raise HTTPException(status_code=403, detail="Device belongs to another user")


def _validate_legacy_user_id(requested_user_id: str | None, user: User) -> None:
    if requested_user_id is not None and requested_user_id != user.id:
        raise HTTPException(
            status_code=403,
            detail="Request user_id does not match the authenticated user",
        )


def _to_out(device: Device) -> DeviceOut:
    status = device.status
    return DeviceOut(
        id=device.id,
        name=device.name,
        device_id=device.device_id,
        os=device.os,
        icon=device.icon,
        status=status.status if status else "offline",
        location=device.location,
        ping=status.ping if status else None,
        last_seen=status.last_seen if status else "离线",
        cpu=status.cpu if status else None,
        ram=status.ram if status else None,
        disk=status.disk if status else None,
        ip=device.ip,
        group=device.group,
        favorite=device.favorite,
        is_bound=device.is_bound,
    )


@router.post("/register", response_model=DeviceRegisterResponse)
async def register_device(
    payload: DeviceRegisterRequest,
    db: AsyncSession = Depends(get_db),
) -> DeviceRegisterResponse:
    existing = await db.scalar(
        select(Device).where(Device.motherboard_serial == payload.motherboard_serial)
    )
    os_type = payload.os_version.split()[0] if payload.os_version else "Unknown"

    if existing:
        if payload.hostname:
            existing.hostname = payload.hostname
        if payload.os_version:
            existing.os_version = payload.os_version
            existing.os = os_type
        if payload.cpu_info:
            existing.cpu_info = payload.cpu_info
        if payload.total_memory_mb:
            existing.total_memory_mb = payload.total_memory_mb
        if payload.gpu_info:
            existing.gpu_info = payload.gpu_info
        if payload.device_name:
            existing.name = payload.device_name

        await db.commit()
        await db.refresh(existing)
        access_token = create_access_token(existing.device_id, existing.name, "device")
        return DeviceRegisterResponse(
            device_id=existing.device_id,
            device_name=existing.name,
            access_token=access_token,
        )

    device_id = generate_device_id_from_serial(payload.motherboard_serial)
    id_conflict = await db.scalar(select(Device).where(Device.device_id == device_id))
    if id_conflict:
        import uuid

        device_id = f"{device_id}-{uuid.uuid4().hex[:4]}"

    new_device = Device(
        name=payload.device_name or payload.hostname,
        device_id=device_id,
        os=os_type,
        os_version=payload.os_version,
        hostname=payload.hostname,
        motherboard_serial=payload.motherboard_serial,
        cpu_info=payload.cpu_info,
        total_memory_mb=payload.total_memory_mb,
        gpu_info=payload.gpu_info,
        is_bound=False,
    )
    db.add(new_device)
    await db.commit()
    await db.refresh(new_device)
    access_token = create_access_token(new_device.device_id, new_device.name, "device")
    return DeviceRegisterResponse(
        device_id=new_device.device_id,
        device_name=new_device.name,
        access_token=access_token,
    )


@router.get("/check/{motherboard_serial}")
async def check_device_registration(
    motherboard_serial: str,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> dict:
    device = await db.scalar(
        select(Device).where(Device.motherboard_serial == motherboard_serial)
    )
    if not device or not _can_access_device(current_user, device):
        return {"registered": False}
    return {
        "registered": True,
        "device_id": device.device_id,
        "device_name": device.name,
        "is_bound": device.is_bound,
    }


@router.post("/bind")
async def bind_device(
    payload: DeviceBindRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> dict:
    _validate_legacy_user_id(payload.user_id, current_user)
    device = await db.scalar(select(Device).where(Device.device_id == payload.device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    if device.is_bound and device.bound_user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Device is already bound")

    device.is_bound = True
    device.bound_user_id = current_user.id
    device.bound_at = datetime.utcnow()
    await db.commit()
    return {
        "message": "Device bound successfully",
        "device_id": device.device_id,
        "user_id": current_user.id,
    }


@router.get("", response_model=list[DeviceOut])
async def list_devices(
    q: str | None = Query(default=None),
    status: str | None = Query(default=None),
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> list[DeviceOut]:
    stmt: Select[tuple[Device]] = select(Device).options(selectinload(Device.status))
    if not _is_admin(current_user):
        stmt = stmt.where(Device.bound_user_id == current_user.id)
    if q:
        stmt = stmt.where(Device.name.ilike(f"%{q}%"))
    rows = (await db.scalars(stmt)).all()
    if not _is_admin(current_user):
        rows = [item for item in rows if item.bound_user_id == current_user.id]
    result = [_to_out(item) for item in rows]
    if status:
        result = [item for item in result if item.status == status]
    return result


@router.get("/{device_id}", response_model=DeviceOut)
async def get_device(
    device_id: str,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> DeviceOut:
    stmt = (
        select(Device)
        .where(Device.id == device_id)
        .options(selectinload(Device.status))
    )
    device = await db.scalar(stmt)
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    _require_device_access(current_user, device, hide=True)
    return _to_out(device)


@router.post("/auto-bind", response_model=DeviceAutoBindResponse)
async def auto_bind_device(
    payload: DeviceAutoBindRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> DeviceAutoBindResponse:
    _validate_legacy_user_id(payload.user_id, current_user)
    device = await db.scalar(select(Device).where(Device.device_id == payload.device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    if device.is_bound and device.bound_user_id != current_user.id:
        raise HTTPException(status_code=403, detail="Device is already bound")

    is_new_binding = not device.is_bound
    device.is_bound = True
    device.bound_user_id = current_user.id
    device.bound_at = datetime.utcnow()
    await db.commit()
    return DeviceAutoBindResponse(
        success=True,
        message="Device bound successfully",
        kicked_user=None,
        is_new_binding=is_new_binding,
    )


@router.post("/unbind")
async def unbind_device(
    payload: DeviceUnbindRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> dict:
    _validate_legacy_user_id(payload.user_id, current_user)
    device = await db.scalar(select(Device).where(Device.device_id == payload.device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    if not device.is_bound:
        if not _is_admin(current_user):
            raise HTTPException(status_code=404, detail="Device not found")
        return {"message": "Device not bound", "success": True}
    _require_device_access(current_user, device, hide=False)

    device.is_bound = False
    device.bound_user_id = None
    device.bound_at = None
    await db.commit()
    return {"message": "Device unbound successfully", "success": True}


@router.get("/{device_id}/binding-status", response_model=DeviceBindingStatus)
async def get_binding_status(
    device_id: str,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> DeviceBindingStatus:
    device = await db.scalar(select(Device).where(Device.device_id == device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    _require_device_access(current_user, device, hide=True)

    bound_username = None
    if device.is_bound and device.bound_user_id:
        user = await db.scalar(select(User).where(User.id == device.bound_user_id))
        bound_username = user.username if user else None
    return DeviceBindingStatus(
        is_bound=device.is_bound,
        bound_user_id=device.bound_user_id,
        bound_username=bound_username,
        bound_at=device.bound_at.isoformat() if device.bound_at else None,
    )


@router.patch("/{device_id}/rename", response_model=DeviceRenameResponse)
async def rename_device(
    device_id: str,
    payload: DeviceRenameRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> DeviceRenameResponse:
    device = await db.scalar(select(Device).where(Device.device_id == device_id))
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    _require_device_access(current_user, device, hide=False)

    device.name = payload.name
    await db.commit()
    return DeviceRenameResponse(
        success=True,
        message="Device renamed successfully",
        device_id=device.device_id,
        new_name=device.name,
    )
