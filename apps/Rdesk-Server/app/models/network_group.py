from datetime import datetime
from uuid import uuid4

from sqlalchemy import Boolean, DateTime, String, Text, ForeignKey
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.db.session import Base


class NetworkGroup(Base):
    """用户级网络分组模型

    每个用户可以创建多个网络分组，设备可以加入多个分组。
    分组之间互不干扰，实现多租户隔离。
    """
    __tablename__ = "network_groups"

    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=lambda: str(uuid4()))
    user_id: Mapped[str] = mapped_column(String(36), ForeignKey("users.id"), index=True)
    name: Mapped[str] = mapped_column(String(128), nullable=False)
    description: Mapped[str | None] = mapped_column(Text, nullable=True)
    is_enabled: Mapped[bool] = mapped_column(Boolean, default=True)
    created_at: Mapped[datetime] = mapped_column(DateTime, default=datetime.utcnow)
    updated_at: Mapped[datetime] = mapped_column(DateTime, default=datetime.utcnow, onupdate=datetime.utcnow)

    # Relationships
    device_associations: Mapped[list["DeviceNetworkGroup"]] = relationship(
        "DeviceNetworkGroup", back_populates="network_group", cascade="all, delete-orphan"
    )
