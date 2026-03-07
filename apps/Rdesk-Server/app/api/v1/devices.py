from datetime import datetime
from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy import Select, select
from sqlalchemy.ext.asyncio import AsyncSession
from sqlalchemy.orm import selectinload

from app.core.security import create_access_token, get_current_user
from app.db.session import get_db
from app.models.device import Device, generate_device_id_from_serial
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
    """
    设备注册

    根据主板序列号生成设备ID。如果设备已存在，返回现有设备信息。
    如果设备不存在，创建新设备。
    """
    # 检查设备是否已存在
    existing = await db.scalar(
        select(Device).where(Device.motherboard_serial == payload.motherboard_serial)
    )

    # 根据 OS 版本确定 OS 类型
    os_type = payload.os_version.split()[0] if payload.os_version else "Unknown"

    if existing:
        # 设备已存在，更新信息
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

        # 生成访问令牌
        access_token = create_access_token(
            existing.device_id, existing.name, "device"
        )

        return DeviceRegisterResponse(
            device_id=existing.device_id,
            device_name=existing.name,
            access_token=access_token,
        )
    else:
        # 新设备，生成设备ID
        device_id = generate_device_id_from_serial(payload.motherboard_serial)

        # 检查 device_id 是否冲突（极小概率）
        id_conflict = await db.scalar(
            select(Device).where(Device.device_id == device_id)
        )
        if id_conflict:
            # 如果冲突，添加随机后缀
            import uuid
            device_id = f"{device_id}-{uuid.uuid4().hex[:4]}"

        device_name = payload.device_name or payload.hostname

        new_device = Device(
            name=device_name,
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

        # 生成访问令牌
        access_token = create_access_token(
            new_device.device_id, new_device.name, "device"
        )

        return DeviceRegisterResponse(
            device_id=new_device.device_id,
            device_name=new_device.name,
            access_token=access_token,
        )


@router.get("/check/{motherboard_serial}")
async def check_device_registration(
    motherboard_serial: str,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """
    检查设备是否已注册
    """
    device = await db.scalar(
        select(Device).where(Device.motherboard_serial == motherboard_serial)
    )

    if device:
        return {
            "registered": True,
            "device_id": device.device_id,
            "device_name": device.name,
            "is_bound": device.is_bound,
        }
    else:
        return {"registered": False}


@router.post("/bind")
async def bind_device(
    payload: DeviceBindRequest,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """
    绑定设备到用户

    将设备与用户账户绑定，绑定后只有该用户可以访问此设备。
    """
    device = await db.scalar(
        select(Device).where(Device.device_id == payload.device_id)
    )

    if not device:
        raise HTTPException(status_code=404, detail="Device not found")

    if device.is_bound and device.bound_user_id != payload.user_id:
        raise HTTPException(
            status_code=403, detail="Device is already bound to another user"
        )

    # 绑定设备
    device.is_bound = True
    device.bound_user_id = payload.user_id
    device.bound_at = datetime.utcnow()

    await db.commit()

    return {
        "message": "Device bound successfully",
        "device_id": device.device_id,
        "user_id": payload.user_id,
    }


@router.get("", response_model=list[DeviceOut])
async def list_devices(
    q: str | None = Query(default=None),
    status: str | None = Query(default=None),
    db: AsyncSession = Depends(get_db),
) -> list[DeviceOut]:
    stmt: Select[tuple[Device]] = select(Device).options(selectinload(Device.status))
    if q:
        stmt = stmt.where(Device.name.ilike(f"%{q}%"))
    rows = (await db.scalars(stmt)).all()
    result = [_to_out(item) for item in rows]
    if status:
        result = [item for item in result if item.status == status]
    return result


@router.get("/{device_id}", response_model=DeviceOut)
async def get_device(device_id: str, db: AsyncSession = Depends(get_db)) -> DeviceOut:
    stmt = (
        select(Device)
        .where(Device.id == device_id)
        .options(selectinload(Device.status))
    )
    device = await db.scalar(stmt)
    if not device:
        raise HTTPException(status_code=404, detail="Device not found")
    return _to_out(device)


@router.post("/auto-bind", response_model=DeviceAutoBindResponse)
async def auto_bind_device(
    payload: DeviceAutoBindRequest,
    db: AsyncSession = Depends(get_db),
) -> DeviceAutoBindResponse:
    """
    用户登录时自动绑定设备

    逻辑：
    1. 如果设备未绑定(is_bound=False)：直接绑定当前用户
    2. 如果设备已被其他用户绑定：强制迁移到当前用户
    3. 如果设备已被当前用户绑定：更新绑定时间（续期）

    返回：绑定状态、被踢出的用户信息（如有）
    """
    device = await db.scalar(
        select(Device).where(Device.device_id == payload.device_id)
    )

    if not device:
        raise HTTPException(status_code=404, detail="Device not found")

    kicked_user = None
    is_new_binding = False

    if device.is_bound:
        if device.bound_user_id == payload.user_id:
            # 同一用户，续期
            device.bound_at = datetime.utcnow()
        else:
            # 其他用户，强制迁移
            kicked_user = {
                "user_id": device.bound_user_id,
                "kicked_at": datetime.utcnow().isoformat(),
            }
            is_new_binding = True
    else:
        # 未绑定，直接绑定
        is_new_binding = True

    # 绑定设备
    device.is_bound = True
    device.bound_user_id = payload.user_id
    device.bound_at = datetime.utcnow()

    await db.commit()

    return DeviceAutoBindResponse(
        success=True,
        message="Device bound successfully" if not kicked_user else f"Device migrated from previous user",
        kicked_user=kicked_user,
        is_new_binding=is_new_binding,
    )


@router.post("/unbind")
async def unbind_device(
    payload: DeviceUnbindRequest,
    db: AsyncSession = Depends(get_db),
) -> dict:
    """
    用户登出时解除设备绑定

    逻辑：
    1. 验证设备确实绑定到该用户
    2. 解除绑定（is_bound=False, bound_user_id=None, bound_at=None）
    """
    device = await db.scalar(
        select(Device).where(Device.device_id == payload.device_id)
    )

    if not device:
        raise HTTPException(status_code=404, detail="Device not found")

    if not device.is_bound:
        # 设备未绑定，直接返回成功（幂等）
        return {"message": "Device not bound", "success": True}

    if device.bound_user_id != payload.user_id:
        # 设备绑定到其他用户，不允许解绑
        raise HTTPException(
            status_code=403, detail="Device is bound to a different user"
        )

    # 解除绑定
    device.is_bound = False
    device.bound_user_id = None
    device.bound_at = None

    await db.commit()

    return {
        "message": "Device unbound successfully",
        "success": True,
    }


@router.get("/{device_id}/binding-status", response_model=DeviceBindingStatus)
async def get_binding_status(
    device_id: str,
    db: AsyncSession = Depends(get_db),
) -> DeviceBindingStatus:
    """
    获取设备当前绑定状态

    返回：是否已绑定、绑定的用户信息、绑定时间
    """
    device = await db.scalar(
        select(Device).where(Device.device_id == device_id)
    )

    if not device:
        raise HTTPException(status_code=404, detail="Device not found")

    bound_username = None
    if device.is_bound and device.bound_user_id:
        # 获取用户名（需要导入 User 模型）
        from app.models.user import User
        user = await db.scalar(
            select(User).where(User.id == device.bound_user_id)
        )
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
    current_user = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> DeviceRenameResponse:
    """
    重命名设备

    需要用户登录认证。用户只能重命名自己绑定的设备。
    """
    device = await db.scalar(
        select(Device).where(Device.device_id == device_id)
    )

    if not device:
        raise HTTPException(status_code=404, detail="Device not found")

    # 检查设备是否绑定到当前用户
    if device.is_bound and device.bound_user_id != current_user.id:
        raise HTTPException(
            status_code=403, detail="You can only rename devices bound to your account"
        )

    # 更新设备名称
    device.name = payload.name
    await db.commit()

    return DeviceRenameResponse(
        success=True,
        message="Device renamed successfully",
        device_id=device.device_id,
        new_name=device.name,
    )
