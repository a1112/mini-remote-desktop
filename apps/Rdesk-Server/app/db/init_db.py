from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.security import hash_password
from app.models.device import Device, DeviceStatus
from app.models.user import User


async def seed_initial_data(session: AsyncSession) -> None:
    user_exists = await session.scalar(select(User).limit(1))
    if user_exists:
        return

    admin = User(
        username="admin",
        email="admin@rdesk.local",
        password_hash=hash_password("admin123"),
        role="admin",
    )
    session.add(admin)

    devices = [
        Device(
            name="办公室电脑",
            device_id="821456789",
            os="Windows 11 Pro",
            icon="Monitor",
            location="北京",
            ip="192.168.1.101",
            group="工作",
            favorite=True,
            status=DeviceStatus(status="online", ping=18, cpu=34, ram=68, disk=45, last_seen="在线"),
        ),
        Device(
            name="家用 MacBook",
            device_id="334902115",
            os="macOS Sonoma 14.2",
            icon="Laptop",
            location="上海",
            ip="192.168.0.5",
            group="个人",
            favorite=True,
            status=DeviceStatus(status="online", ping=35, cpu=12, ram=42, disk=61, last_seen="在线"),
        ),
        Device(
            name="Linux 服务器",
            device_id="567234891",
            os="Ubuntu 22.04 LTS",
            icon="Server",
            location="深圳",
            ip="10.0.0.15",
            group="服务器",
            favorite=False,
            status=DeviceStatus(status="offline", ping=None, cpu=None, ram=None, disk=None, last_seen="2小时前"),
        ),
    ]
    session.add_all(devices)
    await session.commit()
