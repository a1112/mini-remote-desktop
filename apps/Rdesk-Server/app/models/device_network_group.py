from datetime import datetime
from uuid import uuid4

from sqlalchemy import Boolean, DateTime, ForeignKey, String
from sqlalchemy.orm import Mapped, mapped_column, relationship

from app.db.session import Base


class DeviceNetworkGroup(Base):
    """设备与网络分组的多对多关联表

    实现设备和网络分组之间的多对多关系。
    每个关联记录包含设备在该分组中的启用状态。
    """
    __tablename__ = "device_network_groups"

    id: Mapped[str] = mapped_column(String(36), primary_key=True, default=lambda: str(uuid4()))
    network_group_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("network_groups.id", ondelete="CASCADE"),
        index=True
    )
    device_id: Mapped[str] = mapped_column(
        String(36),
        ForeignKey("devices.id", ondelete="CASCADE"),
        index=True
    )
    is_enabled: Mapped[bool] = mapped_column(Boolean, default=True)
    assigned_at: Mapped[datetime] = mapped_column(DateTime, default=datetime.utcnow)

    # Relationships
    network_group: Mapped["NetworkGroup"] = relationship("NetworkGroup", back_populates="device_associations")
    device: Mapped["Device"] = relationship("Device", back_populates="group_associations")
