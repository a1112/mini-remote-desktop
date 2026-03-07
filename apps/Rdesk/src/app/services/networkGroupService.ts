/**
 * 网络分组服务
 *
 * 提供网络分组的 CRUD 操作和设备管理功能。
 */

const API_BASE = "http://127.0.0.1:9530/api/v1";

/**
 * 网络分组接口
 */
export interface NetworkGroup {
  id: string;
  user_id: string;
  name: string;
  description?: string;
  is_enabled: boolean;
  device_count: number;
  online_device_count: number;
  created_at: string;
  updated_at: string;
}

/**
 * 分组内设备接口
 */
export interface DeviceInGroup {
  id: string;
  device_id: string;
  name: string;
  status: "online" | "offline";
  is_enabled: boolean;
  ip: string;
}

/**
 * 创建网络分组请求
 */
export interface CreateNetworkGroupRequest {
  name: string;
  description?: string;
}

/**
 * 更新网络分组请求
 */
export interface UpdateNetworkGroupRequest {
  name?: string;
  description?: string;
  is_enabled?: boolean;
}

class NetworkGroupService {
  /**
   * 获取用户访问令牌
   */
  private getUserAccessToken(): string | null {
    return localStorage.getItem("rdesk_access_token");
  }

  /**
   * 获取请求头
   */
  private getHeaders(): Record<string, string> {
    const token = this.getUserAccessToken();
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (token) {
      headers["Authorization"] = `Bearer ${token}`;
    }
    return headers;
  }

  /**
   * 获取所有网络分组
   */
  async getNetworkGroups(): Promise<NetworkGroup[]> {
    const response = await fetch(`${API_BASE}/network-groups`, {
      headers: this.getHeaders(),
    });

    if (!response.ok) {
      throw new Error(`获取网络分组失败: ${response.status}`);
    }

    return response.json();
  }

  /**
   * 获取分组详情
   */
  async getNetworkGroup(groupId: string): Promise<NetworkGroup> {
    const response = await fetch(`${API_BASE}/network-groups/${groupId}`, {
      headers: this.getHeaders(),
    });

    if (!response.ok) {
      throw new Error(`获取分组详情失败: ${response.status}`);
    }

    return response.json();
  }

  /**
   * 创建网络分组
   */
  async createNetworkGroup(data: CreateNetworkGroupRequest): Promise<NetworkGroup> {
    const response = await fetch(`${API_BASE}/network-groups`, {
      method: "POST",
      headers: this.getHeaders(),
      body: JSON.stringify(data),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`创建分组失败: ${error}`);
    }

    return response.json();
  }

  /**
   * 更新网络分组
   */
  async updateNetworkGroup(groupId: string, data: UpdateNetworkGroupRequest): Promise<NetworkGroup> {
    const response = await fetch(`${API_BASE}/network-groups/${groupId}`, {
      method: "PATCH",
      headers: this.getHeaders(),
      body: JSON.stringify(data),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`更新分组失败: ${error}`);
    }

    return response.json();
  }

  /**
   * 删除网络分组
   */
  async deleteNetworkGroup(groupId: string): Promise<void> {
    const response = await fetch(`${API_BASE}/network-groups/${groupId}`, {
      method: "DELETE",
      headers: this.getHeaders(),
    });

    if (!response.ok && response.status !== 204) {
      throw new Error(`删除分组失败: ${response.status}`);
    }
  }

  /**
   * 获取分组内的设备列表
   */
  async getGroupDevices(groupId: string): Promise<DeviceInGroup[]> {
    const response = await fetch(`${API_BASE}/network-groups/${groupId}/devices`, {
      headers: this.getHeaders(),
    });

    if (!response.ok) {
      throw new Error(`获取分组设备失败: ${response.status}`);
    }

    return response.json();
  }

  /**
   * 添加设备到分组
   */
  async addDevicesToGroup(groupId: string, deviceIds: string[]): Promise<void> {
    const response = await fetch(`${API_BASE}/network-groups/${groupId}/devices`, {
      method: "POST",
      headers: this.getHeaders(),
      body: JSON.stringify({ device_ids: deviceIds }),
    });

    if (!response.ok && response.status !== 201) {
      throw new Error(`添加设备失败: ${response.status}`);
    }
  }

  /**
   * 从分组移除设备
   */
  async removeDeviceFromGroup(groupId: string, deviceId: string): Promise<void> {
    const response = await fetch(
      `${API_BASE}/network-groups/${groupId}/devices/${deviceId}`,
      {
        method: "DELETE",
        headers: this.getHeaders(),
      }
    );

    if (!response.ok && response.status !== 204) {
      throw new Error(`移除设备失败: ${response.status}`);
    }
  }

  /**
   * 设置设备在分组中的启用状态
   */
  async setDeviceEnabled(groupId: string, deviceId: string, enabled: boolean): Promise<void> {
    const response = await fetch(
      `${API_BASE}/network-groups/${groupId}/devices/${deviceId}`,
      {
        method: "PATCH",
        headers: this.getHeaders(),
        body: JSON.stringify({ is_enabled: enabled }),
      }
    );

    if (!response.ok && response.status !== 204) {
      throw new Error(`设置设备状态失败: ${response.status}`);
    }
  }
}

// 导出单例
export const networkGroupService = new NetworkGroupService();
