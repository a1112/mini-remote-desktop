/**
 * 设备注册服务
 *
 * 自动处理设备注册，类似 RustDesk 的行为：
 * 1. 首次启动时自动获取硬件信息并注册
 * 2. 注册成功后保存设备信息到本地
 * 3. 后续启动时验证设备状态
 */

import { ipcRegisterDevice } from "../adapters/tauri";
import { isTauriRuntime } from "../utils/runtime";

interface HardwareInfo {
  motherboard_serial: string;
  hostname: string;
  os_type: string;
  os_version: string;
  cpu_info: {
    name: string;
    vendor_id: string;
    cores: number;
    max_frequency_mhz?: number;
  };
  total_memory_mb: number;
  gpu_info: Array<{
    name: string;
    vendor: string;
    memory_mb?: number;
  }>;
}

interface DeviceRegistrationResponse {
  device_id: string;
  device_name: string;
  access_token: string;
}

interface StoredDeviceInfo {
  device_id: string;
  device_name: string;
  access_token: string;
  motherboard_serial: string;
  registered_at: string;
}

const DEVICE_INFO_KEY = "rdesk_device_info";
const LOCAL_ACCESS_TOKEN = "local-p2p";
const API_BASE = (
  (import.meta as any).env?.VITE_RDESK_SERVER_URL ?? "http://127.0.0.1:9530/api/v1"
).replace(/\/+$/, "");
const DEVICE_REGISTRATION_MODE = (
  (import.meta as any).env?.VITE_RDESK_DEVICE_REGISTRATION ?? "local"
).toLowerCase();

/**
 * 设备注册服务类
 */
class DeviceRegistrationService {
  private deviceInfo: StoredDeviceInfo | null = null;
  private initializing = false;
  private initPromise: Promise<StoredDeviceInfo | null> | null = null;

  /**
   * 初始化设备注册服务
   * 自动检查注册状态，如果未注册则自动注册
   */
  async initialize(): Promise<StoredDeviceInfo | null> {
    // 如果已经在初始化，返回现有 Promise
    if (this.initPromise) {
      return this.initPromise;
    }

    this.initPromise = this._initialize();

    try {
      const result = await this.initPromise;
      return result;
    } finally {
      this.initPromise = null;
    }
  }

  private async _initialize(): Promise<StoredDeviceInfo | null> {
    const useServerRegistration = this.shouldUseServerRegistration();

    // 1. 检查本地存储
    const stored = this.getStoredDeviceInfo();
    if (stored) {
      console.log("[DeviceService] 找到本地设备信息:", stored.device_id);
      this.deviceInfo = stored;
      void this.syncWithLocalService(stored);

      if (!useServerRegistration || this.isLocalOnlyDevice(stored)) {
        return stored;
      }

      // 验证设备是否仍然有效
      try {
        await this.verifyDevice(stored.motherboard_serial);
        console.log("[DeviceService] 设备验证成功");
        return stored;
      } catch (err) {
        console.warn("[DeviceService] 设备验证失败，需要重新注册:", err);
        // 验证失败，清除本地存储
        this.clearStoredDeviceInfo();
      }
    }

    // 2. 获取硬件信息并注册
    try {
      const hardwareInfo = await this.getHardwareInfo();
      if (!useServerRegistration) {
        return this.saveLocalDeviceInfo(hardwareInfo);
      }

      const registration = await this.registerDevice(hardwareInfo);

      const deviceInfo: StoredDeviceInfo = {
        device_id: registration.device_id,
        device_name: registration.device_name,
        access_token: registration.access_token,
        motherboard_serial: hardwareInfo.motherboard_serial,
        registered_at: new Date().toISOString(),
      };

      this.saveDeviceInfo(deviceInfo);
      this.deviceInfo = deviceInfo;
      void this.syncWithLocalService(deviceInfo);

      console.log("[DeviceService] 设备注册成功:", deviceInfo.device_id);
      return deviceInfo;
    } catch (err) {
      try {
        const hardwareInfo = await this.getHardwareInfo();
        return this.saveLocalDeviceInfo(hardwareInfo);
      } catch (fallbackErr) {
        console.error("[DeviceService] LAN fallback registration failed:", fallbackErr);
      }
      console.error("[DeviceService] 设备注册失败:", err);
      return null;
    }
  }

  private shouldUseServerRegistration(): boolean {
    return DEVICE_REGISTRATION_MODE === "server" || DEVICE_REGISTRATION_MODE === "cloud";
  }

  private isLocalOnlyDevice(info: StoredDeviceInfo): boolean {
    return info.access_token === LOCAL_ACCESS_TOKEN || info.device_id.startsWith("lan-");
  }

