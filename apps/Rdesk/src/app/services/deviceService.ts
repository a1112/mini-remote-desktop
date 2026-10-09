/**
 * 设备注册服务
 *
 * 自动处理设备注册，类似 RustDesk 的行为：
 * 1. 首次启动时自动获取硬件信息并注册
 * 2. 注册成功后保存设备信息到本地
 * 3. 后续启动时验证设备状态
 */

import { useEffect, useState } from "react";
import { ipcBindPublicDevice, ipcUnbindPublicDevice, ipcPublicServerStatus, ipcRegisterDevice, registerDevice as registerDeviceCommand } from "../adapters/tauri";
import { isTauriRuntime } from "../utils/runtime";
import { DEFAULT_PUBLIC_API_URL, DEVICE_REGISTRATION_MODE, SERVER_API_URL } from "./serverConfig";

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
const SERVICE_MANAGED_TOKEN = "service-managed";
const API_BASE = SERVER_API_URL;

/**
 * 设备注册服务类
 */
class DeviceRegistrationService {
  private deviceInfo: StoredDeviceInfo | null = null;
  private registrationError: string | null = null;
  private bindingError: string | null = null;
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
    if (!isTauriRuntime()) return null;
    const useServerRegistration = this.shouldUseServerRegistration();
    this.registrationError = null;
    const stored = this.getStoredDeviceInfo();
    if (this.shouldUseServiceManagedRegistration()) {
      return this.restoreServiceManagedDevice(stored);
    }
    if (stored && !this.isLocalOnlyDevice(stored) && stored.access_token !== SERVICE_MANAGED_TOKEN) {
      // A failed refresh must never destroy a server-assigned identity.
      this.deviceInfo = stored;
      try {
        const hardwareInfo = await this.getHardwareInfo();
        const registration = await this.registerDevice(hardwareInfo, { deviceToken: stored.access_token }, stored.device_name);
        if (registration.device_id !== stored.device_id) {
          throw new Error("服务器返回的设备身份不匹配，请联系管理员");
        }
        return this.saveServerDeviceInfo(hardwareInfo, registration, stored.registered_at);
      } catch (error) {
        this.registrationError = error instanceof Error ? error.message : "设备刷新失败，请稍后重试";
        void this.syncWithLocalService(stored);
        return stored;
      }
    }
    if (useServerRegistration) {
      this.registrationError = "需要设备登记码，请向服务器管理员获取一次性登记码后注册";
      return null;
    }
    try {
      if (stored) {
        const refreshed = await this.refreshStoredLocalDeviceInfo(stored);
        this.deviceInfo = refreshed;
        void this.syncWithLocalService(refreshed);
        return refreshed;
      }
      const hardwareInfo = await this.getHardwareInfo();
      return this.saveLocalDeviceInfo(hardwareInfo);
    } catch {
      this.registrationError = "无法读取设备硬件信息，请在桌面客户端重试";
      return null;
    }
  }

  /** One-time enrollment is supplied by the user and never persisted. */
  async enroll(enrollmentToken: string, deviceName?: string): Promise<StoredDeviceInfo> {
    if (this.initPromise) await this.initPromise;
    if (!/^[A-Za-z0-9_-]{43}$/.test(enrollmentToken.trim())) {
      throw new Error("请输入管理员提供的有效设备登记码（43 位）");
    }
    const hardwareInfo = await this.getHardwareInfo();
    const registration = await this.registerDevice(hardwareInfo, { enrollmentToken: enrollmentToken.trim() }, deviceName);
    this.registrationError = null;
    const info = this.saveServerDeviceInfo(hardwareInfo, registration);
    await this.bindManagedDeviceIfLoggedIn();
    return info;
  }

  async recoverDeviceCredential(deviceToken: string): Promise<StoredDeviceInfo> {
    if (this.initPromise) await this.initPromise;
    if (!/^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(deviceToken.trim())) {
      throw new Error("请输入管理员换发的新设备凭据");
    }
    const stored = this.getStoredDeviceInfo();
    const hardwareInfo = await this.getHardwareInfo();
    const registration = await this.registerDevice(hardwareInfo, { deviceToken: deviceToken.trim() }, stored?.device_name);
    if (stored && !this.isLocalOnlyDevice(stored) && stored.device_id !== registration.device_id) {
      throw new Error("服务器返回的设备身份不匹配，请联系管理员");
    }
    this.registrationError = null;
    const info = this.saveServerDeviceInfo(hardwareInfo, registration, stored?.registered_at);
    await this.bindManagedDeviceIfLoggedIn();
    return info;
  }

  private saveServerDeviceInfo(hardwareInfo: HardwareInfo, registration: DeviceRegistrationResponse, registeredAt?: string): StoredDeviceInfo {
    const info: StoredDeviceInfo = {
      ...registration,
      motherboard_serial: hardwareInfo.motherboard_serial,
      registered_at: registeredAt ?? new Date().toISOString(),
    };
    this.saveDeviceInfo(info);
    this.deviceInfo = info;
    if (info.access_token !== SERVICE_MANAGED_TOKEN) void this.syncWithLocalService(info);
    return info;
  }

  private shouldUseServiceManagedRegistration(): boolean {
    return this.shouldUseServerRegistration() && API_BASE === DEFAULT_PUBLIC_API_URL;
  }

  private async restoreServiceManagedDevice(stored: StoredDeviceInfo | null): Promise<StoredDeviceInfo | null> {
    try {
      const result = await ipcPublicServerStatus();
      if (!result.ok) throw new Error("无法读取本机设备登记状态，请确认后台服务已启动");
      const status = result.value;
      if (!status.device_registered || !status.device_id) {
        this.deviceInfo = null;
        this.registrationError = "等待本机服务自动登记设备，无需登录账号";
        return null;
      }
      const info: StoredDeviceInfo = {
        device_id: status.device_id,
        device_name: status.device_name ?? stored?.device_name ?? "本机设备",
        access_token: SERVICE_MANAGED_TOKEN,
        motherboard_serial: "service-managed",
        registered_at: stored?.registered_at ?? new Date().toISOString(),
      };
      this.saveDeviceInfo(info);
      this.deviceInfo = info;
      await this.bindManagedDeviceIfLoggedIn();
      return info;
    } catch {
      this.registrationError = "无法读取本机设备登记状态，请确认后台服务已启动后重试";
      // A cached code is display metadata only. It never proves public connectivity.
      this.deviceInfo = stored?.access_token === SERVICE_MANAGED_TOKEN ? stored : null;
      return this.deviceInfo;
    }
  }

  isServiceManagedRegistration(): boolean {
    return isTauriRuntime() && this.shouldUseServiceManagedRegistration();
  }

  getRegistrationError(): string | null {
    return this.registrationError ?? this.bindingError;
  }

  private recordBindingError(message: string | null): void {
    this.bindingError = message;
    window.dispatchEvent(new Event("rdesk:device-binding-changed"));
  }

  private async bindManagedDeviceIfLoggedIn(): Promise<void> {
    if (this.deviceInfo?.access_token === SERVICE_MANAGED_TOKEN && this.getUserAccessToken()) {
      await this.bindDevice("");
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

  private async refreshStoredLocalDeviceInfo(
    stored: StoredDeviceInfo
  ): Promise<StoredDeviceInfo> {
    const hardwareInfo = await this.getHardwareInfo();
    const hostname = hardwareInfo.hostname.trim();
    if (!hostname || !this.shouldReplaceStoredLocalDeviceName(stored.device_name)) {
      return stored;
    }

    const refreshed: StoredDeviceInfo = {
      ...stored,
      device_name: hostname,
      motherboard_serial: stored.motherboard_serial || hardwareInfo.motherboard_serial,
    };
    this.saveDeviceInfo(refreshed);
    return refreshed;
  }

  private shouldReplaceStoredLocalDeviceName(deviceName: string): boolean {
    const value = deviceName.trim();
    return (
      value.length === 0 ||
      value === "开发测试机" ||
      value === "开发服务器" ||
      value === "Rdesk LAN Device" ||
      value === "This device"
    );
  }

  /**
   * 获取硬件信息（通过 Tauri）
   */
  private async getHardwareInfo(): Promise<HardwareInfo> {
    // 检查 Tauri 环境是否可用
    const tauri = typeof window !== "undefined" ? window.__TAURI__ : undefined;
    const isTauriAvailable = typeof tauri?.invoke === "function";

    if (!isTauriAvailable) throw new Error("仅桌面客户端可以读取设备硬件信息");
    return tauri.invoke<HardwareInfo>("get_hardware_info");
  }

  /**
   * 注册设备到服务器
   */
  private async registerDevice(
    hardwareInfo: HardwareInfo,
    credentials: { enrollmentToken?: string; deviceToken?: string },
    deviceName?: string
  ): Promise<DeviceRegistrationResponse> {
    const result = await registerDeviceCommand({
      motherboardSerial: hardwareInfo.motherboard_serial,
      hostname: hardwareInfo.hostname,
      osVersion: hardwareInfo.os_version,
      cpuInfo: JSON.stringify(hardwareInfo.cpu_info),
      totalMemoryMb: hardwareInfo.total_memory_mb,
      gpuInfo: JSON.stringify(hardwareInfo.gpu_info),
      deviceName: deviceName || hardwareInfo.hostname,
      apiBase: API_BASE,
      ...credentials,
    });
    if (!result.ok) throw new Error(result.error.message);
    return result.value;
  }

  /**
   * 获取本地存储的设备信息
   */
  private getStoredDeviceInfo(): StoredDeviceInfo | null {
    try {
      const stored = localStorage.getItem(DEVICE_INFO_KEY);
      if (stored) {
        const info = JSON.parse(stored) as StoredDeviceInfo;
        if (info.motherboard_serial?.startsWith("MOCK-") || info.access_token?.startsWith("mock-")) {
          this.clearStoredDeviceInfo();
          return null;
        }
        return info;
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
    const token = this.deviceInfo?.access_token;
    return token === SERVICE_MANAGED_TOKEN ? null : token ?? null;
  }

  private async syncWithLocalService(info: StoredDeviceInfo): Promise<void> {
    if (!isTauriRuntime() || info.access_token === SERVICE_MANAGED_TOKEN) return;
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
    if (!this.deviceInfo && this.initPromise) await this.initPromise;
    if (!this.deviceInfo) {
      return { success: false, message: "登记设备后将自动绑定当前登录账户" };
    }
    if (this.deviceInfo.access_token === SERVICE_MANAGED_TOKEN || this.shouldUseServiceManagedRegistration()) {
      const userToken = this.getUserAccessToken();
      if (!userToken) return { success: false, message: "请先登录再绑定本机设备" };
      try {
        const result = await ipcBindPublicDevice(userToken);
        const message = result.ok ? "本机设备已绑定当前账户" : result.error.message;
        this.recordBindingError(result.ok ? null : message);
        return { success: result.ok, message };
      } catch {
        const message = "无法绑定本机设备，请确认后台服务已启动后重试";
        this.recordBindingError(message);
        return { success: false, message };
      }
    }

    try {
      const userToken = this.getUserAccessToken();
      const response = await fetch(`${API_BASE}/devices/auto-bind`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(userToken ? { "Authorization": `Bearer ${userToken}` } : {}),
          "X-Rdesk-Device-Authorization": `Bearer ${this.deviceInfo.access_token}`,
        },
        body: JSON.stringify({
          device_id: this.deviceInfo.device_id,
          user_id: userId,
        }),
      });

      if (!response.ok) {
        return { success: false, message: "设备绑定失败，请重新登录后重试" };
      }

      const data = await response.json();
      return data;
    } catch {
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
  async unbindDevice(userId: string, deviceId?: string): Promise<boolean> {
    if (!this.deviceInfo && this.initPromise) await this.initPromise;
    const targetDeviceId = deviceId ?? this.deviceInfo?.device_id;
    if (!targetDeviceId) {
      console.warn("[DeviceService] 设备未注册，无法解绑");
      return false;
    }
    if (this.deviceInfo?.access_token === SERVICE_MANAGED_TOKEN || this.shouldUseServiceManagedRegistration()) {
      if (targetDeviceId !== this.deviceInfo?.device_id) {
        this.recordBindingError("后台服务只允许解绑当前本机设备");
        return false;
      }
      const userToken = this.getUserAccessToken();
      if (!userToken) return false;
      try {
        const result = await ipcUnbindPublicDevice(userToken);
        this.recordBindingError(result.ok ? null : result.error.message);
        return result.ok;
      } catch {
        this.recordBindingError("无法解绑本机设备，请确认后台服务已启动后重试");
        return false;
      }
    }

    try {
      const userToken = this.getUserAccessToken();
      const response = await fetch(`${API_BASE}/devices/unbind`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          ...(userToken ? { "Authorization": `Bearer ${userToken}` } : {}),
          ...(targetDeviceId === this.deviceInfo?.device_id ? { "X-Rdesk-Device-Authorization": `Bearer ${this.deviceInfo.access_token}` } : {}),
        },
        body: JSON.stringify({
          device_id: targetDeviceId,
          user_id: userId,
        }),
      });

      if (!response.ok) {
        return false;
      }

      return true;
    } catch {
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
    return this.initialize();
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
  const [registrationError, setRegistrationError] = useState<string | null>(null);

  const updateInfo = (info: StoredDeviceInfo | null) => {
    setDeviceId(info?.device_id ?? null);
    setDeviceName(info?.device_name ?? null);
    setIsRegistered(Boolean(info));
    setRegistrationError(deviceService.getRegistrationError());
  };

  useEffect(() => {
    let cancelled = false;
    let timer: number | undefined;
    let attempts = 0;
    const deadline = Date.now() + 60_000;
    const run = async () => {
      if (cancelled) return;
      if (attempts > 0 && Date.now() >= deadline) {
        setRegistrationError("自动登记暂未完成，请检查网络后刷新登记状态重试；无需登录账号");
        return;
      }
      attempts += 1;
      const info = await deviceService.initialize();
      if (cancelled) return;
      updateInfo(info);
      setIsLoading(false);
      if (!info && deviceService.isServiceManagedRegistration()) {
        if (attempts < 30 && Date.now() < deadline) {
          timer = window.setTimeout(() => { void run(); }, 2000);
        } else {
          setRegistrationError("自动登记暂未完成，请检查网络后刷新登记状态重试；无需登录账号");
        }
      }
    };
    const onBindingChanged = () => {
      const info = deviceService.getDeviceInfo();
      if (info && timer !== undefined) window.clearTimeout(timer);
      updateInfo(info);
    };
    window.addEventListener("rdesk:device-binding-changed", onBindingChanged);
    void run();
    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
      window.removeEventListener("rdesk:device-binding-changed", onBindingChanged);
    };
  }, []);

  return {
    deviceId,
    deviceName,
    isRegistered,
    isLoading,
    registrationError,
    refresh: () => updateInfo(deviceService.getDeviceInfo()),
    getAccessToken: () => deviceService.getAccessToken(),
    serviceManagedRegistration: deviceService.isServiceManagedRegistration(),
    reregister: async () => {
      const info = await deviceService.reregister();
      updateInfo(info);
      return info;
    },
  };
}
