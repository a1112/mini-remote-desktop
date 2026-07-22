"""网络分组相关的 Schema 定义

定义网络分组创建、更新、查询和响应的数据结构。
"""
from pydantic import BaseModel, Field
from typing import Optional, List
from datetime import datetime


class NetworkGroupCreate(BaseModel):
    """创建网络分组的请求"""
    name: str = Field(..., min_length=1, max_length=128, description="分组名称")
    description: Optional[str] = Field(None, description="分组描述")


class NetworkGroupUpdate(BaseModel):
    """更新网络分组的请求"""
    name: Optional[str] = Field(None, min_length=1, max_length=128, description="分组名称")
    description: Optional[str] = Field(None, description="分组描述")
    is_enabled: Optional[bool] = Field(None, description="是否启用分组")


class NetworkGroupOut(BaseModel):
    """网络分组响应"""
    id: str
    user_id: str
    name: str
    description: Optional[str]
    is_enabled: bool
    device_count: int = Field(default=0, description="分组内的设备总数")
    online_device_count: int = Field(default=0, description="分组内的在线设备数")
    created_at: datetime
    updated_at: datetime

    class Config:
        from_attributes = True


class DeviceInGroupOut(BaseModel):
    """分组内设备的响应"""
    id: str
    device_id: str
    name: str
    status: str = Field(description="设备状态: online/offline")
    is_enabled: bool = Field(description="在该分组中的启用状态")
    ip: str

    class Config:
        from_attributes = True


class AddDevicesRequest(BaseModel):
    """添加设备到分组的请求"""
    device_ids: List[str] = Field(..., min_length=1, description="要添加的设备ID列表")


class SetDeviceEnabledRequest(BaseModel):
    """设置设备在分组中启用状态的请求"""
    is_enabled: bool = Field(..., description="是否启用设备")