  private saveLocalDeviceInfo(hardwareInfo: HardwareInfo): StoredDeviceInfo {
    const fallbackId = `lan-${hardwareInfo.motherboard_serial.replace(/[^a-zA-Z0-9]/g, "").slice(-16)}`;
    const localInfo: StoredDeviceInfo = {
      device_id: fallbackId,
      device_name: hardwareInfo.hostname || "Rdesk LAN Device",
      access_token: LOCAL_ACCESS_TOKEN,
      motherboard_serial: hardwareInfo.motherboard_serial,
      registered_at: new Date().toISOString(),
    };
    this.saveDeviceInfo(localInfo);
    this.deviceInfo = localInfo;
    void this.syncWithLocalService(localInfo);
    return localInfo;
  }

  /**
   * 获取硬件信息（通过 Tauri）
   */
  private async getHardwareInfo(): Promise<HardwareInfo> {
    // 检查 Tauri 环境是否可用
    const tauri = typeof window !== "undefined" ? window.__TAURI__ : undefined;
    const isTauriAvailable = typeof tauri?.invoke === "function";

    if (isTauriAvailable) {
      try {
        return await tauri.invoke<HardwareInfo>("get_hardware_info");
      } catch (err) {
        console.warn("[DeviceService] Tauri 调用失败，使用模拟数据:", err);
        return this.getMockHardwareInfo();
      }
    }
    // 开发模式模拟数据
    return this.getMockHardwareInfo();
  }

  /**
   * 注册设备到服务器
   */
  private async registerDevice(
    hardwareInfo: HardwareInfo
  ): Promise<DeviceRegistrationResponse> {
    const payload = {
      motherboard_serial: hardwareInfo.motherboard_serial,
      hostname: hardwareInfo.hostname,
      os_version: hardwareInfo.os_version,
      device_name: hardwareInfo.hostname,
      cpu_info: JSON.stringify(hardwareInfo.cpu_info),
      total_memory_mb: hardwareInfo.total_memory_mb,
      gpu_info: JSON.stringify(hardwareInfo.gpu_info),
    };

    const response = await fetch(`${API_BASE}/devices/register`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(`注册失败: ${error}`);
    }

    return await response.json();
  }

  /**
   * 验证设备是否已注册
   */
  private async verifyDevice(motherboardSerial: string): Promise<boolean> {
    const response = await fetch(
      `${API_BASE}/devices/check/${motherboardSerial}`
    );

    if (!response.ok) {
      throw new Error("设备验证失败");
    }

    const data = await response.json();
    return data.registered === true;
  }

  /**
   * 获取本地存储的设备信息
   */
  private getStoredDeviceInfo(): StoredDeviceInfo | null {
    try {
      const stored = localStorage.getItem(DEVICE_INFO_KEY);
      if (stored) {
        return JSON.parse(stored);
      }
    } catch (err) {
      console.warn("[DeviceService] 读取本地存储失败:", err);
    }
    return null;
  }

  /**
   * 保存设备信息到本地存储
   */
  private saveDeviceInfo(info: StoredDeviceInfo): void {
    try {
      localStorage.setItem(DEVICE_INFO_KEY, JSON.stringify(info));
    } catch (err) {
      console.warn("[DeviceService] 保存本地存储失败:", err);
    }
  }

  /**
   * 清除本地存储的设备信息
   */
  private clearStoredDeviceInfo(): void {
    try {
      localStorage.removeItem(DEVICE_INFO_KEY);
    } catch (err) {
      console.warn("[DeviceService] 清除本地存储失败:", err);
    }
  }

  /**
   * 获取当前设备信息
   */
  getDeviceInfo(): StoredDeviceInfo | null {
    return this.deviceInfo;
  }

  /**
   * 获取设备 ID
   */
  getDeviceId(): string | null {
    return this.deviceInfo?.device_id ?? null;
  }

  /**
   * 获取访问令牌
   */
  getAccessToken(): string | null {
    return this.deviceInfo?.access_token ?? null;
  }

  private async syncWithLocalService(info: StoredDeviceInfo): Promise<void> {
    if (!isTauriRuntime()) return;
    const result = await ipcRegisterDevice(info.device_id, info.device_name);
    if (!result.ok) {
      console.warn("[DeviceService] mrd-service device registration failed:", result.error.message);
    }
  }

  /**
   * 获取用户访问令牌（JWT）
   */
  private getUserAccessToken(): string | null {
    return localStorage.getItem("rdesk_access_token");
  }

  /**
   * 用户登录时绑定设备
   * @param userId 用户ID
   * @returns 绑定结果
   */
  async bindDevice(userId: string): Promise<{
    success: boolean;
    message: string;
    kickedUser?: { user_id: string; kicked_at: string } | null;
    isNewBinding?: boolean;
  }> {
    if (!this.deviceInfo) {
      console.warn("[DeviceService] 设备未注册，无法绑定");
      return { success: false, message: "设备未注册" };
    }

    try {
      const userToken = this.getUserAccessToken();
      const response = await fetch(`${API_BASE}/devices/auto-bind`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(userToken ? { "Authorization": `Bearer ${userToken}` } : {}),
        },
        body: JSON.stringify({
          device_id: this.deviceInfo.device_id,
          user_id: userId,
        }),
      });

      if (!response.ok) {
        const error = await response.text();
        console.error("[DeviceService] 绑定失败:", error);
        return { success: false, message: `绑定失败: ${error}` };
      }

      const data = await response.json();
      console.log("[DeviceService] 设备绑定成功:", data);
      return data;
    } catch (e) {
      console.error("[DeviceService] 绑定请求失败:", e);
      return { success: false, message: "网络错误" };
    }
  }

  /**
   * 重命名设备
   * @param deviceId 设备ID
   * @param newName 新名称
   * @returns 是否成功
   */
  async renameDevice(deviceId: string, newName: string): Promise<boolean> {
    const userToken = this.getUserAccessToken();
    if (!userToken) {
      console.warn("[DeviceService] 未登录，无法重命名设备");
      return false;
    }

    try {
      const response = await fetch(`${API_BASE}/devices/${deviceId}/rename`, {
        method: "PATCH",
        headers: {
          "Content-Type": "application/json",
          "Authorization": `Bearer ${userToken}`,
        },
        body: JSON.stringify({ name: newName }),
      });

      if (!response.ok) {
        console.error("[DeviceService] 重命名失败:", await response.text());
        return false;
      }

      return true;
    } catch (e) {
      console.error("[DeviceService] 重命名请求失败:", e);
      return false;
    }
  }

  /**
   * 用户登出时解绑设备
   * @param userId 用户ID
   * @returns 解绑结果
   */
  async unbindDevice(userId: string): Promise<boolean> {
    if (!this.deviceInfo) {
      console.warn("[DeviceService] 设备未注册，无法解绑");
      return false;
    }

    try {
      const userToken = this.getUserAccessToken();
      const response = await fetch(`${API_BASE}/devices/unbind`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(userToken ? { "Authorization": `Bearer ${userToken}` } : {}),
        },
        body: JSON.stringify({
          device_id: this.deviceInfo.device_id,
          user_id: userId,
        }),
      });

      if (!response.ok) {
        console.error("[DeviceService] 解绑失败:", await response.text());
        return false;
      }

      console.log("[DeviceService] 设备解绑成功");
      return true;
    } catch (e) {
      console.error("[DeviceService] 解绑请求失败:", e);
      return false;
    }
  }

  /**
   * 获取设备绑定状态
   * @returns 绑定状态
   */
  async getBindingStatus(): Promise<{
    isBound: boolean;
    boundUserId: string | null;
    boundUsername: string | null;
    boundAt: string | null;
  } | null> {
    if (!this.deviceInfo) {
      return null;
    }

    try {
      const response = await fetch(
        `${API_BASE}/devices/${this.deviceInfo.device_id}/binding-status`
      );

      if (!response.ok) {
        return null;
      }

      const data = await response.json();
      return {
        isBound: data.is_bound,
        boundUserId: data.bound_user_id,
        boundUsername: data.bound_username,
        boundAt: data.bound_at,
      };
    } catch (e) {
      console.error("[DeviceService] 获取绑定状态失败:", e);
      return null;
    }
  }

  /**
   * 强制重新注册设备
   */
  async reregister(): Promise<StoredDeviceInfo | null> {
    this.clearStoredDeviceInfo();
    this.deviceInfo = null;
    return this.initialize();
  }

  /**
   * 开发模式模拟硬件信息
   */
  private getMockHardwareInfo(): HardwareInfo {
    const mockSerial = localStorage.getItem("mock_device_serial") ||
      "MOCK-" + Math.random().toString(36).substring(2, 10).toUpperCase();
    localStorage.setItem("mock_device_serial", mockSerial);

    return {
      motherboard_serial: mockSerial,
      hostname: "开发测试机",
      os_type: "windows",
      os_version: "Windows 11 Pro 23H2 Build 22631",
      cpu_info: {
        name: "Intel Core i7-12700K",
        vendor_id: "GenuineIntel",
        cores: 12,
        max_frequency_mhz: 3500,
      },
      total_memory_mb: 32768,
      gpu_info: [
        {
          name: "NVIDIA GeForce RTX 3060",
          vendor: "NVIDIA",
          memory_mb: 12288,
        },
      ],
    };
  }
}

// 导出单例
export const deviceService = new DeviceRegistrationService();

// React Hook
export function useDeviceRegistration() {
  const [deviceId, setDeviceId] = useState<string | null>(null);
  const [deviceName, setDeviceName] = useState<string | null>(null);
  const [isRegistered, setIsRegistered] = useState(false);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    deviceService.initialize().then((info) => {
      if (info) {
        setDeviceId(info.device_id);
        setDeviceName(info.device_name);
        setIsRegistered(true);
      }
      setIsLoading(false);
    });
  }, []);

  return {
    deviceId,
    deviceName,
    isRegistered,
    isLoading,
    getAccessToken: () => deviceService.getAccessToken(),
    reregister: () => deviceService.reregister(),
  };
}

import { useState, useEffect } from "react";
